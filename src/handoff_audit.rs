use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use crate::handoff_paths::{ChecklistItem, extract_paths, extract_paths_from_text};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditResult {
    pub handoff_file: PathBuf,
    pub total_items: usize,
    pub likely_implemented: usize,
    pub evidence: Vec<AuditEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditConfidence {
    ExplicitMatch,
    HeuristicMatch,
    Inconclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditEvidence {
    pub checklist_item: String,
    pub file_exists: bool,
    pub symbol_found: bool,
    pub test_found: bool,
    pub confidence: AuditConfidence,
    pub matched_paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditStatus {
    Proceed,
    FlagReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditOutput {
    pub status: AuditStatus,
    pub reason: Option<String>,
    pub result: AuditResult,
}

pub fn handle(handoff_path: &Path, json: bool) -> Result<()> {
    let result = audit_handoff(handoff_path)?;
    let output = audit_output(&result);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&output).context("serializing audit output")?
        );
        return Ok(());
    }

    println!("{}", format_audit_report(&output));

    if output.status == AuditStatus::FlagReview {
        anyhow::bail!(
            "{}",
            output.reason.as_deref().unwrap_or("handoff appears stale")
        );
    }

    Ok(())
}

pub fn audit_handoff(handoff_path: &Path) -> Result<AuditResult> {
    let parsed = extract_paths(handoff_path)?;
    let content = fs::read_to_string(handoff_path).context("failed to read handoff document")?;
    let repo_root = handoff_repo_root(handoff_path);
    let checklist_items = parsed
        .checklist_items
        .iter()
        .filter(|item| !item.checked)
        .collect::<Vec<_>>();
    let mut evidence = Vec::with_capacity(checklist_items.len());
    let mut likely_implemented = 0;

    for item in checklist_items {
        let step_context = step_block_for_line(&content, item.line_number);
        let item_paths = referenced_paths_for_item(item, &step_context);
        let existing_paths = existing_paths(repo_root.as_path(), &item_paths);
        let search_terms = search_terms_for_item(item);
        let symbol_found = symbol_found_in_paths(&existing_paths, &search_terms);
        let test_found = tests_found_for_item(
            repo_root.as_path(),
            &existing_paths,
            &item_paths,
            &search_terms,
        );
        let confidence = classify_confidence(&existing_paths, symbol_found, test_found);

        if confidence == AuditConfidence::ExplicitMatch {
            likely_implemented += 1;
        }

        evidence.push(AuditEvidence {
            checklist_item: item.text.clone(),
            file_exists: !existing_paths.is_empty(),
            symbol_found,
            test_found,
            confidence,
            matched_paths: existing_paths
                .iter()
                .map(|path| relativize_path(repo_root.as_path(), path))
                .collect(),
        });
    }

    Ok(AuditResult {
        handoff_file: handoff_path.to_path_buf(),
        total_items: evidence.len(),
        likely_implemented,
        evidence,
    })
}

#[must_use]
pub fn is_stale_signal(result: &AuditResult) -> bool {
    result.total_items > 0 && result.likely_implemented * 2 > result.total_items
}

#[must_use]
pub fn audit_output(result: &AuditResult) -> AuditOutput {
    let stale = is_stale_signal(result);
    AuditOutput {
        status: if stale {
            AuditStatus::FlagReview
        } else {
            AuditStatus::Proceed
        },
        reason: stale.then(|| {
            format!(
                "handoff appears stale: {} of {} checklist items have explicit implementation evidence",
                result.likely_implemented, result.total_items
            )
        }),
        result: result.clone(),
    }
}

pub fn format_audit_report(output: &AuditOutput) -> String {
    let mut rendered = match output.status {
        AuditStatus::Proceed => format!(
            "handoff audit: {} of {} checklist items have explicit implementation evidence",
            output.result.likely_implemented, output.result.total_items
        ),
        AuditStatus::FlagReview => output.reason.clone().unwrap_or_else(|| {
            format!(
                "handoff appears stale: {} of {} checklist items have explicit implementation evidence",
                output.result.likely_implemented, output.result.total_items
            )
        }),
    };

    for evidence in &output.result.evidence {
        let matched_paths = if evidence.matched_paths.is_empty() {
            "none".to_string()
        } else {
            evidence.matched_paths.join(", ")
        };
        let _ = write!(
            rendered,
            "\n- {} (confidence={:?}, file_exists={}, symbol_found={}, test_found={}, matched_paths={})",
            evidence.checklist_item,
            evidence.confidence,
            evidence.file_exists,
            evidence.symbol_found,
            evidence.test_found,
            matched_paths
        );
    }

    rendered
}

fn handoff_repo_root(handoff_path: &Path) -> PathBuf {
    handoff_path
        .ancestors()
        .find(|ancestor| ancestor.join(".handoffs").exists())
        .map_or_else(
            || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            Path::to_path_buf,
        )
}

fn step_block_for_line(content: &str, line_number: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return String::new();
    }

    let index = line_number.saturating_sub(1).min(lines.len() - 1);
    let mut start = 0;
    for cursor in (0..=index).rev() {
        if lines[cursor].trim_start().starts_with("### Step ") {
            start = cursor;
            break;
        }
    }

    let mut end = lines.len();
    for (cursor, line) in lines.iter().enumerate().skip(index + 1) {
        if line.trim_start().starts_with("### Step ") {
            end = cursor;
            break;
        }
    }

    lines[start..end].join("\n")
}

