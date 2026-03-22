use serde::{Deserialize, Serialize};
use crate::bucket::{KBucket, NodeInfo, K};
use crate::node_id::NodeId;

/// Tabla de routing Kademlia con 256 k-buckets (uno por bit de distancia).
#[derive(Debug, Serialize, Deserialize)]
pub struct RoutingTable {
    pub local_id: NodeId,
    buckets: Vec<KBucket>,
}

impl RoutingTable {
    pub fn new(local_id: NodeId) -> Self {
        Self {
            local_id,
            buckets: (0..=256).map(|_| KBucket::new()).collect(),
        }
    }

    /// Añade o actualiza un nodo en la tabla.
    pub fn upsert(&mut self, node: NodeInfo) -> bool {
        if node.id == self.local_id { return false; }
        let idx = self.local_id.bucket_index(&node.id).min(255);
        self.buckets[idx].upsert(node)
    }

    /// Elimina un nodo de la tabla.
    pub fn remove(&mut self, id: &NodeId) {
        let idx = self.local_id.bucket_index(id).min(255);
        self.buckets[idx].remove(id);
    }

    /// Devuelve los K nodos más cercanos al target.
    pub fn closest(&self, target: &NodeId, count: usize) -> Vec<NodeInfo> {
        let mut all: Vec<NodeInfo> = self.buckets.iter()
            .flat_map(|b| b.nodes().iter().cloned())
            .collect();

        all.sort_by_key(|n| n.id.distance(target));
        all.truncate(count);
        all
    }

    /// Número total de nodos en la tabla.
    pub fn len(&self) -> usize {
        self.buckets.iter().map(|b| b.len()).sum()
    }

    pub fn is_empty(&self) -> bool { self.len() == 0 }

    /// Todos los nodos de la tabla (para persistencia).
    pub fn all_nodes(&self) -> Vec<NodeInfo> {
        self.buckets.iter()
            .flat_map(|b| b.nodes().iter().cloned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(seed: &[u8]) -> NodeInfo {
        NodeInfo::new(NodeId::from_seed(seed), "127.0.0.1:0".into())
    }

    #[test]
    fn upsert_and_len() {
        let local = NodeId::from_seed(b"local");
        let mut t = RoutingTable::new(local);
        t.upsert(make_node(b"a"));
        t.upsert(make_node(b"b"));
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn self_not_inserted() {
        let local = NodeId::from_seed(b"local");
        let mut t = RoutingTable::new(local);
        t.upsert(NodeInfo::new(local, "127.0.0.1:0".into()));
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn closest_sorted_by_distance() {
        let local = NodeId::from_seed(b"local");
        let mut t = RoutingTable::new(local);
        for i in 0..10 {
            t.upsert(make_node(format!("node{}", i).as_bytes()));
        }
        let target = NodeId::from_seed(b"node3");
        let closest = t.closest(&target, 3);
        assert_eq!(closest.len(), 3);
        // El más cercano debe ser node3
        assert_eq!(closest[0].id, target);
    }

    #[test]
    fn remove_decrements_len() {
        let local = NodeId::from_seed(b"local");
        let mut t = RoutingTable::new(local);
        let n = make_node(b"a");
        let id = n.id;
        t.upsert(n);
        assert_eq!(t.len(), 1);
        t.remove(&id);
        assert_eq!(t.len(), 0);
    }
}
