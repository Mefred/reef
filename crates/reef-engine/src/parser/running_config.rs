//! Parser for `show running-config`.
//!
//! Converts the full running configuration into a structured `DeviceConfig`.
//! Tracks nested config blocks (interface, vlan, line, access-list, dhcp pool)
//! so indented sub-commands are attributed to the right parent.
//! Unknown lines are preserved as warnings so nothing is silently lost.

use super::{ParseResult, ParseWarning};
use crate::model::{
    AclConfig, AclRule, AclType, Action, AddressSpec, DeviceConfig, DhcpPool, InterfaceConfig,
    InterfaceMode, NatConfig, NatPool, NatRule, PortSpec, Protocol, Secret, SshConfig, SshVersion,
    StaticRoute, VlanConfig,
};
use std::net::IpAddr;

/// Which config block we are currently inside.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ConfigBlock {
    None,
    Interface(String),
    Vlan(u16),
    Line(String),
    AccessList(String, AclType),
    DhcpPool(String),
}

pub fn parse_running_config(output: &str) -> ParseResult<DeviceConfig> {
    let mut config = DeviceConfig::default();
    let mut warnings = Vec::new();
    let mut block: ConfigBlock = ConfigBlock::None;

    for (i, line) in output.lines().enumerate() {
        let trimmed = line.trim();

        // Skip comments and header lines.
        if trimmed.is_empty()
            || trimmed.starts_with("!")
            || trimmed.starts_with("Current configuration")
            || trimmed.starts_with("last configuration change")
        {
            continue;
        }

        // Top-level commands (no leading whitespace in running-config output).
        if !line.starts_with(' ') && !line.starts_with('\t') {
            block = parse_top_level(trimmed, &mut config, &mut warnings);
            continue;
        }

        // Indented sub-command: attribute to current block.
        match &block {
            ConfigBlock::Interface(name) => {
                if let Some(w) = parse_interface_sub(trimmed, name, &mut config) {
                    warnings.push(w);
                }
            }
            ConfigBlock::Vlan(id) => {
                if let Some(w) = parse_vlan_sub(trimmed, *id, &mut config) {
                    warnings.push(w);
                }
            }
            ConfigBlock::Line(name) => {
                if let Some(w) = parse_line_sub(trimmed, name, &mut config) {
                    warnings.push(w);
                }
            }
            ConfigBlock::AccessList(name, acl_type) => {
                if let Some(w) = parse_acl_sub(trimmed, name, *acl_type, &mut config) {
                    warnings.push(w);
                }
            }
            ConfigBlock::DhcpPool(name) => {
                if let Some(w) = parse_dhcp_sub(trimmed, name, &mut config) {
                    warnings.push(w);
                }
            }
            ConfigBlock::None => {
                warnings.push(ParseWarning::new(
                    i + 1,
                    trimmed.to_string(),
                    "indented line with no active block".to_string(),
                ));
            }
        }
    }

    ParseResult::new(config).with_warnings(warnings)
}

