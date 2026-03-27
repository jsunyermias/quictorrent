use std::path::{Path, PathBuf};
use anyhow::{anyhow, Context, Result};

use bitturbulence_dht::{DhtConfig, DhtNode};
use bitturbulence_pieces::piece_root;
use bitturbulence_protocol::{FileEntry, Metainfo, Priority, piece_length_for_size};
use bitturbulence_tracker::server::{ServerConfig, TrackerServer};
use bitturbulence_tracker::PeerStore;

use crate::config::Config;
use crate::ipc_proto::{IpcRequest, IpcResponse};
use crate::state::{ClientState, DownloadState, FlowEntry};

/// Añade un BitFlow al cliente.
pub async fn cmd_add(
    meta_path: &Path,
    save_path: Option<PathBuf>,
    _priority: Priority,
    state_path: &Path,
    config: &Config,
) -> Result<()> {
    let save_path = save_path.unwrap_or_else(|| config.download_dir.clone());
    std::fs::create_dir_all(&save_path)?;

    // Leer el metainfo
    let meta_bytes = std::fs::read(meta_path)
        .with_context(|| format!("cannot read {:?}", meta_path))?;
    let meta: bitturbulence_protocol::Metainfo = serde_json::from_slice(&meta_bytes)
        .with_context(|| "invalid metainfo file")?;

    let info_hash_hex = hex::encode(meta.info_hash);
    let id = info_hash_hex[..8].to_string();

    let mut state = ClientState::load(state_path)?;

    if state.get(&id).is_some() {
        println!("flow {} already added", id);
        return Ok(());
    }

    // Guardar copia del .bitflow en el directorio de configuración.
    let torrents_dir = config.state_file.parent()
        .unwrap_or(Path::new("."))
        .join("flows");
    std::fs::create_dir_all(&torrents_dir)?;
    let meta_dest = torrents_dir.join(format!("{}.bitflow", id));
    std::fs::copy(meta_path, &meta_dest)
        .with_context(|| format!("copying metainfo to {:?}", meta_dest))?;

    let total_size = meta.total_size();
    let entry = FlowEntry {
        id:             id.clone(),
        info_hash:      info_hash_hex,
        name:           meta.name.clone(),
        save_path,
        metainfo_path:  meta_dest,
        state:          DownloadState::Queued,
        downloaded:     0,
        total_size,
        peers:          0,
    };

    state.add(entry);
    state.save(state_path)?;

    println!("added flow [{}] {} ({} bytes, {} files)",
        id, meta.name, total_size, meta.files.len());

    Ok(())
}

/// Muestra el estado de todos los BitFlows.
/// Si el daemon está activo, fusiona los datos en tiempo real vía IPC.
pub async fn cmd_status(state_path: &Path, socket_path: &Path) -> Result<()> {
    let state = ClientState::load(state_path)?;

    if state.flows.is_empty() {
        println!("no flows");
        return Ok(());
    }

    // Obtener datos en tiempo real del daemon (si está corriendo).
    let live = match try_ipc(socket_path, &IpcRequest::Status).await {
        Some(IpcResponse::Status { flows }) => flows,
        _ => vec![],
    };

    println!("{:<8}  {:<30}  {:>7}  {:>6}  {:>5}  STATE",
        "ID", "NAME", "SIZE", "PROG%", "PEERS");
    println!("{}", "-".repeat(72));

    let mut entries: Vec<_> = state.flows.values().collect();
    entries.sort_by(|a, b| a.id.cmp(&b.id));

    for e in entries {
        // Preferir datos en tiempo real si el daemon está activo.
        let (downloaded, peers, state_str) = live.iter()
            .find(|l| l.id == e.id)
            .map(|l| (l.downloaded, l.peers, l.state.clone()))
            .unwrap_or((e.downloaded, e.peers, e.state.to_string()));

        let total = if e.total_size > 0 { e.total_size } else { 1 };
        let pct   = downloaded as f32 / total as f32 * 100.0;
        println!("{:<8}  {:<30}  {:>7}  {:>5.1}%  {:>5}  {}",
            e.id,
            truncate(&e.name, 30),
            format_size(e.total_size),
            pct,
            peers,
            state_str,
        );
    }

    Ok(())
}

