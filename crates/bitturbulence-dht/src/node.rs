use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rand::RngCore;
use tokio::net::UdpSocket;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, info, warn};

use crate::{
    bucket::NodeInfo,
    error::Result,
    message::DhtMessage,
    node_id::NodeId,
    store::ValueStore,
    table::RoutingTable,
};

const _ALPHA: usize = 3;  // Paralelismo de queries Kademlia (pendiente de implementar)
const K:     usize = 20;  // Nodos por bucket

const DHT_QUERY_TIMEOUT: Duration = Duration::from_millis(500);

/// Handle para interactuar con el nodo DHT en ejecución.
#[derive(Clone)]
pub struct DhtHandle {
    node:        Arc<DhtNode>,
    send_tx:     mpsc::Sender<(String, DhtMessage)>,
    incoming_tx: broadcast::Sender<(String, DhtMessage)>,
    local_addr:  SocketAddr,
}

impl DhtHandle {
    pub fn local_addr(&self) -> SocketAddr { self.local_addr }
    pub fn local_id(&self)   -> NodeId     { self.node.local_id }

    /// Ping a todos los bootstrap_nodes configurados.
    pub async fn bootstrap(&self) {
        for addr in &self.node.config.bootstrap_nodes {
            let msg = DhtMessage::Ping { sender: self.node.local_id };
            let _ = self.send_tx.send((addr.clone(), msg)).await;
        }
        if !self.node.config.bootstrap_nodes.is_empty() {
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    /// Consulta peers para un info_hash a los K nodos más cercanos.
    /// Devuelve las direcciones de peers recopiladas en DHT_QUERY_TIMEOUT.
    pub async fn get_peers(&self, info_hash: [u8; 32]) -> Vec<String> {
        use std::collections::HashSet;
        let target  = NodeId::from_bytes(&info_hash).unwrap_or_else(|_| NodeId::random());
        let closest = self.node.closest_nodes(&target);
        if closest.is_empty() { return vec![]; }

        let mut rx = self.incoming_tx.subscribe();
        for n in &closest {
            let msg = DhtMessage::GetPeers { sender: self.node.local_id, info_hash };
            let _ = self.send_tx.send((n.addr.clone(), msg)).await;
        }

        let mut peers: HashSet<String> = HashSet::new();
        let deadline = tokio::time::Instant::now() + DHT_QUERY_TIMEOUT;
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok((_, DhtMessage::GotPeers { peers: p, nodes: n, .. }))) => {
                    for peer in p  { peers.insert(peer); }
                    for node in n  { self.node.add_node(node.id, node.addr.clone()); }
                }
                _ => break,
            }
        }
        peers.into_iter().collect()
    }

    /// Anuncia nuestra presencia para un info_hash:
    /// 1. GetPeers para obtener tokens de los nodos más cercanos
    /// 2. AnnouncePeer con cada token recibido
    pub async fn announce(&self, info_hash: [u8; 32], port: u16) {
        use std::collections::HashMap;
        let target  = NodeId::from_bytes(&info_hash).unwrap_or_else(|_| NodeId::random());
        let closest = self.node.closest_nodes(&target);
        if closest.is_empty() { return; }

        let mut rx = self.incoming_tx.subscribe();
        for n in &closest {
            let msg = DhtMessage::GetPeers { sender: self.node.local_id, info_hash };
            let _ = self.send_tx.send((n.addr.clone(), msg)).await;
        }

        let mut tokens: HashMap<String, [u8; 8]> = HashMap::new();
        let deadline = tokio::time::Instant::now() + DHT_QUERY_TIMEOUT;
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok((addr, DhtMessage::GotPeers { token, nodes: n, .. }))) => {
                    tokens.insert(addr, token);
                    for node in n { self.node.add_node(node.id, node.addr.clone()); }
                }
                _ => break,
            }
        }

        let our_addr = format!("0.0.0.0:{port}");
        for (addr, token) in tokens {
            let msg = DhtMessage::AnnouncePeer {
                sender: self.node.local_id,
                info_hash,
                token,
                addr: our_addr.clone(),
            };
            let _ = self.send_tx.send((addr, msg)).await;
        }
    }

    /// Guarda la tabla de routing en disco si está configurada la ruta.
    pub fn save_routing_table(&self) -> crate::error::Result<()> {
        self.node.save_routing_table()
    }
}

