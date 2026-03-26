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
    /// Handle Claude Code `PreToolUse` adapter events (command rewriting)
    PreToolUse,

    /// Handle Claude Code `PostToolUse` adapter events (error/correction/change capture)
    PostToolUse,

    /// Handle Claude Code `Stop` adapter events (session summary)
    Stop,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Read the current host adapter envelope from stdin.
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;

    match cli.command {
        Commands::PreToolUse => hooks::pre_tool_use::handle(&input),
        Commands::PostToolUse => hooks::post_tool_use::handle(&input),
        Commands::Stop => hooks::stop::handle(&input),
    }
}
