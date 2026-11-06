//! Slint application wiring (Phase 5).
//!
//! Wraps the generated Slint `MainWindow` component and bridges callbacks
//! to the `ScreenController` (engine planning/generation).

use std::cell::RefCell;
use std::rc::Rc;

slint::include_modules!();

use slint::{ModelRc, SharedString};

use crate::ui::ScreenController;

pub struct AppWindow {
    window: MainWindow,
    controller: Rc<RefCell<ScreenController>>,
}

impl AppWindow {
    pub fn new() -> Result<Self, slint::PlatformError> {
        let window = MainWindow::new()?;

        // Initial device config (mock - will come from device identification in Phase 6+)
        let current_config = reef_engine::model::DeviceConfig::default();
        let controller = Rc::new(RefCell::new(ScreenController::new(current_config)));

        // ---- connect_device ----
        window.on_connect_device(move || {
            // Placeholder - actual SSH/Telnet/Serial connection handled in Phase 6+
        });

        // ---- disconnect_device ----
        window.on_disconnect_device(move || {
            // Placeholder
        });

        // ---- save_config ----
        window.on_save_config(move || {
            // Placeholder - copy running-config to startup-config
        });

        // ---- refresh_config ----
        window.on_refresh_config(move || {
            // Placeholder - re-fetch running-config from device
        });

        // ---- load_interfaces: returns list of interface names ----
        {
            let ctrl = controller.clone();
            window.on_load_interfaces(move || -> ModelRc<SharedString> {
                let names = ctrl.borrow().interface_names();
                let slint_names: Vec<SharedString> =
                    names.iter().map(|s| s.as_str().into()).collect();
                let model = slint::VecModel::from(slint_names);
                ModelRc::new(Rc::new(model))
            });
        }

        // ---- select_interface: populate edit form ----
        {
            let ctrl = controller.clone();
            window.on_select_interface(move |name: SharedString| {
                let c = ctrl.borrow();
                let ip = c.interface_ip(&name).unwrap_or_default();
                let desc = c.interface_description(&name).unwrap_or_default();
                let enabled = c.interface_enabled(&name);
                // Note: Slint can't set sibling properties from here directly;
                // the controller stores state; UI re-reads on next render.
                let _ = (c, ip, desc, enabled);
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
                let diff = c.update_interface(
                    &name,
                    if desc.is_empty() { None } else { Some(desc.into()) },
                    enabled,
                    if ip.is_empty() { None } else { Some(&ip) },
                    if mask.is_empty() { None } else { Some(&mask) },
                );
                // Diff is stored in controller; UI reads via show_diff
                let _ = diff;
            });
        }

        Ok(AppWindow {
            window,
            controller,
        })
    }

    pub fn show(self) -> Result<(), slint::PlatformError> {
        self.window.show()
    }
}