/// Configuración del nodo DHT.
#[derive(Debug, Clone)]
pub struct DhtConfig {
    /// Dirección de escucha (ip:port).
    pub bind_addr: String,
    /// Nodos bootstrap configurados por el usuario.
    pub bootstrap_nodes: Vec<String>,
    /// Ruta para persistir la tabla de routing. None = no persistir.
    pub routing_table_path: Option<PathBuf>,
}

impl Default for DhtConfig {
    fn default() -> Self {
        Self {
            bind_addr:           "0.0.0.0:6881".into(),
            bootstrap_nodes:     Vec::new(),
            routing_table_path:  None,
        }
    }
}

/// Estado compartido del nodo DHT.
#[derive(Debug)]
struct DhtState {
    table:  RoutingTable,
    store:  ValueStore,
    tokens: HashMap<NodeId, [u8; 8]>,
}

impl DhtState {
    fn new(local_id: NodeId) -> Self {
        Self {
            table:  RoutingTable::new(local_id),
            store:  ValueStore::new(),
            tokens: HashMap::new(),
        }
    }

    /// Genera y guarda un token para un nodo.
    fn issue_token(&mut self, node_id: &NodeId) -> [u8; 8] {
        let mut token = [0u8; 8];
        rand::thread_rng().fill_bytes(&mut token);
        self.tokens.insert(*node_id, token);
        token
    }

    /// Verifica un token recibido de un nodo.
    fn verify_token(&self, node_id: &NodeId, token: &[u8; 8]) -> bool {
        self.tokens.get(node_id).map(|t| t == token).unwrap_or(false)
    }
}

/// Nodo DHT. Gestiona la tabla de routing, el store y el loop de mensajes.
pub struct DhtNode {
    pub local_id: NodeId,
    config:       DhtConfig,
    state:        Arc<Mutex<DhtState>>,
}

impl DhtNode {
    /// Crea un nuevo nodo DHT. Si hay tabla guardada en disco, la carga.
    pub fn new(config: DhtConfig) -> Result<Self> {
        let local_id = NodeId::random();

        let table = if let Some(ref path) = config.routing_table_path {
            if path.exists() {
                match load_table(path) {
                    Ok(t) => {
                        info!("loaded routing table from {:?} ({} nodes)", path, t.len());
                        t
                    }
                    Err(e) => {
                        warn!("failed to load routing table: {}, starting fresh", e);
                        RoutingTable::new(local_id)
                    }
                }
            } else {
                RoutingTable::new(local_id)
            }
        } else {
            RoutingTable::new(local_id)
        };

        let mut state = DhtState::new(local_id);
        state.table = table;

        Ok(Self {
            local_id,
            config,
            state: Arc::new(Mutex::new(state)),
        })
    }

    /// Procesa un mensaje DHT entrante y devuelve la respuesta opcional.
    pub fn handle_message(
        &self,
        msg: DhtMessage,
        from_addr: &str,
    ) -> Option<DhtMessage> {
        let mut state = self.state.lock().unwrap();

        // Registrar al remitente en la tabla de routing
        state.table.upsert(NodeInfo::new(*msg.sender(), from_addr.to_string()));

        match msg {
            DhtMessage::Ping { sender } => {
                debug!("ping from {:?}", sender);
                Some(DhtMessage::Pong { sender: self.local_id })
            }

            DhtMessage::FindNode { sender: _, target } => {
                let nodes = state.table.closest(&target, K);
                Some(DhtMessage::Nodes { sender: self.local_id, nodes })
            }

            DhtMessage::GetPeers { sender, info_hash } => {
                let token = state.issue_token(&sender);
                let peers = state.store.get_peers(&info_hash);
                let nodes = if peers.is_empty() {
                    let target = NodeId::from_bytes(&info_hash).unwrap_or_else(|_| NodeId::random());
                    state.table.closest(&target, K)
                } else {
                    vec![]
                };
                Some(DhtMessage::GotPeers { sender: self.local_id, token, peers, nodes })
            }

            DhtMessage::AnnouncePeer { sender, info_hash, token, addr } => {
                if !state.verify_token(&sender, &token) {
                    warn!("invalid token from {:?}", sender);
                    return None;
                }
                state.store.announce_peer(info_hash, addr);
                Some(DhtMessage::AnnounceAck { sender: self.local_id })
            }

            DhtMessage::Store { sender, key, value, token } => {
                if !state.verify_token(&sender, &token) {
                    warn!("invalid token from {:?}", sender);
                    return None;
                }
                state.store.put(key, value);
                Some(DhtMessage::StoreAck { sender: self.local_id })
            }

            DhtMessage::Nodes { nodes, .. } => {
                for n in nodes {
                    state.table.upsert(n);
                }
                None
            }

            DhtMessage::Pong { .. }
            | DhtMessage::GotPeers { .. }
            | DhtMessage::AnnounceAck { .. }
            | DhtMessage::StoreAck { .. } => None,
        }
    }

