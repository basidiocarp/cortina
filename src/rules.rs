/// A single pre-write relevance rule.
pub struct RelevanceRule {
    /// Tool operation: `"Write"`, `"Edit"`, `"MultiEdit"`
    pub operation: &'static str,
    /// Optional suffix for target file (None = applies to all files)
    pub file_pattern: Option<&'static str>,
    /// Tools that should have been called before this operation
    pub recommended_tools: &'static [&'static str],
    /// "required" or "recommended" — reserved for future tiered advisory wording
    #[allow(dead_code)] // TODO: use to vary advisory message strength
    pub severity: &'static str,
    /// How far back to check: "session" = all calls this session — reserved for windowed checks
    #[allow(dead_code)] // TODO: use to implement recent_10/recent_5 check windows
    pub check_window: &'static str,
}

/// The default bundled ruleset.
pub const DEFAULT_RULES: &[RelevanceRule] = &[
    RelevanceRule {
        operation: "Write",
        file_pattern: Some("*.rs"),
        recommended_tools: &[
            "mcp__rhizome__get_structure",
            "mcp__rhizome__get_symbols",
            "mcp__rhizome__find_references",
        ],
        severity: "recommended",
        check_window: "session",
    },
    RelevanceRule {
        operation: "Edit",
        file_pattern: Some("*.rs"),
        recommended_tools: &[
            "mcp__rhizome__get_structure",
            "mcp__rhizome__get_symbols",
            "mcp__rhizome__find_references",
        ],
        severity: "recommended",
        check_window: "session",
    },
    RelevanceRule {
        operation: "MultiEdit",
        file_pattern: Some("*.rs"),
        recommended_tools: &[
            "mcp__rhizome__get_structure",
            "mcp__rhizome__get_symbols",
            "mcp__rhizome__find_references",
        ],
        severity: "recommended",
        check_window: "session",
    },
    RelevanceRule {
        operation: "Write",
        file_pattern: Some("*.ts"),
        recommended_tools: &["mcp__rhizome__get_structure", "mcp__rhizome__get_symbols"],
        severity: "recommended",
        check_window: "session",
    },
    RelevanceRule {
        operation: "Edit",
        file_pattern: Some("*.ts"),
        recommended_tools: &["mcp__rhizome__get_structure", "mcp__rhizome__get_symbols"],
        severity: "recommended",
        check_window: "session",
    },
    RelevanceRule {
        operation: "MultiEdit",
        file_pattern: Some("*.ts"),
        recommended_tools: &["mcp__rhizome__get_structure", "mcp__rhizome__get_symbols"],
        severity: "recommended",
        check_window: "session",
    },
];

/// Returns matching rules for the given operation and file path.
///
/// Matches on `operation` (exact) and `file_pattern` (if Some, checks that
/// `file_path` ends with the pattern suffix after stripping the leading `*`).
/// Rules with `file_pattern` are skipped when no `file_path` is provided.
pub fn matching_rules<'a>(
    rules: &'a [RelevanceRule],
    operation: &str,
    file_path: Option<&str>,
) -> Vec<&'a RelevanceRule> {
    rules
        .iter()
        .filter(|rule| {
            if rule.operation != operation {
                return false;
            }
            match (rule.file_pattern, file_path) {
                (None, _) => true,
                (Some(_), None) => false,
                (Some(pattern), Some(path)) => {
                    let suffix = pattern.trim_start_matches('*');
                    path.ends_with(suffix)
                }
            }
        })
        .collect()
}

/// Returns true if at least one recommended tool from `rule` appears in `called_tools`.
pub fn any_recommended_called(rule: &RelevanceRule, called_tools: &[impl AsRef<str>]) -> bool {
    rule.recommended_tools.iter().any(|recommended| {
        called_tools
            .iter()
            .any(|called| called.as_ref() == *recommended)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_rules_filters_by_operation() {
        let rules = matching_rules(DEFAULT_RULES, "Write", Some("main.rs"));
        assert!(rules.iter().all(|r| r.operation == "Write"));
        assert!(!rules.is_empty());
    }

    #[test]
    fn matching_rules_filters_by_file_extension() {
        let rs_rules = matching_rules(DEFAULT_RULES, "Write", Some("src/lib.rs"));
        assert!(!rs_rules.is_empty());
        assert!(rs_rules.iter().all(|r| r.file_pattern == Some("*.rs")));

        let ts_rules = matching_rules(DEFAULT_RULES, "Edit", Some("src/index.ts"));
        assert!(!ts_rules.is_empty());
        assert!(ts_rules.iter().all(|r| r.file_pattern == Some("*.ts")));
    }

    #[test]
    fn matching_rules_skips_pattern_rules_when_no_file_path() {
        let rules = matching_rules(DEFAULT_RULES, "Write", None);
        // All default rules have file_pattern, so none should match
        assert!(rules.is_empty());
    }

    #[test]
    fn matching_rules_returns_empty_for_unmatched_extension() {
        let rules = matching_rules(DEFAULT_RULES, "Write", Some("README.md"));
        assert!(rules.is_empty());
    }

    #[test]
    fn any_recommended_called_returns_true_when_match_found() {
        let rule = &DEFAULT_RULES[0]; // Write *.rs
        let called = vec!["mcp__rhizome__get_symbols", "Read"];
        assert!(any_recommended_called(rule, &called));
    }

    #[test]
    fn any_recommended_called_returns_false_when_no_match() {
        let rule = &DEFAULT_RULES[0]; // Write *.rs
        let called = vec!["Read", "Grep", "Bash"];
        assert!(!any_recommended_called(rule, &called));
    }

    #[test]
    fn any_recommended_called_works_with_empty_called_list() {
        let rule = &DEFAULT_RULES[0];
        let called: Vec<&str> = vec![];
        assert!(!any_recommended_called(rule, &called));
    }
}
