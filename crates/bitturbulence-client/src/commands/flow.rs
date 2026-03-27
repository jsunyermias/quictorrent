use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use bitturbulence_protocol::Priority;

use crate::ipc_proto::{IpcRequest, IpcResponse};
use crate::state::{ClientState, DownloadState, FlowEntry};
use crate::config::Config;
use super::format::{format_size, truncate};
use super::ipc_client::try_ipc;

/// Añade un BitFlow al cliente.
pub async fn cmd_add(
    meta_path: &Path,
    save_path: Option<PathBuf>,
    _priority: Priority,
    state_path: &Path,
    config: &Config,
) -> Result<()> {
    use anyhow::Context;

    let save_path = save_path.unwrap_or_else(|| config.download_dir.clone());
    std::fs::create_dir_all(&save_path)?;

    let meta_bytes =
        std::fs::read(meta_path).with_context(|| format!("cannot read {:?}", meta_path))?;
    let meta: bitturbulence_protocol::Metainfo =
        serde_json::from_slice(&meta_bytes).with_context(|| "invalid metainfo file")?;

    let info_hash_hex = hex::encode(meta.info_hash);
    let id = info_hash_hex[..8].to_string();

    let mut state = ClientState::load(state_path)?;

    if state.get(&id).is_some() {
        println!("flow {} already added", id);
        return Ok(());
    }

    let torrents_dir = config
        .state_file
        .parent()
        .unwrap_or(Path::new("."))
        .join("flows");
    std::fs::create_dir_all(&torrents_dir)?;
    let meta_dest = torrents_dir.join(format!("{}.bitflow", id));
    std::fs::copy(meta_path, &meta_dest)
        .with_context(|| format!("copying metainfo to {:?}", meta_dest))?;

    let total_size = meta.total_size();
    let entry = FlowEntry {
        id: id.clone(),
        info_hash: info_hash_hex,
        name: meta.name.clone(),
        save_path,
        metainfo_path: meta_dest,
        state: DownloadState::Queued,
        downloaded: 0,
        total_size,
        peers: 0,
    };

    state.add(entry);
    state.save(state_path)?;

    println!(
        "added flow [{}] {} ({} bytes, {} files)",
        id,
        meta.name,
        total_size,
        meta.files.len()
    );

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

    let live = match try_ipc(socket_path, &IpcRequest::Status).await {
        Some(IpcResponse::Status { flows }) => flows,
        _ => vec![],
    };

    println!(
        "{:<8}  {:<30}  {:>7}  {:>6}  {:>5}  STATE",
        "ID", "NAME", "SIZE", "PROG%", "PEERS"
    );
    println!("{}", "-".repeat(72));

    let mut entries: Vec<_> = state.flows.values().collect();
    entries.sort_by(|a, b| a.id.cmp(&b.id));

    for e in entries {
        let (downloaded, peers, state_str) = live
            .iter()
            .find(|l| l.id == e.id)
            .map(|l| (l.downloaded, l.peers, l.state.clone()))
            .unwrap_or((e.downloaded, e.peers, e.state.to_string()));

        let total = if e.total_size > 0 { e.total_size } else { 1 };
        let pct = downloaded as f32 / total as f32 * 100.0;
        println!(
            "{:<8}  {:<30}  {:>7}  {:>5.1}%  {:>5}  {}",
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
    if let Some(resp) = try_ipc(socket_path, &IpcRequest::FlowStart { id: id.into() }).await {
        match resp {
            IpcResponse::Ok => {}
            IpcResponse::Error { message } => return Err(anyhow!("{}", message)),
            _ => {}
        }
        let state = ClientState::load(state_path)?;
        if let Some(e) = state.get(id) {
            println!("[{}] {} → downloading", e.id, e.name);
        }
        return Ok(());
    }

    let mut state = ClientState::load(state_path)?;
    let entry = state
        .get_mut(id)
        .ok_or_else(|| anyhow!("flow '{}' not found", id))?;

    match &entry.state {
        DownloadState::Seeding => println!("[{}] already seeding", id),
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
    let entry = state
        .get_mut(id)
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
    let name = state
        .get(id)
        .ok_or_else(|| anyhow!("flow '{}' not found", id))?
        .name
        .clone();
    state
        .flows
        .retain(|k, v| k != id && !v.info_hash.starts_with(id));
    state.save(state_path)?;
    println!("[{}] {} removed", id, name);
    Ok(())
}

/// Muestra los peers de un BitFlow (datos en tiempo real si el daemon está activo).
pub async fn cmd_peers(id: &str, state_path: &Path, socket_path: &Path) -> Result<()> {
    let state = ClientState::load(state_path)?;
    let entry = state
        .get(id)
        .ok_or_else(|| anyhow!("flow '{}' not found", id))?;

    let peers = match try_ipc(socket_path, &IpcRequest::FlowPeers { id: id.into() }).await {
        Some(IpcResponse::Peers { count }) => count,
        _ => entry.peers,
    };

    println!(
        "flow [{}] {} — {} peer(s) connected",
        entry.id, entry.name, peers
    );
    if peers == 0 {
        println!("  (no peers)");
    }
    Ok(())
}
