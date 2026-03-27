//! Tipos del protocolo IPC entre el CLI y el daemon.
//!
//! Protocolo: JSON por líneas (NDJSON) sobre un Unix socket.
//! Cada request y cada response es un objeto JSON terminado en `\n`.

use serde::{Deserialize, Serialize};

/// Petición del CLI al daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum IpcRequest {
    /// Estado de todos los flows activos.
    Status,
    /// Inicia/reanuda la descarga de un flow.
    FlowStart { id: String },
    /// Pausa la descarga de un flow.
    FlowPause { id: String },
    /// Elimina un flow de la lista.
    FlowStop  { id: String },
    /// Número de peers conectados a un flow.
    FlowPeers { id: String },
    /// Comprueba que el daemon está vivo.
    Ping,
    /// Solicita al daemon que se detenga limpiamente.
    Shutdown,
}

/// Respuesta del daemon al CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IpcResponse {
    /// Lista de flows con datos en tiempo real.
    Status { flows: Vec<IpcFlowInfo> },
    /// Operación completada sin resultado extra.
    Ok,
    /// Número de peers conectados al flow solicitado.
    Peers { count: usize },
    /// Error en la operación solicitada.
    Error { message: String },
    /// Respuesta a Ping.
    Pong,
}

/// Información en tiempo real de un flow activo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcFlowInfo {
    pub id:         String,
    pub name:       String,
    /// Estado según el daemon: "downloading" | "seeding"
    pub state:      String,
    pub downloaded: u64,
    pub total_size: u64,
    pub peers:      usize,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip_status() {
        let req = IpcRequest::Status;
        let json = serde_json::to_string(&req).unwrap();
        let back: IpcRequest = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, IpcRequest::Status));
    }

    #[test]
    fn request_roundtrip_flow_start() {
        let req = IpcRequest::FlowStart { id: "abc123".into() };
        let json = serde_json::to_string(&req).unwrap();
        let back: IpcRequest = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, IpcRequest::FlowStart { id } if id == "abc123"));
    }

    #[test]
    fn request_roundtrip_shutdown() {
        let req = IpcRequest::Shutdown;
        let json = serde_json::to_string(&req).unwrap();
        let back: IpcRequest = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, IpcRequest::Shutdown));
    }

    #[test]
    fn response_roundtrip_ok() {
        let resp = IpcResponse::Ok;
        let json = serde_json::to_string(&resp).unwrap();
        let back: IpcResponse = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, IpcResponse::Ok));
    }

    #[test]
    fn response_roundtrip_status() {
        let resp = IpcResponse::Status {
            flows: vec![IpcFlowInfo {
                id:         "deadbeef".into(),
                name:       "test.bin".into(),
                state:      "downloading".into(),
                downloaded: 1024,
                total_size: 4096,
                peers:      2,
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: IpcResponse = serde_json::from_str(&json).unwrap();
        if let IpcResponse::Status { flows } = back {
            assert_eq!(flows.len(), 1);
            assert_eq!(flows[0].id, "deadbeef");
            assert_eq!(flows[0].peers, 2);
        } else {
            panic!("expected Status");
        }
    }

    #[test]
    fn response_roundtrip_error() {
        let resp = IpcResponse::Error { message: "flow not found".into() };
        let json = serde_json::to_string(&resp).unwrap();
        let back: IpcResponse = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, IpcResponse::Error { message } if message == "flow not found"));
    }
}
