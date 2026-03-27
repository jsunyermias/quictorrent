use bytes::Bytes;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// TTL para valores almacenados (24 horas).
const VALUE_TTL: Duration = Duration::from_secs(86400);
/// TTL para peers anunciados (30 minutos).
const PEER_TTL: Duration = Duration::from_secs(1800);

#[derive(Debug, Clone)]
struct ValueEntry {
    value: Bytes,
    expires: Instant,
}

#[derive(Debug, Clone)]
struct PeerEntry {
    addr: String,
    expires: Instant,
}

/// Almacén de valores y peers DHT con TTL.
#[derive(Debug, Default)]
pub struct ValueStore {
    values: HashMap<[u8; 32], Vec<ValueEntry>>,
    peers: HashMap<[u8; 32], Vec<PeerEntry>>,
}

impl ValueStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Almacena un valor bajo una clave.
    pub fn put(&mut self, key: [u8; 32], value: Bytes) {
        self.purge_expired();
        let entry = ValueEntry {
            value,
            expires: Instant::now() + VALUE_TTL,
        };
        self.values.entry(key).or_default().push(entry);
    }

    /// Obtiene todos los valores vigentes bajo una clave.
    pub fn get(&mut self, key: &[u8; 32]) -> Vec<Bytes> {
        self.purge_expired();
        self.values
            .get(key)
            .map(|entries| entries.iter().map(|e| e.value.clone()).collect())
            .unwrap_or_default()
    }

    /// Anuncia un peer para un info_hash.
    pub fn announce_peer(&mut self, info_hash: [u8; 32], addr: String) {
        self.purge_expired();
        let entries = self.peers.entry(info_hash).or_default();
        // Actualizar si ya existe
        if let Some(e) = entries.iter_mut().find(|e| e.addr == addr) {
            e.expires = Instant::now() + PEER_TTL;
            return;
        }
        entries.push(PeerEntry {
            addr,
            expires: Instant::now() + PEER_TTL,
        });
    }

    /// Obtiene peers vigentes para un info_hash.
    pub fn get_peers(&mut self, info_hash: &[u8; 32]) -> Vec<String> {
        self.purge_expired();
        self.peers
            .get(info_hash)
            .map(|entries| entries.iter().map(|e| e.addr.clone()).collect())
            .unwrap_or_default()
    }

    fn purge_expired(&mut self) {
        let now = Instant::now();
        for entries in self.values.values_mut() {
            entries.retain(|e| e.expires > now);
        }
        for entries in self.peers.values_mut() {
            entries.retain(|e| e.expires > now);
        }
        self.values.retain(|_, v| !v.is_empty());
        self.peers.retain(|_, v| !v.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_and_get() {
        let mut s = ValueStore::new();
        let key = [0u8; 32];
        s.put(key, Bytes::from_static(b"hello"));
        let vals = s.get(&key);
        assert_eq!(vals.len(), 1);
        assert_eq!(vals[0], Bytes::from_static(b"hello"));
    }

    #[test]
    fn announce_and_get_peers() {
        let mut s = ValueStore::new();
        let ih = [0xABu8; 32];
        s.announce_peer(ih, "1.2.3.4:6881".into());
        s.announce_peer(ih, "5.6.7.8:6882".into());
        let peers = s.get_peers(&ih);
        assert_eq!(peers.len(), 2);
    }

    #[test]
    fn duplicate_peer_not_duplicated() {
        let mut s = ValueStore::new();
        let ih = [0xCDu8; 32];
        s.announce_peer(ih, "1.2.3.4:6881".into());
        s.announce_peer(ih, "1.2.3.4:6881".into());
        assert_eq!(s.get_peers(&ih).len(), 1);
    }
}