/// Parse a top-level (non-indented) command.
/// Returns the config block to enter (if any). Warnings are pushed to `warnings`.
fn parse_top_level(
    trimmed: &str,
    config: &mut DeviceConfig,
    warnings: &mut Vec<ParseWarning>,
) -> ConfigBlock {
    if trimmed.starts_with("hostname ") {
        config.hostname = Some(trimmed["hostname ".len()..].trim().to_string());
        return ConfigBlock::None;
    }
    if trimmed.starts_with("ip domain-name ") {
        config.ip_domain_name = Some(trimmed["ip domain-name ".len()..].trim().to_string());
        return ConfigBlock::None;
    }
    if trimmed.starts_with("ip name-server ") {
        let servers: Vec<&str> = trimmed["ip name-server ".len()..]
            .trim()
            .split_whitespace()
            .collect();
        for s in servers {
            if let Ok(addr) = s.parse::<std::net::IpAddr>() {
                config.ip_name_server.push(addr.to_string());
            }
        }
        return ConfigBlock::None;
    }
    if trimmed.starts_with("interface ") {
        let name = trimmed["interface ".len()..].trim().to_string();
        config
            .interfaces
            .entry(name.clone())
            .or_insert(InterfaceConfig {
                name: name.clone(),
                enabled: true,
                ..Default::default()
            });
        return ConfigBlock::Interface(name);
    }
    if trimmed.starts_with("ip route ") {
        if let Some(route) = parse_static_route(trimmed) {
            config.ip_routes.push(route);
        } else {
            warnings.push(ParseWarning::new(
                0,
                trimmed.to_string(),
                "unparsed ip route".to_string(),
            ));
        }
        return ConfigBlock::None;
    }
    if trimmed.starts_with("ipv6 route ") {
        if let Some(route) = parse_static_ipv6_route(trimmed) {
            config.ipv6_routes.push(route);
        } else {
            warnings.push(ParseWarning::new(
                0,
                trimmed.to_string(),
                "unparsed ipv6 route".to_string(),
            ));
        }
        return ConfigBlock::None;
    }
    if trimmed.starts_with("vlan ") {
        if let Ok(id) = trimmed["vlan ".len()..].trim().parse::<u16>() {
            config.vlans.entry(id).or_insert(VlanConfig {
                id,
                name: String::new(),
                enabled: true,
            });
            return ConfigBlock::Vlan(id);
        }
        return ConfigBlock::None;
    }
    if trimmed.starts_with("ip access-list ") {
        let rest = trimmed["ip access-list ".len()..].trim();
        let (name, acl_type) = if rest.starts_with("standard ") {
            (
                rest["standard ".len()..].trim().to_string(),
                AclType::Standard,
            )
        } else if rest.starts_with("extended ") {
            (
                rest["extended ".len()..].trim().to_string(),
                AclType::Extended,
            )
        } else {
            (rest.to_string(), AclType::Standard)
        };
        config.acls.entry(name.clone()).or_insert(AclConfig {
            name: name.clone(),
            acl_type,
            rules: Vec::new(),
        });
        return ConfigBlock::AccessList(name, acl_type);
    }
    if trimmed.starts_with("ip dhcp pool ") {
        let name = trimmed["ip dhcp pool ".len()..].trim().to_string();
        config.dhcp_pools.entry(name.clone()).or_insert(DhcpPool {
            name: name.clone(),
            network: "0.0.0.0/0".parse().unwrap(),
            default_router: None,
            dns_servers: Vec::new(),
            lease_time: None,
        });
        return ConfigBlock::DhcpPool(name);
    }
    if let Some(rest) = trimmed.strip_prefix("ip nat pool ") {
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.len() == 5 && parts[3] == "netmask" {
            if let (Ok(start), Ok(end), Ok(netmask)) = (
                parts[1].parse::<std::net::Ipv4Addr>(),
                parts[2].parse::<std::net::Ipv4Addr>(),
                parts[4].parse::<std::net::Ipv4Addr>(),
            ) {
                let nat = config.nat.get_or_insert_with(NatConfig::default);
                nat.pools.push(NatPool {
                    name: parts[0].to_string(),
                    start,
                    end,
                    netmask,
                });
                return ConfigBlock::None;
            }
        }
        warnings.push(ParseWarning::new(
            0,
            trimmed.to_string(),
            "unparsed NAT pool".to_string(),
        ));
        return ConfigBlock::None;
    }
    if let Some(rest) = trimmed.strip_prefix("ip nat inside source list ") {
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.len() >= 3 && parts[1] == "pool" {
            let nat = config.nat.get_or_insert_with(NatConfig::default);
            nat.rules.push(NatRule {
                name: format!("{} -> {}", parts[0], parts[2]),
                source_acl: Some(parts[0].to_string()),
                pool: Some(parts[2].to_string()),
                overload: parts.iter().any(|part| *part == "overload"),
            });
            return ConfigBlock::None;
        }
        warnings.push(ParseWarning::new(
            0,
            trimmed.to_string(),
            "unsupported NAT source rule".to_string(),
        ));
        return ConfigBlock::None;
    }
    if trimmed.starts_with("ip nat ") || trimmed.starts_with("no ip nat ") {
        warnings.push(ParseWarning::new(
            0,
            trimmed.to_string(),
            "unsupported NAT command".to_string(),
        ));
        return ConfigBlock::None;
    }
    if let Some(rest) = trimmed.strip_prefix("ip ssh ") {
        let ssh = config.ssh.get_or_insert(SshConfig {
            enabled: true,
            version: SshVersion::V2,
            timeout: None,
            authentication_retries: None,
        });
        let parts: Vec<&str> = rest.split_whitespace().collect();
        match parts.as_slice() {
            ["version", "1"] => ssh.version = SshVersion::V1,
            ["version", "2"] => ssh.version = SshVersion::V2,
            ["version", "1.99"] => ssh.version = SshVersion::Both,
            ["time-out", value] => ssh.timeout = value.parse().ok(),
            ["authentication-retries", value] => ssh.authentication_retries = value.parse().ok(),
            _ => warnings.push(ParseWarning::new(
                0,
                trimmed.to_string(),
                "unsupported SSH setting".to_string(),
            )),
        }
        return ConfigBlock::None;
    }
    if trimmed == "no ip ssh" {
        let ssh = config.ssh.get_or_insert(SshConfig {
            enabled: false,
            version: SshVersion::V2,
            timeout: None,
            authentication_retries: None,
        });
        ssh.enabled = false;
        return ConfigBlock::None;
    }
    if trimmed.starts_with("line ") {
        let name = trimmed["line ".len()..].trim().to_string();
        config
            .line_configs
            .entry(name.clone())
            .or_insert(crate::model::LineConfig {
                line: name.clone(),
                transport: Vec::new(),
                password: None,
                login: false,
            });
        return ConfigBlock::Line(name);
    }
    if trimmed.starts_with("banner ") {
        config.banner = Some(trimmed.to_string());
        return ConfigBlock::None;
    }

    warnings.push(ParseWarning::new(
        0,
        trimmed.to_string(),
        "unhandled top-level command".to_string(),
    ));
    ConfigBlock::None
}

