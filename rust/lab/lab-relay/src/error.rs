#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("tls: {0}")]
    Tls(#[from] rustls::Error),
    #[error("websocket: {0}")]
    Socket(Box<tungstenite::Error>),
    #[error("certificate: {0}")]
    Certificate(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("expired")]
    Expired,
    #[error("invalid")]
    Invalid,
    #[error("handshake rejected with status {0}")]
    Status(u16),
    #[error("timeout")]
    Timeout,
}
impl From<tungstenite::Error> for Error {
    fn from(error: tungstenite::Error) -> Self {
        match error {
            tungstenite::Error::Http(response) => Self::Status(response.status().as_u16()),
            other => Self::Socket(Box::new(other)),
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;
