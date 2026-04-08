use std::path::PathBuf;
use std::process::Command;

use spore::logging::{SpanContext, subprocess_span, tool_span};
use spore::{Tool, discover};
use tracing::{debug, warn};

#[derive(Debug, Clone, Copy)]
pub enum Importance {
    #[allow(dead_code, reason = "Reserved for future use")]
    Low,
    Medium,
    High,
}

impl Importance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

fn command_path(name: &str) -> Option<PathBuf> {
    let tool = Tool::from_binary_name(name)?;
    discover(tool).map(|info| info.binary_path)
}

fn span_context(tool: &str) -> SpanContext {
    let context = SpanContext::for_app("cortina").with_tool(tool);
    match std::env::current_dir() {
        Ok(path) => context.with_workspace_root(path.display().to_string()),
        Err(_) => context,
    }
}

pub(crate) fn resolved_command(name: &str) -> Option<Command> {
    let binary_path = command_path(name)?;
    Some(Command::new(binary_path))
}

pub fn command_exists(name: &str) -> bool {
    command_path(name).is_some()
}

pub fn store_in_hyphae(topic: &str, content: &str, importance: Importance, project: Option<&str>) {
    let context = span_context("hyphae_store");
    let _tool_span = tool_span("hyphae_store", &context).entered();
    let Some(mut cmd) = resolved_command("hyphae") else {
        debug!("Hyphae binary is not discoverable; skipping store");
        return;
    };
    cmd.args(["store", "--topic", topic])
        .args(["--content", content])
        .args(["--importance", importance.as_str()])
        .args(["--keywords", "cortina,hook"]);

    if let Some(proj) = project {
        cmd.args(["-P", proj]);
    }

    let _spawn_span = subprocess_span("hyphae store", &context).entered();
    if let Err(err) = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        warn!("Failed to spawn hyphae store command: {err}");
    }
}

pub fn spawn_async_checked(cmd: &str, args: &[&str]) -> bool {
    let context = span_context(cmd);
    let _tool_span = tool_span("spawn_async_checked", &context).entered();
    let Some(mut command) = resolved_command(cmd) else {
        debug!("Command {cmd} is not discoverable; skipping async spawn");
        return false;
    };
    for arg in args {
        command.arg(arg);
    }

    let _spawn_span = subprocess_span(cmd, &context).entered();
    match command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => true,
        Err(err) => {
            warn!("Failed to spawn {cmd}: {err}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use spore::Tool;

    #[test]
    fn known_tools_map_to_spore_tooling() {
        assert_eq!(Tool::from_binary_name("mycelium"), Some(Tool::Mycelium));
        assert_eq!(Tool::from_binary_name("hyphae"), Some(Tool::Hyphae));
        assert_eq!(Tool::from_binary_name("rhizome"), Some(Tool::Rhizome));
        assert_eq!(Tool::from_binary_name("cortina"), Some(Tool::Cortina));
        assert_eq!(Tool::from_binary_name("canopy"), Some(Tool::Canopy));
    }

    #[test]
    fn unknown_tools_do_not_claim_spore_support() {
        assert_eq!(Tool::from_binary_name("git"), None);
        assert_eq!(Tool::from_binary_name("python"), None);
    }
}
