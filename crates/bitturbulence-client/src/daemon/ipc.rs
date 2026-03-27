//! Servidor IPC del daemon: escucha un Unix socket y despacha peticiones
//! del CLI sobre los [`FlowCtx`]s activos.
//!
//! Protocolo: JSON por líneas (NDJSON). Cada conexión puede enviar múltiples
//! requests y recibe una response por request. La conexión se cierra cuando
//! el cliente cierra su extremo.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::watch;
use tracing::{debug, info, warn};

use super::context::FlowCtx;
use crate::ipc_proto::{IpcFlowInfo, IpcRequest, IpcResponse};
use crate::state::{ClientState, DownloadState};

/// Arranca el servidor IPC. Retorna cuando recibe una señal de shutdown
/// (enviada por `handle_conn` al procesar `IpcRequest::Shutdown`).
///
/// - `shutdown_tx`: el mismo sender que usa `run_daemon`; cuando el servidor
///   lo dispara, el daemon abandona su select principal.
pub async fn run_ipc_server(
    socket_path: PathBuf,
    flow_ids: Vec<(String, Arc<FlowCtx>)>,
    state_path: PathBuf,
    shutdown_tx: watch::Sender<bool>,
) {
    // Eliminar socket residual de ejecuciones anteriores.
    let _ = std::fs::remove_file(&socket_path);

    let listener = match UnixListener::bind(&socket_path) {
        Ok(l) => l,
        Err(e) => {
            warn!("IPC bind {:?}: {e}", socket_path);
            return;
        }
    };
    info!("IPC socket: {:?}", socket_path);

    let shutdown_tx = Arc::new(shutdown_tx);
    let mut accept_shutdown = shutdown_tx.subscribe();

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        let fids = flow_ids.clone();
                        let sp   = state_path.clone();
                        let tx   = shutdown_tx.clone();
                        tokio::spawn(handle_conn(stream, fids, sp, tx));
                    }
                    Err(e) => { warn!("IPC accept: {e}"); }
                }
            }
            _ = accept_shutdown.changed() => {
                debug!("IPC server shutting down");
                break;
            }
        }
    }

    let _ = std::fs::remove_file(&socket_path);
}

async fn handle_conn(
    stream: tokio::net::UnixStream,
    flow_ids: Vec<(String, Arc<FlowCtx>)>,
    state_path: PathBuf,
    shutdown_tx: Arc<watch::Sender<bool>>,
) {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let req: IpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                send(
                    &mut write_half,
                    &IpcResponse::Error {
                        message: format!("parse error: {e}"),
                    },
                )
                .await;
                continue;
            }
        };
        let shutdown = matches!(req, IpcRequest::Shutdown);
        let resp = dispatch(req, &flow_ids, &state_path).await;
        send(&mut write_half, &resp).await;
        if shutdown {
            let _ = shutdown_tx.send_replace(true);
            return;
        }
    }
}

async fn send<W: AsyncWriteExt + Unpin>(w: &mut W, resp: &IpcResponse) {
    if let Ok(mut json) = serde_json::to_string(resp) {
        json.push('\n');
        let _ = w.write_all(json.as_bytes()).await;
    }
}

async fn dispatch(
    req: IpcRequest,
    flow_ids: &[(String, Arc<FlowCtx>)],
    state_path: &std::path::Path,
) -> IpcResponse {
    match req {
        IpcRequest::Ping => IpcResponse::Pong,

        // Shutdown: la respuesta Ok se envía antes de disparar la señal (en handle_conn).
        IpcRequest::Shutdown => IpcResponse::Ok,

        IpcRequest::Status => {
            let flows = flow_ids
                .iter()
                .map(|(id, ctx)| {
                    let complete = *ctx.complete_tx.borrow();
                    IpcFlowInfo {
                        id: id.clone(),
                        name: ctx.meta.name.clone(),
                        state: if complete {
                            "seeding".into()
                        } else {
                            "downloading".into()
                        },
                        downloaded: ctx.downloaded.load(Ordering::Relaxed),
                        total_size: ctx.meta.total_size(),
                        peers: ctx.peer_count.load(Ordering::Relaxed),
                    }
                })
                .collect();
            IpcResponse::Status { flows }
        }

        IpcRequest::FlowPeers { id } => match flow_ids.iter().find(|(fid, _)| fid == &id) {
            Some((_, ctx)) => IpcResponse::Peers {
                count: ctx.peer_count.load(Ordering::Relaxed),
            },
            None => IpcResponse::Error {
                message: format!("flow '{id}' not active in daemon"),
            },
        },

        IpcRequest::FlowStart { id } => {
            match set_state(state_path, &id, DownloadState::Downloading) {
                Ok(()) => IpcResponse::Ok,
                Err(e) => IpcResponse::Error {
                    message: e.to_string(),
                },
            }
        }

        IpcRequest::FlowPause { id } => match set_state(state_path, &id, DownloadState::Paused) {
            Ok(()) => IpcResponse::Ok,
            Err(e) => IpcResponse::Error {
                message: e.to_string(),
            },
        },

        IpcRequest::FlowStop { id } => match remove_flow(state_path, &id) {
            Ok(()) => IpcResponse::Ok,
            Err(e) => IpcResponse::Error {
                message: e.to_string(),
            },
        },
    }
}

