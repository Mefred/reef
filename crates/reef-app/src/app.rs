//! Slint application wiring.
//!
//! Bridges Slint UI callbacks to the reef-session session manager and
//! reef-engine configuration engine. Uses a tokio runtime to drive async
//! device operations from the Slint event loop.

use std::cell::RefCell;
use std::rc::Rc;

slint::include_modules!();

use slint::{ComponentHandle, ModelRc, SharedString};

use crate::ui::ScreenController;
use reef_engine::model::{DeviceConfig, InterfaceConfig, InterfaceMode, Ipv4Address};
use reef_session::{ConnectionConfig, SessionManager};

fn mask_to_prefix_len(mask: std::net::Ipv4Addr) -> Option<u8> {
    let bits = u32::from_be_bytes(mask.octets());
    if bits == 0 {
        return Some(0);
    }
    let trailing = bits.trailing_zeros();
    if bits == (!0_u32 << trailing) {
        Some((32 - trailing) as u8)
    } else {
        None
    }
}

pub struct AppWindow {
    connection_screen: ConnectionScreen,
    main_window: MainWindow,
    controller: Rc<RefCell<ScreenController>>,
    session_manager: Rc<RefCell<SessionManager>>,
    runtime: tokio::runtime::Runtime,
    current_window: Rc<RefCell<WindowType>>,
}

enum WindowType {
    Connection,
    Device,
}

impl AppWindow {
    pub fn new() -> Result<Self, slint::PlatformError> {
        let connection_screen = ConnectionScreen::new()?;
        let main_window = MainWindow::new()?;

        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| slint::PlatformError::Other(e.to_string().into()))?;
        let rt = runtime.handle().clone();

        let session_manager = Rc::new(RefCell::new(SessionManager::new()));

        // Initial empty device config
        let current_config = reef_engine::model::DeviceConfig::default();
        let controller = Rc::new(RefCell::new(ScreenController::new(current_config)));

        // Bind session manager to controller with a device ID
        controller.borrow_mut().bind_session(
            session_manager.clone(),
            "device-1".to_string(),
        );

        // ===== Connection Screen callbacks =====

        // Connect device
        {
            let ctrl = controller.clone();
            let rt = rt.clone();
            let cs = connection_screen.clone_strong();
            connection_screen.on_connect_device(move |host, port, username, password, enable_secret, transport, _extra1, _extra2| {
                let ctrl = ctrl.clone();
                let rt = rt.clone();
                let cs = cs.clone_strong();
                rt.block_on(async move {
                    let mut c = ctrl.borrow_mut();
                    let mut config = match transport.as_ref() {
                        "Telnet" => ConnectionConfig::telnet(&host, &username),
                        "Serial" => ConnectionConfig::serial(&host),
                        _ => ConnectionConfig::ssh(&host, &username),
                    };
                    config.port = port.trim().parse().unwrap_or(22);
                    config.password = if password.is_empty() { None } else { Some(password.to_string()) };
                    config.enable_secret = if enable_secret.is_empty() { None } else { Some(enable_secret.to_string()) };

                    cs.set_status_text("Connecting...".into());
                    cs.set_connecting(true);

                    match c.connect_device(config).await {
                        Ok(_msg) => {
                            cs.set_status_text("Connected!".into());
                            cs.set_connecting(false);
                        }
                        Err(e) => {
                            cs.set_status_text(format!("Error: {}", e).into());
                            cs.set_connecting(false);
                        }
                    }
                });
            });
        }

        // Load profiles
        {
            let cs = connection_screen.clone_strong();
            connection_screen.on_load_profiles(move || -> slint::ModelRc<SharedString> {
                let names: Vec<SharedString> = vec!["Switch1".into(), "Router2".into()];
                let slint_names: slint::VecModel<SharedString> =
                    names.into_iter().collect();
                slint::ModelRc::new(Rc::new(slint_names))
            });
        }

        Ok(AppWindow {
            connection_screen,
            main_window,
            controller,
            session_manager,
            runtime,
            current_window: Rc::new(RefCell::new(WindowType::Connection)),
        })
    }

    pub fn show(self) -> Result<(), slint::PlatformError> {
        self.connection_screen.show()?;
        slint::run_event_loop()
    }
}
