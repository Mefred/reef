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
use reef_session::{ConnectionConfig, SessionManager};

pub struct AppWindow {
    window: MainWindow,
    controller: Rc<RefCell<ScreenController>>,
    session_manager: Rc<RefCell<SessionManager>>,
    runtime: tokio::runtime::Runtime,
}

impl AppWindow {
    pub fn new() -> Result<Self, slint::PlatformError> {
        let window = MainWindow::new()?;

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

        // ---- connect_device: host, username, password ----
        {
            let ctrl = controller.clone();
            let rt = rt.clone();
            let win = window.clone_strong();
            window.on_connect_device(move |host: SharedString, username: SharedString, password: SharedString| {
                let ctrl = ctrl.clone();
                let rt = rt.clone();
                let win = win.clone_strong();
                rt.block_on(async move {
                    let mut c = ctrl.borrow_mut();
                    let mut config = ConnectionConfig::telnet(&host, &username);
                    if !password.is_empty() {
                        config.password = Some(password.to_string());
                    }
                    match c.connect_device(config).await {
                        Ok(msg) => {
                            win.set_connection_status("connected".into());
                            win.set_device_hostname(c.current_config().hostname.clone().unwrap_or_else(|| host.to_string()).into());
                            win.set_is_connected(true);
                            let _ = msg;
                        }
                        Err(e) => {
                            win.set_connection_status(format!("error: {}", e).into());
                        }
                    }
                });
            });
        }

        // ---- disconnect_device ----
        {
            let ctrl = controller.clone();
            let rt = rt.clone();
            let win = window.clone_strong();
            window.on_disconnect_device(move || {
                let ctrl = ctrl.clone();
                let rt = rt.clone();
                let win = win.clone_strong();
                rt.block_on(async move {
                    let mut c = ctrl.borrow_mut();
                    if let Err(e) = c.disconnect_device().await {
                        win.set_connection_status(format!("disconnect error: {}", e).into());
                    } else {
                        win.set_connection_status("disconnected".into());
                        win.set_is_connected(false);
                        win.set_device_hostname("-".into());
                        c.unbind_session();
                    }
                });
            });
        }

        // ---- save_config ----
        {
            let ctrl = controller.clone();
            let rt = rt.clone();
            let win = window.clone_strong();
            window.on_save_config(move || {
                let ctrl = ctrl.clone();
                let rt = rt.clone();
                let win = win.clone_strong();
                rt.block_on(async move {
                    let mut c = ctrl.borrow_mut();
                    if let Err(e) = c.save_config().await {
                        win.set_connection_status(format!("save error: {}", e).into());
                    }
                });
            });
        }

        // ---- refresh_config ----
        {
            let ctrl = controller.clone();
            let rt = rt.clone();
            let win = window.clone_strong();
            window.on_refresh_config(move || {
                let ctrl = ctrl.clone();
                let rt = rt.clone();
                let win = win.clone_strong();
                rt.block_on(async move {
                    let mut c = ctrl.borrow_mut();
                    match c.refresh_config().await {
                        Ok(diff) => {
                            win.set_diff_text(diff.into());
                        }
                        Err(e) => {
                            win.set_connection_status(format!("refresh error: {}", e).into());
                        }
                    }
                });
            });
        }

        // ---- load_interfaces ----
        {
            let ctrl = controller.clone();
            window.on_load_interfaces(move || -> slint::ModelRc<SharedString> {
                let names = ctrl.borrow().interface_names();
                let slint_names: slint::VecModel<SharedString> =
                    names.iter().map(|s| s.as_str().into()).collect();
                slint::ModelRc::new(Rc::new(slint_names))
            });
        }

        // ---- select_interface ----
        {
            let ctrl = controller.clone();
            window.on_select_interface(move |name: SharedString| {
                let c = ctrl.borrow();
                let _ip = c.interface_ip(&name).unwrap_or_default();
                let _desc = c.interface_description(&name).unwrap_or_default();
                let _enabled = c.interface_enabled(&name);
                let _ = (c, _ip, _desc, _enabled);
            });
        }

        // ---- save_interface: update desired config + re-diff ----
        {
            let ctrl = controller.clone();
            let win = window.clone_strong();
            window.on_save_interface(move |name: SharedString,
                                             desc: SharedString,
                                             enabled: bool,
                                             ip: SharedString,
                                             mask: SharedString| {
                let mut c = ctrl.borrow_mut();
                let diff = c.update_interface(
                    &name,
                    if desc.is_empty() { None } else { Some(desc.into()) },
                    enabled,
                    if ip.is_empty() { None } else { Some(&ip) },
                    if mask.is_empty() { None } else { Some(&mask) },
                );
                win.set_diff_text(diff.into());
                if c.is_disruptive() {
                    win.set_disruptive_warning(
                        "WARNING: This change modifies an interface with an IP address. \
                         This may disrupt management connectivity."
                            .into(),
                    );
                } else {
                    win.set_disruptive_warning("".into());
                }
            });
        }

        // ---- preview_config ----
        {
            let ctrl = controller.clone();
            let win = window.clone_strong();
            window.on_preview_config(move || {
                let diff = ctrl.borrow().diff().to_string();
                win.set_diff_text(diff.into());
            });
        }

        // ---- confirm_disruptive ----
        {
            let ctrl = controller.clone();
            let rt = rt.clone();
            let win = window.clone_strong();
            window.on_confirm_disruptive(move |_confirmation: SharedString| {
                let ctrl = ctrl.clone();
                let rt = rt.clone();
                let win = win.clone_strong();
                rt.block_on(async move {
                    let mut c = ctrl.borrow_mut();
                    match c.apply_changes().await {
                        Ok(msg) => {
                            win.set_diff_text(msg.into());
                            win.set_disruptive_warning("".into());
                        }
                        Err(e) => {
                            win.set_connection_status(format!("apply error: {}", e).into());
                        }
                    }
                });
            });
        }

        // ---- apply_config ----
        {
            let ctrl = controller.clone();
            let rt = rt.clone();
            let win = window.clone_strong();
            window.on_apply_config(move || {
                let ctrl = ctrl.clone();
                let rt = rt.clone();
                let win = win.clone_strong();
                rt.block_on(async move {
                    let mut c = ctrl.borrow_mut();
                    if c.is_disruptive() {
                        win.set_disruptive_warning(
                            "Disruptive change detected. Confirm to proceed.".into(),
                        );
                    } else {
                        match c.apply_changes().await {
                            Ok(msg) => {
                                win.set_diff_text(msg.into());
                            }
                            Err(e) => {
                                win.set_connection_status(format!("apply error: {}", e).into());
                            }
                        }
                    }
                });
            });
        }

        // ---- rollback_config ----
        {
            let ctrl = controller.clone();
            let rt = rt.clone();
            let win = window.clone_strong();
            window.on_rollback_config(move || {
                let ctrl = ctrl.clone();
                let rt = rt.clone();
                let win = win.clone_strong();
                rt.block_on(async move {
                    let mut c = ctrl.borrow_mut();
                    match c.rollback_changes().await {
                        Ok(msg) => {
                            win.set_diff_text(msg.into());
                            win.set_disruptive_warning("".into());
                        }
                        Err(e) => {
                            win.set_connection_status(format!("rollback error: {}", e).into());
                        }
                    }
                });
            });
        }

        Ok(AppWindow {
            window,
            controller,
            session_manager,
            runtime,
        })
    }

    pub fn show(self) -> Result<(), slint::PlatformError> {
        self.window.show()
    }
}
