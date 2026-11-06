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
    #[error("session not found: {0}")]
    NotFound(String),
    #[error("timeout")]
    Timeout,
    #[error("parse error: {0}")]
    Parse(String),
    #[error("host key verification failed: {0}")]
    HostKey(String),
    #[error("host key changed: {0}")]
    HostKeyChanged(String),
    #[error("host key enrollment required: {0}")]
    HostKeyEnrollmentRequired(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<ssh2::Error> for Error {
    fn from(e: ssh2::Error) -> Error {
        Error::Transport(format!("ssh2: {}", e))
    }
}
