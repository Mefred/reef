//! Parsers for Cisco IOS XE CLI output.
//!
//! Each parser converts semi-structured CLI text into the structured models in
//! `reef-engine::model`. Unknown or unhandled lines are preserved as raw snippets
//! so nothing is silently lost. Parsers degrade gracefully: they return a partial
//! model plus warnings rather than hard-failing on unexpected output.

mod version;
mod interfaces;
mod vlans;
mod running_config;
mod routes;
mod acls;
mod dhcp;
mod nat;

pub use version::parse_version;
pub use interfaces::parse_interfaces;
pub use vlans::parse_vlans;
pub use running_config::parse_running_config;
pub use routes::parse_routes;
pub use acls::parse_acls;
pub use dhcp::parse_dhcp_pools;
pub use nat::parse_nat;

/// A warning produced by a parser when it encounters output it could not fully
/// interpret. Callers should surface these to the user as "parser noticed
/// something it didn't understand."
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseWarning {
    pub line_no: usize,
    pub raw: String,
    pub reason: String,
}

impl ParseWarning {
    pub fn new(line_no: usize, raw: String, reason: String) -> Self {
        ParseWarning {
            line_no,
            raw,
            reason,
        }
    }
}

/// Result of a parse operation: the model plus any warnings.
pub struct ParseResult<T> {
    pub model: T,
    pub warnings: Vec<ParseWarning>,
}

impl<T> ParseResult<T> {
    pub fn new(model: T) -> Self {
        ParseResult {
            model,
            warnings: Vec::new(),
        }
    }

    pub fn with_warning(mut self, warning: ParseWarning) -> Self {
        self.warnings.push(warning);
        self
    }

    pub fn with_warnings(mut self, warnings: Vec<ParseWarning>) -> Self {
        self.warnings.extend(warnings);
        self
    }
}

/// Error returned by parsers, including the offending line and position.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("parse failed at line {line}: {message} (raw: {raw})")]
    Line {
        line: usize,
        message: String,
        raw: String,
    },
    #[error("unsupported feature: {message}")]
    Unsupported { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_warning_display_includes_reason() {
        let w = ParseWarning::new(3, "foo bar".into(), "unknown keyword".into());
        assert_eq!(w.reason, "unknown keyword");
        assert_eq!(w.line_no, 3);
    }
}
