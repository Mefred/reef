//! Integration tests for Reef configuration management.
//!
//! Tests the complete configuration engine workflow: parse, plan, generate,
//! diff, verify - using captured fixtures and a mock device.

use reef_engine::model::{DeviceConfig, InterfaceConfig, InterfaceMode};
use reef_engine::planner::{plan, ChangeOp};
use reef_engine::generator::generate;
use reef_engine::diff::render;
use reef_engine::verifier::{verify, VerificationResult};
use std::collections::HashMap;
use std::net::Ipv4Addr;

// ============================================================================
// Test helpers
// ============================================================================

fn test_config() -> DeviceConfig {
    DeviceConfig {
        hostname: Some("Switch1".to_string()),
        ip_domain_name: Some("example.com".to_string()),
        ip_name_server: vec!["8.8.8.8".to_string(), "8.8.4.4".to_string()],
        interfaces: {
            let mut m = HashMap::new();
            m.insert(
                "GigabitEthernet1/0/1".to_string(),
                InterfaceConfig {
                    name: "GigabitEthernet1/0/1".to_string(),
                    description: Some("Uplink".to_string()),
                    enabled: true,
                    mode: InterfaceMode::Routed,
                    ipv4: Some(reef_engine::model::Ipv4Address {
                        address: Ipv4Addr::new(10, 0, 0, 1),
                        prefix_len: 24,
                    }),
                    ..Default::default()
                },
            );
            m.insert(
                "GigabitEthernet1/0/2".to_string(),
                InterfaceConfig {
                    name: "GigabitEthernet1/0/2".to_string(),
                    description: Some("Downlink".to_string()),
                    enabled: true,
                    mode: InterfaceMode::Access,
                    access_vlan: Some(10),
                    ..Default::default()
                },
            );
            m.insert(
                "Vlan10".to_string(),
                InterfaceConfig {
                    name: "Vlan10".to_string(),
                    description: Some("Sales VLAN".to_string()),
                    enabled: true,
                    mode: InterfaceMode::Routed,
                    ipv4: Some(reef_engine::model::Ipv4Address {
                        address: Ipv4Addr::new(192, 168, 10, 1),
                        prefix_len: 24,
                    }),
                    ..Default::default()
                },
            );
            m
        },
        vlans: {
            let mut m = HashMap::new();
            m.insert(
                10,
                reef_engine::model::VlanConfig {
                    id: 10,
                    name: "Sales".to_string(),
                    enabled: true,
                },
            );
            m
        },
        ip_routes: vec![reef_engine::model::StaticRoute {
            prefix: "10.1.0.0/16".parse().unwrap(),
            next_hop: "10.0.0.2".parse().unwrap(),
            admin_distance: Some(1),
            permanent: false,
        }],
        ..Default::default()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_plan_interface_add() {
    let current = test_config();
    let mut desired = current.clone();

    // Add a new interface
    desired.interfaces.insert(
        "GigabitEthernet1/0/3".to_string(),
        InterfaceConfig {
            name: "GigabitEthernet1/0/3".to_string(),
            description: Some("New".to_string()),
            enabled: true,
            mode: InterfaceMode::Routed,
            ipv4: Some(reef_engine::model::Ipv4Address {
                address: Ipv4Addr::new(10, 0, 1, 1),
                prefix_len: 24,
            }),
            ..Default::default()
        },
    );

    let cs = plan(&current, &desired);
    let add_entries: Vec<_> = cs
        .interface_changes
        .iter()
        .filter(|(_, op, _)| *op == ChangeOp::Add)
        .collect();
    assert_eq!(add_entries.len(), 1);
    assert_eq!(add_entries[0].0, "GigabitEthernet1/0/3");
    assert!(cs.affects_connectivity); // new interface with IP
}

#[test]
fn test_plan_interface_modify() {
    let current = test_config();
    let mut desired = current.clone();

    // Modify interface description
    desired.interfaces.get_mut("GigabitEthernet1/0/1").unwrap().description = Some("Changed".to_string());

    let cs = plan(&current, &desired);
    let modify_entries: Vec<_> = cs
        .interface_changes
        .iter()
        .filter(|(_, op, _)| *op == ChangeOp::Modify)
        .collect();
    assert_eq!(modify_entries.len(), 1);
    assert_eq!(modify_entries[0].0, "GigabitEthernet1/0/1");
}

#[test]
fn test_plan_interface_remove() {
    let current = test_config();
    let mut desired = current.clone();
    desired.interfaces.remove("GigabitEthernet1/0/2");

    let cs = plan(&current, &desired);
    let remove_entries: Vec<_> = cs
        .interface_changes
        .iter()
        .filter(|(_, op, _)| *op == ChangeOp::Remove)
        .collect();
    assert_eq!(remove_entries.len(), 1);
    assert_eq!(remove_entries[0].0, "GigabitEthernet1/0/2");
    assert!(!cs.affects_connectivity); // removed interface has no IP
}

#[test]
fn test_plan_interface_ip_change_is_disruptive() {
    let current = test_config();
    let mut desired = current.clone();

    // Change IP on interface that has an IP - disruptive
    if let Some(iface) = desired.interfaces.get_mut("GigabitEthernet1/0/1") {
        iface.ipv4 = Some(reef_engine::model::Ipv4Address {
            address: Ipv4Addr::new(10, 0, 0, 2),
            prefix_len: 24,
        });
    }

    let cs = plan(&current, &desired);
    assert!(cs.affects_connectivity);
}

#[test]
fn test_generate_interface_commands() {
    let current = test_config();
    let mut desired = current.clone();

    // Modify interface: change IP and description
    if let Some(iface) = desired.interfaces.get_mut("GigabitEthernet1/0/1") {
        iface.description = Some("New Uplink".to_string());
        iface.ipv4 = Some(reef_engine::model::Ipv4Address {
            address: Ipv4Addr::new(10, 0, 0, 2),
            prefix_len: 24,
        });
    }

    let cs = plan(&current, &desired);
    let batch = generate(&cs);

    assert!(batch.enter_config_mode);
    assert!(batch.commands.iter().any(|c| c == "interface GigabitEthernet1/0/1"));
    assert!(batch
        .commands
        .iter()
        .any(|c| c.contains("no ip address"))); // old IP removed first
    assert!(batch
        .commands
        .iter()
        .any(|c| c.contains("description New Uplink")));
    assert!(batch
        .commands
        .iter()
        .any(|c| c.contains("ip address 10.0.0.2 255.255.255.0")));
    assert!(batch.exit_config_mode);
}

#[test]
fn test_generate_add_interface() {
    let current = test_config();
    let mut desired = current.clone();

    desired.interfaces.insert(
        "Loopback0".to_string(),
        InterfaceConfig {
            name: "Loopback0".to_string(),
            description: Some("Loopback".to_string()),
            enabled: true,
            mode: InterfaceMode::Routed,
            ipv4: Some(reef_engine::model::Ipv4Address {
                address: Ipv4Addr::new(1, 1, 1, 1),
                prefix_len: 32,
            }),
            ..Default::default()
        },
    );

    let cs = plan(&current, &desired);
    let batch = generate(&cs);

    assert!(batch.commands.iter().any(|c| c == "interface Loopback0"));
    assert!(batch
        .commands
        .iter()
        .any(|c| c.contains("description Loopback")));
    assert!(batch.commands.iter().any(|c| c == "ip address 1.1.1.1 255.255.255.255"));
}

#[test]
fn test_generate_remove_interface_with_ip() {
    let current = test_config();
    let mut desired = current.clone();
    desired.interfaces.remove("GigabitEthernet1/0/1");

    let cs = plan(&current, &desired);
    let batch = generate(&cs);

    // Removing interface with IP should first remove the IP
    assert!(batch.commands.iter().any(|c| c == "no ip address"));
    assert!(batch.commands.iter().any(|c| c == "no interface GigabitEthernet1/0/1"));
}

#[test]
fn test_generate_static_route() {
    let current = test_config();
    let mut desired = current.clone();

    desired.ip_routes.push(reef_engine::model::StaticRoute {
        prefix: "10.2.0.0/16".parse().unwrap(),
        next_hop: "10.0.0.3".parse().unwrap(),
        admin_distance: Some(5),
        permanent: false,
    });

    let cs = plan(&current, &desired);
    let batch = generate(&cs);

    assert!(batch
        .commands
        .iter()
        .any(|c| c == "ip route 10.2.0.0 255.255.0.0 10.0.0.3 5"));
}

#[test]
fn test_diff_rendering() {
    let current = test_config();
    let mut desired = current.clone();

    if let Some(iface) = desired.interfaces.get_mut("GigabitEthernet1/0/1") {
        iface.description = Some("Changed".to_string());
    }

    let cs = plan(&current, &desired);
    let diff = render(&cs);

    assert!(diff.contains("GigabitEthernet1/0/1:"));
    assert!(diff.contains("[MODIFY]"));
    assert!(diff.contains("description Changed"));
}

#[test]
fn test_diff_no_changes() {
    let current = test_config();
    let desired = current.clone();

    let cs = plan(&current, &desired);
    let diff = render(&cs);
    assert_eq!(diff, "No changes.\n");
}

#[test]
fn test_verify_match() {
    let current = test_config();
    let desired = current.clone();

    let result = verify(&current, &desired);
    assert!(result.is_success());
}

#[test]
fn test_verify_drift_interface() {
    let current = test_config();
    let mut desired = current.clone();

    if let Some(iface) = desired.interfaces.get_mut("GigabitEthernet1/0/1") {
        iface.description = Some("Changed".to_string());
    }

    let result = verify(&current, &desired);
    assert!(!result.is_success());

    if let VerificationResult::Drift { mismatches } = result {
        assert!(mismatches.iter().any(|m| m.contains("GigabitEthernet1/0/1")));
    } else {
        panic!("Expected Drift");
    }
}

#[test]
fn test_verify_drift_vlan() {
    let current = test_config();
    let mut desired = current.clone();

    if let Some(vlan) = desired.vlans.get_mut(&10) {
        vlan.name = "Changed".to_string();
    }

    let result = verify(&current, &desired);
    assert!(!result.is_success());

    if let VerificationResult::Drift { mismatches } = result {
        assert!(mismatches.iter().any(|m| m.contains("vlan 10")));
    } else {
        panic!("Expected Drift");
    }
}

#[test]
fn test_verify_drift_route() {
    let current = test_config();
    let mut desired = current.clone();

    desired.ip_routes.push(reef_engine::model::StaticRoute {
        prefix: "10.2.0.0/16".parse().unwrap(),
        next_hop: "10.0.0.3".parse().unwrap(),
        admin_distance: Some(5),
        permanent: false,
    });

    let result = verify(&current, &desired);
    assert!(!result.is_success());
}

#[test]
fn test_verify_drift_hostname() {
    let current = test_config();
    let mut desired = current.clone();
    desired.hostname = Some("Changed".to_string());

    let result = verify(&current, &desired);
    assert!(!result.is_success());
}

#[test]
fn test_idempotent_planning() {
    let current = test_config();
    let mut desired = current.clone();

    // Make a change
    if let Some(iface) = desired.interfaces.get_mut("GigabitEthernet1/0/2") {
        iface.description = Some("Test".to_string());
    }

    // Plan from current to desired - should have changes
    let cs1 = plan(&current, &desired);
    let count1 = cs1
        .interface_changes
        .iter()
        .filter(|(_, op, _)| *op != ChangeOp::NoChange)
        .count();
    assert!(count1 > 0);

    // Plan from desired to desired - should have no changes
    let cs2 = plan(&desired, &desired);
    let count2 = cs2
        .interface_changes
        .iter()
        .filter(|(_, op, _)| *op != ChangeOp::NoChange)
        .count();
    assert_eq!(count2, 0);
}

#[test]
fn test_complete_workflow() {
    let current = test_config();
    let mut desired = current.clone();

    // Make multiple changes
    if let Some(iface) = desired.interfaces.get_mut("GigabitEthernet1/0/1") {
        iface.description = Some("Updated Uplink".to_string());
    }
    desired.interfaces.insert(
        "Loopback0".to_string(),
        InterfaceConfig {
            name: "Loopback0".to_string(),
            description: Some("Mgmt".to_string()),
            enabled: true,
            mode: InterfaceMode::Routed,
            ipv4: Some(reef_engine::model::Ipv4Address {
                address: Ipv4Addr::new(1, 1, 1, 1),
                prefix_len: 32,
            }),
            ..Default::default()
        },
    );

    // Plan
    let cs = plan(&current, &desired);
    assert!(cs.interface_changes.len() >= 1);

    // Generate commands
    let batch = generate(&cs);
    assert!(!batch.commands.is_empty());

    // Diff
    let diff = render(&cs);
    assert!(!diff.is_empty());
    assert!(diff != "No changes.\n");

    // Verify (should detect drift since current != desired)
    let result = verify(&current, &desired);
    assert!(!result.is_success());

    // Verify after applying (desired vs desired)
    let result2 = verify(&desired, &desired);
    assert!(result2.is_success());
}
