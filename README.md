# Cortina

Hook runner for AI coding agents. Reads the current hook-event envelope from stdin, detects patterns in tool results, and stores signals in Hyphae. Today that envelope comes from Claude Code, but Cortina’s internal event handling is normalized so the runtime logic is less tied to one host-specific parser shape. One Rust binary replaces five JavaScript files and two shell scripts.

Named after the fungal cortina—a veil between the cap and stipe that intercepts what passes between them.

Part of the [Basidiocarp ecosystem](https://github.com/basidiocarp).

## How It Works

Claude Code currently fires hook events at three points: before a tool runs, after it completes, and when the session ends. Cortina handles all three.

```
Claude Code                    Cortina                         Ecosystem
───────────                    ───────                         ─────────
PreToolUse  ──stdin JSON──►    cortina pre-tool-use    ──►     Rewrite via Mycelium
PostToolUse ──stdin JSON──►    cortina post-tool-use   ──►     Store to Hyphae
Stop        ──stdin JSON──►    cortina stop            ──►     Session summary
```

PostToolUse does the heavy lifting. It watches for failed commands, self-corrections (an edit immediately after a write to the same file), test failures, and accumulated code changes. When it detects a pattern, it stores a memory in Hyphae with the right topic so future sessions can recall it.

The Stop hook writes a session summary: which files changed, what errors occurred, what decisions were made.

## What Gets Captured

| Signal | Trigger | Stored as |
|--------|---------|-----------|
| Error | Bash exit code != 0 | `errors/active` |
| Resolution | Same command succeeds after failure | `errors/resolved` |
| Self-correction | Edit after recent Write to same file | `corrections` |
| Test failure | Test runner with failures | `tests/failed` |
| Test fix | Test passes after failure | `tests/resolved` |
| Code changes | 5+ edits + successful build | Triggers `rhizome export` |
| Doc changes | 3+ doc edits | Triggers `hyphae ingest-file` |
| Session end | Stop event | `session/{project}` |

## Install

Stipe handles this as part of ecosystem setup:

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
