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
        controller
            .borrow_mut()
            .bind_session(session_manager.clone(), "device-1".to_string());

        // ===== Connection Screen callbacks =====

        // Connect device
        {
            let ctrl = controller.clone();
            let rt = rt.clone();
            let cs = connection_screen.clone_strong();
            let mw = main_window.clone_strong();
            connection_screen.on_connect_device(
                move |host,
                      port,
                      username,
                      password,
                      enable_secret,
                      transport,
                      _extra1,
                      _extra2| {
                    let ctrl = ctrl.clone();
                    let rt = rt.clone();
                    let cs = cs.clone_strong();
                    let mw = mw.clone_strong();
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

                        cs.set_status_text("Connecting...".into());
                        cs.set_connecting(true);

                        match c.connect_device(config).await {
                            Ok(_msg) => {
                                cs.set_status_text("Connected!".into());
                                cs.set_connecting(false);

                                // Populate device info from controller
                                let cc = c.current_config();
                                mw.set_device_hostname(
                                    cc.hostname
                                        .clone()
                                        .unwrap_or_else(|| host.to_string())
                                        .into(),
                                );
                                mw.set_device_model("Cisco IOS XE".into());
                                mw.set_device_version("Unknown".into());
                                mw.set_connection_status("connected".into());
                                mw.set_is_connected(true);
                                mw.set_diff_text("No pending changes.".into());
                                mw.set_disruptive_warning("".into());
                                mw.set_has_pending_changes(false);
                                mw.set_pending_text("No pending changes".into());

                                // Switch windows
                                let _ = cs.hide();
                                let _ = mw.show();
                            }
                            Err(e) => {
                                cs.set_status_text(format!("Error: {}", e).into());
                                cs.set_connecting(false);
                            }
                        }
                    });
                },
            );
        }

        // Load profiles
        {
            let cs = connection_screen.clone_strong();
            connection_screen.on_load_profiles(move || -> slint::ModelRc<SharedString> {
                let names: Vec<SharedString> = vec!["Switch1".into(), "Router2".into()];
                let slint_names: slint::VecModel<SharedString> = names.into_iter().collect();
                slint::ModelRc::new(Rc::new(slint_names))
            });
        }

        // ===== Main Window callbacks =====

        // Disconnect device
        {
            let ctrl = controller.clone();
            let rt = rt.clone();
            let mw = main_window.clone_strong();
            main_window.on_disconnect_device(move || {
                let ctrl = ctrl.clone();
                let rt = rt.clone();
                let mw = mw.clone_strong();
                rt.block_on(async move {
                    let mut c = ctrl.borrow_mut();
                    match c.disconnect_device().await {
                        Ok(_) => {
                            mw.set_connection_status("disconnected".into());
                            mw.set_is_connected(false);
                            mw.set_device_hostname("-".into());
                            mw.set_device_model("-".into());
                            mw.set_device_version("-".into());
                            mw.set_device_uptime("-".into());
                            mw.set_diff_text("No pending changes.".into());
                            mw.set_disruptive_warning("".into());
                            mw.set_has_pending_changes(false);
                            mw.set_pending_text("No pending changes".into());
                        }
                        Err(e) => {
                            mw.set_connection_status(format!("Error: {}", e).into());
                        }
                    }
                });
            });
        }

        // Refresh device
        {
            let ctrl = controller.clone();
            let rt = rt.clone();
            let mw = main_window.clone_strong();
            main_window.on_refresh_device(move || {
                let ctrl = ctrl.clone();
                let rt = rt.clone();
                let mw = mw.clone_strong();
                rt.block_on(async move {
                    let mut c = ctrl.borrow_mut();
                    match c.refresh_config().await {
                        Ok(diff) => {
                            mw.set_diff_text(diff.into());
                        }
                        Err(e) => {
                            mw.set_connection_status(format!("Error: {}", e).into());
                        }
                    }
                });
            });
        }

        // Save config
        {
            let ctrl = controller.clone();
            let rt = rt.clone();
            let mw = main_window.clone_strong();
            main_window.on_save_config(move || {
                let ctrl = ctrl.clone();
                let rt = rt.clone();
                let mw = mw.clone_strong();
                rt.block_on(async move {
                    let mut c = ctrl.borrow_mut();
                    match c.save_config().await {
                        Ok(_) => {
                            mw.set_connection_status("Config saved".into());
                        }
                        Err(e) => {
                            mw.set_connection_status(format!("Error: {}", e).into());
                        }
                    }
                });
            });
        }

        // Preview config
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_preview_config(move || {
                let c = ctrl.borrow();
                mw.set_diff_text(c.diff().to_string().into());
            });
        }

        // Apply config
        {
            let ctrl = controller.clone();
            let rt = rt.clone();
            let mw = main_window.clone_strong();
            main_window.on_apply_config(move || {
                let ctrl = ctrl.clone();
                let rt = rt.clone();
                let mw = mw.clone_strong();
                rt.block_on(async move {
                    let mut c = ctrl.borrow_mut();
                    match c.apply_changes().await {
                        Ok(msg) => {
                            mw.set_diff_text(msg.into());
                            mw.set_disruptive_warning("".into());
                            mw.set_has_pending_changes(false);
                            mw.set_pending_text("No pending changes".into());
                        }
                        Err(e) => {
                            mw.set_connection_status(format!("Error: {}", e).into());
                        }
                    }
                });
            });
        }

        // Rollback config
        {
            let ctrl = controller.clone();
            let rt = rt.clone();
            let mw = main_window.clone_strong();
            main_window.on_rollback_config(move || {
                let ctrl = ctrl.clone();
                let rt = rt.clone();
                let mw = mw.clone_strong();
                rt.block_on(async move {
                    let mut c = ctrl.borrow_mut();
                    match c.rollback_changes().await {
                        Ok(msg) => {
                            mw.set_diff_text(msg.into());
                            mw.set_disruptive_warning("".into());
                            mw.set_has_pending_changes(false);
                            mw.set_pending_text("No pending changes".into());
                        }
                        Err(e) => {
                            mw.set_connection_status(format!("Error: {}", e).into());
                        }
                    }
                });
            });
        }

        // Confirm disruptive
        {
            let ctrl = controller.clone();
            let rt = rt.clone();
            let mw = main_window.clone_strong();
            main_window.on_confirm_disruptive(move |_confirmation: SharedString| {
                let ctrl = ctrl.clone();
                let rt = rt.clone();
                let mw = mw.clone_strong();
                rt.block_on(async move {
                    let mut c = ctrl.borrow_mut();
                    match c.apply_changes().await {
                        Ok(msg) => {
                            mw.set_diff_text(msg.into());
                            mw.set_disruptive_warning("".into());
                            mw.set_has_pending_changes(false);
                            mw.set_pending_text("No pending changes".into());
                        }
                        Err(e) => {
                            mw.set_connection_status(format!("Error: {}", e).into());
                        }
                    }
                });
            });
        }

        // ===== Interface callbacks =====

        // Load interfaces
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_load_interfaces(move || -> slint::ModelRc<SharedString> {
                let names = ctrl.borrow().interface_names();
                let slint_names: slint::VecModel<SharedString> =
                    names.iter().map(|s| s.as_str().into()).collect();
                slint::ModelRc::new(Rc::new(slint_names))
            });
        }

        // Select interface
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_select_interface(move |name: SharedString| {
                let mut c = ctrl.borrow_mut();
                c.set_selected_interface(name.clone().into());
                let ip_full = c.interface_ip(&name).unwrap_or_default();
                let mask = c.interface_mask(&name).unwrap_or_default();
                let desc = c.interface_description(&name).unwrap_or_default();
                let enabled = c.interface_enabled(&name);
                mw.set_selected_interface(name.clone());
                mw.set_edit_description(desc.into());
                mw.set_edit_enabled(enabled);
                mw.set_edit_ip(ip_full.into());
                mw.set_edit_mask(mask.into());
            });
        }

        // Save interface
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_save_interface(
                move |name: SharedString,
                      desc: SharedString,
                      enabled: bool,
                      ip: SharedString,
                      mask: SharedString| {
                    let mut c = ctrl.borrow_mut();
                    let diff = c.update_interface(
                        &name,
                        if desc.is_empty() {
                            None
                        } else {
                            Some(desc.into())
                        },
                        enabled,
                        if ip.is_empty() { None } else { Some(&ip) },
                        if mask.is_empty() { None } else { Some(&mask) },
                    );
                    mw.set_diff_text(diff.into());
                    if c.is_disruptive() {
                        mw.set_disruptive_warning(
                            "WARNING: This change modifies an interface with an IP address. \
                         This may disrupt management connectivity."
                                .into(),
                        );
                        mw.set_has_pending_changes(true);
                        mw.set_pending_text("Unsaved changes pending".into());
                    } else {
                        mw.set_disruptive_warning("".into());
                    }
                },
            );
        }

        // ===== VLAN callbacks =====

        // Load VLANs
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_load_vlans(move || -> slint::ModelRc<SharedString> {
                let names = ctrl.borrow().load_vlans();
                let slint_names: slint::VecModel<SharedString> =
                    names.iter().map(|s| s.as_str().into()).collect();
                slint::ModelRc::new(Rc::new(slint_names))
            });
        }

        // Select VLAN
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_select_vlan(move |name: SharedString| {
                let mut c = ctrl.borrow_mut();
                let (vlan_id, desc, enabled) = c.select_vlan(&name);
                mw.set_selected_vlan(vlan_id.into());
                mw.set_edit_vlan_name(desc.into());
                mw.set_edit_vlan_enabled(enabled);
            });
        }

        // Save VLAN
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_save_vlan(
                move |name: SharedString, desc: SharedString, enabled: bool| {
                    let mut c = ctrl.borrow_mut();
                    let diff = c.save_vlan(&name, &desc, enabled);
                    mw.set_diff_text(diff.into());
                    if c.is_disruptive() {
                        mw.set_disruptive_warning(
                            "WARNING: VLAN changes may affect interface connectivity.".into(),
                        );
                        mw.set_has_pending_changes(true);
                        mw.set_pending_text("Unsaved changes pending".into());
                    } else {
                        mw.set_disruptive_warning("".into());
                    }
                },
            );
        }

        // ===== Route callbacks =====

        // Load routes
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_load_routes(move || -> slint::ModelRc<SharedString> {
                let names = ctrl.borrow().load_routes();
                let slint_names: slint::VecModel<SharedString> =
                    names.iter().map(|s| s.as_str().into()).collect();
                slint::ModelRc::new(Rc::new(slint_names))
            });
        }

        // Save route
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_save_route(
                move |prefix: SharedString,
                      next_hop: SharedString,
                      distance: SharedString,
                      _permanent: SharedString| {
                    let mut c = ctrl.borrow_mut();
                    let diff = c.save_route(&prefix, &next_hop, &distance, &_permanent);
                    mw.set_diff_text(diff.into());
                    if c.is_disruptive() {
                        mw.set_disruptive_warning(
                            "WARNING: Route changes may affect routing connectivity.".into(),
                        );
                        mw.set_has_pending_changes(true);
                        mw.set_pending_text("Unsaved changes pending".into());
                    } else {
                        mw.set_disruptive_warning("".into());
                    }
                },
            );
        }

        // ===== ACL callbacks =====

        // Load ACLs
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_load_acls(move || -> slint::ModelRc<SharedString> {
                let names = ctrl.borrow().load_acls();
                let slint_names: slint::VecModel<SharedString> =
                    names.iter().map(|s| s.as_str().into()).collect();
                slint::ModelRc::new(Rc::new(slint_names))
            });
        }

        // Select ACL
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_select_acl(move |name: SharedString| {
                let c = ctrl.borrow();
                let (acl_name, action, protocol, source, destination) = c.select_acl(&name);
                mw.set_edit_acl_name(acl_name.into());
                mw.set_edit_acl_action(action.into());
                mw.set_edit_acl_protocol(protocol.into());
                mw.set_edit_acl_source(source.into());
                mw.set_edit_acl_dest(destination.into());
            });
        }

        // Save ACL rule
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_save_acl_rule(
                move |name: SharedString,
                      action: SharedString,
                      protocol: SharedString,
                      source: SharedString,
                      destination: SharedString,
                      ports: SharedString,
                      wildcard: SharedString| {
                    let mut c = ctrl.borrow_mut();
                    let diff = c.save_acl_rule(
                        &name,
                        &action,
                        &protocol,
                        &source,
                        &destination,
                        &ports,
                        &wildcard,
                    );
                    mw.set_diff_text(diff.into());
                    if c.is_disruptive() {
                        mw.set_disruptive_warning(
                            "WARNING: ACL changes may affect traffic filtering.".into(),
                        );
                        mw.set_has_pending_changes(true);
                        mw.set_pending_text("Unsaved changes pending".into());
                    } else {
                        mw.set_disruptive_warning("".into());
                    }
                },
            );
        }

        // ===== DHCP callbacks =====

        // Load DHCP pools
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_load_dhcp_pools(move || -> slint::ModelRc<SharedString> {
                let names = ctrl.borrow().load_dhcp_pools();
                let slint_names: slint::VecModel<SharedString> =
                    names.iter().map(|s| s.as_str().into()).collect();
                slint::ModelRc::new(Rc::new(slint_names))
            });
        }

        // Select DHCP pool
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_select_dhcp_pool(move |name: SharedString| {
                let c = ctrl.borrow();
                let (pool_name, network, mask, router, dns, lease) = c.select_dhcp_pool(&name);
                mw.set_edit_dhcp_name(pool_name.into());
                mw.set_edit_dhcp_network(network.into());
                mw.set_edit_dhcp_mask(mask.into());
                mw.set_edit_dhcp_router(router.into());
                mw.set_edit_dhcp_dns(dns.into());
                mw.set_edit_dhcp_lease(lease.into());
            });
        }

        // Save DHCP pool
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_save_dhcp_pool(
                move |name: SharedString,
                      network: SharedString,
                      mask: SharedString,
                      router: SharedString,
                      dns: SharedString,
                      lease: SharedString| {
                    let mut c = ctrl.borrow_mut();
                    let diff = c.save_dhcp_pool(&name, &network, &mask, &router, &dns, &lease);
                    mw.set_diff_text(diff.into());
                    if c.is_disruptive() {
                        mw.set_disruptive_warning(
                            "WARNING: DHCP changes may affect IP assignment.".into(),
                        );
                        mw.set_has_pending_changes(true);
                        mw.set_pending_text("Unsaved changes pending".into());
                    } else {
                        mw.set_disruptive_warning("".into());
                    }
                },
            );
        }

        // ===== NAT callbacks =====

        // Load NAT config
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_load_nat_config(move || -> slint::SharedString {
                let (nat_type, inside, outside) = ctrl.borrow().load_nat_config();
                format!("{}|{}|{}", nat_type, inside, outside).into()
            });
        }

        // Save NAT config
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_save_nat_config(
                move |nat_type: SharedString,
                      inside: SharedString,
                      outside: SharedString,
                      _extra: SharedString| {
                    let mut c = ctrl.borrow_mut();
                    let diff = c.save_nat_config(&nat_type, &inside, &outside, &_extra);
                    mw.set_diff_text(diff.into());
                    if c.is_disruptive() {
                        mw.set_disruptive_warning(
                            "WARNING: NAT changes may affect traffic translation.".into(),
                        );
                        mw.set_has_pending_changes(true);
                        mw.set_pending_text("Unsaved changes pending".into());
                    } else {
                        mw.set_disruptive_warning("".into());
                    }
                },
            );
        }

        // ===== SSH callbacks =====

        // Load SSH config
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_load_ssh_config(move || -> slint::SharedString {
                let (enabled, version, timeout, retries) = ctrl.borrow().load_ssh_config();
                format!("{}|{}|{}|{}", enabled, version, timeout, retries).into()
            });
        }

        // Save SSH config
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_save_ssh_config(
                move |enabled: bool,
                      version: SharedString,
                      timeout: SharedString,
                      retries: SharedString| {
                    let mut c = ctrl.borrow_mut();
                    let diff = c.save_ssh_config(enabled, &version, &timeout, &retries);
                    mw.set_diff_text(diff.into());
                    if c.is_disruptive() {
                        mw.set_disruptive_warning(
                            "WARNING: SSH access changes may affect management access.".into(),
                        );
                        mw.set_has_pending_changes(true);
                        mw.set_pending_text("Unsaved changes pending".into());
                    } else {
                        mw.set_disruptive_warning("".into());
                    }
                },
            );
        }

        // ===== System callbacks =====

        // Load system config
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_load_system_config(move || -> slint::SharedString {
                let (hostname, domain, dns) = ctrl.borrow().load_system_config();
                format!("{}|{}|{}", hostname, domain, dns).into()
            });
        }

        // Save system config
        {
            let ctrl = controller.clone();
            let mw = main_window.clone_strong();
            main_window.on_save_system_config(
                move |hostname: SharedString, domain: SharedString, dns: SharedString| {
                    let mut c = ctrl.borrow_mut();
                    let diff = c.save_system_config(&hostname, &domain, &dns);
                    mw.set_diff_text(diff.into());
                    if c.is_disruptive() {
                        mw.set_disruptive_warning(
                            "WARNING: System changes may affect device identification.".into(),
                        );
                        mw.set_has_pending_changes(true);
                        mw.set_pending_text("Unsaved changes pending".into());
                    } else {
                        mw.set_disruptive_warning("".into());
                    }
                },
            );
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