fn referenced_paths_for_item(item: &ChecklistItem, step_context: &str) -> Vec<String> {
    let mut paths = extract_paths_from_text(step_context);
    paths.extend(extract_paths_from_text(&item.text));
    dedupe_non_empty(paths)
}

fn search_terms_for_item(item: &ChecklistItem) -> Vec<String> {
    let path_terms: HashSet<String> = extract_paths_from_text(&item.text)
        .into_iter()
        .flat_map(|path| extract_identifier_like_terms(&path))
        .collect();

    dedupe_non_empty(
        extract_identifier_like_terms(&item.text)
            .into_iter()
            .filter(|term| !path_terms.contains(term))
            .collect(),
    )
}

fn extract_identifier_like_terms(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == '-' {
            current.push(ch);
        } else if !current.is_empty() {
            push_term(&mut terms, &current);
            current.clear();
        }
    }

    if !current.is_empty() {
        push_term(&mut terms, &current);
    }

    terms
}

fn push_term(terms: &mut Vec<String>, term: &str) {
    let normalized = term.trim_matches('-');
    if normalized.is_empty() || normalized.len() < 4 {
        return;
    }
    if normalized.chars().all(char::is_lowercase) && !normalized.contains('_') {
        return;
    }
    terms.push(normalized.replace('-', "_"));
}

fn dedupe_non_empty(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for value in values {
        let value = value.trim().to_string();
        if value.is_empty() {
            continue;
        }
        if seen.insert(value.clone()) {
            deduped.push(value);
        }
    }
    deduped
}

fn existing_paths(repo_root: &Path, item_paths: &[String]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();

    for item_path in item_paths {
        let candidate = resolve_repo_path(repo_root, item_path);
        if candidate.exists() && seen.insert(candidate.clone()) {
            paths.push(candidate);
        }
    }

    paths
}

fn resolve_repo_path(repo_root: &Path, path: &str) -> PathBuf {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        repo_root.join(candidate)
    }
}

fn symbol_found_in_paths(paths: &[PathBuf], terms: &[String]) -> bool {
    paths
        .iter()
        .filter(|path| !path_is_test_file(path))
        .any(|path| file_contains_any_term(path, terms))
}

fn tests_found_for_item(
    repo_root: &Path,
    existing_paths: &[PathBuf],
    item_paths: &[String],
    terms: &[String],
) -> bool {
    related_test_paths(repo_root, existing_paths, item_paths)
        .iter()
        .any(|path| file_contains_tests_with_terms(path, terms))
}

