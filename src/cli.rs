use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "cortina",
    version,
    about = "Lifecycle signal runner for AI coding agents"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Handle host adapter events through an explicit adapter surface
    Adapter {
        #[command(subcommand)]
        adapter: crate::adapters::AdapterCommand,
    },

    /// Compatibility alias for `cortina adapter claude-code pre-tool-use`
    #[command(name = "pre-tool-use", hide = true)]
    PreToolUse,

    /// Compatibility alias for `cortina adapter claude-code post-tool-use`
    #[command(name = "post-tool-use", hide = true)]
    PostToolUse,

    /// Compatibility alias for `cortina adapter claude-code user-prompt-submit`
    #[command(name = "user-prompt-submit", hide = true)]
    UserPromptSubmit,

    /// Compatibility alias for `cortina adapter claude-code pre-compact`
    #[command(name = "pre-compact", hide = true)]
    PreCompact,

    /// Compatibility alias for `cortina adapter claude-code stop`
    #[command(name = "stop", hide = true)]
    Stop,

    /// Compatibility alias for `cortina adapter claude-code session-end`
    #[command(name = "session-end", hide = true)]
    SessionEnd,

    /// Audit a handoff for stale implementation signals before dispatch
    #[command(name = "audit-handoff")]
    AuditHandoff {
        /// Print structured JSON output instead of human-readable text
        #[arg(long)]
        json: bool,

        #[arg(value_name = "PATH")]
        path: PathBuf,
    },

    /// Show the active capture policy
    Policy {
        /// Print policy as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show scoped lifecycle state for the current worktree
    Status {
        /// Print status as JSON
        #[arg(long)]
        json: bool,

        /// Override the working directory used for scope selection
        #[arg(long)]
        cwd: Option<String>,
    },

    /// Check Cortina runtime prerequisites and temp-state health
    Doctor {
        /// Print doctor report as JSON
        #[arg(long)]
        json: bool,

        /// Override the working directory used for scope selection
        #[arg(long)]
        cwd: Option<String>,
    },

    /// Output statusline for Claude Code's statusLine.command
    Statusline {
        /// Disable ANSI color output
        #[arg(long)]
        no_color: bool,
    },
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::{Cli, Commands};

    #[test]
    fn parses_audit_handoff_command() {
        let cli = Cli::try_parse_from(["cortina", "audit-handoff", ".handoffs/cortina/example.md"])
            .expect("expected audit-handoff command to parse");

        assert!(matches!(
            cli.command,
            Commands::AuditHandoff { json: false, .. }
        ));
    }

    #[test]
    fn parses_audit_handoff_json_command() {
        let cli = Cli::try_parse_from([
            "cortina",
            "audit-handoff",
            "--json",
            ".handoffs/cortina/example.md",
        ])
        .expect("expected audit-handoff --json command to parse");

        assert!(matches!(
            cli.command,
            Commands::AuditHandoff { json: true, .. }
        ));
    }

    #[test]
    fn parses_explicit_adapter_command() {
        let cli = Cli::try_parse_from(["cortina", "adapter", "claude-code", "pre-tool-use"])
            .expect("expected adapter command to parse");

        assert!(matches!(cli.command, Commands::Adapter { .. }));
    }

    #[test]
    fn parses_explicit_session_end_adapter_command() {
        let cli = Cli::try_parse_from(["cortina", "adapter", "claude-code", "session-end"])
            .expect("expected session-end adapter command to parse");

        assert!(matches!(cli.command, Commands::Adapter { .. }));
    }

    #[test]
    fn parses_explicit_volva_adapter_command() {
        let cli = Cli::try_parse_from(["cortina", "adapter", "volva", "hook-event"])
            .expect("expected volva adapter command to parse");

        assert!(matches!(cli.command, Commands::Adapter { .. }));
    }

    #[test]
    fn keeps_compatibility_aliases_hidden_but_valid() {
        let cli = Cli::try_parse_from(["cortina", "pre-tool-use"])
            .expect("expected compatibility alias to parse");

        assert!(matches!(cli.command, Commands::PreToolUse));
        let prompt_submit = Cli::try_parse_from(["cortina", "user-prompt-submit"])
            .expect("expected prompt-submit alias to parse");
        assert!(matches!(prompt_submit.command, Commands::UserPromptSubmit));
        let pre_compact = Cli::try_parse_from(["cortina", "pre-compact"])
            .expect("expected pre-compact alias to parse");
        assert!(matches!(pre_compact.command, Commands::PreCompact));
        let session_end = Cli::try_parse_from(["cortina", "session-end"])
            .expect("expected session-end alias to parse");
        assert!(matches!(session_end.command, Commands::SessionEnd));
        let help = Cli::command().render_long_help().to_string();
        assert!(!help.contains("pre-tool-use"));
        assert!(!help.contains("user-prompt-submit"));
        assert!(!help.contains("pre-compact"));
        assert!(!help.contains("session-end"));
        assert!(help.contains("audit-handoff"));
    }

    #[test]
    fn parses_policy_command() {
        let cli = Cli::try_parse_from(["cortina", "policy"]).expect("expected policy command");
        assert!(matches!(cli.command, Commands::Policy { json: false }));

        let cli = Cli::try_parse_from(["cortina", "policy", "--json"])
            .expect("expected policy --json command");
        assert!(matches!(cli.command, Commands::Policy { json: true }));
    }

    #[test]
    fn parses_status_and_doctor_commands() {
        let cli = Cli::try_parse_from(["cortina", "status"]).expect("expected status command");
        assert!(matches!(
            cli.command,
            Commands::Status {
                json: false,
                cwd: None
            }
        ));

        let cli = Cli::try_parse_from(["cortina", "doctor", "--json", "--cwd", "/tmp/demo"])
            .expect("expected doctor command");
        assert!(matches!(
            cli.command,
            Commands::Doctor {
                json: true,
                cwd: Some(_)
            }
        ));

        let cli = Cli::try_parse_from(["cortina", "statusline", "--no-color"])
            .expect("expected statusline command");
        assert!(matches!(
            cli.command,
            Commands::Statusline { no_color: true }
        ));
    }

    #[test]
    fn help_includes_policy_command() {
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("policy"));
        assert!(help.contains("statusline"));
        assert!(help.contains("audit-handoff"));
    }
}
