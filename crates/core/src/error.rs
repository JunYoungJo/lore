use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Permanent,
    Transient,
    Rejected,
}

#[derive(Debug)]
pub struct Error {
    pub failure: Failure,
    pub message: String,
}

impl Error {
    pub fn permanent(message: impl Into<String>) -> Self {
        Self { failure: Failure::Permanent, message: message.into() }
    }

    pub fn transient(message: impl Into<String>) -> Self {
        Self { failure: Failure::Transient, message: message.into() }
    }

    pub fn rejected(message: impl Into<String>) -> Self {
        Self { failure: Failure::Rejected, message: message.into() }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::transient(format!("상태 DB 오류: {error}"))
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::transient(format!("파일 오류: {error}"))
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::transient(format!("JSON 오류: {error}"))
    }
}

pub type Result<T> = std::result::Result<T, Error>;
