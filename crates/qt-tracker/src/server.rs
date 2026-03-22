use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use tracing::info;

use crate::{
    store::PeerStore,
    types::{AnnounceRequest, AnnounceResponse, PeerInfo, ScrapeResponse},
};

/// Configuración del servidor tracker.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Dirección de escucha.
    pub bind_addr: SocketAddr,
    /// Si true, require token de autenticación en el header `X-Auth-Token`.
    pub require_auth: bool,
    /// Token válido (si require_auth = true).
    pub auth_token: Option<String>,
    /// Intervalo sugerido a los clientes (segundos).
    pub announce_interval: u32,
    /// Intervalo mínimo (segundos).
    pub min_interval: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr:         "0.0.0.0:6969".parse().unwrap(),
            require_auth:      false,
            auth_token:        None,
            announce_interval: 1800,
            min_interval:      60,
        }
    }
}

#[derive(Clone)]
struct AppState {
    store:  PeerStore,
    config: Arc<ServerConfig>,
}

/// Servidor tracker HTTP (sobre TCP — HTTP/3 requeriría quinn+axum integración
/// que está en desarrollo; usamos HTTP/1.1+TLS por ahora y añadimos H3 después).
pub struct TrackerServer {
    config: ServerConfig,
    store:  PeerStore,
}

impl TrackerServer {
    pub fn new(config: ServerConfig, store: PeerStore) -> Self {
        Self { config, store }
    }

    pub async fn run(self) -> crate::error::Result<()> {
        let addr = self.config.bind_addr;
        let state = AppState {
            store:  self.store.clone(),
            config: Arc::new(self.config),
        };

        self.store.start_flush_task();

        let app = Router::new()
            .route("/announce", post(handle_announce))
            .route("/scrape",   get(handle_scrape))
            .with_state(state);

        info!("tracker listening on {}", addr);
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;
        Ok(())
    }
}

fn check_auth(headers: &HeaderMap, config: &ServerConfig) -> bool {
    if !config.require_auth { return true; }
    let expected = match &config.auth_token {
        Some(t) => t,
        None    => return true,
    };
    headers.get("X-Auth-Token")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == expected)
        .unwrap_or(false)
}

async fn handle_announce(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AnnounceRequest>,
) -> Result<Json<AnnounceResponse>, StatusCode> {
    if !check_auth(&headers, &state.config) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let peers = state.store.announce(&req).map_err(|_| StatusCode::BAD_REQUEST)?;
    let (complete, incomplete, _) = {
        let ih = hex::decode(&req.info_hash)
            .ok()
            .and_then(|b| b.try_into().ok())
            .unwrap_or([0u8; 32]);
        state.store.scrape(&ih)
    };

    let peer_list: Vec<PeerInfo> = peers.into_iter().map(|r| PeerInfo {
        peer_id: hex::encode(r.peer_id),
        addr:    r.addr,
    }).collect();

    Ok(Json(AnnounceResponse {
        interval:     state.config.announce_interval,
        min_interval: Some(state.config.min_interval),
        peers:        peer_list,
        complete,
        incomplete,
    }))
}

#[derive(Deserialize)]
struct ScrapeQuery {
    info_hash: String,
}

async fn handle_scrape(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ScrapeQuery>,
) -> Result<Json<ScrapeResponse>, StatusCode> {
    if !check_auth(&headers, &state.config) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let ih: [u8; 32] = hex::decode(&q.info_hash)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or(StatusCode::BAD_REQUEST)?;

    let (complete, incomplete, downloaded) = state.store.scrape(&ih);
    Ok(Json(ScrapeResponse { complete, incomplete, downloaded }))
}