fn related_test_paths(
    repo_root: &Path,
    existing_paths: &[PathBuf],
    item_paths: &[String],
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    for path in existing_paths {
        if path_is_test_file(path) && seen.insert(path.clone()) {
            candidates.push(path.clone());
        }
    }

    for item_path in item_paths {
        let resolved = resolve_repo_path(repo_root, item_path);
        let Some(parent) = resolved.parent() else {
            continue;
        };
        let Some(stem) = resolved.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(file_name) = resolved.file_name().and_then(|value| value.to_str()) else {
            continue;
        };

        let path_variants = [
            parent.join(format!("{stem}_tests.rs")),
            parent.join("tests.rs"),
            parent.join("tests").join(file_name),
            parent.join("tests").join(format!("{stem}_tests.rs")),
            repo_root.join("tests").join(file_name),
            repo_root.join("tests").join(format!("{stem}_tests.rs")),
        ];

        for variant in path_variants {
            if variant.exists() && seen.insert(variant.clone()) {
                candidates.push(variant);
            }
        }
    }

    candidates
}

fn path_is_test_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.contains("test"))
        || path
            .components()
            .any(|component| component.as_os_str() == "tests")
}

fn file_contains_any_term(path: &Path, terms: &[String]) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    terms.iter().any(|term| content.contains(term))
}

fn file_contains_tests_with_terms(path: &Path, terms: &[String]) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    content.contains("#[test]") && terms.iter().any(|term| content.contains(term))
}

fn classify_confidence(
    existing_paths: &[PathBuf],
    symbol_found: bool,
    test_found: bool,
) -> AuditConfidence {
    if !existing_paths.is_empty() && (symbol_found || test_found) {
        AuditConfidence::ExplicitMatch
    } else if !existing_paths.is_empty() {
        AuditConfidence::HeuristicMatch
    } else {
        AuditConfidence::Inconclusive
    }
}

