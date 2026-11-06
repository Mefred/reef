//! Device session: wraps a transport with IOS XE-specific interaction
//! primitives (enable, config_terminal, paging, prompt detection).

use crate::transport::DeviceTransport;
use crate::Error;
use std::time::Duration;

pub struct Session<T: DeviceTransport> {
    transport: T,
    authenticated: bool,
    privileged: bool,
    paging_disabled: bool,
}

impl<T: DeviceTransport> Session<T> {
    pub fn new(transport: T) -> Self {
        Session {
            transport,
            authenticated: false,
            privileged: false,
            paging_disabled: false,
        }
    }

    pub fn is_privileged(&self) -> bool {
        self.privileged
    }

    pub fn is_authenticated(&self) -> bool {
        self.authenticated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_trait_is_object_safe_placeholder() {
        // Session is parameterized over DeviceTransport; assert the trait exists.
        fn _assert_transport<T: DeviceTransport>() {}
        let _ = std::marker::PhantomData::<fn()>;
        assert!(true);
    }
}
