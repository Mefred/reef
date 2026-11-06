//! Slint application wiring.
//!
//! Bridges Slint UI callbacks to the reef-session session manager and
//! reef-engine configuration engine. Uses a tokio runtime to drive async
//! device operations from the Slint event loop.

use std::cell::RefCell;
use std::rc::Rc;

slint::include_modules!();

use slint::{ModelRc, SharedString};

use crate::ui::ScreenController;

pub struct AppWindow {
    window: MainWindow,
    controller: Rc<RefCell<ScreenController>>,
    runtime: tokio::runtime::Runtime,
}

impl AppWindow {
    pub fn new() -> Result<Self, slint::PlatformError> {
        let window = MainWindow::new()?;

        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| slint::PlatformError::Other(e.to_string().into()))?;

        // Initial empty device config
        let current_config = reef_engine::model::DeviceConfig::default();
        let controller = Rc::new(RefCell::new(ScreenController::new(current_config)));

        // ---- connect_device: host, username, password ----
        {
            let ctrl = controller.clone();
            let handle = runtime.handle().clone();
            window.on_connect_device(move |host: SharedString, username: SharedString, _password: SharedString| {
                let ctrl = ctrl.clone();
                let handle = handle.clone();
                let _ = (ctrl, handle, host, username);
                // Placeholder - real connection management via SessionManager
            });
        }

        // ---- disconnect_device ----
        {
            let ctrl = controller.clone();
            window.on_disconnect_device(move || {
                ctrl.borrow_mut().set_has_session(false);
            });
        }

        // ---- save_config ----
        {
            let _ctrl = controller.clone();
            window.on_save_config(move || {
                // Placeholder - copy running-config to startup-config
            });
        }

        // ---- refresh_config ----
        {
            let _ctrl = controller.clone();
            window.on_refresh_config(move || {
                // Placeholder - re-fetch running-config from device
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
            window.on_save_interface(move |name: SharedString,
                                             desc: SharedString,
                                             enabled: bool,
                                             ip: SharedString,
                                             mask: SharedString| {
                let mut c = ctrl.borrow_mut();
                let _diff = c.update_interface(
                    &name,
                    if desc.is_empty() { None } else { Some(desc.into()) },
                    enabled,
                    if ip.is_empty() { None } else { Some(&ip) },
                    if mask.is_empty() { None } else { Some(&mask) },
                );
                let _ = _diff;
            });
        }

        // ---- preview_config ----
        {
            let ctrl = controller.clone();
            window.on_preview_config(move || {
                let _diff = ctrl.borrow().diff().to_string();
            });
        }

        // ---- confirm_disruptive ----
        {
            let ctrl = controller.clone();
            window.on_confirm_disruptive(move |_confirmation: SharedString| {
                let ctrl = ctrl.clone();
                let current = ctrl.borrow().current_config().clone();
                let desired = ctrl.borrow().desired_config().clone();
                drop(ctrl);
                let change_set = reef_engine::planner::plan(&current, &desired);
                let _batch = reef_engine::generator::generate(&change_set);
            });
        }

        // ---- apply_config ----
        {
            let ctrl = controller.clone();
            window.on_apply_config(move || {
                let c = ctrl.borrow();
                let is_disruptive = c.is_disruptive();
                let _diff = c.diff().to_string();
                drop(c);
                if is_disruptive {
                    ctrl.borrow_mut().set_has_session(false);
                } else {
                    // Non-disruptive apply
                }
            });
        }

        // ---- rollback_config ----
        {
            window.on_rollback_config(move || {
                // Placeholder - live rollback requires device session
            });
        }

        Ok(AppWindow {
            window,
            controller,
            runtime,
        })
    }

    pub fn show(self) -> Result<(), slint::PlatformError> {
        self.window.show()
    }
}
