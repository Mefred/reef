//! Errors for the session layer.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("transport error: {0}")]
    Transport(String),
    #[error("authentication failed")]
    AuthFailed,
    #[error("privilege escalation failed")]
    PrivilegeEscalationFailed,
    #[error("not connected")]
    NotConnected,
    #[error("timeout")]
    Timeout,
    #[error("parse error: {0}")]
    Parse(String),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Transport(e.to_string())
    }
}
