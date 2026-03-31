use std::io::{self, Read};

use anyhow::Result;
use clap::{Parser, Subcommand};

mod adapters;
mod events;
mod hooks;
mod outcomes;
mod policy;
mod status;
mod utils;

use adapters::{AdapterCommand, ClaudeCodeEventCommand};

#[derive(Parser)]
#[command(
    name = "cortina",
    version,
    about = "Lifecycle signal runner for AI coding agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Handle host adapter events through an explicit adapter surface
    Adapter {
        #[command(subcommand)]
        adapter: AdapterCommand,
    },

    /// Compatibility alias for `cortina adapter claude-code pre-tool-use`
    #[command(name = "pre-tool-use", hide = true)]
    PreToolUse,

    /// Compatibility alias for `cortina adapter claude-code post-tool-use`
    #[command(name = "post-tool-use", hide = true)]
    PostToolUse,

    /// Compatibility alias for `cortina adapter claude-code stop`
    #[command(name = "stop", hide = true)]
    Stop,

    /// Compatibility alias for `cortina adapter claude-code session-end`
    #[command(name = "session-end", hide = true)]
    SessionEnd,

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
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Adapter { adapter } => {
            let input = read_stdin()?;
            adapters::handle_adapter_command(&adapter, &input)
        }
        Commands::PreToolUse => {
            let input = read_stdin()?;
            adapters::handle_legacy_claude_command(ClaudeCodeEventCommand::PreToolUse, &input)
        }
        Commands::PostToolUse => {
            let input = read_stdin()?;
            adapters::handle_legacy_claude_command(ClaudeCodeEventCommand::PostToolUse, &input)
        }
        Commands::Stop => {
            let input = read_stdin()?;
            adapters::handle_legacy_claude_command(ClaudeCodeEventCommand::Stop, &input)
        }
        Commands::SessionEnd => {
            let input = read_stdin()?;
            adapters::handle_legacy_claude_command(ClaudeCodeEventCommand::SessionEnd, &input)
        }
        Commands::Policy { json } => print_policy(json),
        Commands::Status { json, cwd } => status::print_status(json, cwd.as_deref()),
        Commands::Doctor { json, cwd } => status::print_doctor(json, cwd.as_deref()),
    }
}

fn read_stdin() -> Result<String> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    Ok(input)
}

fn print_policy(json: bool) -> Result<()> {
    let policy = policy::capture_policy();
    if json {
        println!("{}", serde_json::to_string_pretty(policy)?);
        return Ok(());
    }

    println!("Cortina capture policy");
    println!(
        "outcome_dedupe_window_ms={}",
        policy.outcome_dedupe_window_ms
    );
    println!("correction_window_ms={}", policy.correction_window_ms);
    println!("edit_cleanup_age_ms={}", policy.edit_cleanup_age_ms);
    println!("export_threshold={}", policy.export_threshold);
    println!("ingest_threshold={}", policy.ingest_threshold);
    println!(
        "outcome_attribution_grace_ms={}",
        policy.outcome_attribution_grace_ms
    );
    println!("max_outcome_events={}", policy.max_outcome_events);
    println!(
        "fallback_session_memory_on_end_failure={}",
        policy.fallback_session_memory_on_end_failure
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::{Cli, Commands};

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
    fn keeps_compatibility_aliases_hidden_but_valid() {
        let cli = Cli::try_parse_from(["cortina", "pre-tool-use"])
            .expect("expected compatibility alias to parse");

        assert!(matches!(cli.command, Commands::PreToolUse));
        let session_end = Cli::try_parse_from(["cortina", "session-end"])
            .expect("expected session-end alias to parse");
        assert!(matches!(session_end.command, Commands::SessionEnd));
        let help = Cli::command().render_long_help().to_string();
        assert!(!help.contains("pre-tool-use"));
        assert!(!help.contains("session-end"));
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
    }

    #[test]
    fn help_includes_policy_command() {
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("policy"));
    }
}
