use std::net::SocketAddr;
use quinn::Endpoint;
use tracing::{info, warn};

use crate::{
    config::{client_config, server_config},
    connection::PeerConnection,
    error::{Result, TransportError},
};

pub struct QuicEndpoint {
    endpoint: Endpoint,
}

impl QuicEndpoint {
    pub fn bind(bind_addr: SocketAddr) -> Result<Self> {
        let server_cfg = server_config()?;
        let client_cfg = client_config()?;

        let mut endpoint = Endpoint::server(server_cfg, bind_addr)?;
        endpoint.set_default_client_config(client_cfg);

        info!("QUIC endpoint bound to {}", bind_addr);
        Ok(Self { endpoint })
    }

    pub fn client_only() -> Result<Self> {
        let client_cfg = client_config()?;
        let bind_addr: SocketAddr = "0.0.0.0:0".parse().unwrap();

        let mut endpoint = Endpoint::client(bind_addr)?;
        endpoint.set_default_client_config(client_cfg);

        Ok(Self { endpoint })
    }

    pub async fn connect(&self, addr: SocketAddr) -> Result<PeerConnection> {
        let conn = self.endpoint
            .connect(addr, "quictorrent")?
            .await?;

        info!("Connected to peer {}", addr);
        Ok(PeerConnection::new(conn))
    }

    pub async fn accept(&self) -> Option<Result<PeerConnection>> {
        let incoming = self.endpoint.accept().await?;
        let addr = incoming.remote_address();

        match incoming.await {
            Ok(conn) => {
                info!("Accepted connection from {}", addr);
                Some(Ok(PeerConnection::new(conn)))
            }
            Err(e) => {
                warn!("Failed to accept connection from {}: {}", addr, e);
                Some(Err(TransportError::Connection(e)))
            }
        }
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.endpoint.local_addr()?)
    }

    pub async fn shutdown(&self) {
        self.endpoint.close(quinn::VarInt::from_u32(0), b"shutdown");
        self.endpoint.wait_idle().await;
    }
}
