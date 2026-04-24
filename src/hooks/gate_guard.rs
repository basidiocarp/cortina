use std::collections::HashMap;
use std::time::{Duration, Instant};

/// State for a single gate instance.
#[derive(Debug, Clone)]
pub enum GateState {
    /// First attempt blocked. Return the fact template to the model.
    Blocked { fact_template: String },
    /// Model is investigating. Gate will allow on next call that includes facts.
    #[allow(dead_code)]
    Investigating { started_at: Instant },
    /// Investigation accepted. Operation is allowed for `ttl` from `allowed_at`.
    Allowed { allowed_at: Instant },
}

impl GateState {
    const TTL: Duration = Duration::from_secs(30 * 60); // 30 minutes

    pub fn is_expired(&self) -> bool {
        match self {
            GateState::Allowed { allowed_at } => allowed_at.elapsed() > Self::TTL,
            GateState::Investigating { started_at } => started_at.elapsed() > Self::TTL,
            GateState::Blocked { .. } => false,
        }
    }
}

/// Gate key: stable identifier for one (tool, target) pair.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GateKey {
    pub tool: String,
    /// For file-targeted tools: canonical path. For Bash: hash of command prefix.
    pub target: String,
}

pub type GateMap = HashMap<GateKey, GateState>;

/// Decision returned by the gate evaluation.
pub enum GateDecision {
    /// Allow the operation to proceed.
    Allow,
    /// Block with a message explaining what facts are needed.
    Block { message: String },
}

// ─────────────────────────────────────────────────────────────────────────
// Fact templates by gate type
// ─────────────────────────────────────────────────────────────────────────

pub const EDIT_GATE_TEMPLATE: &str = "[cortina] Before this edit can proceed, gather these facts:\n\
1. Run Grep to find every file that imports or calls the target file or symbol.\n\
2. List every public API that would change signature or behavior.\n\
3. Confirm the data schema (field names, types, required/optional) before and after.\n\
4. Quote the verbatim user instruction that authorized this edit.\n\
Present these facts in your next message, then retry.\n";

pub const WRITE_GATE_TEMPLATE: &str = "[cortina] Before creating this file, gather these facts:\n\
1. Run Glob to confirm no file at this path already exists.\n\
2. Identify the caller: what existing code will import or invoke this new file?\n\
3. Confirm the schema or interface contract (if the file exports anything).\n\
Present these facts in your next message, then retry.\n";

pub const DESTRUCTIVE_BASH_TEMPLATE: &str = "[cortina] This command is destructive and cannot be undone without manual intervention.\n\
Before proceeding, state:\n\
1. The complete list of targets that will be affected (files, rows, branches, etc.).\n\
2. The rollback procedure if this command produces the wrong outcome.\n\
Present these facts in your next message, then retry.\n";

pub const ROUTINE_BASH_TEMPLATE: &str =
    "[cortina] Confirm the purpose of this command in one sentence, then retry.";

// ─────────────────────────────────────────────────────────────────────────
// Destructive bash pattern detection
// ─────────────────────────────────────────────────────────────────────────

/// Returns true if the bash command matches a destructive pattern that triggers the gate every time.
pub fn is_destructive_bash(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let patterns = [
        "rm -rf",
        "rm -fr",
        "git reset --hard",
        "git clean -f",
        "git clean -fd",
        "drop table",
        "drop database",
        "truncate table",
        "git push --force",
        "git push -f",
        "dd if=",
        "mkfs.",
    ];
    patterns.iter().any(|p| lower.contains(p))
}

/// Returns true if the bash command is read-only git and bypasses the routine gate.
pub fn is_readonly_git(command: &str) -> bool {
    let lower = command.trim().to_ascii_lowercase();
    let bypassed = [
        "git log",
        "git diff",
        "git status",
        "git show",
        "git branch",
        "git remote -v",
    ];
    bypassed.iter().any(|p| lower.starts_with(p))
}

// ─────────────────────────────────────────────────────────────────────────
// Gate evaluation
// ─────────────────────────────────────────────────────────────────────────

