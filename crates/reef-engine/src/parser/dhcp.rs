//! Parser for `show ip dhcp pool`.

use super::{ParseResult, ParseWarning};
use crate::model::DhcpPool;

pub fn parse_dhcp_pools(output: &str) -> ParseResult<Vec<DhcpPool>> {
    let mut pools = Vec::new();
    let mut warnings = Vec::new();

    for (i, line) in output.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("Pool") || trimmed.starts_with("---") {
            continue;
        }
        // Best-effort; full DHCP pool parsing is out of initial scope.
        warnings.push(ParseWarning::new(
            i + 1,
            trimmed.to_string(),
            "DHCP pool parsing not yet implemented".to_string(),
        ));
    }

    ParseResult::new(pools).with_warnings(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_output_returns_no_pools() {
        let r = parse_dhcp_pools("");
        assert!(r.model.is_empty());
    }
}
