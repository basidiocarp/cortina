use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::handoff_paths::ChecklistItem;

const PASTE_START: &str = "<!-- PASTE START -->";
const PASTE_END: &str = "<!-- PASTE END -->";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffAudit {
    pub file: PathBuf,
    pub total_checkboxes: usize,
    pub checked_checkboxes: usize,
    pub unchecked_checkboxes: Vec<ChecklistItem>,
    pub empty_paste_markers: Vec<usize>,
}

pub fn audit_handoff(path: &Path) -> Result<HandoffAudit> {
    let content = fs::read_to_string(path).context("failed to read handoff document")?;
    audit_handoff_content(path, &content)
}

pub(crate) fn audit_handoff_content(path: &Path, content: &str) -> Result<HandoffAudit> {
    let checklist_items = content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| parse_checklist_item(line, index + 1))
        .collect::<Vec<_>>();
    let checked_checkboxes = checklist_items.iter().filter(|item| item.checked).count();
    let unchecked_checkboxes: Vec<ChecklistItem> = checklist_items
        .into_iter()
        .filter(|item| !item.checked)
        .collect();

    Ok(HandoffAudit {
        file: path.to_path_buf(),
        total_checkboxes: checked_checkboxes + unchecked_checkboxes.len(),
        checked_checkboxes,
        unchecked_checkboxes,
        empty_paste_markers: find_empty_paste_markers(content),
    })
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

fn find_empty_paste_markers(content: &str) -> Vec<usize> {
    let mut empty_markers = Vec::new();
    let mut active_start = None;
    let mut has_content = false;

    for (index, line) in content.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();

        if trimmed == PASTE_START {
            active_start = Some(line_number);
            has_content = false;
            continue;
        }

        if trimmed == PASTE_END {
            if let Some(start_line) = active_start.take()
                && !has_content
            {
                empty_markers.push(start_line);
            }
            has_content = false;
            continue;
        }

        if active_start.is_some() && !trimmed.is_empty() {
            has_content = true;
        }
    }

    if let Some(start_line) = active_start
        && !has_content
    {
        empty_markers.push(start_line);
    }

    empty_markers
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::audit_handoff;

    fn write_handoff(dir: &TempDir, content: &str) -> std::path::PathBuf {
        let path = dir.path().join("handoff.md");
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn handoff_lint_counts_all_checked_boxes() {
        let dir = TempDir::new().unwrap();
        let path = write_handoff(
            &dir,
            r#"# Handoff

- [x] First
- [X] Second

<!-- PASTE START -->
ok
<!-- PASTE END -->
"#,
        );

        let audit = audit_handoff(&path).unwrap();

        assert_eq!(audit.total_checkboxes, 2);
        assert_eq!(audit.checked_checkboxes, 2);
        assert!(audit.unchecked_checkboxes.is_empty());
        assert!(audit.empty_paste_markers.is_empty());
    }

    #[test]
    fn handoff_lint_tracks_unchecked_boxes() {
        let dir = TempDir::new().unwrap();
        let path = write_handoff(
            &dir,
            r#"# Handoff

- [ ] First
- [x] Second
- [ ] Third
"#,
        );

        let audit = audit_handoff(&path).unwrap();

        assert_eq!(audit.total_checkboxes, 3);
        assert_eq!(audit.checked_checkboxes, 1);
        assert_eq!(audit.unchecked_checkboxes.len(), 2);
        assert_eq!(audit.unchecked_checkboxes[0].line_number, 3);
        assert_eq!(audit.unchecked_checkboxes[1].line_number, 5);
    }

    #[test]
    fn handoff_lint_detects_empty_paste_markers() {
        let dir = TempDir::new().unwrap();
        let path = write_handoff(
            &dir,
            r#"# Handoff

**Output:**
<!-- PASTE START -->

<!-- PASTE END -->

**More output:**
<!-- PASTE START -->
present
<!-- PASTE END -->

**Unclosed output:**
<!-- PASTE START -->
"#,
        );

        let audit = audit_handoff(&path).unwrap();

        assert_eq!(audit.empty_paste_markers, vec![4, 14]);
    }
}
