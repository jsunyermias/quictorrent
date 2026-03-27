use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use anyhow::Result;

/// Configuración global del cliente.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Directorio de descarga por defecto.
    pub download_dir: PathBuf,
    /// Puerto QUIC de escucha.
    pub listen_port:  u16,
    /// Trackers por defecto.
    pub trackers:     Vec<String>,
    /// Bootstrap nodes para la DHT.
    pub dht_bootstrap: Vec<String>,
    /// Token de autenticación para el tracker propio (opcional).
    pub tracker_auth_token: Option<String>,
    /// Ruta del fichero de estado.
    pub state_file:   PathBuf,
    /// Ruta de la tabla de routing DHT.
    pub dht_routing_table: PathBuf,
    /// Ruta del socket Unix IPC.
    pub socket_path: PathBuf,
    /// Ruta del PID file del daemon.
    pub pid_file: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        let home = dirs_next::home_dir()
            .unwrap_or_else(|| PathBuf::from("."));
        let base = home.join(".submarine");
        Self {
            download_dir:      base.join("downloads"),
            listen_port:       6881,
            trackers:          vec![],
            dht_bootstrap:     vec![],
            tracker_auth_token: None,
            state_file:        base.join("state.json"),
            dht_routing_table: base.join("routing.json"),
            socket_path:       base.join("submarine.sock"),
            pid_file:          base.join("submarine.pid"),
        }
    }
}

impl Config {
    pub fn load(path: &std::path::Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let json = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&json)?)
    }


}
