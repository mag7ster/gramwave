#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AppError {
    #[error("Operation cancelled")]
    Cancelled,
    #[error("{0} timed out")]
    Timeout(&'static str),
    #[error("{0}: too many pending operations")]
    Overloaded(&'static str),
    #[error("{0} is unavailable")]
    Unavailable(&'static str),
    #[error("Storage: {0}")]
    Storage(String),
    #[error("Telegram {code}: {message}")]
    Telegram { code: i32, message: String },
    #[error("Player: {0}")]
    Player(String),
    #[error("Invalid service response: {0}")]
    Protocol(String),
    #[error("Task panicked")]
    Panicked,
}
