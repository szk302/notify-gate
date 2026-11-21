//! Errors and exit codes.

use std::fmt;

/// An error that ends a command with a specific exit code.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Runtime error: exit code 1.
    #[error("{0}")]
    Runtime(String),
    /// Usage or configuration error: exit code 2.
    #[error("{0}")]
    Usage(String),
}

impl Error {
    pub fn code(&self) -> i32 {
        match self {
            Error::Runtime(_) => 1,
            Error::Usage(_) => 2,
        }
    }

    pub fn runtime(msg: impl fmt::Display) -> Self {
        Error::Runtime(msg.to_string())
    }

    pub fn usage(msg: impl fmt::Display) -> Self {
        Error::Usage(msg.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Runtime(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Print a warning to standard error.
pub fn warn(msg: impl fmt::Display) {
    eprintln!("ngate: warning: {msg}");
}
