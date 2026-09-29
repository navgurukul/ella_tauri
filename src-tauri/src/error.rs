use thiserror::Error;

#[derive(Debug, Error)]
pub enum EllaError {
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("Local engine error: {0}")]
    Engine(String),
    /// A server answered, but not with the file: the status decides whether
    /// waiting and trying again can help.
    #[error("{message}")]
    HttpStatus { status: u16, message: String },
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("File or process error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Local engine request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Invalid local engine response: {0}")]
    Json(#[from] serde_json::Error),
}

pub type EllaResult<T> = Result<T, EllaError>;
