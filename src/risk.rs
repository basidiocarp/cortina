use std::path::Path;

/// Four-axis risk score for a single tool call.
///
/// Cortina captures and emits this score alongside lifecycle signals but does NOT
/// enforce or block — that is the responsibility of downstream consumers such as
/// volva and cap.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolRisk {
    /// How dangerous the tool category is (read=0.0, write=0.3, execute=0.7, network=0.5, delete=1.0)
    pub base_risk: f32,
    /// Sensitivity of the target file (config/secrets paths = higher)
    pub file_sensitivity: f32,
    /// How many files/systems could be affected
    pub blast_radius: f32,
    /// Whether the action can be undone (delete/overwrite = 1.0, read = 0.0)
    pub irreversibility: f32,
}

impl ToolRisk {
    /// Composite score: equal-weighted average of four axes, result in 0.0–1.0.
    pub fn composite(&self) -> f32 {
        (self.base_risk + self.file_sensitivity + self.blast_radius + self.irreversibility) / 4.0
    }
}

/// Advisory risk level derived from a composite score.
///
/// Cortina never actually blocks tool calls — `Block` is advisory only and
/// signals that downstream consumers should take action.
#[derive(Debug, Clone, PartialEq)]
pub enum RiskLevel {
    /// composite < 0.3
    Allow,
    /// composite 0.3..0.7
    Review,
    /// composite >= 0.7  (advisory — cortina does not block)
    Block,
}

impl RiskLevel {
    pub fn from_score(score: f32) -> Self {
        if score < 0.3 {
            Self::Allow
        } else if score < 0.7 {
            Self::Review
        } else {
            Self::Block
        }
    }
}

/// Classify a tool call by name and optional target path.
///
/// Returns a [`ToolRisk`] (the four raw axes) and the derived [`RiskLevel`].
/// The file path is used only to score sensitivity — pass `None` when unknown.
pub fn classify_tool_call(tool_name: &str, file_path: Option<&str>) -> (ToolRisk, RiskLevel) {
    let (base_risk, irreversibility, blast_radius) = base_axes(tool_name);
    let file_sensitivity = file_path.map_or(0.2, score_file_sensitivity);

    let risk = ToolRisk {
        base_risk,
        file_sensitivity,
        blast_radius,
        irreversibility,
    };
    let level = RiskLevel::from_score(risk.composite());
    (risk, level)
}

/// Returns `(base_risk, irreversibility, blast_radius)` for a tool name.
fn base_axes(tool_name: &str) -> (f32, f32, f32) {
    match tool_name {
        // Read-only tools — safe to observe, reversible, narrow scope.
        "Read" | "Glob" | "Grep" | "LS" => (0.0, 0.0, 0.1),

        // Write/edit tools — mutate state, broader scope, hard to fully undo.
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => (0.3, 0.7, 0.4),

        // Shell execution — arbitrary effects, broad blast radius.
        "Bash" => (0.7, 0.6, 0.7),

        // Network tools — external side-effects, but stateless from local FS perspective.
        "WebFetch" | "WebSearch" => (0.2, 0.0, 0.2),

        // Agent/task delegation — indirect effects, hard to fully predict scope.
        "Agent" | "Task" => (0.4, 0.3, 0.6),

        // Unknown or future tools — assume low-moderate risk.
        _ => (0.2, 0.2, 0.2),
    }
}

