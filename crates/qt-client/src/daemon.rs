use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use rand::RngCore;
use tokio::sync::Mutex;
use tokio::time::{interval, timeout};
use tracing::{debug, info, warn};

use qt_pieces::{PiecePicker, TorrentStore, verify_piece};
use qt_protocol::{AuthPayload, Message, Metainfo, PROTOCOL_VERSION};
use qt_tracker::{AnnounceEvent, AnnounceRequest, TrackerClient};
use qt_transport::{PeerConnection, QuicEndpoint};

use crate::config::Config;
use crate::state::{ClientState, DownloadState};

const HELLO_TIMEOUT:       Duration = Duration::from_secs(10);
const REQUEST_INTERVAL:    Duration = Duration::from_millis(500);
const KEEPALIVE_INTERVAL:  Duration = Duration::from_secs(60);
const ANNOUNCE_INTERVAL:   Duration = Duration::from_secs(300);
const STATE_SAVE_INTERVAL: Duration = Duration::from_secs(30);

// ── Estado compartido de un torrent ───────────────────────────────────────────

pub struct TorrentCtx {
    pub meta:       Metainfo,
    pub store:      TorrentStore,
    /// Un PiecePicker por archivo.
    pickers:        Vec<Mutex<PiecePicker>>,
    /// have[fi][pi] = pieza verificada y completa.
    have:           Mutex<Vec<Vec<bool>>>,
    /// Bytes descargados y verificados.
    pub downloaded: AtomicU64,
    /// Peers conectados actualmente.
    pub peer_count: AtomicUsize,
}

impl TorrentCtx {
    pub async fn new(meta: Metainfo, save_path: &Path, seeding: bool) -> Result<Arc<Self>> {
        let store = TorrentStore::open(save_path, &meta).await
            .context("opening torrent store")?;

        let mut pickers   = Vec::with_capacity(meta.files.len());
        let mut have_init = Vec::with_capacity(meta.files.len());

        for fi in 0..meta.files.len() {
            let num = store.file(fi).num_pieces() as usize;
            let mut picker = PiecePicker::new(num);
            let file_have = if seeding {
                for pi in 0..num { picker.mark_complete(pi); }
                vec![true; num]
            } else {
                vec![false; num]
            };
            pickers.push(Mutex::new(picker));
            have_init.push(file_have);
        }

        Ok(Arc::new(Self {
            meta,
            store,
            pickers,
            have: Mutex::new(have_init),
            downloaded: AtomicU64::new(0),
            peer_count: AtomicUsize::new(0),
        }))
    }

    /// Bitfield que anunciamos al peer para el archivo `fi`.
    pub async fn our_bitfield(&self, fi: usize) -> Vec<bool> {
        self.have.lock().await[fi].clone()
    }

    /// Escribe un bloque; si cubre la pieza completa, la verifica.
    /// Devuelve `true` si el bloque fue aceptado.
    pub async fn receive_block(
        &self,
        fi:    usize,
        piece: u32,
        begin: u32,
        data:  &[u8],
    ) -> bool {
        if let Err(e) = self.store.write_block(fi, piece, begin, data).await {
            warn!(fi, piece, "write_block: {e}");
            return false;
        }

        // Verificar cuando recibimos la pieza completa en un solo bloque.
        let piece_len = self.store.file(fi).piece_len(piece) as usize;
        if begin == 0 && data.len() == piece_len {
            let expected = &self.meta.files[fi].piece_hashes[piece as usize];
            if verify_piece(data, expected) {
                {
                    let mut have = self.have.lock().await;
                    have[fi][piece as usize] = true;
                }
                self.pickers[fi].lock().await.mark_complete(piece as usize);
                self.downloaded.fetch_add(data.len() as u64, Ordering::Relaxed);
                info!(fi, piece, "piece verified ok");
                true
            } else {
                warn!(fi, piece, "hash mismatch — rejected");
                self.pickers[fi].lock().await.mark_failed(piece as usize);
                false
            }
        } else {
            // Bloque parcial: aceptado pero no verificado todavía.
            true
        }
    }

    pub async fn is_complete(&self) -> bool {
        for picker in &self.pickers {
            if !picker.lock().await.is_complete() { return false; }
        }
        true
    }
}

// ── Seguimiento de disponibilidad del peer ────────────────────────────────────

#[derive(Clone)]
enum PeerAvail {
    Unknown,
    HaveAll,
    HaveNone,
    Bitmap(Vec<bool>),
}

