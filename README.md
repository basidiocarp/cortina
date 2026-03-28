# Cortina

Lifecycle signal runner for AI coding agents. Cortina reads the current host adapter envelope from stdin, normalizes it into internal signal types, detects patterns in tool results, and stores signals in Hyphae. Today the shipped adapter surface is Claude Code hooks, but the runtime logic now sits behind an explicit adapter boundary instead of treating one host envelope as the core model. One Rust binary replaces five JavaScript files and two shell scripts.

Named after the fungal cortina—a veil between the cap and stipe that intercepts what passes between them.

Part of the [Basidiocarp ecosystem](https://github.com/basidiocarp).

## How It Works

Claude Code currently fires hook events at three points: before a tool runs, after it completes, and when the session ends. Cortina currently ships a Claude adapter for all three, then normalizes those inputs before running shared logic.

```
Claude Code                    Cortina                         Ecosystem
───────────                    ───────                         ─────────
PreToolUse  ──stdin JSON──►    Claude adapter          ──►     Rewrite via Mycelium
PostToolUse ──stdin JSON──►    Claude adapter          ──►     Store to Hyphae
Stop        ──stdin JSON──►    Claude adapter          ──►     Session summary
```

Preferred adapter-oriented CLI:

```bash
cortina adapter claude-code pre-tool-use
cortina adapter claude-code post-tool-use
cortina adapter claude-code stop
```

Compatibility aliases still work:

```bash
cortina pre-tool-use
cortina post-tool-use
cortina stop
```

The CLI entrypoint dispatches through the adapter layer rather than calling Claude-specific handlers directly. Adding a new host should be an adapter/module change, not a rewrite of the shared signal pipeline.

PostToolUse does the heavy lifting. It watches for failed commands, self-corrections (an edit immediately after a write to the same file), test failures, successful build/test validation, and accumulated code changes. When it detects a pattern, it stores a memory in Hyphae with the right topic so future sessions can recall it. When Cortina is about to emit a structured correction, recovery, or validation signal, it also tries to ensure a Hyphae session exists for the current worktree. Those sessions are scoped per worktree hash so parallel workers in the same project do not collapse into one active session, and Cortina now checks liveness through structured `hyphae session status --id <session-id>` output instead of parsing human-readable session listings. Structured writes remain best-effort rather than guaranteed.

If a structured Hyphae session is active, the Stop hook tries to end it with a structured summary: which files changed, what errors occurred, what tools were used, and the final outcome. If no structured session exists, or structured shutdown fails, Cortina falls back to the older direct `session/{project}` memory write.

## What Gets Captured

| Signal | Trigger | Stored as |
|--------|---------|-----------|
| Error | Bash exit code != 0 | `errors/active` |
| Resolution | Same command succeeds after failure | `errors/resolved` + `hyphae feedback signal error_resolved` |
| Self-correction | Edit after recent Write to same file | `corrections` + `hyphae feedback signal correction` |
| Validation pass | Build or test command succeeds | `hyphae feedback signal build_passed` / `test_passed` |
| Test failure | Test runner with failures | `tests/failed` |
| Test fix | Test passes after failure | `tests/resolved` |
| Code changes | 5+ edits + successful build | Triggers `rhizome export` |
| Doc changes | 3+ doc edits | Triggers `hyphae ingest-file` |
| Session end | Stop event | `hyphae session end` with summary fallback to `session/{project}` |

## Install

Stipe handles the current Claude adapter registration as part of ecosystem setup:

```bash
stipe install cortina
stipe init              # registers hooks in settings.json
```

Or build from source:

```bash
cargo install --git https://github.com/basidiocarp/cortina
```

## Development

```bash
cargo build --release
cargo test
cargo clippy
cargo fmt
```

## License

MIT
