use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("bencode decode error: {0}")]
    BencodeDecodeError(String),

    #[error("missing required field in torrent: {0}")]
    MissingField(&'static str),

    #[error("invalid info hash length: expected {expected}, got {got}")]
    InvalidInfoHashLength { expected: usize, got: usize },

    #[error("unknown message id: {0}")]
    UnknownMessageId(u8),

    #[error("invalid message length: expected at least {expected}, got {got}")]
    InvalidMessageLength { expected: usize, got: usize },

    #[error("frame too large: {0} bytes")]
    FrameTooLarge(usize),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, ProtocolError>;