impl PeerAvail {
    fn as_bitfield(&self, num: usize) -> Vec<bool> {
        match self {
            Self::Unknown | Self::HaveNone => vec![false; num],
            Self::HaveAll                  => vec![true;  num],
            Self::Bitmap(v) => {
                let mut out = v.clone();
                out.resize(num, false);
                out
            }
        }
    }
}

// ── Helpers de bitmap ─────────────────────────────────────────────────────────

/// Convierte un bitfield booleano al formato compacto (bit 7 del byte 0 = pieza 0).
fn bitfield_to_bytes(bits: &[bool]) -> Vec<u8> {
    let mut bytes = vec![0u8; bits.len().div_ceil(8)];
    for (i, &has) in bits.iter().enumerate() {
        if has { bytes[i / 8] |= 0x80 >> (i % 8); }
    }
    bytes
}

/// Convierte bytes compactos a bitfield booleano.
fn bytes_to_bitfield(bytes: &[u8], num: usize) -> Vec<bool> {
    (0..num)
        .map(|i| bytes.get(i / 8).is_some_and(|b| (b >> (7 - (i % 8))) & 1 == 1))
        .collect()
}

// ── Bucle de un peer ──────────────────────────────────────────────────────────

pub async fn run_peer(
    conn:      PeerConnection,
    ctx:       Arc<TorrentCtx>,
    peer_id:   [u8; 32],
    outbound:  bool,
) {
    let addr = conn.remote_addr();
    ctx.peer_count.fetch_add(1, Ordering::Relaxed);

    if let Err(e) = peer_loop(conn, &ctx, &peer_id, outbound).await {
        debug!(%addr, "peer error: {e}");
    } else {
        info!(%addr, "peer disconnected cleanly");
    }

    ctx.peer_count.fetch_sub(1, Ordering::Relaxed);
}

