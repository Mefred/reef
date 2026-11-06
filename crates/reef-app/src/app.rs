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

        // ---- connect_device ----
        {
            let ctrl = controller.clone();
            let rt = rt.clone();
            let win = window.clone_strong();
            window.on_connect_device(move |host: SharedString,
                                         port: SharedString,
                                         username: SharedString,
                                         password: SharedString,
                                         enable_secret: SharedString,
                                         transport: SharedString,
                                         _extra1: SharedString,
                                         _extra2: SharedString| {
                let ctrl = ctrl.clone();
                let rt = rt.clone();
                let win = win.clone_strong();
                rt.block_on(async move {
                    let mut c = ctrl.borrow_mut();
                    let mut config = match transport.as_ref() {
                        "Telnet" => ConnectionConfig::telnet(&host, &username),
                        "Serial" => ConnectionConfig::serial(&host),
                        _ => ConnectionConfig::ssh(&host, &username),
                    };
                    config.port = port.trim().parse().unwrap_or(22);
                    config.password = if password.is_empty() {
                        None
                    } else {
                        Some(password.to_string())
                    };
                    config.enable_secret = if enable_secret.is_empty() {
                        None
                    } else {
                        Some(enable_secret.to_string())
                    };
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
                    } else {
                        win.set_connection_status("Config saved to startup-config".into());
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
            let win = window.clone_strong();
            window.on_select_interface(move |name: SharedString| {
                let mut c = ctrl.borrow_mut();
                c.set_selected_interface(name.clone().into());
                let ip_full = c.interface_ip(&name).unwrap_or_default();
                let mask = c.interface_mask(&name).unwrap_or_default();
                let desc = c.interface_description(&name).unwrap_or_default();
                let enabled = c.interface_enabled(&name);
                win.set_selected_interface(name.clone());
                win.set_edit_description(desc.into());
                win.set_edit_enabled(enabled);
                win.set_edit_ip(ip_full.into());
                win.set_edit_mask(mask.into());
            });
        }

        // ---- save_interface ----
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

        // ---- load_vlans ----
        {
            let ctrl = controller.clone();
            window.on_load_vlans(move || -> slint::ModelRc<SharedString> {
                let names = ctrl.borrow().load_vlans();
                let slint_names: slint::VecModel<SharedString> =
                    names.iter().map(|s| s.as_str().into()).collect();
                slint::ModelRc::new(Rc::new(slint_names))
            });
        }

        // ---- select_vlan ----
        {
            let ctrl = controller.clone();
            let win = window.clone_strong();
            window.on_select_vlan(move |name: SharedString| {
                let mut c = ctrl.borrow_mut();
                let (vlan_id, desc, enabled) = c.select_vlan(&name);
                win.set_selected_vlan(vlan_id.into());
                win.set_edit_vlan_name(desc.into());
                win.set_edit_vlan_enabled(enabled);
            });
        }

        // ---- save_vlan ----
        {
            let ctrl = controller.clone();
            let win = window.clone_strong();
            window.on_save_vlan(move |name: SharedString,
                                     desc: SharedString,
                                     enabled: bool| {
                let mut c = ctrl.borrow_mut();
                let diff = c.save_vlan(&name, &desc, enabled);
                win.set_diff_text(diff.into());
                if c.is_disruptive() {
                    win.set_disruptive_warning(
                        "WARNING: VLAN changes may affect interface connectivity."
                            .into(),
                    );
                } else {
                    win.set_disruptive_warning("".into());
                }
            });
        }

        // ---- load_routes ----
        {
            let ctrl = controller.clone();
            window.on_load_routes(move || -> slint::ModelRc<SharedString> {
                let names = ctrl.borrow().load_routes();
                let slint_names: slint::VecModel<SharedString> =
                    names.iter().map(|s| s.as_str().into()).collect();
                slint::ModelRc::new(Rc::new(slint_names))
            });
        }

        // ---- save_route ----
        {
            let ctrl = controller.clone();
            let win = window.clone_strong();
            window.on_save_route(move |prefix: SharedString,
                                       next_hop: SharedString,
                                       distance: SharedString| {
                let mut c = ctrl.borrow_mut();
                let diff = c.save_route(&prefix, &next_hop, &distance, "");
                win.set_diff_text(diff.into());
                if c.is_disruptive() {
                    win.set_disruptive_warning(
                        "WARNING: Route changes may affect routing connectivity."
                            .into(),
                    );
                } else {
                    win.set_disruptive_warning("".into());
                }
            });
        }

        // ---- load_acls ----
        {
            let ctrl = controller.clone();
            window.on_load_acls(move || -> slint::ModelRc<SharedString> {
                let names = ctrl.borrow().load_acls();
                let slint_names: slint::VecModel<SharedString> =
                    names.iter().map(|s| s.as_str().into()).collect();
                slint::ModelRc::new(Rc::new(slint_names))
            });
        }

        // ---- select_acl ----
        {
            let ctrl = controller.clone();
            let win = window.clone_strong();
            window.on_select_acl(move |name: SharedString| {
                let c = ctrl.borrow();
                let (acl_name, action, protocol, source, destination) =
                    c.select_acl(&name);
                win.set_edit_acl_name(acl_name.into());
                win.set_edit_acl_action(action.into());
                win.set_edit_acl_protocol(protocol.into());
                win.set_edit_acl_source(source.into());
                win.set_edit_acl_dest(destination.into());
            });
        }

        // ---- save_acl_rule ----
        {
            let ctrl = controller.clone();
            let win = window.clone_strong();
            window.on_save_acl_rule(
                move |name: SharedString,
                      action: SharedString,
                      protocol: SharedString,
                      source: SharedString,
                      destination: SharedString| {
                let mut c = ctrl.borrow_mut();
                let diff = c.save_acl_rule(
                    &name,
                    &action,
                    &protocol,
                    &source,
                    &destination,
                    "",
                    "",
                );
                win.set_diff_text(diff.into());
                if c.is_disruptive() {
                    win.set_disruptive_warning(
                        "WARNING: ACL changes may affect traffic filtering."
                            .into(),
                    );
                } else {
                    win.set_disruptive_warning("".into());
                }
            });
        }

        // ---- load_dhcp_pools ----
        {
            let ctrl = controller.clone();
            window.on_load_dhcp_pools(move || -> slint::ModelRc<SharedString> {
                let names = ctrl.borrow().load_dhcp_pools();
                let slint_names: slint::VecModel<SharedString> =
                    names.iter().map(|s| s.as_str().into()).collect();
                slint::ModelRc::new(Rc::new(slint_names))
            });
        }

        // ---- select_dhcp_pool ----
        {
            let ctrl = controller.clone();
            let win = window.clone_strong();
            window.on_select_dhcp_pool(move |name: SharedString| {
                let c = ctrl.borrow();
                let (pool_name, network, mask, router, dns, lease) =
                    c.select_dhcp_pool(&name);
                win.set_edit_dhcp_name(pool_name.into());
                win.set_edit_dhcp_network(network.into());
                win.set_edit_dhcp_mask(mask.into());
                win.set_edit_dhcp_router(router.into());
                win.set_edit_dhcp_dns(dns.into());
                win.set_edit_dhcp_lease(lease.into());
            });
        }

        // ---- save_dhcp_pool ----
        {
            let ctrl = controller.clone();
            let win = window.clone_strong();
            window.on_save_dhcp_pool(
                move |name: SharedString,
                      network: SharedString,
                      mask: SharedString,
                      router: SharedString,
                      dns: SharedString,
                      lease: SharedString| {
                    let mut c = ctrl.borrow_mut();
                    let diff = c.save_dhcp_pool(
                        &name,
                        &network,
                        &mask,
                        &router,
                        &dns,
                        &lease,
                    );
                    win.set_diff_text(diff.into());
                    if c.is_disruptive() {
                        win.set_disruptive_warning(
                            "WARNING: DHCP changes may affect IP assignment."
                                .into(),
                        );
                    } else {
                        win.set_disruptive_warning("".into());
                    }
                },
            );
        }

        // ---- load_nat_config ----
        {
            let ctrl = controller.clone();
            let win = window.clone_strong();
            window.on_load_nat_config(move || -> slint::SharedString {
                let (nat_type, inside, outside) =
                    ctrl.borrow().load_nat_config();
                format!("{}|{}|{}", nat_type, inside, outside).into()
            });
        }

        // ---- save_nat_config ----
        {
            let ctrl = controller.clone();
            let win = window.clone_strong();
            window.on_save_nat_config(move |nat_type: SharedString,
                                            inside: SharedString,
                                            outside: SharedString| {
                let mut c = ctrl.borrow_mut();
                let diff = c.save_nat_config(
                    &nat_type,
                    &inside,
                    &outside,
                    "",
                );
                win.set_diff_text(diff.into());
                if c.is_disruptive() {
                    win.set_disruptive_warning(
                        "WARNING: NAT changes may affect traffic translation."
                            .into(),
                    );
                } else {
                    win.set_disruptive_warning("".into());
                }
            });
        }

        // ---- load_ssh_config ----
        {
            let ctrl = controller.clone();
            let win = window.clone_strong();
            window.on_load_ssh_config(move || -> slint::SharedString {
                let (enabled, version, timeout, retries) =
                    ctrl.borrow().load_ssh_config();
                format!("{}|{}|{}|{}", enabled, version, timeout, retries).into()
            });
        }

        // ---- save_ssh_config ----
        {
            let ctrl = controller.clone();
            let win = window.clone_strong();
            window.on_save_ssh_config(
                move |enabled: bool,
                      version: SharedString,
                      timeout: SharedString,
                      retries: SharedString| {
                    let mut c = ctrl.borrow_mut();
                    let diff = c.save_ssh_config(
                        enabled,
                        &version,
                        &timeout,
                        &retries,
                    );
                    win.set_diff_text(diff.into());
                    if c.is_disruptive() {
                        win.set_disruptive_warning(
                            "WARNING: SSH access changes may affect management access."
                                .into(),
                        );
                    } else {
                        win.set_disruptive_warning("".into());
                    }
                },
            );
        }

        // ---- edit_ssh_enabled_changed ----
        {
            let ctrl = controller.clone();
            let win = window.clone_strong();
            window.on_edit_ssh_enabled_changed(move |enabled: bool| {
                let mut c = ctrl.borrow_mut();
                let diff = c.save_ssh_config(
                    enabled,
                    &win.get_edit_ssh_version(),
                    &win.get_edit_ssh_timeout(),
                    &win.get_edit_ssh_retries(),
                );
                win.set_diff_text(diff.into());
                if c.is_disruptive() {
                    win.set_disruptive_warning(
                        "WARNING: SSH access changes may affect management access."
                            .into(),
                    );
                } else {
                    win.set_disruptive_warning("".into());
                }
            });
        }

        // ---- load_system_config ----
        {
            let ctrl = controller.clone();
            let win = window.clone_strong();
            window.on_load_system_config(move || -> slint::SharedString {
                let (hostname, domain, dns) = ctrl.borrow().load_system_config();
                format!("{}|{}|{}", hostname, domain, dns).into()
            });
        }

        // ---- save_system_config ----
        {
            let ctrl = controller.clone();
            let win = window.clone_strong();
            window.on_save_system_config(move |hostname: SharedString,
                                              domain: SharedString,
                                              dns: SharedString| {
                let mut c = ctrl.borrow_mut();
                let diff = c.save_system_config(&hostname, &domain, &dns);
                win.set_diff_text(diff.into());
                if c.is_disruptive() {
                    win.set_disruptive_warning(
                        "WARNING: System changes may affect device identification."
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
                            win.set_connection_status(
                                format!("rollback error: {}", e).into(),
                            );
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
        self.window.show()?;
        slint::run_event_loop()
    }
}
