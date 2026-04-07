use anyhow::Result;
use clap::Subcommand;

use crate::hooks;

pub mod claude_code;
pub mod volva;

#[derive(Subcommand)]
pub enum AdapterCommand {
    /// Handle Claude Code hook adapter events
    #[command(name = "claude-code")]
    ClaudeCode {
        #[command(subcommand)]
        event: ClaudeCodeEventCommand,
    },

    /// Handle Volva runtime adapter events
    #[command(name = "volva")]
    Volva {
        #[command(subcommand)]
        event: VolvaEventCommand,
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

    /// Handle `UserPromptSubmit` adapter events (prompt capture)
    #[command(name = "user-prompt-submit")]
    UserPromptSubmit,

    /// Handle `PreCompact` adapter events (compaction snapshot capture)
    #[command(name = "pre-compact")]
    PreCompact,

    /// Handle `Stop` adapter events (session summary)
    #[command(name = "stop")]
    Stop,

    /// Handle `SessionEnd` adapter events (session summary)
    #[command(name = "session-end")]
    SessionEnd,
}

#[derive(Clone, Copy, Subcommand)]
pub enum VolvaEventCommand {
    /// Handle a normalized Volva hook event
    #[command(name = "hook-event")]
    HookEvent,
}

pub fn handle_adapter_command(adapter: &AdapterCommand, input: &str) -> Result<()> {
    match adapter {
        AdapterCommand::ClaudeCode { event } => handle_claude_code_event(*event, input),
        AdapterCommand::Volva { event } => handle_volva_event(*event, input),
    }
}

pub fn handle_legacy_claude_command(event: ClaudeCodeEventCommand, input: &str) -> Result<()> {
    handle_claude_code_event(event, input)
}

fn handle_claude_code_event(event: ClaudeCodeEventCommand, input: &str) -> Result<()> {
    match event {
        ClaudeCodeEventCommand::PreToolUse => hooks::pre_tool_use::handle(input),
        ClaudeCodeEventCommand::PostToolUse => hooks::post_tool_use::handle(input),
        ClaudeCodeEventCommand::UserPromptSubmit => hooks::user_prompt_submit::handle(input),
        ClaudeCodeEventCommand::PreCompact => hooks::pre_compact::handle(input),
        ClaudeCodeEventCommand::Stop | ClaudeCodeEventCommand::SessionEnd => {
            hooks::stop::handle(input)
        }
    }
}

fn handle_volva_event(event: VolvaEventCommand, input: &str) -> Result<()> {
    match event {
        VolvaEventCommand::HookEvent => volva::handle_hook_event(input),
    }
}