async fn peer_loop(
    conn:     PeerConnection,
    ctx:      &TorrentCtx,
    peer_id:  &[u8; 32],
    outbound: bool,
) -> Result<()> {
    // ── Handshake ──────────────────────────────────────────────────────────
    let (mut writer, mut reader) = if outbound {
        let (mut w, mut r) = conn.open_bidi_stream().await?;

        let hello = Message::Hello {
            version:   PROTOCOL_VERSION,
            peer_id:   *peer_id,
            info_hash: ctx.meta.info_hash,
            auth:      AuthPayload::None,
        };
        timeout(HELLO_TIMEOUT, w.send(hello)).await
            .map_err(|_| anyhow!("hello send timeout"))?
            .context("sending hello")?;

        let ack = timeout(HELLO_TIMEOUT, r.next()).await
            .map_err(|_| anyhow!("hello ack timeout"))?
            .ok_or_else(|| anyhow!("disconnected during hello"))??;

        match ack {
            Message::HelloAck { accepted: true,  .. } => {}
            Message::HelloAck { accepted: false, reason, .. } =>
                return Err(anyhow!("hello rejected: {}", reason.unwrap_or_default())),
            _ => return Err(anyhow!("expected HelloAck")),
        }
        // El iniciador abre el stream de datos.
        conn.open_bidi_stream().await?
    } else {
        // El inbound peer ya hizo el hello en el accept loop.
        // Esperamos el stream de datos que abrirá el iniciador.
        conn.accept_bidi_stream().await?
    };

    // ── Anunciar nuestra disponibilidad ────────────────────────────────────
    let num_files = ctx.meta.files.len();
    for fi in 0..num_files {
        let bits = ctx.our_bitfield(fi).await;
        let msg = if bits.iter().all(|&h| h) {
            Message::HaveAll  { file_index: fi as u16 }
        } else if bits.iter().all(|&h| !h) {
            Message::HaveNone { file_index: fi as u16 }
        } else {
            Message::HaveBitmap {
                file_index: fi as u16,
                bitmap:     Bytes::from(bitfield_to_bytes(&bits)),
            }
        };
        writer.send(msg).await?;
    }

    // ── Estado del peer ────────────────────────────────────────────────────
    let mut peer_avail: Vec<PeerAvail> = vec![PeerAvail::Unknown; num_files];
    // Pieza en vuelo: (fi, piece_index)
    let mut in_flight: Option<(usize, u32)> = None;

    let mut req_timer = interval(REQUEST_INTERVAL);
    let mut ka_timer  = interval(KEEPALIVE_INTERVAL);
    req_timer.tick().await;  // consumir tick inicial
    ka_timer.tick().await;

    // ── Bucle principal ────────────────────────────────────────────────────
    loop {
        tokio::select! {
            msg_opt = reader.next() => {
                let msg = match msg_opt {
                    Some(Ok(m))  => m,
                    Some(Err(e)) => return Err(e.into()),
                    None         => return Ok(()),
                };

                match msg {
                    Message::KeepAlive => {}

                    Message::HaveAll { file_index: fi } => {
                        let fi = fi as usize;
                        if fi < num_files {
                            let num = ctx.store.file(fi).num_pieces() as usize;
                            let old = peer_avail[fi].as_bitfield(num);
                            ctx.pickers[fi].lock().await.remove_peer_bitfield(&old);
                            ctx.pickers[fi].lock().await.add_peer_bitfield(&vec![true; num]);
                            peer_avail[fi] = PeerAvail::HaveAll;
                        }
                    }

                    Message::HaveNone { file_index: fi } => {
                        let fi = fi as usize;
                        if fi < num_files {
                            let num = ctx.store.file(fi).num_pieces() as usize;
                            let old = peer_avail[fi].as_bitfield(num);
                            ctx.pickers[fi].lock().await.remove_peer_bitfield(&old);
                            peer_avail[fi] = PeerAvail::HaveNone;
                        }
                    }

                    Message::HavePiece { file_index: fi, piece_index: pi } => {
                        let fi = fi as usize;
                        if fi < num_files {
                            ctx.pickers[fi].lock().await.add_peer_have(pi as usize);
                            match &mut peer_avail[fi] {
                                PeerAvail::Bitmap(v) => {
                                    if (pi as usize) < v.len() { v[pi as usize] = true; }
                                }
                                other => {
                                    let num = ctx.store.file(fi).num_pieces() as usize;
                                    let mut b = other.as_bitfield(num);
                                    if (pi as usize) < num { b[pi as usize] = true; }
                                    *other = PeerAvail::Bitmap(b);
                                }
                            }
                        }
                    }

                    Message::HaveBitmap { file_index: fi, bitmap } => {
                        let fi = fi as usize;
                        if fi < num_files {
                            let num = ctx.store.file(fi).num_pieces() as usize;
                            let old = peer_avail[fi].as_bitfield(num);
                            let new_bits = bytes_to_bitfield(&bitmap, num);
                            ctx.pickers[fi].lock().await.remove_peer_bitfield(&old);
                            ctx.pickers[fi].lock().await.add_peer_bitfield(&new_bits);
                            peer_avail[fi] = PeerAvail::Bitmap(new_bits);
                        }
                    }

                    Message::Piece { file_index: fi, piece_index: pi, begin, data } => {
                        let fi = fi as usize;
                        ctx.receive_block(fi, pi, begin, &data).await;
                        if in_flight == Some((fi, pi)) {
                            in_flight = None;
                        }
                    }

                    Message::Request { file_index: fi, piece_index: pi, begin, length } => {
                        let fi_u = fi as usize;
                        let have = ctx.our_bitfield(fi_u).await;
                        if have.get(pi as usize).copied().unwrap_or(false) {
                            match ctx.store.read_block(fi_u, pi, begin, length).await {
                                Ok(data) => {
                                    writer.send(Message::Piece {
                                        file_index: fi, piece_index: pi, begin,
                                        data: Bytes::from(data),
                                    }).await?;
                                }
                                Err(e) => {
                                    warn!(fi, pi, "read_block: {e}");
                                    writer.send(Message::Reject {
                                        file_index: fi, piece_index: pi, begin, length,
                                    }).await?;
                                }
                            }
                        } else {
                            writer.send(Message::Reject {
                                file_index: fi, piece_index: pi, begin, length,
                            }).await?;
                        }
                    }

                    Message::Reject { .. } => {
                        in_flight = None;
                    }

                    Message::Bye { reason } => {
                        info!("peer bye: {reason}");
                        return Ok(());
                    }

                    _ => {}
                }
            }

            _ = req_timer.tick() => {
                if in_flight.is_none() {
                    // Buscar una pieza para solicitar (rarest-first por archivo).
                    'pick: for (fi, avail) in peer_avail.iter().enumerate() {
                        let num       = ctx.store.file(fi).num_pieces() as usize;
                        let peer_bits = avail.as_bitfield(num);
                        if peer_bits.iter().all(|&h| !h) { continue; }

                        if let Some(pi) = ctx.pickers[fi].lock().await.pick(&peer_bits) {
                            let length = ctx.store.file(fi).piece_len(pi as u32) as u32;
                            writer.send(Message::Request {
                                file_index:  fi as u16,
                                piece_index: pi as u32,
                                begin:       0,
                                length,
                            }).await?;
                            in_flight = Some((fi, pi as u32));
                            break 'pick;
                        }
                    }
                }
            }

            _ = ka_timer.tick() => {
                writer.send(Message::KeepAlive).await?;
            }
        }
    }
}