    /// Guarda la tabla de routing en disco.
    pub fn save_routing_table(&self) -> Result<()> {
        if let Some(ref path) = self.config.routing_table_path {
            let state = self.state.lock().unwrap();
            save_table(&state.table, path)?;
            info!("saved routing table to {:?}", path);
        }
        Ok(())
    }

    /// Devuelve los K nodos más cercanos a un target.
    pub fn closest_nodes(&self, target: &NodeId) -> Vec<NodeInfo> {
        let state = self.state.lock().unwrap();
        state.table.closest(target, K)
    }

    /// Añade manualmente un nodo (útil para bootstrap).
    pub fn add_node(&self, id: NodeId, addr: String) {
        let mut state = self.state.lock().unwrap();
        state.table.upsert(NodeInfo::new(id, addr));
    }

    pub fn routing_table_size(&self) -> usize {
        self.state.lock().unwrap().table.len()
    }

    pub fn config(&self) -> &DhtConfig { &self.config }

    /// Enlaza el socket UDP y arranca el loop de mensajes DHT.
    pub async fn bind(self) -> crate::error::Result<DhtHandle> {
        let socket = UdpSocket::bind(&self.config.bind_addr).await?;
        let local_addr = socket.local_addr()?;
        let (send_tx, send_rx)   = mpsc::channel::<(String, DhtMessage)>(256);
        let (incoming_tx, _)     = broadcast::channel::<(String, DhtMessage)>(256);
        let node = Arc::new(self);
        let handle = DhtHandle {
            node: node.clone(),
            send_tx,
            incoming_tx: incoming_tx.clone(),
            local_addr,
        };
        tokio::spawn(udp_loop(socket, node, send_rx, incoming_tx));
        Ok(handle)
    }
}

async fn udp_loop(
    socket:      UdpSocket,
    node:        Arc<DhtNode>,
    mut send_rx: mpsc::Receiver<(String, DhtMessage)>,
    incoming_tx: broadcast::Sender<(String, DhtMessage)>,
) {
    let mut buf = vec![0u8; 65535];
    loop {
        tokio::select! {
            result = socket.recv_from(&mut buf) => {
                let (len, from) = match result {
                    Ok(x)  => x,
                    Err(e) => { warn!("dht recv: {e}"); break; }
                };
                let bytes = bytes::Bytes::copy_from_slice(&buf[..len]);
                let msg = match DhtMessage::decode(bytes) {
                    Ok(m)  => m,
                    Err(e) => { debug!("dht decode: {e}"); continue; }
                };
                let addr = from.to_string();
                if let Some(resp) = node.handle_message(msg.clone(), &addr) {
                    let _ = socket.send_to(&resp.encode(), from).await;
                }
                let _ = incoming_tx.send((addr, msg));
            }
            Some((addr, msg)) = send_rx.recv() => {
                if let Err(e) = socket.send_to(&msg.encode(), &addr).await {
                    debug!("dht send to {addr}: {e}");
                }
            }
        }
    }
}

fn save_table(table: &RoutingTable, path: &std::path::Path) -> Result<()> {
    let nodes = table.all_nodes();
    let json = serde_json::to_string(&nodes)?;
    std::fs::write(path, json)?;
    Ok(())
}