/// Evaluate whether a gate should allow or block the operation.
///
/// On first call (gate not in map): returns Block with the appropriate fact template.
/// On retry with investigation content: returns Allow.
/// Expired gates are treated as new gates (restart the cycle).
pub fn evaluate_gate(key: &GateKey, map: &mut GateMap, has_investigation: bool) -> GateDecision {
    // Clean up expired gates.
    if let Some(state) = map.get(key) {
        if state.is_expired() {
            map.remove(key);
        }
    }

    match map.get(key) {
        None => {
            // First call: determine which template to use based on tool type.
            let template = match key.tool.as_str() {
                "Edit" | "MultiEdit" => EDIT_GATE_TEMPLATE.to_string(),
                "Write" => WRITE_GATE_TEMPLATE.to_string(),
                "Bash" => {
                    // For Bash, we'll determine the template later based on the actual command.
                    // For now, default to routine. The caller will override this.
                    ROUTINE_BASH_TEMPLATE.to_string()
                }
                _ => return GateDecision::Allow, // Unknown tool, allow by default.
            };

            map.insert(
                key.clone(),
                GateState::Blocked {
                    fact_template: template.clone(),
                },
            );

            GateDecision::Block { message: template }
        }
        Some(GateState::Blocked { fact_template }) => {
            if has_investigation {
                // Model provided investigation content. Transition to Allowed.
                map.insert(
                    key.clone(),
                    GateState::Allowed {
                        allowed_at: Instant::now(),
                    },
                );
                GateDecision::Allow
            } else {
                // No investigation yet. Re-emit the template.
                GateDecision::Block {
                    message: fact_template.clone(),
                }
            }
        }
        Some(GateState::Investigating { .. }) => {
            if has_investigation {
                // Model provided facts. Transition to Allowed.
                map.insert(
                    key.clone(),
                    GateState::Allowed {
                        allowed_at: Instant::now(),
                    },
                );
                GateDecision::Allow
            } else {
                // Still investigating. Keep blocking until facts arrive.
                let template = match key.tool.as_str() {
                    "Edit" | "MultiEdit" => EDIT_GATE_TEMPLATE.to_string(),
                    "Write" => WRITE_GATE_TEMPLATE.to_string(),
                    _ => ROUTINE_BASH_TEMPLATE.to_string(),
                };
                GateDecision::Block { message: template }
            }
        }
        Some(GateState::Allowed { .. }) => {
            // Gate is already allowed and within TTL. Proceed.
            GateDecision::Allow
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_destructive_bash_patterns() {
        assert!(is_destructive_bash("rm -rf /tmp/data"));
        assert!(is_destructive_bash("rm -fr /tmp/data"));
        assert!(is_destructive_bash("git reset --hard"));
        assert!(is_destructive_bash("git clean -f"));
        assert!(is_destructive_bash("git clean -fd"));
        assert!(is_destructive_bash("DROP TABLE users"));
        assert!(is_destructive_bash("drop database mydb"));
        assert!(is_destructive_bash("TRUNCATE TABLE logs"));
        assert!(is_destructive_bash("git push --force"));
        assert!(is_destructive_bash("git push -f"));
        assert!(is_destructive_bash("dd if=/dev/zero of=/tmp/test"));
        assert!(is_destructive_bash("mkfs.ext4 /dev/sda1"));

        assert!(!is_destructive_bash("git log"));
        assert!(!is_destructive_bash("ls -la"));
        assert!(!is_destructive_bash("cargo test"));
    }

    #[test]
    fn test_readonly_git_patterns() {
        assert!(is_readonly_git("git log"));
        assert!(is_readonly_git("git diff"));
        assert!(is_readonly_git("git status"));
        assert!(is_readonly_git("git show"));
        assert!(is_readonly_git("git branch"));
        assert!(is_readonly_git("git remote -v"));

        assert!(!is_readonly_git("git commit -m 'test'"));
        assert!(!is_readonly_git("git push"));
        assert!(!is_readonly_git("git reset --hard"));
    }

    #[test]
    fn test_gate_first_call_blocks() {
        let mut map = GateMap::new();
        let key = GateKey {
            tool: "Edit".to_string(),
            target: "/tmp/src/main.rs".to_string(),
        };

        let decision = evaluate_gate(&key, &mut map, false);
        match decision {
            GateDecision::Block { message } => {
                assert!(message.contains("Grep"));
                assert!(message.contains("public API"));
            }
            GateDecision::Allow => panic!("Expected block on first call"),
        }
    }

    #[test]
    fn test_gate_second_call_with_investigation_allows() {
        let mut map = GateMap::new();
        let key = GateKey {
            tool: "Write".to_string(),
            target: "/tmp/new_file.rs".to_string(),
        };

        // First call: blocked
        let decision1 = evaluate_gate(&key, &mut map, false);
        assert!(matches!(decision1, GateDecision::Block { .. }));

        // Second call with investigation: allowed
        let decision2 = evaluate_gate(&key, &mut map, true);
        assert!(matches!(decision2, GateDecision::Allow));

        // Third call: still allowed (within TTL)
        let decision3 = evaluate_gate(&key, &mut map, false);
        assert!(matches!(decision3, GateDecision::Allow));
    }

    #[test]
    fn test_gate_blocked_state_reblocks_without_investigation() {
        let mut map = GateMap::new();
        let key = GateKey {
            tool: "Edit".to_string(),
            target: "/tmp/src/lib.rs".to_string(),
        };

        // First call: blocked
        let decision1 = evaluate_gate(&key, &mut map, false);
        assert!(matches!(decision1, GateDecision::Block { .. }));

        // Second call without investigation: still blocked
        let decision2 = evaluate_gate(&key, &mut map, false);
        assert!(matches!(decision2, GateDecision::Block { .. }));
    }

    #[test]
    fn test_multiedit_uses_edit_template() {
        let mut map = GateMap::new();
        let key = GateKey {
            tool: "MultiEdit".to_string(),
            target: "/tmp/src/complex.rs".to_string(),
        };

        let decision = evaluate_gate(&key, &mut map, false);
        match decision {
            GateDecision::Block { message } => {
                assert!(message.contains("Grep"));
                assert_eq!(
                    message.trim(),
                    EDIT_GATE_TEMPLATE.trim(),
                    "MultiEdit should use EDIT_GATE_TEMPLATE"
                );
            }
            GateDecision::Allow => panic!("Expected block on first call"),
        }
    }

    #[test]
    fn test_bash_routine_template() {
        let mut map = GateMap::new();
        let key = GateKey {
            tool: "Bash".to_string(),
            target: "command_hash".to_string(),
        };

        let decision = evaluate_gate(&key, &mut map, false);
        match decision {
            GateDecision::Block { message } => {
                assert!(message.contains("purpose"));
                assert_eq!(message.trim(), ROUTINE_BASH_TEMPLATE.trim());
            }
            GateDecision::Allow => panic!("Expected block on first call"),
        }
    }
}