// ── Accept loop (QUIC inbound) ────────────────────────────────────────────────

async fn accept_loop(
    endpoint: Arc<QuicEndpoint>,
    torrents: Arc<HashMap<[u8; 32], Arc<TorrentCtx>>>,
    peer_id:  [u8; 32],
) {
    loop {
        match endpoint.accept().await {
            None => { info!("QUIC endpoint closed"); break; }
            Some(Err(e)) => { warn!("accept error: {e}"); }
            Some(Ok(conn)) => {
                let torrents = torrents.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_inbound(conn, torrents, peer_id).await {
                        debug!("inbound handshake: {e}");
                    }
                });
            }
        }
    }
}

async fn handle_inbound(
    conn:     PeerConnection,
    torrents: Arc<HashMap<[u8; 32], Arc<TorrentCtx>>>,
    peer_id:  [u8; 32],
) -> Result<()> {
    // Aceptar y gestionar el stream del hello.
    let (mut w, mut r) = conn.accept_bidi_stream().await?;

    let msg = timeout(HELLO_TIMEOUT, r.next()).await
        .map_err(|_| anyhow!("hello timeout"))?
        .ok_or_else(|| anyhow!("disconnected during hello"))??;

    let info_hash = match msg {
        Message::Hello { info_hash, .. } => info_hash,
        _ => return Err(anyhow!("expected Hello")),
    };

    match torrents.get(&info_hash) {
        Some(ctx) => {
            let ack = Message::HelloAck { peer_id, accepted: true, reason: None };
            timeout(HELLO_TIMEOUT, w.send(ack)).await
                .map_err(|_| anyhow!("ack timeout"))??;
            drop(w); drop(r);

            run_peer(conn, ctx.clone(), peer_id, false).await;
            Ok(())
        }
        None => {
            let ack = Message::HelloAck {
                peer_id,
                accepted: false,
                reason:   Some("torrent not found".into()),
            };
            let _ = timeout(HELLO_TIMEOUT, w.send(ack)).await;
            Ok(())
        }
    }
}

// ── Tracker announce loop ─────────────────────────────────────────────────────

async fn tracker_loop(
    tracker_url:  String,
    ctx:          Arc<TorrentCtx>,
    endpoint:     Arc<QuicEndpoint>,
    peer_id:      [u8; 32],
    listen_port:  u16,
    known_peers:  Arc<Mutex<HashSet<String>>>,
) {
    let client = match TrackerClient::new(&tracker_url, None) {
        Ok(c)  => c,
        Err(e) => { warn!("tracker client: {e}"); return; }
    };

    let listen_addr = format!("0.0.0.0:{listen_port}");
    let mut timer   = interval(ANNOUNCE_INTERVAL);
    let mut event   = AnnounceEvent::Started;

    loop {
        timer.tick().await;

        let downloaded = ctx.downloaded.load(Ordering::Relaxed);
        let left       = ctx.meta.total_size().saturating_sub(downloaded);

        let req = AnnounceRequest::new(
            &ctx.meta.info_hash, &peer_id, &listen_addr,
            event, 0, downloaded, left,
        );

        match client.announce(req).await {
            Ok(resp) => {
                info!(tracker = %tracker_url, peers = resp.peers.len(), "announce ok");

                for peer_info in resp.peers {
                    let addr_str = peer_info.addr.clone();
                    let is_new = known_peers.lock().await.insert(addr_str.clone());
                    if !is_new { continue; }

                    let ep   = endpoint.clone();
                    let ctx2 = ctx.clone();
                    tokio::spawn(async move {
                        match addr_str.parse::<std::net::SocketAddr>() {
                            Ok(addr) => match ep.connect(addr).await {
                                Ok(conn) => run_peer(conn, ctx2, peer_id, true).await,
                                Err(e)   => debug!("connect {addr}: {e}"),
                            },
                            Err(_) => warn!("invalid peer addr: {addr_str}"),
                        }
                    });
                }

                event = if ctx.is_complete().await {
                    AnnounceEvent::Completed
                } else {
                    AnnounceEvent::Update
                };
            }
            Err(e) => warn!("announce to {tracker_url}: {e}"),
        }
    }
}

