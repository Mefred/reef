//! IOS XE prompt detection.
//!
//! Detects the current CLI mode from the prompt text so the session knows
//! whether it is at user EXEC, privileged EXEC, or a configuration sub-mode.

use regex::Regex;
use std::time::Duration;

/// The detected CLI mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptMode {
    /// User EXEC (`>`), unprivileged.
    UserExec,
    /// Privileged EXEC (`#`), can configure.
    PrivilegedExec,
    /// Global configuration mode (`(config)#`).
    GlobalConfig,
    /// Interface configuration mode (`(config-if)#`).
    InterfaceConfig,
    /// Line configuration mode (`(config-line)#`).
    LineConfig,
    /// Some other configuration sub-mode.
    OtherConfig(String),
    /// A password prompt is active (`Password:`).
    PasswordPrompt,
    /// Unknown prompt — return the raw text for the caller to inspect.
    Unknown(String),
}

/// A detected prompt: its mode and the raw matched text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedPrompt {
    pub mode: PromptMode,
    pub raw: String,
}

lazy_static::lazy_static! {
    /// Matches privileged EXEC prompts like `Router#` or `Switch#`.
    static ref PRIV_EXEC_RE: Regex =
        Regex::new(r"^(?P<host>\S+)#\s*$").unwrap();
    /// Matches user EXEC prompts like `Router>`.
    static ref USER_EXEC_RE: Regex =
        Regex::new(r"^(?P<host>\S+)>\s*$").unwrap();
    /// Matches global config prompts like `Router(config)#`.
    static ref GLOBAL_CONFIG_RE: Regex =
        Regex::new(r"^(?P<host>\S+)\(config\)#\s*$").unwrap();
    /// Matches interface config prompts like `Router(config-if)#`.
    static ref INTERFACE_CONFIG_RE: Regex =
        Regex::new(r"^(?P<host>\S+)\(config-if\)#\s*$").unwrap();
    /// Matches line config prompts like `Router(config-line)#`.
    static ref LINE_CONFIG_RE: Regex =
        Regex::new(r"^(?P<host>\S+)\(config-line\)#\s*$").unwrap();
    /// Matches password prompts.
    static ref PASSWORD_RE: Regex =
        Regex::new(r"(?i)password\s*:?\s*$").unwrap();
}

/// Detect the prompt mode from a line of text.
pub fn detect(text: &str) -> Option<DetectedPrompt> {
    let trimmed = text.trim_end();

    // Check more specific configuration sub-modes before the general
    // privileged-exec regex (which would otherwise match "Router(config)#").
    if GLOBAL_CONFIG_RE.is_match(trimmed) {
        return Some(DetectedPrompt {
            mode: PromptMode::GlobalConfig,
            raw: trimmed.to_string(),
        });
    }
    if INTERFACE_CONFIG_RE.is_match(trimmed) {
        return Some(DetectedPrompt {
            mode: PromptMode::InterfaceConfig,
            raw: trimmed.to_string(),
        });
    }
    if LINE_CONFIG_RE.is_match(trimmed) {
        return Some(DetectedPrompt {
            mode: PromptMode::LineConfig,
            raw: trimmed.to_string(),
        });
    }
    if PRIV_EXEC_RE.is_match(trimmed) {
        return Some(DetectedPrompt {
            mode: PromptMode::PrivilegedExec,
            raw: trimmed.to_string(),
        });
    }
    if USER_EXEC_RE.is_match(trimmed) {
        return Some(DetectedPrompt {
            mode: PromptMode::UserExec,
            raw: trimmed.to_string(),
        });
    }
    if PASSWORD_RE.is_match(trimmed) {
        return Some(DetectedPrompt {
            mode: PromptMode::PasswordPrompt,
            raw: trimmed.to_string(),
        });
    }

    // Unknown but non-empty line that looks like a prompt (ends with > or #)
    if trimmed.ends_with('>') || trimmed.ends_with('#') {
        return Some(DetectedPrompt {
            mode: PromptMode::OtherConfig(trimmed.to_string()),
            raw: trimmed.to_string(),
        });
    }

    None
}

/// Default timeout for waiting on a prompt.
pub const PROMPT_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_privileged_exec() {
        let p = detect("Router#").unwrap();
        assert_eq!(p.mode, PromptMode::PrivilegedExec);
    }

    #[test]
    fn detects_user_exec() {
        let p = detect("Switch>").unwrap();
        assert_eq!(p.mode, PromptMode::UserExec);
    }

    #[test]
    fn detects_global_config() {
        let p = detect("Router(config)#").unwrap();
        assert_eq!(p.mode, PromptMode::GlobalConfig);
    }

    #[test]
    fn detects_interface_config() {
        let p = detect("Router(config-if)#").unwrap();
        assert_eq!(p.mode, PromptMode::InterfaceConfig);
    }

    #[test]
    fn detects_password_prompt() {
        let p = detect("Password:").unwrap();
        assert_eq!(p.mode, PromptMode::PasswordPrompt);
    }

    #[test]
    fn ignores_plain_output() {
        assert!(detect(" GigabitEthernet0/0/0   10.0.0.1   up   up").is_none());
    }
}
