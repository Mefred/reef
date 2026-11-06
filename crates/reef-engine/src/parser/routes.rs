//! Parser for `show ip route` / `show ipv6 route`.

use super::{ParseResult, ParseWarning};
use crate::model::StaticRoute;

pub fn parse_routes(output: &str) -> ParseResult<Vec<StaticRoute>> {
    let mut routes = Vec::new();
    let mut warnings = Vec::new();

    for (i, line) in output.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("Codes:")
            || trimmed.starts_with("Gateway of last resort")
            || trimmed.starts_with("Routing entry for")
        {
            continue;
        }
        // Best-effort: static routes appear as "S    10.1.0.0/16 [1/0] via 10.0.0.1"
        if let Some(r) = parse_static_route(trimmed) {
            routes.push(r);
            continue;
        }
        warnings.push(ParseWarning::new(
            i + 1,
            trimmed.to_string(),
            "unhandled route line".to_string(),
        ));
    }

    ParseResult::new(routes).with_warnings(warnings)
}

fn parse_static_route(line: &str) -> Option<StaticRoute> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() || parts[0] != "S" {
        return None;
    }
    if parts.len() < 3 {
        return None;
    }
    let prefix: ipnet::IpNet = parts[1].parse().ok()?;
    // Find the next-hop after "via"
    let via_idx = parts.iter().position(|p| *p == "via")?;
    let next_hop: std::net::IpAddr = parts.get(via_idx + 1)?.parse().ok()?;
    let mut admin_distance = None;
    for p in &parts {
        if p.starts_with('[') && p.ends_with(']') {
            let inner = &p[1..p.len() - 1];
            let dist: u16 = inner.split('/').next()?.parse().ok()?;
            admin_distance = Some(dist);
        }
    }
    Some(StaticRoute {
        prefix,
        next_hop,
        admin_distance,
        permanent: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_static_route() {
        let out = "S    10.1.0.0/16 [1/0] via 10.0.0.1\n";
        let r = parse_routes(out);
        assert_eq!(r.model.len(), 1);
        assert_eq!(r.model[0].next_hop.to_string(), "10.0.0.1");
        assert_eq!(r.model[0].admin_distance, Some(1));
    }
}