/// Parse an indented sub-command inside an interface block.
fn parse_interface_sub(
    trimmed: &str,
    name: &str,
    config: &mut DeviceConfig,
) -> Option<ParseWarning> {
    if trimmed == "ip nat inside" || trimmed == "ip nat outside" {
        let nat = config.nat.get_or_insert_with(NatConfig::default);
        let interfaces = if trimmed.ends_with("inside") {
            &mut nat.inside_interfaces
        } else {
            &mut nat.outside_interfaces
        };
        if !interfaces.iter().any(|interface| interface == name) {
            interfaces.push(name.to_string());
        }
        return None;
    }
    if trimmed == "no ip nat inside" || trimmed == "no ip nat outside" {
        if let Some(nat) = config.nat.as_mut() {
            let interfaces = if trimmed.ends_with("inside") {
                &mut nat.inside_interfaces
            } else {
                &mut nat.outside_interfaces
            };
            interfaces.retain(|interface| interface != name);
        }
        return None;
    }
    let iface = match config.interfaces.get_mut(name) {
        Some(i) => i,
        None => {
            return Some(ParseWarning::new(
                0,
                trimmed.to_string(),
                "unknown interface".to_string(),
            ))
        }
    };

    if trimmed.starts_with("description ") {
        iface.description = Some(trimmed["description ".len()..].trim().to_string());
        return None;
    }
    if trimmed == "shutdown" {
        iface.enabled = false;
        return None;
    }
    if trimmed == "no shutdown" {
        iface.enabled = true;
        return None;
    }
    if trimmed.starts_with("ip address ") {
        let rest = trimmed["ip address ".len()..].trim();
        if let Some(addr) = parse_ipv4_address(rest) {
            iface.ipv4 = Some(addr);
        } else {
            return Some(ParseWarning::new(
                0,
                trimmed.to_string(),
                "unparsed ip address".to_string(),
            ));
        }
        return None;
    }
    if trimmed == "no ip address" {
        iface.ipv4 = None;
        return None;
    }
    if trimmed.starts_with("ipv6 address ") {
        let rest = trimmed["ipv6 address ".len()..].trim();
        if let Some(addr) = parse_ipv6_address(rest) {
            iface.ipv6 = Some(addr);
        } else {
            return Some(ParseWarning::new(
                0,
                trimmed.to_string(),
                "unparsed ipv6 address".to_string(),
            ));
        }
        return None;
    }
    if trimmed.starts_with("mtu ") {
        if let Ok(mtu) = trimmed["mtu ".len()..].trim().parse::<u16>() {
            iface.mtu = Some(mtu);
        }
        return None;
    }
    if trimmed.starts_with("speed ") {
        let s = trimmed["speed ".len()..].trim();
        iface.speed = if s == "auto" {
            Some(crate::model::Speed::Auto)
        } else {
            None
        };
        return None;
    }
    if trimmed.starts_with("duplex ") {
        let s = trimmed["duplex ".len()..].trim();
        iface.duplex = match s {
            "auto" => Some(crate::model::Duplex::Auto),
            "half" => Some(crate::model::Duplex::Half),
            "full" => Some(crate::model::Duplex::Full),
            _ => None,
        };
        return None;
    }
    if trimmed == "switchport mode access" {
        iface.mode = InterfaceMode::Access;
        return None;
    }
    if trimmed == "switchport mode trunk" {
        iface.mode = InterfaceMode::Trunk;
        return None;
    }
    if trimmed == "switchport mode routed" || trimmed == "no switchport" {
        iface.mode = InterfaceMode::Routed;
        return None;
    }
    if trimmed.starts_with("switchport access vlan ") {
        if let Ok(vlan) = trimmed["switchport access vlan ".len()..]
            .trim()
            .parse::<u16>()
        {
            iface.access_vlan = Some(vlan);
            iface.mode = InterfaceMode::Access;
        }
        return None;
    }
    if trimmed.starts_with("switchport trunk native vlan ") {
        if let Ok(vlan) = trimmed["switchport trunk native vlan ".len()..]
            .trim()
            .parse::<u16>()
        {
            iface.native_vlan = Some(vlan);
        }
        return None;
    }
    if trimmed.starts_with("switchport trunk allowed vlan ") {
        let list = trimmed["switchport trunk allowed vlan ".len()..].trim();
        iface.trunk_vlans = parse_vlan_list(list);
        return None;
    }

    Some(ParseWarning::new(
        0,
        trimmed.to_string(),
        "unhandled interface sub-command".to_string(),
    ))
}

