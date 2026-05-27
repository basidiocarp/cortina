use std::path::PathBuf;
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::sync::OnceLock;
#[cfg(unix)]
use std::time::Duration;

use spore::Tool;
#[cfg(not(test))]
use spore::discover;
use spore::logging::{SpanContext, subprocess_span, tool_span};
use spore::telemetry::TraceContextCarrier;
use tracing::{debug, warn};

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
    // In tests, skip the discovery probe (which spawns a real process and can
    // stall for up to 5 s) and return the bare binary name so the injected mock
    // runner controls all I/O without real subprocess overhead.
    #[cfg(test)]
    {
        Tool::from_binary_name(name)?;
        Some(PathBuf::from(name))
    }

    #[cfg(not(test))]
    {
        let tool = Tool::from_binary_name(name)?;
        discover(tool).map(|info| info.binary_path)
    }
}

fn span_context(tool: &str) -> SpanContext {
    let context = SpanContext::for_app("cortina").with_tool(tool);
    match std::env::current_dir() {
        Ok(path) => context.with_workspace_root(path.display().to_string()),
        Err(_) => context,
    }
}

fn diagnostic_stderr() -> Stdio {
    #[cfg(test)]
    {
        Stdio::null()
    }
    #[cfg(not(test))]
    {
        Stdio::inherit()
    }
}

pub(crate) fn resolved_command(name: &str) -> Option<Command> {
    let binary_path = command_path(name)?;
    Some(Command::new(binary_path))
}

pub fn command_exists(name: &str) -> bool {
    command_path(name).is_some()
}

// ---------------------------------------------------------------------------
// Hyphae socket endpoint discovery
// ---------------------------------------------------------------------------

/// Cached unix-socket path for the hyphae service endpoint.
///
/// Read from `~/.config/hyphae/hyphae.endpoint.json` on first call. `None`
/// means the descriptor is absent or the transport is not unix-socket — cortina
/// falls back to the CLI spawn path in that case.
#[cfg(unix)]
static HYPHAE_SOCKET_PATH: OnceLock<Option<String>> = OnceLock::new();

#[cfg(unix)]
fn hyphae_socket_path() -> Option<&'static str> {
    HYPHAE_SOCKET_PATH
        .get_or_init(|| {
            let descriptor_path = spore::paths::config_dir("hyphae").ok()?.join("hyphae.endpoint.json");
            let json = std::fs::read_to_string(descriptor_path).ok()?;
            let v: serde_json::Value = serde_json::from_str(&json).ok()?;
            if v.get("transport").and_then(|t| t.as_str()) != Some("unix-socket") {
                return None;
            }
            v.get("endpoint")?.as_str().map(String::from)
        })
        .as_deref()
}

/// Send a fire-and-forget JSON-RPC 2.0 request to the hyphae unix socket.
///
/// Opens, writes, reads one response line, and closes. Blocks briefly — callers
/// must spawn a background thread if blocking would stall the hook.
#[cfg(unix)]
#[allow(clippy::needless_pass_by_value)]
fn socket_call(socket_path: &str, method: &str, params: serde_json::Value) -> anyhow::Result<()> {
    use std::io::{BufRead, BufReader, Write};
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;

    let stream = UnixStream::connect(socket_path)?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;

    let mut writer = stream.try_clone()?;

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    writer.write_all(serde_json::to_string(&request)?.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;

    // Explicitly drop write handle before reading to cleanly signal write-side EOF.
    drop(writer);

    let reader = BufReader::new(&stream);
    // reader.lines() moves reader, releasing the borrow on stream before any shutdown calls.
    let next_line = reader.lines().next();
    let line = match next_line {
        Some(Ok(line)) => line,
        Some(Err(e)) => {
            let _ = stream.shutdown(Shutdown::Both);
            return Err(e.into());
        }
        None => {
            let _ = stream.shutdown(Shutdown::Both);
            return Err(anyhow::anyhow!("no response from hyphae socket"));
        }
    };

    // Validate the response is not a JSON-RPC error object before returning Ok.
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&line) {
        if let Some(error) = parsed.get("error") {
            let _ = stream.shutdown(Shutdown::Both);
            return Err(anyhow::anyhow!("hyphae socket returned error: {error}"));
        }
    }

    let _ = stream.shutdown(Shutdown::Both);
    Ok(())
}

// ---------------------------------------------------------------------------
// hyphae_memory_store — socket preferred, CLI fallback
// ---------------------------------------------------------------------------

pub fn store_in_hyphae(
    topic: &str,
    content: &str,
    importance: Importance,
    project: Option<&str>,
    agent_id: Option<&str>,
) {
    let span_ctx = span_context("hyphae_store");
    let _tool_span = tool_span("hyphae_store", &span_ctx).entered();

    #[cfg(unix)]
    if let Some(socket) = hyphae_socket_path() {
        let mut params = serde_json::json!({
            "topic": topic,
            "content": content,
            "importance": importance.as_str(),
            "keywords": ["cortina", "hook"],
        });
        if let Some(proj) = project {
            params["project"] = serde_json::json!(proj);
        }
        if let Some(id) = agent_id {
            params["agent_id"] = serde_json::json!(id);
        }

        let socket = socket.to_string();
        let _spawn_span = subprocess_span("hyphae store (socket)", &span_ctx).entered();
        std::thread::spawn(move || {
            if let Err(e) = socket_call(&socket, "hyphae_memory_store", params) {
                warn!("hyphae_memory_store socket call failed: {e}");
            }
        });
        return;
    }

    // [COMPATIBILITY FALLBACK] hyphae socket endpoint unavailable — CLI only
    debug!("hyphae socket endpoint unavailable; using CLI fallback for store_in_hyphae");
    store_in_hyphae_cli(topic, content, importance, project, agent_id, &span_ctx);
}

