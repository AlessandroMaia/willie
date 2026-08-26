//! Harnesses are the agent CLIs Willie launches and supervises.
//!
//! Every harness is described by a *capability matrix*: plain data that
//! tells the rest of the system what the CLI can do (resume a session,
//! stream headless output, …). Consumers branch on the matrix, never on
//! the harness id, so adding a second harness touches one file.

/// How a harness resumes a previous conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resume {
    /// The harness cannot resume; a new session starts from scratch.
    None,
    /// The harness resumes a conversation given its own session id.
    ById,
    /// The harness resumes the most recent conversation for the project.
    Continue,
}

/// What a harness can do. Data, not behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarnessCapabilities {
    /// Runs as an interactive terminal UI.
    pub interactive_tui: bool,
    /// How sessions can be resumed.
    pub resume: Resume,
    /// Supports a non-interactive mode that streams structured output.
    pub headless_stream: bool,
}

/// An agent CLI Willie knows how to run.
pub trait Harness: std::fmt::Debug {
    /// Stable identifier, e.g. `claude-code`.
    fn id(&self) -> &'static str;
    /// Executable name looked up inside the distro.
    fn binary_name(&self) -> &'static str;
    /// Capability matrix.
    fn capabilities(&self) -> HarnessCapabilities;
}

/// Claude Code, the first supported harness.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClaudeCode;

impl Harness for ClaudeCode {
    fn id(&self) -> &'static str {
        "claude-code"
    }

    fn binary_name(&self) -> &'static str {
        "claude"
    }

    fn capabilities(&self) -> HarnessCapabilities {
        HarnessCapabilities {
            interactive_tui: true,
            resume: Resume::ById,
            headless_stream: true,
        }
    }
}

/// All harnesses compiled into this build.
#[must_use]
pub fn registry() -> Vec<Box<dyn Harness>> {
    vec![Box::new(ClaudeCode)]
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn harness_ids_are_unique() {
        let ids: HashSet<&str> = registry().iter().map(|h| h.id()).collect();
        assert_eq!(ids.len(), registry().len());
    }

    #[test]
    fn claude_code_resumes_by_id() {
        assert_eq!(ClaudeCode.capabilities().resume, Resume::ById);
        assert_eq!(ClaudeCode.binary_name(), "claude");
    }
}
