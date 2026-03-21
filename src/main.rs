use std::io::{self, Read};

use anyhow::Result;
use clap::{Parser, Subcommand};

mod hooks;
mod utils;

#[derive(Parser)]
#[command(name = "cortina", version, about = "Hook runner for AI coding agents")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Handle PreToolUse events (command rewriting)
    PreToolUse,

    /// Handle PostToolUse events (error/correction/change capture)
    PostToolUse,

    /// Handle Stop events (session summary)
    Stop,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Read JSON from stdin (Claude Code hook protocol)
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;

    match cli.command {
        Commands::PreToolUse => hooks::pre_tool_use::handle(&input),
        Commands::PostToolUse => hooks::post_tool_use::handle(&input),
        Commands::Stop => hooks::stop::handle(&input),
    }
}