/// Inicia la descarga de un BitFlow.
pub async fn cmd_start(id: &str, state_path: &Path, socket_path: &Path) -> Result<()> {
    // Si el daemon está activo, la escritura pasa por él (evita carreras con state.json).
    if let Some(resp) = try_ipc(socket_path, &IpcRequest::FlowStart { id: id.into() }).await {
        match resp {
            IpcResponse::Ok => {}
            IpcResponse::Error { message } => return Err(anyhow!("{}", message)),
            _ => {}
        }
        // Reconfirmar con state.json para el mensaje de feedback.
        let state = ClientState::load(state_path)?;
        if let Some(e) = state.get(id) {
            println!("[{}] {} → downloading", e.id, e.name);
        }
        return Ok(());
    }

    // Fallback: sin daemon.
    let mut state = ClientState::load(state_path)?;
    let entry = state.get_mut(id)
        .ok_or_else(|| anyhow!("flow '{}' not found", id))?;

    match &entry.state {
        DownloadState::Seeding     => println!("[{}] already seeding", id),
        DownloadState::Downloading => println!("[{}] already downloading", id),
        _ => {
            entry.state = DownloadState::Downloading;
            println!("[{}] {} → downloading", id, entry.name);
        }
    }
    state.save(state_path)?;
    Ok(())
}

/// Pausa la descarga de un BitFlow.
pub async fn cmd_pause(id: &str, state_path: &Path, socket_path: &Path) -> Result<()> {
    if let Some(resp) = try_ipc(socket_path, &IpcRequest::FlowPause { id: id.into() }).await {
        match resp {
            IpcResponse::Ok => {}
            IpcResponse::Error { message } => return Err(anyhow!("{}", message)),
            _ => {}
        }
        println!("[{}] → paused", id);
        return Ok(());
    }

    let mut state = ClientState::load(state_path)?;
    let entry = state.get_mut(id)
        .ok_or_else(|| anyhow!("flow '{}' not found", id))?;
    entry.state = DownloadState::Paused;
    println!("[{}] {} → paused", id, entry.name);
    state.save(state_path)?;
    Ok(())
}

/// Detiene y elimina un BitFlow de la lista.
pub async fn cmd_stop(id: &str, state_path: &Path, socket_path: &Path) -> Result<()> {
    if let Some(resp) = try_ipc(socket_path, &IpcRequest::FlowStop { id: id.into() }).await {
        match resp {
            IpcResponse::Ok => {}
            IpcResponse::Error { message } => return Err(anyhow!("{}", message)),
            _ => {}
        }
        println!("[{}] removed", id);
        return Ok(());
    }

    let mut state = ClientState::load(state_path)?;
    let name = state.get(id)
        .ok_or_else(|| anyhow!("flow '{}' not found", id))?
        .name.clone();
    state.flows.retain(|k, v| k != id && !v.info_hash.starts_with(id));
    state.save(state_path)?;
    println!("[{}] {} removed", id, name);
    Ok(())
}

/// Muestra los peers de un BitFlow (datos en tiempo real si el daemon está activo).
pub async fn cmd_peers(id: &str, state_path: &Path, socket_path: &Path) -> Result<()> {
    let state = ClientState::load(state_path)?;
    let entry = state.get(id)
        .ok_or_else(|| anyhow!("flow '{}' not found", id))?;

    let peers = match try_ipc(socket_path, &IpcRequest::FlowPeers { id: id.into() }).await {
        Some(IpcResponse::Peers { count }) => count,
        _ => entry.peers,
    };

    println!("flow [{}] {} — {} peer(s) connected", entry.id, entry.name, peers);
    if peers == 0 {
        println!("  (no peers)");
    }
    Ok(())
}

