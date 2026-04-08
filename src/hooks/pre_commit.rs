use anyhow::Result;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::handoff_lint::audit_handoff;
use crate::utils::{load_json_file, scope_hash, temp_state_path};

pub fn handoff_pre_commit_warnings(command: &str, cwd: Option<&str>) -> Vec<String> {
    if !looks_like_git_commit(command) {
        return Vec::new();
    }

    let Some(cwd) = cwd else {
        return Vec::new();
    };

    let cwd = Path::new(cwd);
    let Some(workspace_root) = workspace_root_from_cwd(cwd) else {
        return Vec::new();
    };
    let handoff_files = session_handoff_files(cwd, &workspace_root);
    validate_session_handoffs(&handoff_files).unwrap_or_default()
}

fn looks_like_git_commit(command: &str) -> bool {
    let tokens = command.split_whitespace().collect::<Vec<_>>();
    tokens
        .windows(2)
        .any(|pair| pair[0] == "git" && pair[1] == "commit")
}

fn workspace_root_from_cwd(cwd: &Path) -> Option<PathBuf> {
    cwd.ancestors()
        .find(|ancestor| ancestor.join(".handoffs/HANDOFFS.md").exists())
        .map(Path::to_path_buf)
}

#[derive(serde::Deserialize)]
struct EditEntry {
    file: String,
}

fn session_handoff_files(cwd: &Path, workspace_root: &Path) -> Vec<PathBuf> {
    let cwd_hash = scope_hash(cwd.to_str());
    let workspace_hash = scope_hash(workspace_root.to_str());
    let mut resolved = BTreeSet::new();

    for hash in [cwd_hash, workspace_hash] {
        for entry in recent_edit_paths(&hash) {
            if let Some(path) =
                resolve_modified_handoff_path(Path::new(&entry), cwd, workspace_root)
            {
                resolved.insert(path);
            }
        }
    }

    resolved.into_iter().collect()
}

fn recent_edit_paths(hash: &str) -> Vec<String> {
    load_json_file::<Vec<EditEntry>>(temp_state_path("edits", hash, "json"))
        .unwrap_or_default()
        .into_iter()
        .map(|entry| entry.file)
        .collect()
}

pub fn validate_session_handoffs(handoff_files: &[PathBuf]) -> Result<Vec<String>> {
    let mut warnings = Vec::new();

    for handoff_path in handoff_files {
        if !handoff_path.exists() {
            continue;
        }

        let audit = audit_handoff(&handoff_path)?;
        if !audit.empty_paste_markers.is_empty() {
            warnings.push(format!(
                "cortina: active handoff {} has empty paste markers at lines {}",
                handoff_path.display(),
                audit
                    .empty_paste_markers
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        if !audit.unchecked_checkboxes.is_empty() {
            warnings.push(format!(
                "cortina: active handoff {} still has unchecked items at {}",
                handoff_path.display(),
                audit
                    .unchecked_checkboxes
                    .iter()
                    .map(|item| format!("line {}: {}", item.line_number, item.text))
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
        }
    }

    Ok(warnings)
}

fn resolve_modified_handoff_path(
    session_file: &Path,
    cwd: &Path,
    workspace_root: &Path,
) -> Option<PathBuf> {
    if !session_file.extension().is_some_and(|ext| ext == "md") {
        return None;
    }

    let candidates = if session_file.is_absolute() {
        vec![session_file.to_path_buf()]
    } else {
        vec![
            cwd.join(session_file),
            workspace_root.join(session_file),
            workspace_root.join(session_file.strip_prefix("./").unwrap_or(session_file)),
        ]
    };

    candidates.into_iter().find(|candidate| {
        candidate.exists()
            && candidate
                .components()
                .any(|component| component.as_os_str() == ".handoffs")
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use crate::utils::{save_json_file, scope_hash, temp_state_path};

    use super::{handoff_pre_commit_warnings, looks_like_git_commit, validate_session_handoffs};

    #[test]
    fn detects_git_commit_commands() {
        assert!(looks_like_git_commit("git commit -m test"));
        assert!(looks_like_git_commit("env FOO=1 git commit --amend"));
        assert!(!looks_like_git_commit("git status"));
    }

    #[test]
    fn validate_session_handoffs_warns_on_empty_paste_markers() {
        let dir = TempDir::new().unwrap();
        let handoff_path = dir.path().join(".handoffs/cross-project/example.md");
        fs::create_dir_all(handoff_path.parent().unwrap()).unwrap();
        fs::write(
            &handoff_path,
            r#"# Handoff

- [x] done

<!-- PASTE START -->

<!-- PASTE END -->
"#,
        )
        .unwrap();

        let warnings = validate_session_handoffs(&[handoff_path]).unwrap();

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("empty paste markers"));
    }

    #[test]
    fn validate_session_handoffs_warns_on_unchecked_items() {
        let dir = TempDir::new().unwrap();
        let handoff_path = dir.path().join(".handoffs/cross-project/example.md");
        fs::create_dir_all(handoff_path.parent().unwrap()).unwrap();
        fs::write(
            &handoff_path,
            r#"# Handoff

- [ ] still open
"#,
        )
        .unwrap();

        let warnings = validate_session_handoffs(&[handoff_path]).unwrap();

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("unchecked items"));
    }

    #[test]
    fn skips_non_commit_commands() {
        assert!(handoff_pre_commit_warnings("git status", None).is_empty());
    }

    #[test]
    fn handoff_pre_commit_warnings_use_current_session_handoff_files() {
        let dir = TempDir::new().unwrap();
        let workspace_root = dir.path();
        let repo_root = workspace_root.join("cortina");
        let handoff_path = workspace_root.join(".handoffs/cross-project/example.md");
        fs::create_dir_all(handoff_path.parent().unwrap()).unwrap();
        fs::create_dir_all(&repo_root).unwrap();
        fs::write(workspace_root.join(".handoffs/HANDOFFS.md"), "# index\n").unwrap();
        fs::write(
            &handoff_path,
            r#"# Handoff

- [ ] still open
"#,
        )
        .unwrap();

        let hash = scope_hash(repo_root.to_str());
        let edits_path = temp_state_path("edits", &hash, "json");
        save_json_file(
            &edits_path,
            &vec![serde_json::json!({
                "file": ".handoffs/cross-project/example.md",
                "old_string": "",
                "new_string": "",
                "timestamp": 1
            })],
        )
        .unwrap();

        let warnings = handoff_pre_commit_warnings("git commit -m test", repo_root.to_str());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("unchecked items"));
    }
}