/// Score how sensitive the targeted file path is.
///
/// The path is lowercased before matching so that the keyword checks are
/// case-insensitive.  Extension comparisons use [`Path::extension`] to avoid
/// the `case_sensitive_file_extension_comparisons` lint.
fn score_file_sensitivity(path: &str) -> f32 {
    let lower = path.to_lowercase();

    // Secrets and credentials — highest sensitivity.
    if lower.contains(".env")
        || lower.contains("secret")
        || lower.contains("key")
        || lower.contains("credential")
        || lower.contains("token")
    {
        return 0.9;
    }

    // Config and manifest files — medium-high.
    // `Cargo.toml` is caught by the `.toml` arm below.
    let p = Path::new(&lower);
    if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
        if matches!(ext, "toml" | "yaml" | "yml" | "json") {
            return 0.5;
        }

        // Source files — medium.
        if matches!(ext, "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "c" | "cpp" | "h") {
            return 0.3;
        }
    }

    // Default — unknown or low-sensitivity file.
    0.2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn read_tool_allows() {
        let (risk, level) = classify_tool_call("Read", None);
        assert_eq!(level, RiskLevel::Allow, "composite={}", risk.composite());
    }

    #[test]
    fn glob_tool_allows() {
        let (risk, level) = classify_tool_call("Glob", None);
        assert_eq!(level, RiskLevel::Allow, "composite={}", risk.composite());
    }

    #[test]
    fn grep_tool_allows() {
        let (risk, level) = classify_tool_call("Grep", None);
        assert_eq!(level, RiskLevel::Allow, "composite={}", risk.composite());
    }

    #[test]
    fn write_to_secrets_path_is_review_or_block() {
        let (risk, level) = classify_tool_call("Write", Some("/project/.env"));
        let score = risk.composite();
        assert!(
            matches!(level, RiskLevel::Review | RiskLevel::Block),
            "expected Review or Block for .env write, got {level:?} (composite={score})"
        );
    }

    #[test]
    fn bash_is_review_or_block() {
        let (risk, level) = classify_tool_call("Bash", None);
        let score = risk.composite();
        assert!(
            matches!(level, RiskLevel::Review | RiskLevel::Block),
            "expected Review or Block for Bash, got {level:?} (composite={score})"
        );
    }

    #[test]
    fn env_file_scores_high_sensitivity() {
        // test via classify_tool_call with a read tool so only file sensitivity drives
        let (risk, _level) = classify_tool_call("Read", Some("/home/user/.env"));
        assert!(
            approx_eq(risk.file_sensitivity, 0.9),
            "expected .env to score 0.9 sensitivity, got {}",
            risk.file_sensitivity
        );
    }

    #[test]
    fn unknown_tool_uses_reasonable_default() {
        let (risk, level) = classify_tool_call("SomeFutureTool", None);
        // Unknown tool should be Allow or Review — not an extreme score.
        assert_ne!(
            level,
            RiskLevel::Block,
            "unknown tool should not immediately be Block (composite={})",
            risk.composite()
        );
        // All axes should be the unknown-tool defaults.
        assert!(approx_eq(risk.base_risk, 0.2), "base_risk={}", risk.base_risk);
        assert!(
            approx_eq(risk.irreversibility, 0.2),
            "irreversibility={}",
            risk.irreversibility
        );
        assert!(
            approx_eq(risk.blast_radius, 0.2),
            "blast_radius={}",
            risk.blast_radius
        );
    }

    #[test]
    fn write_tool_is_in_write_category() {
        let (risk, _) = classify_tool_call("Write", None);
        assert!(approx_eq(risk.base_risk, 0.3), "base_risk={}", risk.base_risk);
        assert!(
            approx_eq(risk.irreversibility, 0.7),
            "irreversibility={}",
            risk.irreversibility
        );
    }

    #[test]
    fn composite_is_equal_weight_average() {
        let risk = ToolRisk {
            base_risk: 0.4,
            file_sensitivity: 0.4,
            blast_radius: 0.4,
            irreversibility: 0.4,
        };
        let expected = 0.4_f32;
        assert!(
            approx_eq(risk.composite(), expected),
            "composite={} expected={}",
            risk.composite(),
            expected
        );
    }

    #[test]
    fn risk_level_thresholds() {
        assert_eq!(RiskLevel::from_score(0.0), RiskLevel::Allow);
        assert_eq!(RiskLevel::from_score(0.29), RiskLevel::Allow);
        assert_eq!(RiskLevel::from_score(0.3), RiskLevel::Review);
        assert_eq!(RiskLevel::from_score(0.69), RiskLevel::Review);
        assert_eq!(RiskLevel::from_score(0.7), RiskLevel::Block);
        assert_eq!(RiskLevel::from_score(1.0), RiskLevel::Block);
    }
}