fn parse_vlan_sub(trimmed: &str, id: u16, config: &mut DeviceConfig) -> Option<ParseWarning> {
    let vlan = match config.vlans.get_mut(&id) {
        Some(v) => v,
        None => {
            return Some(ParseWarning::new(
                0,
                trimmed.to_string(),
                "unknown vlan".to_string(),
            ))
        }
    };
    if trimmed.starts_with("name ") {
        vlan.name = trimmed["name ".len()..].trim().to_string();
        return None;
    }
    if trimmed == "shutdown" {
        vlan.enabled = false;
        return None;
    }
    if trimmed == "no shutdown" {
        vlan.enabled = true;
        return None;
    }
    None
}

fn parse_line_sub(trimmed: &str, name: &str, config: &mut DeviceConfig) -> Option<ParseWarning> {
    let line = match config.line_configs.get_mut(name) {
        Some(l) => l,
        None => {
            return Some(ParseWarning::new(
                0,
                trimmed.to_string(),
                "unknown line".to_string(),
            ))
        }
    };
    if trimmed.starts_with("password ") {
        line.password = Some(Secret(trimmed["password ".len()..].trim().to_string()));
        return None;
    }
    if trimmed == "login"
        || trimmed == "login local"
        || trimmed.starts_with("login authentication ")
    {
        line.login = true;
        return None;
    }
    if trimmed == "no login" {
        line.login = false;
        return None;
    }
    if trimmed.starts_with("transport input ") {
        let inputs: Vec<&str> = trimmed["transport input ".len()..]
            .trim()
            .split_whitespace()
            .collect();
        for inp in inputs {
            match inp {
                "ssh" => {
                    if !line.transport.contains(&crate::model::Transport::Ssh) {
                        line.transport.push(crate::model::Transport::Ssh);
                    }
                }
                "telnet" => {
                    if !line.transport.contains(&crate::model::Transport::Telnet) {
                        line.transport.push(crate::model::Transport::Telnet);
                    }
                }
                "all" => {
                    line.transport = vec![crate::model::Transport::All];
                }
                "none" => {
                    line.transport = vec![crate::model::Transport::None];
                }
                _ => {}
            }
        }
        return None;
    }
    None
}

