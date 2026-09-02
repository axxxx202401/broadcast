use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("AES decryption failed: {0}")]
    AesDecrypt(String),
    #[error("AES encryption failed: {0}")]
    AesEncrypt(String),
    #[error("TCP frame malformed: {0}")]
    TcpFrame(String),
    #[error("Proto parse error: {0}")]
    ProtoParse(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("Business error {code}: {message}")]
    Business { code: i32, message: String },
    #[error("Database error: {0}")]
    Db(String),
    #[error("Configuration error: {0}")]
    Config(String),
    #[error("Login failed: {0}")]
    Login(String),
    #[error("Chat client is already connected")]
    AlreadyConnected,
}

pub type AppResult<T> = Result<T, AppError>;
