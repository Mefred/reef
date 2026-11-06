//! Session manager: owns active device sessions keyed by device id.
//!
//! Phase 1 stub: tracks sessions by device id. The generic Session<T> is
//! stored behind a trait object via the concrete transport type.

use std::collections::HashMap;

pub struct SessionManager {
    sessions: HashMap<String, String>, // device_id -> transport type name
}

impl SessionManager {
    pub fn new() -> Self {
        SessionManager {
            sessions: HashMap::new(),
        }
    }

    pub fn register(&mut self, device_id: &str, transport: &str) {
        self.sessions.insert(device_id.to_string(), transport.to_string());
    }

    pub fn is_registered(&self, device_id: &str) -> bool {
        self.sessions.contains_key(device_id)
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
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
        assert_eq!(m.len(), 0);
    }

    #[test]
    fn manager_register() {
        let mut m = SessionManager::new();
        m.register("sw1", "ssh");
        assert!(m.is_registered("sw1"));
        assert_eq!(m.len(), 1);
    }
}
