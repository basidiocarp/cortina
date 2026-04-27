use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Determines the workspace root for path validation.
fn get_workspace_root() -> PathBuf {
    std::env::var("CORTINA_WORKSPACE_ROOT")
        .or_else(|_| std::env::var("WORKSPACE_ROOT"))
        .map_or_else(
            |_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            PathBuf::from,
        )
}

/// Canonicalizes a path and returns it only if it stays within the workspace root.
/// Out-of-root paths are silently skipped — no error is returned to the caller.
///
/// Both absolute paths and relative paths with traversal components (e.g. `../../etc/passwd`)
/// are checked. Relative paths are joined against the workspace root before checking.
pub(crate) fn canonicalize_and_gate(candidate: &str) -> Option<String> {
    let candidate_path = Path::new(candidate);
    let workspace_root = get_workspace_root();

    // Resolve to an absolute path. For relative paths, join against workspace root
    // so traversal components like `../../etc/passwd` are normalised correctly.
    let joined = if candidate_path.is_absolute() {
        candidate_path.to_path_buf()
    } else {
        workspace_root.join(candidate_path)
    };

    // Prefer real canonicalization (resolves symlinks). Fall back to manual component
    // normalization for paths that don't exist yet (common for planned file references).
    let resolved = joined.canonicalize().unwrap_or_else(|_| normalize_path(&joined));

    if resolved.starts_with(&workspace_root) {
        // Return the original candidate string — callers only need the gated decision.
        Some(candidate.to_string())
    } else {
        None
    }
}

/// Resolves `.` and `..` components without requiring the path to exist on disk.
fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            c => normalized.push(c),
        }
    }
    normalized
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffPaths {
    pub handoff_file: PathBuf,
    pub referenced_paths: Vec<String>,
    pub checklist_items: Vec<ChecklistItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecklistItem {
    pub line_number: usize,
    pub text: String,
    pub checked: bool,
}

pub fn extract_paths(handoff_path: &Path) -> Result<HandoffPaths> {
    let content = fs::read_to_string(handoff_path).context("failed to read handoff document")?;
    let mut checklist_items = Vec::new();
    let referenced_paths = extract_paths_from_text(&content);

    for (index, line) in content.lines().enumerate() {
        if let Some(item) = parse_checklist_item(line, index + 1) {
            checklist_items.push(item);
        }
    }

    Ok(HandoffPaths {
        handoff_file: handoff_path.to_path_buf(),
        referenced_paths,
        checklist_items,
    })
}

pub(crate) fn extract_paths_from_text(text: &str) -> Vec<String> {
    let mut referenced_paths = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut in_code_block = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        collect_path_candidates(line, &mut referenced_paths, &mut seen_paths);
        if !in_code_block {
            collect_backtick_paths(line, &mut referenced_paths, &mut seen_paths);
        }
    }

    referenced_paths
}

fn parse_checklist_item(line: &str, line_number: usize) -> Option<ChecklistItem> {
    let trimmed = line.trim_start();
    let (checked, text) = if let Some(text) = trimmed.strip_prefix("- [ ]") {
        (false, text)
    } else if let Some(text) = trimmed.strip_prefix("- [x]") {
        (true, text)
    } else if let Some(text) = trimmed.strip_prefix("- [X]") {
        (true, text)
    } else if let Some(text) = trimmed.strip_prefix("* [ ]") {
        (false, text)
    } else if let Some(text) = trimmed.strip_prefix("* [x]") {
        (true, text)
    } else if let Some(text) = trimmed.strip_prefix("* [X]") {
        (true, text)
    } else {
        return None;
    };

    Some(ChecklistItem {
        line_number,
        text: text.trim().to_string(),
        checked,
    })
}

fn collect_backtick_paths(
    line: &str,
    referenced_paths: &mut Vec<String>,
    seen_paths: &mut HashSet<String>,
) {
    let mut in_backticks = false;
    let mut current = String::new();

    for ch in line.chars() {
        if ch == '`' {
            if in_backticks {
                push_path_candidate(&current, referenced_paths, seen_paths);
                current.clear();
            }
            in_backticks = !in_backticks;
            continue;
        }

        if in_backticks {
            current.push(ch);
        }
    }
}

fn collect_path_candidates(
    text: &str,
    referenced_paths: &mut Vec<String>,
    seen_paths: &mut HashSet<String>,
) {
    for token in text.split_whitespace() {
        let candidate = token.trim_matches(|ch: char| {
            matches!(
                ch,
                '`' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | ':' | '*'
            )
        });
        push_path_candidate(candidate, referenced_paths, seen_paths);
    }
}