fn parse_acl_sub(
    trimmed: &str,
    name: &str,
    acl_type: AclType,
    config: &mut DeviceConfig,
) -> Option<ParseWarning> {
    let acl = match config.acls.get_mut(name) {
        Some(a) => a,
        None => {
            return Some(ParseWarning::new(
                0,
                trimmed.to_string(),
                "unknown acl".to_string(),
            ))
        }
    };
    acl.acl_type = acl_type;

    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    let start = if parts[0].parse::<u16>().is_ok() {
        1
    } else {
        0
    };
    if parts.len() <= start {
        return None;
    }
    let action = match parts[start] {
        "permit" => Action::Permit,
        "deny" => Action::Deny,
        _ => return None,
    };
    if start + 1 >= parts.len() {
        return None;
    }
    let protocol = match parts[start + 1] {
        "ip" => Protocol::Ip,
        "tcp" => Protocol::Tcp,
        "udp" => Protocol::Udp,
        "icmp" => Protocol::Icmp,
        s => Protocol::Other(s.to_string()),
    };
    let source = if start + 2 < parts.len() {
        parse_address_spec(parts[start + 2])
    } else {
        AddressSpec::Any
    };
    let destination = if start + 3 < parts.len() {
        parse_address_spec(parts[start + 3])
    } else {
        AddressSpec::Any
    };

    acl.rules.push(AclRule {
        action,
        protocol: Some(protocol),
        source,
        destination,
        ports: None,
    });
    None
}

fn parse_dhcp_sub(trimmed: &str, name: &str, config: &mut DeviceConfig) -> Option<ParseWarning> {
    let pool = match config.dhcp_pools.get_mut(name) {
        Some(p) => p,
        None => {
            return Some(ParseWarning::new(
                0,
                trimmed.to_string(),
                "unknown dhcp pool".to_string(),
            ))
        }
    };
    if trimmed.starts_with("network ") {
        let rest = trimmed["network ".len()..].trim();
        if let Ok(net) = rest.parse::<ipnet::IpNet>() {
            pool.network = net;
        }
        return None;
    }
    if trimmed.starts_with("default-router ") {
        let addr = trimmed["default-router ".len()..].trim();
        if let Ok(ip) = addr.parse::<std::net::IpAddr>() {
            pool.default_router = Some(ip);
        }
        return None;
    }
    if trimmed.starts_with("dns-server ") {
        let addrs: Vec<&str> = trimmed["dns-server ".len()..]
            .trim()
            .split_whitespace()
            .collect();
        for a in addrs {
            if let Ok(ip) = a.parse::<std::net::IpAddr>() {
                pool.dns_servers.push(ip);
            }
        }
        return None;
    }
    if trimmed.starts_with("lease ") {
        let parts: Vec<&str> = trimmed["lease ".len()..]
            .trim()
            .split_whitespace()
            .collect();
        if parts.len() >= 3 {
            if let (Ok(d), Ok(h), Ok(m)) = (
                parts[0].parse::<u64>(),
                parts[1].parse::<u64>(),
                parts[2].parse::<u64>(),
            ) {
                pool.lease_time = Some(std::time::Duration::from_secs(
                    d * 86400 + h * 3600 + m * 60,
                ));
            }
        }
        return None;
    }
    None
}

/// Parse a static IPv4 route: "ip route 10.1.0.0 255.255.255.0 10.0.0.1 [distance]"
fn parse_static_route(trimmed: &str) -> Option<StaticRoute> {
    let rest = trimmed["ip route ".len()..].trim();
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let (prefix, next_hop) = if parts[0].contains('/') {
        let prefix: ipnet::IpNet = parts[0].parse().ok()?;
        let next_hop: IpAddr = parts[1].parse().ok()?;
        (prefix, next_hop)
    } else if parts.len() >= 3 {
        let addr: std::net::Ipv4Addr = parts[0].parse().ok()?;
        let mask: std::net::Ipv4Addr = parts[1].parse().ok()?;
        let prefix_len = mask_to_prefix_len(mask)?;
        let prefix: ipnet::IpNet = ipnet::IpNet::new(IpAddr::V4(addr), prefix_len).ok()?;
        let next_hop: IpAddr = parts[2].parse().ok()?;
        (prefix, next_hop)
    } else {
        return None;
    };
    let mut admin_distance = None;
    // Admin distance may appear as [5/0] in show output, or as a bare
    // trailing number in running-config (e.g. "ip route ... 10.0.0.3 5").
    for p in &parts {
        if p.starts_with('[') && p.ends_with(']') {
            let inner = &p[1..p.len() - 1];
            let dist: u16 = inner.split('/').next()?.parse().ok()?;
            admin_distance = Some(dist);
        }
    }
    if admin_distance.is_none() {
        // Check for a bare trailing numeric admin distance.
        if let Some(last) = parts.last() {
            if let Ok(d) = last.parse::<u16>() {
                // Only treat as distance if it's not the next-hop (already parsed).
                // In "ip route A.B.C.D/MASK NHOP DIST", last is the distance.
                admin_distance = Some(d);
            }
        }
    }
    Some(StaticRoute {
        prefix,
        next_hop,
        admin_distance,
        permanent: false,
    })
}