// ── State persistence loop ────────────────────────────────────────────────────

async fn state_save_loop(
    state_path:  std::path::PathBuf,
    torrent_ids: Vec<(String, Arc<TorrentCtx>)>,
) {
    let mut timer = interval(STATE_SAVE_INTERVAL);
    timer.tick().await;  // omitir primera tick inmediata

    loop {
        timer.tick().await;

        let mut state = match ClientState::load(&state_path) {
            Ok(s)  => s,
            Err(e) => { warn!("load state: {e}"); continue; }
        };

        for (id, ctx) in &torrent_ids {
            if let Some(entry) = state.get_mut(id) {
                entry.downloaded = ctx.downloaded.load(Ordering::Relaxed);
                entry.peers      = ctx.peer_count.load(Ordering::Relaxed);
                if ctx.is_complete().await && entry.state == DownloadState::Downloading {
                    entry.state = DownloadState::Seeding;
                    info!("[{id}] {} — completed!", entry.name);
                }
            }
        }

        if let Err(e) = state.save(&state_path) {
            warn!("save state: {e}");
        }
    }
}

// ── Punto de entrada del daemon ───────────────────────────────────────────────

pub async fn run_daemon(config: &Config, state_path: &Path) -> Result<()> {
    // Peer ID aleatorio para esta sesión.
    let mut our_peer_id = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut our_peer_id);

    // Endpoint QUIC.
    let bind_addr: std::net::SocketAddr = format!("0.0.0.0:{}", config.listen_port)
        .parse()
        .context("parsing listen addr")?;
    let endpoint = Arc::new(QuicEndpoint::bind(bind_addr)?);

    // Cargar torrents activos desde el estado.
    let state = ClientState::load(state_path)?;
    let mut torrents: HashMap<[u8; 32], Arc<TorrentCtx>> = HashMap::new();
    let mut torrent_ids: Vec<(String, Arc<TorrentCtx>)>  = Vec::new();

    for entry in state.torrents.values() {
        let active = matches!(entry.state, DownloadState::Downloading | DownloadState::Seeding);
        if !active { continue; }

        if entry.metainfo_path.as_os_str().is_empty() {
            warn!("[{}] no metainfo_path — re-add the torrent", entry.id);
            continue;
        }

        let meta_bytes = match std::fs::read(&entry.metainfo_path) {
            Ok(b)  => b,
            Err(e) => { warn!("[{}] read metainfo: {e}", entry.id); continue; }
        };
        let meta: Metainfo = match serde_json::from_slice(&meta_bytes) {
            Ok(m)  => m,
            Err(e) => { warn!("[{}] parse metainfo: {e}", entry.id); continue; }
        };

        let seeding = entry.state == DownloadState::Seeding;
        let ctx = match TorrentCtx::new(meta, &entry.save_path, seeding).await {
            Ok(c)  => c,
            Err(e) => { warn!("[{}] init ctx: {e}", entry.id); continue; }
        };

        info!("[{}] {} — {}", entry.id, entry.name,
            if seeding { "seeding" } else { "downloading" });

        torrents.insert(ctx.meta.info_hash, ctx.clone());
        torrent_ids.push((entry.id.clone(), ctx));
    }

    let torrents = Arc::new(torrents);

    // Lanzar announce loops por torrent y tracker.
    for (_, ctx) in &torrent_ids {
        let known_peers = Arc::new(Mutex::new(HashSet::<String>::new()));
        for tracker_url in &ctx.meta.trackers {
            tokio::spawn(tracker_loop(
                tracker_url.clone(),
                ctx.clone(),
                endpoint.clone(),
                our_peer_id,
                config.listen_port,
                known_peers.clone(),
            ));
        }
    }

    // Lanzar el loop de persistencia de estado.
    if !torrent_ids.is_empty() {
        tokio::spawn(state_save_loop(state_path.to_path_buf(), torrent_ids));
    }

    // Bucle de aceptación de conexiones QUIC (bloquea hasta que el endpoint cierre).
    accept_loop(endpoint, torrents, our_peer_id).await;

    Ok(())
}
