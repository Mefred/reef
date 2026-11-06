//! Session manager: owns active device sessions keyed by device id.

use crate::{Session, Error};
use std::collections::HashMap;

pub struct SessionManager {
    sessions: HashMap<String, Box<dyn std::any::Any>>,
}

impl SessionManager {
    pub fn new() -> Self {
        SessionManager {
            sessions: HashMap::new(),
        }
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_starts_empty() {
        let m = SessionManager::new();
        assert!(m.sessions.is_empty());
    }
}