/// Parse a static IPv6 route: "ipv6 route 2001:db8::/32 2001:db8::1"
fn parse_static_ipv6_route(trimmed: &str) -> Option<StaticRoute> {
    let rest = trimmed["ipv6 route ".len()..].trim();
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let prefix: ipnet::IpNet = parts[0].parse().ok()?;
    let next_hop: IpAddr = parts[1].parse().ok()?;
    Some(StaticRoute {
        prefix,
        next_hop,
        admin_distance: None,
        permanent: false,
    })
}

fn parse_vlan_list(list: &str) -> Vec<u16> {
    let mut out = Vec::new();
    for part in list.split(',') {
        let part = part.trim();
        if part.contains('-') {
            let bounds: Vec<&str> = part.split('-').collect();
            if bounds.len() == 2 {
                if let (Ok(lo), Ok(hi)) = (bounds[0].parse::<u16>(), bounds[1].parse::<u16>()) {
                    for v in lo..=hi {
                        out.push(v);
                    }
                }
            }
        } else if let Ok(v) = part.parse::<u16>() {
            out.push(v);
        }
    }
    out
}

fn parse_address_spec(token: &str) -> AddressSpec {
    match token {
        "any" => AddressSpec::Any,
        "host" => AddressSpec::Any,
        s if s.contains('/') => s
            .parse::<ipnet::IpNet>()
            .map(AddressSpec::Network)
            .unwrap_or(AddressSpec::Any),
        s => s
            .parse::<IpAddr>()
            .map(AddressSpec::Host)
            .unwrap_or(AddressSpec::Any),
    }
}

fn parse_ipv4_address(rest: &str) -> Option<crate::model::Ipv4Address> {
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let addr: std::net::Ipv4Addr = parts[0].parse().ok()?;
    let mask: std::net::Ipv4Addr = parts[1].parse().ok()?;
    let prefix_len = mask_to_prefix_len(mask)?;
    Some(crate::model::Ipv4Address {
        address: addr,
        prefix_len,
    })
}

