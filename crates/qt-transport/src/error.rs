use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("TLS error: {0}")]
    Tls(String),

    #[error("quinn connect error: {0}")]
    Connect(#[from] quinn::ConnectError),

    #[error("quinn connection error: {0}")]
    Connection(#[from] quinn::ConnectionError),

    #[error("quinn write error: {0}")]
    Write(#[from] quinn::WriteError),

    #[error("quinn read error: {0}")]
    Read(#[from] quinn::ReadError),

    #[error("quinn read to end error: {0}")]
    ReadToEnd(#[from] quinn::ReadToEndError),

    #[error("certificate generation error: {0}")]
    CertGen(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("protocol error: {0}")]
    Protocol(#[from] qt_protocol::ProtocolError),
}

pub type Result<T> = std::result::Result<T, TransportError>;