fn set_state(
    state_path: &std::path::Path,
    id: &str,
    new_state: DownloadState,
) -> anyhow::Result<()> {
    let mut state = ClientState::load(state_path)?;
    let entry = state
        .get_mut(id)
        .ok_or_else(|| anyhow::anyhow!("flow '{id}' not found"))?;
    entry.state = new_state;
    state.save(state_path)?;
    Ok(())
}

fn remove_flow(state_path: &std::path::Path, id: &str) -> anyhow::Result<()> {
    let mut state = ClientState::load(state_path)?;
    if state.get(id).is_none() {
        return Err(anyhow::anyhow!("flow '{id}' not found"));
    }
    // Eliminar por id corto o info_hash completo.
    state
        .flows
        .retain(|k, v| k != id && !v.info_hash.starts_with(id));
    state.save(state_path)?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    async fn ipc_roundtrip(socket_path: &std::path::Path, req: &IpcRequest) -> IpcResponse {
        let stream = UnixStream::connect(socket_path).await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        let json = serde_json::to_string(req).unwrap();
        write_half
            .write_all(format!("{json}\n").as_bytes())
            .await
            .unwrap();
        let mut lines = BufReader::new(read_half).lines();
        let line = lines.next_line().await.unwrap().unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[tokio::test]
    async fn ping_pong() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("test.sock");
        let (shutdown_tx, _) = watch::channel(false);

        tokio::spawn(run_ipc_server(
            sock.clone(),
            vec![],
            dir.path().join("state.json"),
            shutdown_tx,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let resp = ipc_roundtrip(&sock, &IpcRequest::Ping).await;
        assert!(matches!(resp, IpcResponse::Pong));
    }

    #[tokio::test]
    async fn status_empty_when_no_flows() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("test.sock");
        let (shutdown_tx, _) = watch::channel(false);

        tokio::spawn(run_ipc_server(
            sock.clone(),
            vec![],
            dir.path().join("state.json"),
            shutdown_tx,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let resp = ipc_roundtrip(&sock, &IpcRequest::Status).await;
        if let IpcResponse::Status { flows } = resp {
            assert!(flows.is_empty());
        } else {
            panic!("expected Status");
        }
    }

    #[tokio::test]
    async fn shutdown_triggers_server_exit() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("test.sock");
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

        tokio::spawn(run_ipc_server(
            sock.clone(),
            vec![],
            dir.path().join("state.json"),
            shutdown_tx,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let resp = ipc_roundtrip(&sock, &IpcRequest::Shutdown).await;
        assert!(matches!(resp, IpcResponse::Ok));

        // La señal de shutdown debe haberse disparado.
        tokio::time::timeout(std::time::Duration::from_millis(100), shutdown_rx.changed())
            .await
            .expect("timeout esperando señal shutdown")
            .unwrap();
        assert!(*shutdown_rx.borrow());
    }

    #[tokio::test]
    async fn flow_peers_unknown_id_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("test.sock");
        let (shutdown_tx, _) = watch::channel(false);

        tokio::spawn(run_ipc_server(
            sock.clone(),
            vec![],
            dir.path().join("state.json"),
            shutdown_tx,
        ));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let resp = ipc_roundtrip(
            &sock,
            &IpcRequest::FlowPeers {
                id: "notexist".into(),
            },
        )
        .await;
        assert!(matches!(resp, IpcResponse::Error { .. }));
    }
}
