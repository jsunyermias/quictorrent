use crate::error::{DhtError, Result};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// Identificador de nodo DHT: 256 bits (SHA-256).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId([u8; 32]);

impl NodeId {
    /// Genera un NodeId aleatorio.
    pub fn random() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        Self(bytes)
    }

    /// Deriva un NodeId desde una clave pública o cualquier bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 32 {
            return Err(DhtError::InvalidNodeId(bytes.len()));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(bytes);
        Ok(Self(arr))
    }

    /// Genera un NodeId determinista desde un seed (SHA-256 del seed).
    pub fn from_seed(seed: &[u8]) -> Self {
        let hash = Sha256::digest(seed);
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&hash);
        Self(arr)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Distancia XOR entre dos NodeIds.
    pub fn distance(&self, other: &NodeId) -> Distance {
        let mut d = [0u8; 32];
        for (i, byte) in d.iter_mut().enumerate() {
            *byte = self.0[i] ^ other.0[i];
        }
        Distance(d)
    }

    /// Índice del bucket para este NodeId relativo a `self`.
    /// Retorna el número del bit más significativo de la distancia XOR.
    pub fn bucket_index(&self, other: &NodeId) -> usize {
        let dist = self.distance(other);
        dist.leading_zeros()
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeId({})", hex::encode(&self.0[..4]))
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

/// Distancia XOR entre dos NodeIds.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Distance([u8; 32]);

impl Distance {
    /// Número de bits cero iniciales (equivale al índice del bucket).
    pub fn leading_zeros(&self) -> usize {
        let mut count = 0;
        for byte in &self.0 {
            if *byte == 0 {
                count += 8;
            } else {
                count += byte.leading_zeros() as usize;
                break;
            }
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_to_self_is_zero() {
        let id = NodeId::random();
        let d = id.distance(&id);
        assert_eq!(d, Distance([0u8; 32]));
        assert_eq!(d.leading_zeros(), 256);
    }

    #[test]
    fn distance_is_symmetric() {
        let a = NodeId::random();
        let b = NodeId::random();
        assert_eq!(a.distance(&b), b.distance(&a));
    }

    #[test]
    fn bucket_index_range() {
        let a = NodeId::random();
        let b = NodeId::random();
        let idx = a.bucket_index(&b);
        assert!(idx <= 256);
    }

    #[test]
    fn from_seed_deterministic() {
        let a = NodeId::from_seed(b"test");
        let b = NodeId::from_seed(b"test");
        assert_eq!(a, b);
    }
}
