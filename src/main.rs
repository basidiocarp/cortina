use std::io::{self, Read};

use anyhow::Result;
use clap::Parser;
use spore::logging::{SpanContext, root_span, workflow_span};
use tracing::Level;

mod adapters;
mod cli;
mod events;
mod handoff_audit;
mod handoff_lint;
mod handoff_paths;
mod hooks;
mod outcomes;
mod policy;
mod risk;
mod rules;
mod status;
mod statusline;
#[cfg(test)]
mod test_support;
mod tool_usage;
mod utils;

use adapters::ClaudeCodeEventCommand;
use cli::{Cli, Commands};

fn main() -> Result<()> {
    spore::logging::init_app("cortina", Level::WARN);

    // Initialize OTel tracer — no-op when OTEL_EXPORTER_OTLP_ENDPOINT is not set
    let _telemetry = spore::telemetry::init_tracer("cortina")
        .unwrap_or_else(|e| {
            tracing::debug!("OTel init skipped: {}", e);
            spore::telemetry::TelemetryInit::disabled("cortina")
        });

    let _runtime_span = root_span(&span_context_for_cwd(std::env::current_dir().ok())).entered();
    let cli = Cli::parse();

    match cli.command {
        Commands::Adapter { adapter } => {
            let input = read_stdin()?;
            let _workflow_span = workflow_span(
                adapter_operation_name(&adapter),
                &span_context_for_input(&input),
            )
            .entered();
            adapters::handle_adapter_command(&adapter, &input)
        }
        Commands::PreToolUse => {
            let input = read_stdin()?;
            let _workflow_span =
                workflow_span("claude_code_pre_tool_use", &span_context_for_input(&input))
                    .entered();
            adapters::handle_legacy_claude_command(ClaudeCodeEventCommand::PreToolUse, &input)
        }
        Commands::PostToolUse => {
            let input = read_stdin()?;
            let _workflow_span =
                workflow_span("claude_code_post_tool_use", &span_context_for_input(&input))
                    .entered();
            adapters::handle_legacy_claude_command(ClaudeCodeEventCommand::PostToolUse, &input)
        }
        Commands::UserPromptSubmit => {
            let input = read_stdin()?;
            let _workflow_span = workflow_span(
                "claude_code_user_prompt_submit",
                &span_context_for_input(&input),
            )
            .entered();
            adapters::handle_legacy_claude_command(ClaudeCodeEventCommand::UserPromptSubmit, &input)
        }
        Commands::PreCompact => {
            let input = read_stdin()?;
            let _workflow_span =
                workflow_span("claude_code_pre_compact", &span_context_for_input(&input)).entered();
            adapters::handle_legacy_claude_command(ClaudeCodeEventCommand::PreCompact, &input)
        }
        Commands::Stop => {
            let input = read_stdin()?;
            let _workflow_span =
                workflow_span("claude_code_stop", &span_context_for_input(&input)).entered();
            adapters::handle_legacy_claude_command(ClaudeCodeEventCommand::Stop, &input)
        }
        Commands::SessionEnd => {
            let input = read_stdin()?;
            let _workflow_span =
                workflow_span("claude_code_session_end", &span_context_for_input(&input)).entered();
            adapters::handle_legacy_claude_command(ClaudeCodeEventCommand::SessionEnd, &input)
        }
        Commands::AuditHandoff { json, path } => {
            let _workflow_span =
                workflow_span("audit_handoff", &span_context_for_cwd(None)).entered();
            handoff_audit::handle(&path, json)
        }
        Commands::Policy { json } => {
            let _workflow_span = workflow_span("policy", &span_context_for_cwd(None)).entered();
            print_policy(json)
        }
        Commands::Status { json, cwd } => {
            let _workflow_span = workflow_span(
                "status",
                &span_context_for_cwd(cwd.as_deref().map(std::path::PathBuf::from)),
            )
            .entered();
            status::print_status(json, cwd.as_deref())
        }
        Commands::Doctor { json, cwd } => {
            let _workflow_span = workflow_span(
                "doctor",
                &span_context_for_cwd(cwd.as_deref().map(std::path::PathBuf::from)),
            )
            .entered();
            status::print_doctor(json, cwd.as_deref())
        }
        Commands::Statusline { no_color } => {
            let _workflow_span = workflow_span("statusline", &span_context_for_cwd(None)).entered();
            statusline::handle_stdin(no_color)
        }
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

fn span_context_for_cwd(cwd: Option<std::path::PathBuf>) -> SpanContext {
    let context = SpanContext::for_app("cortina");
    match cwd {
        Some(path) => context.with_workspace_root(path.display().to_string()),
        None => context,
    }
}

fn span_context_for_input(input: &str) -> SpanContext {
    let value = serde_json::from_str::<serde_json::Value>(input).ok();
    let cwd = value
        .as_ref()
        .and_then(|value| {
            value
                .get("cwd")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    value
                        .get("workspace")
                        .and_then(|workspace| workspace.get("current_dir"))
                        .and_then(serde_json::Value::as_str)
                })
                .map(std::path::PathBuf::from)
        })
        .or_else(|| std::env::current_dir().ok());
    let context = span_context_for_cwd(cwd);
    match value
        .as_ref()
        .and_then(|value| value.get("session_id"))
        .and_then(serde_json::Value::as_str)
    {
        Some(session_id) => context.with_session_id(session_id.to_string()),
        None => context,
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::span_context_for_input;

    #[test]
    fn span_context_for_input_includes_session_and_workspace_context() {
        let context = span_context_for_input(
            r#"{
                "session_id": "ses-123",
                "cwd": "/tmp/demo"
            }"#,
        );

        assert_eq!(context.session_id.as_deref(), Some("ses-123"));
        assert_eq!(context.workspace_root.as_deref(), Some("/tmp/demo"));
    }
}

fn adapter_operation_name(adapter: &adapters::AdapterCommand) -> &'static str {
    match adapter {
        adapters::AdapterCommand::ClaudeCode { event } => match event {
            ClaudeCodeEventCommand::PreToolUse => "adapter_claude_code_pre_tool_use",
            ClaudeCodeEventCommand::PostToolUse => "adapter_claude_code_post_tool_use",
            ClaudeCodeEventCommand::UserPromptSubmit => "adapter_claude_code_user_prompt_submit",
            ClaudeCodeEventCommand::PreCompact => "adapter_claude_code_pre_compact",
            ClaudeCodeEventCommand::Stop => "adapter_claude_code_stop",
            ClaudeCodeEventCommand::SessionEnd => "adapter_claude_code_session_end",
        },
        adapters::AdapterCommand::Volva { .. } => "adapter_volva_hook_event",
    }
}