fn store_in_hyphae_cli(
    topic: &str,
    content: &str,
    importance: Importance,
    project: Option<&str>,
    agent_id: Option<&str>,
    span_ctx: &SpanContext,
) {
    let Some(mut cmd) = resolved_command("hyphae") else {
        debug!("Hyphae binary is not discoverable; skipping store");
        return;
    };
    cmd.args(["store", "--topic", topic])
        .args(["--content", content])
        .args(["--importance", importance.as_str()])
        .args(["--keywords", "cortina,hook"]);

    if let Some(proj) = project {
        cmd.args(["-P", proj]);
    }

    if let Some(id) = agent_id {
        cmd.args(["--agent-id", id]);
    }

    if let Some(carrier) = TraceContextCarrier::from_current() {
        cmd.env("TRACEPARENT", &carrier.traceparent);
        if let Some(ref ts) = carrier.tracestate {
            cmd.env("TRACESTATE", ts);
        }
    }

    let _spawn_span = subprocess_span("hyphae store", span_ctx).entered();
    if let Err(err) = cmd
        .stdout(std::process::Stdio::null())
        .stderr(diagnostic_stderr())
        .spawn()
    {
        warn!("Failed to spawn hyphae store command: {err}");
    }
}

// ---------------------------------------------------------------------------
// compact_summary artifact store — socket preferred, CLI fallback
// ---------------------------------------------------------------------------

/// Store a typed `compact_summary` artifact in Hyphae.
///
/// Uses topic `artifact/compact_summary/{session_id}` so the artifact is
/// queryable by convention. Failures are logged and swallowed so they cannot
/// break the existing pre-compact capture flow.
pub fn store_compact_summary_artifact(payload: &str, project: Option<&str>) {
    let span_ctx = span_context("hyphae_store_artifact");
    let _tool_span = tool_span("hyphae_store_artifact", &span_ctx).entered();

    let session_id = serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| v["session_id"].as_str().map(ToString::to_string))
        .unwrap_or_else(|| "unknown".to_string());

    let topic = format!("artifact/compact_summary/{session_id}");

    #[cfg(unix)]
    if let Some(socket) = hyphae_socket_path() {
        let mut params = serde_json::json!({
            "topic": topic,
            "content": payload,
            "importance": Importance::High.as_str(),
            "keywords": ["cortina", "hook", "compact_summary", "artifact"],
        });
        if let Some(proj) = project {
            params["project"] = serde_json::json!(proj);
        }

        let socket = socket.to_string();
        let _spawn_span = subprocess_span("hyphae store artifact (socket)", &span_ctx).entered();
        std::thread::spawn(move || {
            if let Err(e) = socket_call(&socket, "hyphae_memory_store", params) {
                warn!("hyphae_memory_store socket call failed for compact_summary: {e}");
            }
        });
        return;
    }

    // [COMPATIBILITY FALLBACK] hyphae socket endpoint unavailable — CLI only
    warn!(
        "hyphae socket endpoint unavailable; \
         using CLI fallback for store_compact_summary_artifact"
    );
    store_compact_summary_artifact_cli(&topic, payload, project, &span_ctx);
}

fn store_compact_summary_artifact_cli(
    topic: &str,
    payload: &str,
    project: Option<&str>,
    span_ctx: &SpanContext,
) {
    let Some(mut cmd) = resolved_command("hyphae") else {
        debug!("Hyphae binary is not discoverable; skipping compact_summary artifact store");
        return;
    };

    cmd.args(["store", "--topic", topic])
        .args(["--content", payload])
        .args(["--importance", Importance::High.as_str()])
        .args(["--keywords", "cortina,hook,compact_summary,artifact"]);

    if let Some(proj) = project {
        cmd.args(["-P", proj]);
    }

    if let Some(carrier) = TraceContextCarrier::from_current() {
        cmd.env("TRACEPARENT", &carrier.traceparent);
        if let Some(ref ts) = carrier.tracestate {
            cmd.env("TRACESTATE", ts);
        }
    }

    let _spawn_span = subprocess_span("hyphae store artifact", span_ctx).entered();
    if let Err(err) = cmd
        .stdout(std::process::Stdio::null())
        .stderr(diagnostic_stderr())
        .spawn()
    {
        warn!("Failed to spawn hyphae store for compact_summary artifact: {err}");
    }
}

// ---------------------------------------------------------------------------
// Generic async spawn helper — out of scope for socket migration
// ---------------------------------------------------------------------------

pub fn spawn_async_checked(cmd: &str, args: &[&str]) -> bool {
    let context = span_context(cmd);
    let _tool_span = tool_span("spawn_async_checked", &context).entered();
    let Some(mut command) = resolved_command(cmd) else {
        debug!("Command {cmd} is not discoverable; skipping async spawn");
        return false;
    };
    for arg in args {
        command.arg(arg);
    }

    if let Some(carrier) = TraceContextCarrier::from_current() {
        command.env("TRACEPARENT", &carrier.traceparent);
        if let Some(ref ts) = carrier.tracestate {
            command.env("TRACESTATE", ts);
        }
    }

    let _spawn_span = subprocess_span(cmd, &context).entered();
    match command
        .stdout(std::process::Stdio::null())
        .stderr(diagnostic_stderr())
        .spawn()
    {
        Ok(_) => true,
        Err(err) => {
            warn!("Failed to spawn {cmd}: {err}");
            false
        }
    }
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
