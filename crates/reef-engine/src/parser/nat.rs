//! Parser for NAT show output.

use super::{ParseResult, ParseWarning};

pub fn parse_nat(output: &str) -> ParseResult<Vec<String>> {
    let mut rules = Vec::new();
    let mut warnings = Vec::new();

    for (i, line) in output.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.contains("nat ") {
            rules.push(trimmed.to_string());
        } else {
            warnings.push(ParseWarning::new(
                i + 1,
                trimmed.to_string(),
                "NAT line not parsed into model".to_string(),
            ));
        }
    }

    ParseResult::new(rules).with_warnings(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_nat_lines() {
        let r = parse_nat("ip nat inside source list 1 pool POOL overload\n");
        assert_eq!(r.model.len(), 1);
    }
}
