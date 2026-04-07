use std::io::{self, Read};

use anyhow::Result;
use clap::Parser;

mod adapters;
mod cli;
mod events;
mod handoff_audit;
mod handoff_lint;
mod handoff_paths;
mod hooks;
mod outcomes;
mod policy;
mod status;
mod statusline;
#[cfg(test)]
mod test_support;
mod utils;

use adapters::ClaudeCodeEventCommand;
use cli::{Cli, Commands};

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
        Commands::UserPromptSubmit => {
            let input = read_stdin()?;
            adapters::handle_legacy_claude_command(ClaudeCodeEventCommand::UserPromptSubmit, &input)
        }
        Commands::PreCompact => {
            let input = read_stdin()?;
            adapters::handle_legacy_claude_command(ClaudeCodeEventCommand::PreCompact, &input)
        }
        Commands::Stop => {
            let input = read_stdin()?;
            adapters::handle_legacy_claude_command(ClaudeCodeEventCommand::Stop, &input)
        }
        Commands::SessionEnd => {
            let input = read_stdin()?;
            adapters::handle_legacy_claude_command(ClaudeCodeEventCommand::SessionEnd, &input)
        }
        Commands::AuditHandoff { json, path } => handoff_audit::handle(&path, json),
        Commands::Policy { json } => print_policy(json),
        Commands::Status { json, cwd } => status::print_status(json, cwd.as_deref()),
        Commands::Doctor { json, cwd } => status::print_doctor(json, cwd.as_deref()),
        Commands::Statusline { no_color } => statusline::handle_stdin(no_color),
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
        "stale_handoff_detection_enabled={}",
        policy.stale_handoff_detection_enabled
    );
    println!("handoff_lint_enabled={}", policy.handoff_lint_enabled);
    println!(
        "rhizome_suggest_threshold={}",
        policy.rhizome_suggest_threshold
    );
    println!("rhizome_suggest_every={}", policy.rhizome_suggest_every);
    println!("rhizome_suggest_enabled={}", policy.rhizome_suggest_enabled);
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