fn push_path_candidate(
    candidate: &str,
    referenced_paths: &mut Vec<String>,
    seen_paths: &mut HashSet<String>,
) {
    let candidate = candidate.trim();
    if !looks_like_path(candidate) {
        return;
    }

    // Canonicalize and gate: skip if outside workspace root
    let Some(canonical) = canonicalize_and_gate(candidate) else {
        return;
    };

    if seen_paths.insert(canonical.clone()) {
        referenced_paths.push(canonical);
    }
}

fn looks_like_path(candidate: &str) -> bool {
    if candidate.is_empty() || candidate.contains(' ') || candidate.starts_with('-') {
        return false;
    }

    if candidate.starts_with("http://") || candidate.starts_with("https://") {
        return false;
    }

    if candidate.contains('/') || candidate.contains('\\') {
        return true;
    }

    if candidate.starts_with('.') && candidate.contains('.') {
        return true;
    }

    if candidate.contains('.') {
        let leaf = candidate.rsplit('/').next().unwrap_or(candidate);
        return leaf.chars().any(char::is_lowercase) || leaf.contains("Cargo");
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_handoff(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn extracts_paths_from_files_to_modify_section() {
        let dir = TempDir::new().unwrap();
        let path = write_handoff(
            &dir,
            "test.md",
            r"# Handoff

#### Files to modify

- **`cortina/src/handoff_paths.rs`** — parser
- **`cortina/src/hooks/stop.rs`** — hook
",
        );

        let extracted = extract_paths(&path).unwrap();
        assert!(
            extracted
                .referenced_paths
                .iter()
                .any(|value| value == "cortina/src/handoff_paths.rs")
        );
        assert!(
            extracted
                .referenced_paths
                .iter()
                .any(|value| value == "cortina/src/hooks/stop.rs")
        );
    }

    #[test]
    fn extracts_inline_backtick_paths_from_checklist_items() {
        let dir = TempDir::new().unwrap();
        let path = write_handoff(
            &dir,
            "checklist.md",
            r"# Handoff

- [ ] Update `cortina/src/handoff_audit.rs`
- [x] Wire `canopy/src/runtime.rs`
",
        );

        let extracted = extract_paths(&path).unwrap();
        assert!(
            extracted
                .referenced_paths
                .iter()
                .any(|value| value == "cortina/src/handoff_audit.rs")
        );
        assert!(
            extracted
                .referenced_paths
                .iter()
                .any(|value| value == "canopy/src/runtime.rs")
        );
    }

    #[test]
    fn returns_checklist_items_with_checked_status() {
        let dir = TempDir::new().unwrap();
        let path = write_handoff(
            &dir,
            "code-block.md",
            r"# Handoff

```rust
cortina/src/cli.rs
```

- [ ] First item
- [x] Second item
",
        );

        let extracted = extract_paths(&path).unwrap();
        assert_eq!(extracted.checklist_items.len(), 2);
        assert!(!extracted.checklist_items[0].checked);
        assert!(extracted.checklist_items[1].checked);
        assert!(
            extracted
                .referenced_paths
                .iter()
                .any(|value| value == "cortina/src/cli.rs")
        );
    }

    #[test]
    fn silently_skips_paths_outside_workspace_root() {
        let dir = TempDir::new().unwrap();
        let path = write_handoff(
            &dir,
            "security.md",
            r"# Handoff

#### Files to modify

- **`cortina/src/handoff_paths.rs`** — valid path
- **`/etc/passwd`** — invalid path
- **`/tmp/secret-file`** — invalid path

- [ ] Update path validation
",
        );

        let extracted = extract_paths(&path).unwrap();

        // Should contain the valid relative path (or its canonical form)
        assert!(
            extracted.referenced_paths.iter().any(|p| p.contains("handoff_paths.rs")),
            "Valid relative path should be included: {:?}",
            extracted.referenced_paths
        );

        // Should NOT contain absolute paths outside workspace
        assert!(
            !extracted
                .referenced_paths
                .iter()
                .any(|p| p.contains("/etc/passwd")),
            "Path /etc/passwd should be excluded: {:?}",
            extracted.referenced_paths
        );
        assert!(
            !extracted
                .referenced_paths
                .iter()
                .any(|p| p.contains("/tmp/secret-file")),
            "Path /tmp/secret-file should be excluded: {:?}",
            extracted.referenced_paths
        );
    }
}
