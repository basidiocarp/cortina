use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::Value;

const DEFAULT_CONTEXT_LIMIT: usize = 200_000;

#[derive(Debug, Default, Deserialize)]
struct StatuslineInput {
    #[serde(default)]
    transcript_path: Option<String>,
    #[serde(default)]
    model: Option<StatuslineModel>,
    #[serde(default)]
    workspace: Option<StatuslineWorkspace>,
}

#[derive(Debug, Default, Deserialize)]
struct StatuslineModel {
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct StatuslineWorkspace {
    #[serde(default)]
    current_dir: Option<String>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct TokenUsage {
    input_tokens: usize,
    output_tokens: usize,
    cache_read_input_tokens: usize,
    cache_creation_input_tokens: usize,
}

impl TokenUsage {
    fn total_tokens(self) -> usize {
        self.input_tokens + self.output_tokens
    }

    fn has_data(self) -> bool {
        self.input_tokens > 0
            || self.output_tokens > 0
            || self.cache_read_input_tokens > 0
            || self.cache_creation_input_tokens > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Pricing {
    input_per_million: f64,
    output_per_million: f64,
    cache_read_per_million: f64,
    cache_creation_per_million: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SavingsStat {
    saved_tokens: usize,
    input_tokens: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct StatuslineView {
    context_pct: Option<u8>,
    usage: Option<TokenUsage>,
    cost: Option<f64>,
    model_name: String,
    branch: Option<String>,
    savings: Option<SavingsStat>,
}

pub fn handle(input: &str, no_color: bool) -> Result<()> {
    let input = if input.trim().is_empty() {
        StatuslineInput::default()
    } else {
        serde_json::from_str::<StatuslineInput>(input)?
    };

    let usage = input
        .transcript_path
        .as_deref()
        .and_then(|path| read_transcript_usage(path).ok())
        .filter(|usage| usage.has_data());
    let model_name = input
        .model
        .and_then(|model| model.display_name)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let pricing = pricing_for_model(&model_name);
    let context_pct = usage.map(|usage| {
        let pct = ((usage.total_tokens() as f64 / DEFAULT_CONTEXT_LIMIT as f64) * 100.0).round();
        pct.clamp(0.0, 999.0) as u8
    });
    let cost = usage
        .zip(pricing)
        .map(|(usage, pricing)| cost_for_usage(usage, pricing));
    let branch = input
        .workspace
        .and_then(|workspace| workspace.current_dir)
        .as_deref()
        .and_then(git_branch_for_workspace);
    let savings = current_runtime_session_id()
        .as_deref()
        .and_then(|session_id| mycelium_session_savings(session_id).ok().flatten())
        .filter(|stat| stat.saved_tokens > 0);

    let view = StatuslineView {
        context_pct,
        usage,
        cost,
        model_name: compact_model_name(&model_name),
        branch,
        savings,
    };

    println!("{}", render_statusline(&view, !no_color));
    Ok(())
}

fn read_transcript_usage(path: &str) -> Result<TokenUsage> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut usage = TokenUsage::default();

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Ok(entry) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if !is_assistant_entry(&entry) {
            continue;
        }

        let usage_value = entry
            .get("message")
            .and_then(|message| message.get("usage"))
            .or_else(|| entry.get("usage"));

        usage.input_tokens += usage_field(usage_value, "input_tokens");
        usage.output_tokens += usage_field(usage_value, "output_tokens");
        usage.cache_read_input_tokens += usage_field(usage_value, "cache_read_input_tokens");
        usage.cache_creation_input_tokens +=
            usage_field(usage_value, "cache_creation_input_tokens");
    }

    Ok(usage)
}

fn is_assistant_entry(entry: &Value) -> bool {
    entry.get("type").and_then(Value::as_str) == Some("assistant")
}

fn usage_field(usage: Option<&Value>, field: &str) -> usize {
    usage
        .and_then(|value| value.get(field))
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
}

fn pricing_for_model(display_name: &str) -> Option<Pricing> {
    let normalized = display_name.to_ascii_lowercase();
    if normalized.contains("opus") {
        Some(Pricing {
            input_per_million: 15.0,
            output_per_million: 75.0,
            cache_read_per_million: 1.5,
            cache_creation_per_million: 18.75,
        })
    } else if normalized.contains("sonnet") {
        Some(Pricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
            cache_read_per_million: 0.30,
            cache_creation_per_million: 3.75,
        })
    } else if normalized.contains("haiku") {
        Some(Pricing {
            input_per_million: 0.80,
            output_per_million: 4.0,
            cache_read_per_million: 0.08,
            cache_creation_per_million: 1.0,
        })
    } else {
        None
    }
}

fn cost_for_usage(usage: TokenUsage, pricing: Pricing) -> f64 {
    ((usage.input_tokens as f64 * pricing.input_per_million)
        + (usage.output_tokens as f64 * pricing.output_per_million)
        + (usage.cache_read_input_tokens as f64 * pricing.cache_read_per_million)
        + (usage.cache_creation_input_tokens as f64 * pricing.cache_creation_per_million))
        / 1_000_000.0
}

fn compact_model_name(display_name: &str) -> String {
    let normalized = display_name
        .trim()
        .to_ascii_lowercase()
        .replace("claude ", "")
        .replace(' ', "-");
    if normalized.is_empty() {
        "unknown".to_string()
    } else {
        normalized
    }
}

fn git_branch_for_workspace(cwd: &str) -> Option<String> {
    let cwd = Path::new(cwd);
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!branch.is_empty() && branch != "HEAD").then_some(branch)
}

fn current_runtime_session_id() -> Option<String> {
    std::env::var("CLAUDE_SESSION_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn mycelium_session_savings(session_id: &str) -> Result<Option<SavingsStat>> {
    let db_path = mycelium_db_path()?;
    mycelium_session_savings_at_path(&db_path, session_id)
}

fn mycelium_session_savings_at_path(
    db_path: &Path,
    session_id: &str,
) -> Result<Option<SavingsStat>> {
    if !db_path.exists() {
        return Ok(None);
    }

    let conn = Connection::open(db_path)?;
    let row = conn
        .query_row(
            "SELECT COALESCE(SUM(saved_tokens), 0), COALESCE(SUM(input_tokens), 0)
             FROM commands
             WHERE session_id = ?1",
            params![session_id],
            |row| {
                Ok(SavingsStat {
                    saved_tokens: row.get::<_, i64>(0)? as usize,
                    input_tokens: row.get::<_, i64>(1)? as usize,
                })
            },
        )
        .optional()?;
    Ok(row)
}

fn mycelium_db_path() -> Result<PathBuf> {
    Ok(spore::paths::db_path(
        "mycelium",
        "history.db",
        "MYCELIUM_DB_PATH",
        None,
    )?)
}

fn render_statusline(view: &StatuslineView, color: bool) -> String {
    let mut segments = Vec::new();
    let context = match view.context_pct {
        Some(pct) => format!("ctx:{pct}%"),
        None => "ctx:--".to_string(),
    };
    let context_code = match view.context_pct {
        Some(pct) if pct >= 85 => "31",
        Some(pct) if pct >= 60 => "33",
        Some(_) => "32",
        None => "2",
    };
    segments.push(paint(&context, context_code, color));

    let usage = match view.usage {
        Some(usage) => format!(
            "in:{} out:{} cache:{}",
            format_tokens(usage.input_tokens),
            format_tokens(usage.output_tokens),
            format_tokens(usage.cache_read_input_tokens + usage.cache_creation_input_tokens)
        ),
        None => "--".to_string(),
    };
    segments.push(paint(&usage, "36", color));

    let cost = view
        .cost
        .map_or_else(|| "--".to_string(), |cost| format!("${cost:.2}"));
    segments.push(paint(&cost, "35", color));
    segments.push(paint(&view.model_name, "34", color));

    if let Some(branch) = &view.branch {
        segments.push(paint(branch, "2", color));
    }

    if let Some(savings) = &view.savings {
        segments.push(paint(
            &format!("mycelium:↓{}", format_tokens(savings.saved_tokens)),
            "32",
            color,
        ));
    }

    segments.join(" │ ")
}

fn format_tokens(value: usize) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn paint(value: &str, code: &str, color: bool) -> String {
    if color {
        format!("\u{1b}[{code}m{value}\u{1b}[0m")
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn compact_model_name_normalizes_claude_labels() {
        assert_eq!(compact_model_name("Claude Sonnet 4.6"), "sonnet-4.6");
        assert_eq!(compact_model_name("Claude Opus 4.6"), "opus-4.6");
        assert_eq!(compact_model_name(""), "unknown");
    }

    #[test]
    fn read_transcript_usage_sums_assistant_usage() {
        let temp_dir = std::env::temp_dir().join("cortina-statusline-usage");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        let transcript = temp_dir.join("transcript.jsonl");
        fs::write(
            &transcript,
            concat!(
                "{\"type\":\"assistant\",\"message\":{\"usage\":{\"input_tokens\":1200,\"output_tokens\":300,\"cache_read_input_tokens\":500,\"cache_creation_input_tokens\":100}}}\n",
                "{\"type\":\"human\",\"text\":\"ignored\"}\n",
                "{\"type\":\"assistant\",\"usage\":{\"input_tokens\":800,\"output_tokens\":200,\"cache_read_input_tokens\":100,\"cache_creation_input_tokens\":50}}\n"
            ),
        )
        .unwrap();

        let usage = read_transcript_usage(transcript.to_str().unwrap()).unwrap();
        assert_eq!(
            usage,
            TokenUsage {
                input_tokens: 2000,
                output_tokens: 500,
                cache_read_input_tokens: 600,
                cache_creation_input_tokens: 150,
            }
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn render_statusline_without_color_is_compact() {
        let line = render_statusline(
            &StatuslineView {
                context_pct: Some(42),
                usage: Some(TokenUsage {
                    input_tokens: 45_000,
                    output_tokens: 12_000,
                    cache_read_input_tokens: 80_000,
                    cache_creation_input_tokens: 9_000,
                }),
                cost: Some(1.23),
                model_name: "sonnet-4.6".to_string(),
                branch: Some("main".to_string()),
                savings: Some(SavingsStat {
                    saved_tokens: 8_200,
                    input_tokens: 10_000,
                }),
            },
            false,
        );

        assert_eq!(
            line,
            "ctx:42% │ in:45.0K out:12.0K cache:89.0K │ $1.23 │ sonnet-4.6 │ main │ mycelium:↓8.2K"
        );
    }

    #[test]
    fn render_statusline_degrades_gracefully() {
        let line = render_statusline(
            &StatuslineView {
                context_pct: None,
                usage: None,
                cost: None,
                model_name: "unknown".to_string(),
                branch: None,
                savings: None,
            },
            false,
        );

        assert_eq!(line, "ctx:-- │ -- │ -- │ unknown");
    }

    #[test]
    fn mycelium_session_savings_reads_sqlite() {
        let temp_dir = std::env::temp_dir().join("cortina-statusline-mycelium");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        let db_path = temp_dir.join("history.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "CREATE TABLE commands (
                session_id TEXT,
                input_tokens INTEGER NOT NULL,
                saved_tokens INTEGER NOT NULL
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO commands (session_id, input_tokens, saved_tokens) VALUES (?1, ?2, ?3)",
            params!["session-123", 1200_i64, 800_i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO commands (session_id, input_tokens, saved_tokens) VALUES (?1, ?2, ?3)",
            params!["session-123", 300_i64, 100_i64],
        )
        .unwrap();

        let stat = mycelium_session_savings_at_path(&db_path, "session-123")
            .unwrap()
            .expect("session savings should exist");

        assert_eq!(stat.saved_tokens, 900);
        assert_eq!(stat.input_tokens, 1500);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
