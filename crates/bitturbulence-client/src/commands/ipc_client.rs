use std::path::Path;

use crate::ipc_proto::{IpcRequest, IpcResponse};

/// Intenta conectar al daemon y enviar una petición IPC.
/// Devuelve `None` si el daemon no está activo o si hay error de comunicación.
pub(super) async fn try_ipc(socket_path: &Path, req: &IpcRequest) -> Option<IpcResponse> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    let stream = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        UnixStream::connect(socket_path),
    )
    .await
    .ok()?
    .ok()?;

    let (read_half, mut write_half) = stream.into_split();
    let json = serde_json::to_string(req).ok()?;
    write_half
        .write_all(format!("{json}\n").as_bytes())
        .await
        .ok()?;

    let mut lines = BufReader::new(read_half).lines();
    let line = tokio::time::timeout(std::time::Duration::from_millis(500), lines.next_line())
        .await
        .ok()?
        .ok()??;

    serde_json::from_str(&line).ok()
}