fn load_table(path: &std::path::Path) -> Result<RoutingTable> {
    let json = std::fs::read_to_string(path)?;
    let nodes: Vec<NodeInfo> = serde_json::from_str(&json)?;
    // Usamos un ID temporal — se sobreescribirá con el local_id real
    let mut table = RoutingTable::new(NodeId::random());
    for n in nodes {
        table.upsert(n);
    }
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> DhtConfig {
        DhtConfig {
            bind_addr:           "127.0.0.1:0".into(),
            bootstrap_nodes:     vec![],
            routing_table_path:  None,
        }
    }

    #[test]
    fn ping_pong() {
        let node = DhtNode::new(make_config()).unwrap();
        let msg = DhtMessage::Ping { sender: NodeId::random() };
        let resp = node.handle_message(msg, "127.0.0.1:9000").unwrap();
        assert!(matches!(resp, DhtMessage::Pong { .. }));
    }

    #[test]
    fn find_node_returns_nodes() {
        let node = DhtNode::new(make_config()).unwrap();
        // Añadir algunos nodos
        for i in 0..5u8 {
            node.add_node(NodeId::from_seed(&[i; 32]), format!("127.0.0.1:{}", 7000 + i as u16));
        }
        let msg = DhtMessage::FindNode {
            sender: NodeId::random(),
            target: NodeId::random(),
        };
        let resp = node.handle_message(msg, "127.0.0.1:9001").unwrap();
        assert!(matches!(resp, DhtMessage::Nodes { .. }));
    }

    #[test]
    fn announce_with_valid_token() {
        let node = DhtNode::new(make_config()).unwrap();
        let sender = NodeId::random();
        let info_hash = [0xABu8; 32];

        // GetPeers primero para obtener token
        let get = DhtMessage::GetPeers { sender, info_hash };
        let resp = node.handle_message(get, "127.0.0.1:9002").unwrap();

        let token = match resp {
            DhtMessage::GotPeers { token, .. } => token,
            _ => panic!("expected GotPeers"),
        };

        // AnnouncePeer con token válido
        let ann = DhtMessage::AnnouncePeer {
            sender, info_hash, token, addr: "127.0.0.1:6881".into(),
        };
        let ack = node.handle_message(ann, "127.0.0.1:9002").unwrap();
        assert!(matches!(ack, DhtMessage::AnnounceAck { .. }));
    }

    #[test]
    fn announce_with_invalid_token_rejected() {
        let node = DhtNode::new(make_config()).unwrap();
        let ann = DhtMessage::AnnouncePeer {
            sender: NodeId::random(),
            info_hash: [0u8; 32],
            token: [0xFF; 8],
            addr: "127.0.0.1:6881".into(),
        };
        assert!(node.handle_message(ann, "127.0.0.1:9003").is_none());
    }

    #[test]
    fn save_and_load_routing_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routing.json");

        let config = DhtConfig {
            routing_table_path: Some(path.clone()),
            ..make_config()
        };

        let node = DhtNode::new(config.clone()).unwrap();
        node.add_node(NodeId::from_seed(b"peer1"), "1.2.3.4:6881".into());
        node.add_node(NodeId::from_seed(b"peer2"), "5.6.7.8:6882".into());
        node.save_routing_table().unwrap();

        let node2 = DhtNode::new(config).unwrap();
        assert_eq!(node2.routing_table_size(), 2);
    }

    #[tokio::test]
    async fn bind_and_ping() {
        let config = DhtConfig { bind_addr: "127.0.0.1:0".into(), ..make_config() };
        let node = DhtNode::new(config).unwrap();
        let handle = node.bind().await.unwrap();
        let _ = handle.local_id();
        let _ = handle.local_addr();
    }

    #[tokio::test]
    async fn get_peers_returns_empty_when_no_nodes() {
        let config = DhtConfig { bind_addr: "127.0.0.1:0".into(), ..make_config() };
        let handle = DhtNode::new(config).unwrap().bind().await.unwrap();
        let peers = handle.get_peers([0xABu8; 32]).await;
        assert!(peers.is_empty(), "no nodes in table, should return empty");
    }

    #[tokio::test]
    async fn two_nodes_discover_peers() {
        let info_hash = [0xABu8; 32];

        // Node A: start and pre-populate with a peer
        let config_a = DhtConfig { bind_addr: "127.0.0.1:0".into(), ..make_config() };
        let node_a_raw = DhtNode::new(config_a).unwrap();
        {
            let fake_id = NodeId::random();
            let get = DhtMessage::GetPeers { sender: fake_id, info_hash };
            let resp = node_a_raw.handle_message(get, "127.0.0.1:9999").unwrap();
            let token = match resp {
                DhtMessage::GotPeers { token, .. } => token,
                _ => panic!(),
            };
            let ann = DhtMessage::AnnouncePeer {
                sender: fake_id, info_hash, token, addr: "10.0.0.1:6881".into(),
            };
            node_a_raw.handle_message(ann, "127.0.0.1:9999");
        }
        let handle_a = node_a_raw.bind().await.unwrap();
        let addr_a = handle_a.local_addr().to_string();

        // Node B: add node A to routing table, then query
        let config_b = DhtConfig { bind_addr: "127.0.0.1:0".into(), ..make_config() };
        let node_b_raw = DhtNode::new(config_b).unwrap();
        node_b_raw.add_node(handle_a.local_id(), addr_a);
        let handle_b = node_b_raw.bind().await.unwrap();

        let peers = handle_b.get_peers(info_hash).await;
        assert!(peers.contains(&"10.0.0.1:6881".to_string()), "should discover announced peer");
    }
}
