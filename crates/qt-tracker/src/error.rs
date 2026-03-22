use thiserror::Error;

#[derive(Debug, Error)]
pub enum TrackerError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid info hash: {0}")]
    InvalidInfoHash(String),

    #[error("authentication required")]
    AuthRequired,

    #[error("authentication failed")]
    AuthFailed,

    #[error("announce rejected: {0}")]
    Rejected(String),
}

pub type Result<T> = std::result::Result<T, TrackerError>;
