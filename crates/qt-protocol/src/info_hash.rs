use sha1::{Digest as _, Sha1};
use sha2::Sha256;
use std::fmt;
use crate::error::{ProtocolError, Result};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct InfoHashV1([u8; 20]);

impl InfoHashV1 {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 20 {
            return Err(ProtocolError::InvalidInfoHashLength { expected: 20, got: bytes.len() });
        }
        let mut arr = [0u8; 20];
        arr.copy_from_slice(bytes);
        Ok(Self(arr))
    }

    pub fn from_info_dict(info_bytes: &[u8]) -> Self {
        let hash = Sha1::digest(info_bytes);
        let mut arr = [0u8; 20];
        arr.copy_from_slice(&hash);
        Self(arr)
    }

    pub fn as_bytes(&self) -> &[u8; 20] { &self.0 }
}

impl fmt::Debug for InfoHashV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "InfoHashV1({})", self) }
}
impl fmt::Display for InfoHashV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in &self.0 { write!(f, "{:02x}", b)?; }
        Ok(())
    }
}
impl TryFrom<&[u8]> for InfoHashV1 {
    type Error = ProtocolError;
    fn try_from(bytes: &[u8]) -> Result<Self> { Self::from_bytes(bytes) }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct InfoHashV2([u8; 32]);

impl InfoHashV2 {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 32 {
            return Err(ProtocolError::InvalidInfoHashLength { expected: 32, got: bytes.len() });
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(bytes);
        Ok(Self(arr))
    }

    pub fn from_info_dict(info_bytes: &[u8]) -> Self {
        let hash = Sha256::digest(info_bytes);
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&hash);
        Self(arr)
    }

    pub fn as_bytes(&self) -> &[u8; 32] { &self.0 }

    pub fn truncated_v1(&self) -> InfoHashV1 {
        let mut arr = [0u8; 20];
        arr.copy_from_slice(&self.0[..20]);
        InfoHashV1(arr)
    }
}

impl fmt::Debug for InfoHashV2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "InfoHashV2({})", self) }
}
impl fmt::Display for InfoHashV2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in &self.0 { write!(f, "{:02x}", b)?; }
        Ok(())
    }
}
impl TryFrom<&[u8]> for InfoHashV2 {
    type Error = ProtocolError;
    fn try_from(bytes: &[u8]) -> Result<Self> { Self::from_bytes(bytes) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InfoHash {
    V1(InfoHashV1),
    V2(InfoHashV2),
    Hybrid { v1: InfoHashV1, v2: InfoHashV2 },
}

impl fmt::Display for InfoHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InfoHash::V1(h)          => write!(f, "v1:{}", h),
            InfoHash::V2(h)          => write!(f, "v2:{}", h),
            InfoHash::Hybrid { v2, .. } => write!(f, "hybrid:{}", v2),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_hex_length() {
        assert_eq!(InfoHashV1::from_info_dict(b"hello").to_string().len(), 40);
    }

    #[test]
    fn v2_hex_length() {
        assert_eq!(InfoHashV2::from_info_dict(b"hello").to_string().len(), 64);
    }

    #[test]
    fn v1_wrong_length_errors() {
        assert!(InfoHashV1::from_bytes(&[0u8; 19]).is_err());
    }
}
