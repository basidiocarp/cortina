use std::io::{self, Read};

use anyhow::Result;
use clap::{Parser, Subcommand};

mod adapters;
mod events;
mod hooks;
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
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Read the current host adapter envelope from stdin.
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;

    match cli.command {
        Commands::Adapter { adapter } => adapters::handle_adapter_command(&adapter, &input),
        Commands::PreToolUse => {
            adapters::handle_legacy_claude_command(ClaudeCodeEventCommand::PreToolUse, &input)
        }
        Commands::PostToolUse => {
            adapters::handle_legacy_claude_command(ClaudeCodeEventCommand::PostToolUse, &input)
        }
        Commands::Stop => {
            adapters::handle_legacy_claude_command(ClaudeCodeEventCommand::Stop, &input)
        }
    }
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
    fn keeps_compatibility_aliases_hidden_but_valid() {
        let cli = Cli::try_parse_from(["cortina", "pre-tool-use"])
            .expect("expected compatibility alias to parse");

        assert!(matches!(cli.command, Commands::PreToolUse));
        let help = Cli::command().render_long_help().to_string();
        assert!(!help.contains("pre-tool-use"));
    }
}
