use std::path::Path;

use anyhow::{anyhow, Context, Result};

use bitturbulence_dht::{DhtConfig, DhtNode};
use bitturbulence_tracker::server::{ServerConfig, TrackerServer};
use bitturbulence_tracker::PeerStore;

use crate::config::Config;
use crate::ipc_proto::IpcRequest;
use super::ipc_client::try_ipc;

/// Detiene el daemon enviando IpcRequest::Shutdown al socket IPC.
pub async fn cmd_serve_stop(config: &Config) -> Result<()> {
    match try_ipc(&config.socket_path, &IpcRequest::Shutdown).await {
        Some(_) => println!("daemon stopped"),
        None => println!("daemon is not running"),
    }
    Ok(())
}

/// Relanza el propio ejecutable sin --background para correr como daemon.
pub fn cmd_serve_background() -> Result<()> {
    let exe = std::env::current_exe().context("getting current executable path")?;

    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| a != "--background" && a != "-d")
        .collect();

    let mut cmd = std::process::Command::new(exe);
    cmd.args(&args);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let child = cmd.spawn().context("spawning daemon process")?;
    println!("daemon started in background (PID {})", child.id());
    Ok(())
}

/// Arranca el servidor: tracker QUIC + DHT + endpoint QUIC + motor de descarga.
pub async fn cmd_serve(config: &Config, state_path: &Path) -> Result<()> {
    let pid = std::process::id();
    if let Err(e) = std::fs::write(&config.pid_file, format!("{}\n", pid)) {
        tracing::warn!("writing pid file: {e}");
    }

    println!("starting bitturbulence daemon (PID {pid})...");

    let dht_config = DhtConfig {
        bind_addr: format!("0.0.0.0:{}", config.listen_port),
        bootstrap_nodes: config.dht_bootstrap.clone(),
        routing_table_path: Some(config.dht_routing_table.clone()),
    };
    let dht = DhtNode::new(dht_config)?;
    println!("  DHT node:  {}", dht.local_id);

    let db_path = config
        .state_file
        .parent()
        .unwrap_or(Path::new("."))
        .join("tracker.db");
    let peer_store = PeerStore::open(Some(&db_path))?;
    let tracker_config = ServerConfig {
        bind_addr: format!("0.0.0.0:{}", config.listen_port + 1)
            .parse()
            .context("parsing tracker bind addr")?,
        require_auth: config.tracker_auth_token.is_some(),
        auth_token: config.tracker_auth_token.clone(),
        announce_interval: 1800,
        min_interval: 60,
    };
    println!(
        "  tracker:   quic://0.0.0.0:{}",
        config.listen_port + 1
    );
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
