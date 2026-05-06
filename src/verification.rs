//! Placeholder interface for two-stage hook verification.
//!
//! `VerificationRubric` and its supporting types define the structure for
//! rubric-based hook evaluation. Deterministic checks run first (fast, no model
//! call). The `LlmRubric` field is a placeholder — the LLM grader call itself
//! is deferred to a follow-on handoff. This module is not yet wired into the
//! hook executor; the types are present so callers can begin constructing rubrics.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// Kinds of deterministic checks that can be evaluated without LLM assistance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeterministicCheckKind {
    /// Check that a file exists at the given path.
    /// Relative paths are resolved against the process working directory.
    /// Prefer absolute paths for reliability across hook invocation contexts.
    FileExists,
    /// Check that the hook output contains the given string.
    CanaryString,
    /// Check that the hook exit code matches the given integer value.
    ExitCode,
}

/// A single deterministic check to be evaluated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeterministicCheck {
    pub kind: DeterministicCheckKind,
    /// The check target: a file path, a canary string, or an exit code string.
    pub target: String,
}

/// LLM-based rubric for pass/fail evaluation.
///
/// Placeholder interface only — the LLM grader call is not implemented yet.
/// Present here so callers can define rubrics before the grader is wired in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRubric {
    pub pass_criteria: String,
    pub fail_criteria: String,
}

/// A named rubric for two-stage hook verification.
///
/// When attached to a hook signal, cortina evaluates `deterministic_checks`
/// first. If all pass, and `llm_rubric` is set, the LLM grader is invoked.
/// `exclusions` are patterns that, if found in the output, bypass all checks
/// — they represent known-benign output that should not trigger a fail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationRubric {
    /// Human-readable label for what is being verified.
    pub condition: String,
    /// Fast deterministic checks evaluated before any LLM call.
    pub deterministic_checks: Vec<DeterministicCheck>,
    /// Optional LLM rubric; only evaluated when deterministic checks pass.
    pub llm_rubric: Option<LlmRubric>,
    /// Patterns that bypass all checks when found in the hook output.
    pub exclusions: Vec<String>,
}

/// Evaluate deterministic checks against hook output and exit code.
///
/// Returns `Ok(())` if all checks pass. Returns `Err` with a descriptive
/// message on the first failing check. If any exclusion pattern is found in
/// `output`, all checks are bypassed and `Ok(())` is returned immediately.
pub fn evaluate_deterministic_checks(
    checks: &[DeterministicCheck],
    output: &str,
    exit_code: i32,
    exclusions: &[String],
) -> Result<(), String> {
    for exclusion in exclusions {
        if output.contains(exclusion.as_str()) {
            return Ok(());
        }
    }
    for check in checks {
        match check.kind {
            DeterministicCheckKind::FileExists => {
                if !std::path::Path::new(&check.target).exists() {
                    return Err(format!("file not found: {}", check.target));
                }
            }
            DeterministicCheckKind::CanaryString => {
                if !output.contains(check.target.as_str()) {
                    return Err(format!("canary string not found: {}", check.target));
                }
            }
            DeterministicCheckKind::ExitCode => {
                let expected: i32 = check
                    .target
                    .parse()
                    .map_err(|_| format!("ExitCode target is not a valid integer: {:?}", check.target))?;
                if exit_code != expected {
                    return Err(format!("exit code {exit_code}, expected {expected}"));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn canary_string_check_fails_when_absent() {
        let checks = vec![DeterministicCheck {
            kind: DeterministicCheckKind::CanaryString,
            target: "expected-token".into(),
        }];
        let result = evaluate_deterministic_checks(&checks, "no match here", 0, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn canary_string_check_passes_when_present() {
        let checks = vec![DeterministicCheck {
            kind: DeterministicCheckKind::CanaryString,
            target: "expected-token".into(),
        }];
        let result = evaluate_deterministic_checks(&checks, "output: expected-token", 0, &[]);
        assert!(result.is_ok());
    }

    #[test]
    fn exclusion_pattern_bypasses_fail() {
        let checks = vec![DeterministicCheck {
            kind: DeterministicCheckKind::CanaryString,
            target: "missing-canary".into(),
        }];
        let exclusions = vec!["known-benign-pattern".into()];
        let result =
            evaluate_deterministic_checks(&checks, "known-benign-pattern output", 0, &exclusions);
        assert!(result.is_ok());
    }

    #[test]
    fn exit_code_check_fails_on_mismatch() {
        let checks = vec![DeterministicCheck {
            kind: DeterministicCheckKind::ExitCode,
            target: "0".into(),
        }];
        let result = evaluate_deterministic_checks(&checks, "", 1, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn exit_code_check_passes_on_match() {
        let checks = vec![DeterministicCheck {
            kind: DeterministicCheckKind::ExitCode,
            target: "1".into(),
        }];
        let result = evaluate_deterministic_checks(&checks, "", 1, &[]);
        assert!(result.is_ok());
    }

    #[test]
    fn exit_code_malformed_target_returns_err() {
        let checks = vec![DeterministicCheck {
            kind: DeterministicCheckKind::ExitCode,
            target: "not-a-number".into(),
        }];
        let result = evaluate_deterministic_checks(&checks, "", 0, &[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not a valid integer"));
    }

    #[test]
    fn empty_checks_always_passes() {
        let result = evaluate_deterministic_checks(&[], "", 1, &[]);
        assert!(result.is_ok());
    }

    #[test]
    fn multiple_checks_all_must_pass() {
        let checks = vec![
            DeterministicCheck {
                kind: DeterministicCheckKind::CanaryString,
                target: "success".into(),
            },
            DeterministicCheck {
                kind: DeterministicCheckKind::ExitCode,
                target: "0".into(),
            },
        ];
        let result = evaluate_deterministic_checks(&checks, "output: success", 0, &[]);
        assert!(result.is_ok());
    }

    #[test]
    fn multiple_checks_fail_on_first_failure() {
        let checks = vec![
            DeterministicCheck {
                kind: DeterministicCheckKind::CanaryString,
                target: "missing".into(),
            },
            DeterministicCheck {
                kind: DeterministicCheckKind::ExitCode,
                target: "0".into(),
            },
        ];
        let result = evaluate_deterministic_checks(&checks, "output: success", 0, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn file_exists_check_passes_when_file_present() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "test").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();
        let checks = vec![DeterministicCheck {
            kind: DeterministicCheckKind::FileExists,
            target: path,
        }];
        let result = evaluate_deterministic_checks(&checks, "", 0, &[]);
        assert!(result.is_ok());
    }

    #[test]
    fn file_exists_check_fails_when_absent() {
        let checks = vec![DeterministicCheck {
            kind: DeterministicCheckKind::FileExists,
            target: "/tmp/this-file-does-not-exist-cortina-test-12345".into(),
        }];
        let result = evaluate_deterministic_checks(&checks, "", 0, &[]);
        assert!(result.is_err());
    }
}