/// Crea un fichero .bitflow a partir de un archivo o directorio.
pub fn cmd_create(
    path:     &Path,
    name:     Option<String>,
    trackers: Vec<String>,
    comment:  Option<String>,
    priority: Priority,
    output:   Option<PathBuf>,
) -> Result<()> {
    let abs = path.canonicalize()
        .with_context(|| format!("resolving path {:?}", path))?;

    let default_name = abs.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let name = name.unwrap_or(default_name);

    let files: Vec<FileEntry> = if abs.is_file() {
        let data = std::fs::read(&abs)
            .with_context(|| format!("reading {:?}", abs))?;
        let fname = abs.file_name().unwrap_or_default()
            .to_string_lossy().into_owned();
        vec![build_file_entry(vec![fname], &data, priority)]
    } else if abs.is_dir() {
        let mut raw: Vec<(Vec<String>, Vec<u8>)> = vec![];
        collect_dir_files(&abs, &abs, &mut raw)?;
        raw.sort_by(|(a, _), (b, _)| a.cmp(b));
        raw.into_iter()
            .map(|(p, d)| build_file_entry(p, &d, priority))
            .collect()
    } else {
        return Err(anyhow!("not a file or directory: {:?}", abs));
    };

    if files.is_empty() {
        return Err(anyhow!("no files found in {:?}", abs));
    }

    let mut meta = Metainfo {
        name: name.clone(),
        info_hash: [0u8; 32],
        files,
        trackers,
        comment,
    };
    meta.info_hash = meta.compute_info_hash();

    let hash_hex = hex::encode(meta.info_hash);
    let out_dir = output.unwrap_or_else(|| {
        abs.parent().unwrap_or(Path::new(".")).to_path_buf()
    });
    std::fs::create_dir_all(&out_dir)?;
    let out_path = out_dir.join(format!("{}.bitflow", &hash_hex[..8]));

    let json = serde_json::to_string_pretty(&meta)?;
    std::fs::write(&out_path, &json)
        .with_context(|| format!("writing {:?}", out_path))?;

    println!("created:  {}", out_path.display());
    println!("name:     {}", meta.name);
    println!("hash:     {}", hash_hex);
    println!("files:    {}", meta.files.len());
    println!("size:     {}", format_size(meta.total_size()));
    println!("pieces:   {}", meta.total_pieces());

    Ok(())
}

fn build_file_entry(path: Vec<String>, data: &[u8], priority: Priority) -> FileEntry {
    let size = data.len() as u64;
    let pl   = piece_length_for_size(size) as usize;
    let piece_hashes = if data.is_empty() {
        vec![]
    } else {
        data.chunks(pl).map(|c| piece_root(c)).collect()
    };
    FileEntry { path, size, piece_hashes, priority }
}

fn collect_dir_files(
    root: &Path,
    dir:  &Path,
    out:  &mut Vec<(Vec<String>, Vec<u8>)>,
) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading directory {:?}", dir))?
        .collect::<std::io::Result<_>>()
        .context("enumerating directory entries")?;
    entries.sort_by_key(|e| e.file_name());

    for e in entries {
        let p = e.path();
        if p.is_dir() {
            collect_dir_files(root, &p, out)?;
        } else if p.is_file() {
            let components: Vec<String> = p
                .strip_prefix(root)?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            let data = std::fs::read(&p)
                .with_context(|| format!("reading {:?}", p))?;
            out.push((components, data));
        }
    }
    Ok(())
}

// ── IPC cliente ───────────────────────────────────────────────────────────────

/// Intenta conectar al daemon y enviar una petición IPC.
/// Devuelve `None` si el daemon no está activo o si hay error de comunicación.
async fn try_ipc(socket_path: &Path, req: &IpcRequest) -> Option<IpcResponse> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    let stream = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        UnixStream::connect(socket_path),
    ).await.ok()?.ok()?;

    let (read_half, mut write_half) = stream.into_split();
    let json = serde_json::to_string(req).ok()?;
    write_half.write_all(format!("{json}\n").as_bytes()).await.ok()?;

    let mut lines = BufReader::new(read_half).lines();
    let line = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        lines.next_line(),
    ).await.ok()?.ok()??;

    serde_json::from_str(&line).ok()
}

/// Detiene el daemon enviando IpcRequest::Shutdown al socket IPC.
pub async fn cmd_serve_stop(config: &Config) -> Result<()> {
    match try_ipc(&config.socket_path, &IpcRequest::Shutdown).await {
        Some(_) => println!("daemon stopped"),
        None    => println!("daemon is not running"),
    }
    Ok(())
}

