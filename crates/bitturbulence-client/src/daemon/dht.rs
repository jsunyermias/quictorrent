use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::Mutex;

type KnownPeers = Arc<Mutex<HashSet<String>>>;
use tokio::time::interval;
use tracing::{info, warn};

use bitturbulence_dht::DhtHandle;
use bitturbulence_transport::QuicEndpoint;

use super::context::FlowCtx;
use super::peer::run_peer;

pub const DHT_ANNOUNCE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);

pub async fn dht_loop(
    handle: DhtHandle,
    flows: Vec<(Arc<FlowCtx>, KnownPeers)>,
    listen_port: u16,
    endpoint: Arc<QuicEndpoint>,
    peer_id: [u8; 32],
) {
    handle.bootstrap().await;
    info!(
        "DHT bootstrap complete, local node: {}",
        hex::encode(&handle.local_id().as_bytes()[..4])
    );

    let mut timer = interval(DHT_ANNOUNCE_INTERVAL);
    timer.tick().await;

    loop {
        timer.tick().await;

        for (ctx, known_peers) in &flows {
            let info_hash = ctx.meta.info_hash;

            if *ctx.complete_tx.borrow() {
                handle.announce(info_hash, listen_port).await;
                let id: String = info_hash[..4].iter().map(|b| format!("{b:02x}")).collect();
                info!("[{id}] DHT announce ok");
            }

            let peers = handle.get_peers(info_hash).await;
            for peer_addr in peers {
                let is_new = known_peers.lock().await.insert(peer_addr.clone());
                if !is_new {
                    continue;
                }
                let ep = endpoint.clone();
                let ctx2 = ctx.clone();
                tokio::spawn(async move {
                    match peer_addr.parse::<std::net::SocketAddr>() {
                        Ok(addr) => match ep.connect(addr).await {
                            Ok(conn) => run_peer(conn, ctx2, peer_id, true).await,
                            Err(e) => warn!("dht connect {addr}: {e}"),
                        },
                        Err(_) => warn!("dht invalid peer addr: {peer_addr}"),
                    }
                });
            }
        }

        if let Err(e) = handle.save_routing_table() {
            warn!("dht save routing table: {e}");
        }
    }
}