fn parse_ipv6_address(rest: &str) -> Option<crate::model::Ipv6Address> {
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let addr: std::net::Ipv6Addr = parts[0].parse().ok()?;
    let prefix_len: u8 = parts[1].parse().ok()?;
    Some(crate::model::Ipv6Address {
        address: addr,
        prefix_len,
    })
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_config() {
        let out = "hostname Router1\n\
ip domain-name example.com\n\
ip name-server 8.8.8.8 8.8.4.4\n\
interface GigabitEthernet0/0/0\n description Uplink\n ip address 10.0.0.1 255.255.255.0\n no shutdown\n mtu 1500\n\
interface GigabitEthernet0/0/1\n switchport mode access\n switchport access vlan 10\n shutdown\n\
interface Vlan10\n ip address 192.168.10.1 255.255.255.0\n\
ip route 10.1.0.0 255.255.255.0 10.0.0.2\n\
ip route 10.2.0.0/16 10.0.0.3 5\n\
vlan 10\n name Sales\n\
ip access-list standard ACL1\n 10 permit 1.2.3.4\n 20 deny any\n\
ip dhcp pool POOL1\n network 192.168.20.0 255.255.255.0\n default-router 192.168.20.1\n dns-server 8.8.8.8\n\
line console 0\n password secret\n login\n\
line vty 0 4\n transport input ssh\n";
        let r = parse_running_config(out);
        assert_eq!(r.model.hostname.as_deref(), Some("Router1"));
        assert_eq!(r.model.ip_domain_name.as_deref(), Some("example.com"));
        assert_eq!(
            r.model.ip_name_server,
            vec!["8.8.8.8".to_string(), "8.8.4.4".to_string()]
        );

        let gi0 = r.model.interfaces.get("GigabitEthernet0/0/0").unwrap();
        assert!(gi0.enabled);
        assert_eq!(gi0.description.as_deref(), Some("Uplink"));
        assert_eq!(gi0.ipv4.as_ref().unwrap().address.to_string(), "10.0.0.1");
        assert_eq!(gi0.mtu, Some(1500));

        let gi1 = r.model.interfaces.get("GigabitEthernet0/0/1").unwrap();
        assert!(!gi1.enabled);
        assert_eq!(gi1.mode, InterfaceMode::Access);
        assert_eq!(gi1.access_vlan, Some(10));

        let vlan10 = r.model.vlans.get(&10).unwrap();
        assert_eq!(vlan10.name, "Sales");

        assert_eq!(r.model.ip_routes.len(), 2);
        assert_eq!(r.model.ip_routes[0].next_hop.to_string(), "10.0.0.2");
        assert_eq!(r.model.ip_routes[1].admin_distance, Some(5));

        let acl = r.model.acls.get("ACL1").unwrap();
        assert_eq!(acl.rules.len(), 2);
        assert_eq!(acl.rules[0].action, Action::Permit);

        let pool = r.model.dhcp_pools.get("POOL1").unwrap();
        assert_eq!(
            pool.default_router.as_ref().unwrap().to_string(),
            "192.168.20.1"
        );
        assert_eq!(pool.dns_servers.len(), 1);

        let con = r.model.line_configs.get("console 0").unwrap();
        assert!(con.login);
        assert!(con.password.is_some());

        let vty = r.model.line_configs.get("vty 0 4").unwrap();
        assert!(matches!(vty.transport[0], crate::model::Transport::Ssh));
    }

    #[test]
    fn parses_nat_and_ssh_without_exposing_secrets() {
        let output = r#"ip ssh version 2
ip ssh time-out 45
ip ssh authentication-retries 3
ip nat pool PUBLIC 203.0.113.10 203.0.113.20 netmask 255.255.255.0
ip nat inside source list 10 pool PUBLIC overload
interface GigabitEthernet0/0/0
 ip nat inside
interface GigabitEthernet0/0/1
 ip nat outside
line vty 0 4
 password super-secret
 login local
 transport input ssh
"#;
        let parsed = parse_running_config(output);
        let ssh = parsed.model.ssh.as_ref().unwrap();
        assert!(ssh.enabled);
        assert!(matches!(&ssh.version, SshVersion::V2));
        assert_eq!(ssh.timeout, Some(45));
        assert_eq!(ssh.authentication_retries, Some(3));
        let nat = parsed.model.nat.as_ref().unwrap();
        assert_eq!(nat.inside_interfaces, vec!["GigabitEthernet0/0/0"]);
        assert_eq!(nat.outside_interfaces, vec!["GigabitEthernet0/0/1"]);
        assert_eq!(nat.pools[0].name, "PUBLIC");
        assert_eq!(nat.rules[0].source_acl.as_deref(), Some("10"));
        assert!(nat.rules[0].overload);
        let line = parsed.model.line_configs.get("vty 0 4").unwrap();
        assert!(line.password.is_some());
        assert!(line.login);
        assert!(line.transport.contains(&crate::model::Transport::Ssh));
    }

    #[test]
    fn no_ip_address_removes_ipv4() {
        let out =
            "interface GigabitEthernet0/0/0\n ip address 10.0.0.1 255.255.255.0\n no ip address\n";
        let r = parse_running_config(out);
        let iface = r.model.interfaces.get("GigabitEthernet0/0/0").unwrap();
        assert!(iface.ipv4.is_none());
    }

    #[test]
    fn unknown_lines_produce_warnings() {
        let out = "boot system flash:c9300-universalk9.17.12.0a.SPA.bin\n";
        let r = parse_running_config(out);
        assert!(!r.warnings.is_empty());
    }
}
