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

fn command_path(name: &str) -> Option<PathBuf> {
    let tool = Tool::from_binary_name(name)?;
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
