# Cortina

Hook runner for AI coding agents. Named after the fungal cortina — a veil that sits between the cap and stipe, intercepting what passes between them.

Part of the [Basidiocarp ecosystem](https://github.com/basidiocarp).

## What It Does

Cortina replaces the JavaScript/shell hooks in Lamella and Mycelium with a single Rust binary. It handles Claude Code's hook protocol: reads JSON from stdin, detects patterns, stores signals in Hyphae, and optionally rewrites commands.

```
Claude Code                    Cortina                         Ecosystem
───────────                    ───────                         ─────────
PreToolUse  ──stdin JSON──►    cortina pre-tool-use    ──►     Rewrite via Mycelium
PostToolUse ──stdin JSON──►    cortina post-tool-use   ──►     Store to Hyphae
Stop        ──stdin JSON──►    cortina stop            ──►     Session summary
```

## Why Rust Instead of JavaScript?

The current hooks are JavaScript files requiring Node.js and a `utils.js` dependency. Cortina eliminates:

- Node.js runtime dependency
- `jq` dependency (session-summary.sh)
- File-based state tracking (`/tmp/rhizome-pending-exports-*.txt`)
- Multiple hook scripts (5 JS files + 2 shell scripts → 1 binary)

Faster startup, no interpreter overhead, cross-platform without shell compatibility issues.

## What It Replaces

| Current | Cortina equivalent |
|---------|-------------------|
| `mycelium-rewrite.sh` (PreToolUse) | `cortina pre-tool-use` |
| `capture-errors.js` (PostToolUse) | `cortina post-tool-use` |
| `capture-corrections.js` (PostToolUse) | `cortina post-tool-use` |
| `capture-code-changes.js` (PostToolUse) | `cortina post-tool-use` |
| `capture-test-results.js` (PostToolUse) | `cortina post-tool-use` |
| `session-summary.sh` (Stop) | `cortina stop` |

One binary, three subcommands, all hook types.

## Hook Protocol

Claude Code hooks receive JSON on stdin and optionally write JSON to stdout.

**PreToolUse**: can rewrite tool input (e.g., `git status` → `mycelium git status`)
```json
{"tool_input": {"command": "git status"}, "tool_name": "Bash"}
```

**PostToolUse**: observes tool results, stores feedback signals
```json
{"tool_name": "Bash", "tool_input": {"command": "cargo test"}, "tool_output": {"stdout": "...", "exit_code": 1}}
```

**Stop**: receives session metadata, stores summary
```json
{"session_id": "abc123", "transcript_path": "/path/to/transcript.jsonl", "cwd": "/project"}
```

## What Gets Captured

| Signal | Trigger | Hyphae topic |
|--------|---------|-------------|
| Error encountered | Bash exit code != 0 | `errors/active` |
| Error resolved | Same command succeeds after failure | `errors/resolved` |
| Self-correction | Edit after recent Write to same file | `corrections` |
| Test failure | Test runner output with failures | `tests/failed` |
| Test fix | Test runner passes after failure | `tests/resolved` |
| Code changes | 5+ file edits + successful build | Triggers `rhizome export` |
| Document changes | 3+ doc file edits | Triggers `hyphae ingest-file` |
| Session summary | Stop event | `session/{project}` |

## Installation

Installed by Stipe as part of ecosystem setup:

```bash
stipe install cortina
stipe init              # registers hooks in settings.json
```

Or manually:

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

## Status

Bootstrapped with hook stubs. Implementation pending — absorbing logic from Lamella JS hooks and Mycelium shell hooks.

## License

MIT
