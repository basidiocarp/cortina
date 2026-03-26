use std::io::{self, Read};

use anyhow::Result;
use clap::{Parser, Subcommand};

mod adapters;
mod events;
mod hooks;
mod utils;

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

#[derive(Subcommand)]
enum AdapterCommand {
    /// Handle Claude Code hook adapter events
    #[command(name = "claude-code")]
    ClaudeCode {
        #[command(subcommand)]
        event: ClaudeCodeEventCommand,
    },
}

#[derive(Subcommand)]
enum ClaudeCodeEventCommand {
    /// Handle `PreToolUse` adapter events (command rewriting)
    #[command(name = "pre-tool-use")]
    PreToolUse,

    /// Handle `PostToolUse` adapter events (error/correction/change capture)
    #[command(name = "post-tool-use")]
    PostToolUse,

    /// Handle `Stop` adapter events (session summary)
    #[command(name = "stop")]
    Stop,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Read the current host adapter envelope from stdin.
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;

    match cli.command {
        Commands::Adapter { adapter } => match adapter {
            AdapterCommand::ClaudeCode { event } => match event {
                ClaudeCodeEventCommand::PreToolUse => hooks::pre_tool_use::handle(&input),
                ClaudeCodeEventCommand::PostToolUse => hooks::post_tool_use::handle(&input),
                ClaudeCodeEventCommand::Stop => hooks::stop::handle(&input),
            },
        },
        Commands::PreToolUse => hooks::pre_tool_use::handle(&input),
        Commands::PostToolUse => hooks::post_tool_use::handle(&input),
        Commands::Stop => hooks::stop::handle(&input),
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
