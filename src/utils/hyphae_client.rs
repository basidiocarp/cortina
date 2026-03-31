use std::path::PathBuf;
use std::process::Command;

use spore::{Tool, discover};

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

fn spore_tool(name: &str) -> Option<Tool> {
    match name {
        "mycelium" => Some(Tool::Mycelium),
        "hyphae" => Some(Tool::Hyphae),
        "rhizome" => Some(Tool::Rhizome),
        "cortina" => Some(Tool::Cortina),
        "canopy" => Some(Tool::Canopy),
        _ => None,
    }
}

fn command_path(name: &str) -> Option<PathBuf> {
    let tool = spore_tool(name)?;
    discover(tool).map(|info| info.binary_path)
}

pub(crate) fn resolved_command(name: &str) -> Option<Command> {
    let binary_path = command_path(name)?;
    Some(Command::new(binary_path))
}

pub fn command_exists(name: &str) -> bool {
    command_path(name).is_some()
}

pub fn store_in_hyphae(topic: &str, content: &str, importance: Importance, project: Option<&str>) {
    let Some(mut cmd) = resolved_command("hyphae") else {
        return;
    };
    cmd.args(["store", "--topic", topic])
        .args(["--content", content])
        .args(["--importance", importance.as_str()])
        .args(["--keywords", "cortina,hook"]);

    if let Some(proj) = project {
        cmd.args(["-P", proj]);
    }

    let _ = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub fn spawn_async_checked(cmd: &str, args: &[&str]) -> bool {
    let Some(mut command) = resolved_command(cmd) else {
        return false;
    };
    for arg in args {
        command.arg(arg);
    }

    command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::spore_tool;

    #[test]
    fn known_tools_map_to_spore_tooling() {
        assert_eq!(spore_tool("mycelium"), Some(spore::Tool::Mycelium));
        assert_eq!(spore_tool("hyphae"), Some(spore::Tool::Hyphae));
        assert_eq!(spore_tool("rhizome"), Some(spore::Tool::Rhizome));
        assert_eq!(spore_tool("cortina"), Some(spore::Tool::Cortina));
        assert_eq!(spore_tool("canopy"), Some(spore::Tool::Canopy));
    }

    #[test]
    fn unknown_tools_do_not_claim_spore_support() {
        assert_eq!(spore_tool("git"), None);
        assert_eq!(spore_tool("python"), None);
    }
}