/// Relanza el propio ejecutable sin --background para correr como daemon.
pub fn cmd_serve_background() -> Result<()> {
    let exe = std::env::current_exe()
        .context("getting current executable path")?;

    // Pasar todos los args excepto --background y -d.
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| a != "--background" && a != "-d")
        .collect();

    let mut cmd = std::process::Command::new(exe);
    cmd.args(&args);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());

    // En Unix: nuevo grupo de proceso para no recibir SIGHUP al cerrar la terminal.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let child = cmd.spawn().context("spawning daemon process")?;
    println!("daemon started in background (PID {})", child.id());
    Ok(())
}

/// Arranca el servidor: tracker HTTP + DHT + endpoint QUIC + motor de descarga.
pub async fn cmd_serve(config: &Config, state_path: &Path) -> Result<()> {
    // PID file para que el CLI pueda encontrar el daemon.
    let pid = std::process::id();
    if let Err(e) = std::fs::write(&config.pid_file, format!("{}\n", pid)) {
        tracing::warn!("writing pid file: {e}");
    }

    println!("starting bitturbulence daemon (PID {pid})...");

    // DHT
    let dht_config = DhtConfig {
        bind_addr:           format!("0.0.0.0:{}", config.listen_port),
        bootstrap_nodes:     config.dht_bootstrap.clone(),
        routing_table_path:  Some(config.dht_routing_table.clone()),
    };
    let dht = DhtNode::new(dht_config)?;
    println!("  DHT node:  {}", dht.local_id);

    // Tracker HTTP
    let db_path = config.state_file.parent()
        .unwrap_or(Path::new("."))
        .join("tracker.db");
    let peer_store = PeerStore::open(Some(&db_path))?;
    let tracker_config = ServerConfig {
        bind_addr:         format!("0.0.0.0:{}", config.listen_port + 1)
            .parse()
            .context("parsing tracker bind addr")?,
        require_auth:      config.tracker_auth_token.is_some(),
        auth_token:        config.tracker_auth_token.clone(),
        announce_interval: 1800,
        min_interval:      60,
    };
    println!("  tracker:   http://0.0.0.0:{}/announce", config.listen_port + 1);
    println!("  QUIC:      quic://0.0.0.0:{}", config.listen_port);
    println!("\npress Ctrl+C to stop");

    let tracker = TrackerServer::new(tracker_config, peer_store);

    tokio::select! {
        r = tracker.run() => {
            r.map_err(|e| anyhow!("tracker: {}", e))?;
        }
        r = crate::daemon::run_daemon(config, state_path) => {
            r.map_err(|e| anyhow!("daemon: {}", e))?;
        }
        _ = tokio::signal::ctrl_c() => {
            println!("\nshutting down...");
            dht.save_routing_table()
                .map_err(|e| anyhow!("save routing table: {}", e))?;
            println!("routing table saved");
        }
    }

    let _ = std::fs::remove_file(&config.pid_file);
    Ok(())
}

