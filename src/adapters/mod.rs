use anyhow::Result;
use clap::Subcommand;

use crate::hooks;

pub mod claude_code;

#[derive(Subcommand)]
pub enum AdapterCommand {
    /// Handle Claude Code hook adapter events
    #[command(name = "claude-code")]
    ClaudeCode {
        #[command(subcommand)]
        event: ClaudeCodeEventCommand,
    },
}

#[derive(Clone, Copy, Subcommand)]
pub enum ClaudeCodeEventCommand {
    /// Handle `PreToolUse` adapter events (command rewriting)
    #[command(name = "pre-tool-use")]
    PreToolUse,

    /// Handle `PostToolUse` adapter events (error/correction/change capture)
    #[command(name = "post-tool-use")]
    PostToolUse,

    /// Handle `Stop` adapter events (session summary)
    #[command(name = "stop")]
    Stop,

    /// Handle `SessionEnd` adapter events (session summary)
    #[command(name = "session-end")]
    SessionEnd,
}

pub fn handle_adapter_command(adapter: &AdapterCommand, input: &str) -> Result<()> {
    match adapter {
        AdapterCommand::ClaudeCode { event } => handle_claude_code_event(*event, input),
    }
}

pub fn handle_legacy_claude_command(event: ClaudeCodeEventCommand, input: &str) -> Result<()> {
    handle_claude_code_event(event, input)
}

fn handle_claude_code_event(event: ClaudeCodeEventCommand, input: &str) -> Result<()> {
    match event {
        ClaudeCodeEventCommand::PreToolUse => hooks::pre_tool_use::handle(input),
        ClaudeCodeEventCommand::PostToolUse => hooks::post_tool_use::handle(input),
        ClaudeCodeEventCommand::Stop | ClaudeCodeEventCommand::SessionEnd => {
            hooks::stop::handle(input)
        }
    }
}
