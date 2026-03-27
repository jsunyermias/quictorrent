use std::collections::HashSet;
use std::sync::Arc;

use rand::seq::SliceRandom;
use tokio::sync::Mutex;
use tokio::time::interval;
use tracing::{info, warn};

use bitturbulence_tracker::{AnnounceEvent, AnnounceRequest, TrackerClient};
use bitturbulence_transport::QuicEndpoint;

use super::context::FlowCtx;
use super::peer::run_peer;
use super::ANNOUNCE_INTERVAL;

/// Bucle de announce para un tier de trackers (BEP 12).
///
/// ## Comportamiento
///
/// 1. Shufflea el tier al inicio (BEP 12: orden aleatorio dentro del tier).
/// 2. En cada intervalo de announce, intenta los trackers en orden hasta que
///    uno responde con éxito.
/// 3. Si un tracker responde con éxito, se promueve al inicio del tier para
///    que sea el primero en intentarse en el siguiente ciclo.
/// 4. Si todos los trackers del tier fallan, se registra un warning y se
///    espera al siguiente intervalo.
pub async fn tier_announce_loop(
    tier: Vec<String>,
    ctx: Arc<FlowCtx>,
    endpoint: Arc<QuicEndpoint>,
    peer_id: [u8; 32],
    listen_port: u16,
    advertise_addr: Option<String>,
    known_peers: Arc<Mutex<HashSet<String>>>,
) {
    if tier.is_empty() {
        return;
    }

    // Shuffle inicial del tier (BEP 12).
    let mut tier = tier;
    tier.shuffle(&mut rand::thread_rng());

    let listen_addr = match advertise_addr {
        Some(ref ip) => format!("{ip}:{listen_port}"),
        None => format!("127.0.0.1:{listen_port}"),
    };
    let mut timer = interval(ANNOUNCE_INTERVAL);
    let mut event = AnnounceEvent::Started;

    loop {
        timer.tick().await;

        let downloaded = ctx.downloaded.load(std::sync::atomic::Ordering::Relaxed);
        let left = ctx.meta.total_size().saturating_sub(downloaded);

        // Intentar trackers en orden hasta que uno responda.
        let mut success_idx: Option<usize> = None;
        for (idx, tracker_url) in tier.iter().enumerate() {
            let client = match TrackerClient::new(tracker_url, None) {
                Ok(c) => c,
                Err(e) => {
                    warn!(tracker = %tracker_url, "tracker client init: {e}");
                    continue;
                }
            };

            let req = AnnounceRequest::new(
                &ctx.meta.info_hash,
                &peer_id,
                &listen_addr,
                event,
                0,
                downloaded,
                left,
            );

            match client.announce(req).await {
                Ok(resp) => {
                    info!(tracker = %tracker_url, peers = resp.peers.len(), "announce ok");
                    connect_new_peers(resp.peers, &ctx, &endpoint, peer_id, &known_peers).await;
                    success_idx = Some(idx);
                    break;
                }
                Err(e) => {
                    warn!(tracker = %tracker_url, "announce failed: {e}");
                }
            }
        }

        match success_idx {
            None => warn!(tier_size = tier.len(), "all trackers in tier failed"),
            Some(0) => {} // Ya está en primera posición, nada que hacer.
            Some(idx) => {
                // Promover al frente el tracker que respondió (BEP 12).
                let url = tier.remove(idx);
                tier.insert(0, url);
            }
        }

        event = if ctx.is_complete().await {
            AnnounceEvent::Completed
        } else {
            AnnounceEvent::Update
        };
    }
}

/// Conecta a los peers nuevos devueltos por el tracker.
async fn connect_new_peers(
    peers: Vec<bitturbulence_tracker::PeerInfo>,
    ctx: &Arc<FlowCtx>,
    endpoint: &Arc<QuicEndpoint>,
    peer_id: [u8; 32],
    known_peers: &Arc<Mutex<HashSet<String>>>,
) {
    for peer_info in peers {
        let addr_str = peer_info.addr.clone();
        let is_new = known_peers.lock().await.insert(addr_str.clone());
        if !is_new {
            continue;
        }

        let ep = endpoint.clone();
        let ctx2 = ctx.clone();
        tokio::spawn(async move {
            match addr_str.parse::<std::net::SocketAddr>() {
                Ok(addr) => match ep.connect(addr).await {
                    Ok(conn) => run_peer(conn, ctx2, peer_id, true).await,
                    Err(e) => warn!("connect {addr}: {e}"),
                },
                Err(_) => warn!("invalid peer addr: {addr_str}"),
            }
        });
    }
}