// ── Helpers de formato ────────────────────────────────────────────────────────

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;
    match bytes {
        b if b >= TB => format!("{:.1}T", b as f64 / TB as f64),
        b if b >= GB => format!("{:.1}G", b as f64 / GB as f64),
        b if b >= MB => format!("{:.1}M", b as f64 / MB as f64),
        b if b >= KB => format!("{:.1}K", b as f64 / KB as f64),
        b            => format!("{}B", b),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { return s.to_string(); }
    format!("{}…", &s[..max - 1])
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_single_file_produces_valid_bitflow() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hello.bin");
        std::fs::write(&file, b"hello world this is test data for the bitflow creator").unwrap();

        let out_dir = dir.path().join("out");
        cmd_create(&file, None, vec![], None, Priority::Normal, Some(out_dir.clone())).unwrap();

        let entries: Vec<_> = std::fs::read_dir(&out_dir).unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("bitflow"))
            .collect();
        assert_eq!(entries.len(), 1, "debe haber exactamente 1 .bitflow");

        let json = std::fs::read(&entries[0].path()).unwrap();
        let meta: Metainfo = serde_json::from_slice(&json).unwrap();

        assert_eq!(meta.name, "hello.bin");
        assert_eq!(meta.files.len(), 1);
        assert_eq!(meta.files[0].path, vec!["hello.bin"]);
        assert_ne!(meta.info_hash, [0u8; 32], "info_hash debe estar calculado");
    }

    #[test]
    fn create_info_hash_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("data.bin");
        std::fs::write(&file, &vec![0x42u8; 64 * 1024]).unwrap();

        let out1 = dir.path().join("out1");
        let out2 = dir.path().join("out2");
        cmd_create(&file, None, vec![], None, Priority::Normal, Some(out1.clone())).unwrap();
        cmd_create(&file, None, vec![], None, Priority::Normal, Some(out2.clone())).unwrap();

        let meta1: Metainfo = serde_json::from_slice(
            &std::fs::read(std::fs::read_dir(&out1).unwrap().next().unwrap().unwrap().path()).unwrap()
        ).unwrap();
        let meta2: Metainfo = serde_json::from_slice(
            &std::fs::read(std::fs::read_dir(&out2).unwrap().next().unwrap().unwrap().path()).unwrap()
        ).unwrap();

        assert_eq!(meta1.info_hash, meta2.info_hash, "mismo contenido → mismo info_hash");
    }

    #[test]
    fn create_directory_includes_all_files() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("mydir");
        std::fs::create_dir_all(src.join("subdir")).unwrap();
        std::fs::write(src.join("a.txt"), b"archivo a").unwrap();
        std::fs::write(src.join("b.txt"), b"archivo b").unwrap();
        std::fs::write(src.join("subdir").join("c.txt"), b"archivo c").unwrap();

        let out = dir.path().join("out");
        cmd_create(&src, None, vec![], None, Priority::Normal, Some(out.clone())).unwrap();

        let json = std::fs::read(
            std::fs::read_dir(&out).unwrap().next().unwrap().unwrap().path()
        ).unwrap();
        let meta: Metainfo = serde_json::from_slice(&json).unwrap();

        assert_eq!(meta.name, "mydir");
        assert_eq!(meta.files.len(), 3);
        // Los archivos deben estar ordenados por path
        let paths: Vec<&Vec<String>> = meta.files.iter().map(|f| &f.path).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted, "archivos deben estar en orden determinista");
    }

    #[test]
    fn create_with_trackers_and_comment() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.bin");
        std::fs::write(&file, b"data").unwrap();

        let out = dir.path().join("out");
        cmd_create(
            &file,
            Some("mi torrent".into()),
            vec!["https://tracker.example.com/announce".into()],
            Some("comentario de prueba".into()),
            Priority::High,
            Some(out.clone()),
        ).unwrap();

        let json = std::fs::read(
            std::fs::read_dir(&out).unwrap().next().unwrap().unwrap().path()
        ).unwrap();
        let meta: Metainfo = serde_json::from_slice(&json).unwrap();

        assert_eq!(meta.name, "mi torrent");
        assert_eq!(meta.trackers, vec!["https://tracker.example.com/announce"]);
        assert_eq!(meta.comment, Some("comentario de prueba".into()));
    }

    #[test]
    fn create_piece_hashes_match_verify() {
        // Verifica que los hashes generados por cmd_create son los correctos
        // para que un downloader pueda verificar los datos.
        use bitturbulence_pieces::verify_piece;
        use bitturbulence_protocol::piece_length_for_size;

        let data = vec![0xABu8; 32 * 1024]; // 32 KB
        let dir  = tempfile::tempdir().unwrap();
        let file = dir.path().join("data.bin");
        std::fs::write(&file, &data).unwrap();

        let out = dir.path().join("out");
        cmd_create(&file, None, vec![], None, Priority::Normal, Some(out.clone())).unwrap();

        let json = std::fs::read(
            std::fs::read_dir(&out).unwrap().next().unwrap().unwrap().path()
        ).unwrap();
        let meta: Metainfo = serde_json::from_slice(&json).unwrap();

        let pl = piece_length_for_size(data.len() as u64) as usize;
        for (pi, chunk) in data.chunks(pl).enumerate() {
            assert!(
                verify_piece(chunk, &meta.files[0].piece_hashes[pi]),
                "pieza {pi} debe verificarse con los hashes generados"
            );
        }
    }
}