fn relativize_path(repo_root: &Path, path: &Path) -> String {
    path.strip_prefix(repo_root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn stale_signal_requires_majority_of_items() {
        let result = AuditResult {
            handoff_file: PathBuf::from("demo.md"),
            total_items: 4,
            likely_implemented: 3,
            evidence: Vec::new(),
        };

        assert!(is_stale_signal(&result));
        let result = AuditResult {
            likely_implemented: 2,
            ..result
        };
        assert!(!is_stale_signal(&result));
    }

    #[test]
    fn format_audit_report_includes_item_evidence() {
        let result = AuditResult {
            handoff_file: PathBuf::from("demo.md"),
            total_items: 1,
            likely_implemented: 1,
            evidence: vec![AuditEvidence {
                checklist_item: "use `cortina/src/cli.rs`".to_string(),
                file_exists: true,
                symbol_found: true,
                test_found: true,
                confidence: AuditConfidence::ExplicitMatch,
                matched_paths: vec!["cortina/src/cli.rs".to_string()],
            }],
        };

        let report = format_audit_report(&audit_output(&result));
        assert!(report.contains("1 of 1"));
        assert!(report.contains("confidence=ExplicitMatch"));
        assert!(report.contains("matched_paths=cortina/src/cli.rs"));
    }

    #[test]
    fn audit_handoff_counts_existing_items_in_a_temp_repo() {
        let dir = TempDir::new().unwrap();
        let repo_root = dir.path();
        fs::create_dir_all(repo_root.join(".handoffs/cortina")).unwrap();
        fs::create_dir_all(repo_root.join("src")).unwrap();
        fs::write(
            repo_root.join("src/handoff_audit.rs"),
            r"
pub struct AuditResult;

#[cfg(test)]
mod tests {
    #[test]
    fn audit() {}
}
",
        )
        .unwrap();
        fs::write(
            repo_root.join(".handoffs/cortina/demo.md"),
            r"# Demo

### Step 1

#### Files to modify

- **`src/handoff_audit.rs`**

- [ ] Add `AuditResult`
",
        )
        .unwrap();

        let result = audit_handoff(&repo_root.join(".handoffs/cortina/demo.md")).unwrap();
        assert_eq!(result.total_items, 1);
        assert_eq!(result.likely_implemented, 1);
        assert_eq!(
            result.evidence[0].confidence,
            AuditConfidence::ExplicitMatch
        );
    }

    #[test]
    fn audit_handoff_ignores_checked_checklist_items() {
        let dir = TempDir::new().unwrap();
        let repo_root = dir.path();
        fs::create_dir_all(repo_root.join(".handoffs/cortina")).unwrap();
        fs::create_dir_all(repo_root.join("src")).unwrap();
        fs::write(
            repo_root.join("src/already_done.rs"),
            "pub fn already_done() {}\n",
        )
        .unwrap();
        fs::write(
            repo_root.join(".handoffs/cortina/demo.md"),
            r"# Demo

### Step 1

#### Files to modify

- **`src/already_done.rs`**

- [x] Add `already_done`
- [ ] Tighten dispatch wording
",
        )
        .unwrap();

        let result = audit_handoff(&repo_root.join(".handoffs/cortina/demo.md")).unwrap();
        assert_eq!(result.total_items, 1);
        assert_eq!(result.likely_implemented, 0);
        assert_eq!(result.evidence.len(), 1);
        assert_eq!(
            result.evidence[0].checklist_item,
            "Tighten dispatch wording"
        );
        assert_eq!(
            result.evidence[0].confidence,
            AuditConfidence::HeuristicMatch
        );
    }

    #[test]
    fn audit_handoff_does_not_borrow_paths_from_other_steps() {
        let dir = TempDir::new().unwrap();
        let repo_root = dir.path();
        fs::create_dir_all(repo_root.join(".handoffs/cortina")).unwrap();
        fs::create_dir_all(repo_root.join("src")).unwrap();
        fs::write(
            repo_root.join("src/already_done.rs"),
            "pub fn already_done() {}\n",
        )
        .unwrap();
        fs::write(
            repo_root.join(".handoffs/cortina/demo.md"),
            r"# Demo

### Step 1

#### Files to modify

- **`src/already_done.rs`**

- [ ] Add `already_done`

### Step 2

- [ ] Tighten dispatch wording
",
        )
        .unwrap();

        let result = audit_handoff(&repo_root.join(".handoffs/cortina/demo.md")).unwrap();
        assert_eq!(result.total_items, 2);
        assert_eq!(result.likely_implemented, 1);
        assert_eq!(result.evidence[1].confidence, AuditConfidence::Inconclusive);
    }

    #[test]
    fn audit_handoff_does_not_borrow_identifiers_from_sibling_items() {
        let dir = TempDir::new().unwrap();
        let repo_root = dir.path();
        fs::create_dir_all(repo_root.join(".handoffs/cortina")).unwrap();
        fs::create_dir_all(repo_root.join("src")).unwrap();
        fs::write(repo_root.join("src/shared.rs"), "pub struct FirstThing;\n").unwrap();
        fs::write(
            repo_root.join(".handoffs/cortina/demo.md"),
            r"# Demo

### Step 1

#### Files to modify

- **`src/shared.rs`**

- [ ] Add `FirstThing`
- [ ] Add `SecondThing`
",
        )
        .unwrap();

        let result = audit_handoff(&repo_root.join(".handoffs/cortina/demo.md")).unwrap();
        assert_eq!(result.likely_implemented, 1);
        assert_eq!(
            result.evidence[0].confidence,
            AuditConfidence::ExplicitMatch
        );
        assert_eq!(
            result.evidence[1].confidence,
            AuditConfidence::HeuristicMatch
        );
        assert!(!result.evidence[1].symbol_found);
    }

    #[test]
    fn json_output_flags_review_without_nonzero_exit_contract() {
        let result = AuditResult {
            handoff_file: PathBuf::from("demo.md"),
            total_items: 2,
            likely_implemented: 2,
            evidence: Vec::new(),
        };

        let output = audit_output(&result);
        assert_eq!(output.status, AuditStatus::FlagReview);
        assert!(
            output
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("explicit implementation evidence"))
        );
    }
}
