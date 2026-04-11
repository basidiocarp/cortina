# Cortina

Lifecycle signal runner for AI coding agents. Intercepts host hook events,
classifies useful outcomes, and writes structured session signals into the rest
of the ecosystem.

Named after the fungal cortina, a veil between the cap and stipe that
intercepts what passes between them.

Part of the [Basidiocarp ecosystem](https://github.com/basidiocarp).

---

## The Problem

Agent sessions generate corrections, failed commands, test recoveries, and
session outcomes, but most of that signal disappears unless the host or user
records it manually. The result is weak attribution, lost feedback loops, and
thin session summaries.

## The Solution

Cortina adds an adapter-first lifecycle capture layer. It reads host hook
envelopes from stdin, normalizes them into internal signal types, detects the
outcomes worth keeping, and forwards only the narrow downstream writes that
belong in the ecosystem.

---

## The Ecosystem

| Tool | Purpose |
|------|---------|
| **[cortina](https://github.com/basidiocarp/cortina)** | Lifecycle signal capture and session attribution |
| **[canopy](https://github.com/basidiocarp/canopy)** | Multi-agent coordination runtime |
| **[cap](https://github.com/basidiocarp/cap)** | Web dashboard for the ecosystem |
| **[hyphae](https://github.com/basidiocarp/hyphae)** | Persistent agent memory |
| **[lamella](https://github.com/basidiocarp/lamella)** | Skills, hooks, and plugins for coding agents |
| **[mycelium](https://github.com/basidiocarp/mycelium)** | Token-optimized command output |
| **[rhizome](https://github.com/basidiocarp/rhizome)** | Code intelligence via tree-sitter and LSP |
| **[stipe](https://github.com/basidiocarp/stipe)** | Ecosystem installer and manager |
| **[volva](https://github.com/basidiocarp/volva)** | Execution-host runtime layer |

> **Boundary:** `cortina` owns host lifecycle capture and signal
> classification. Host-specific quirks stay in adapters. `hyphae` owns memory
> persistence, `rhizome` owns code graph export, `stipe` owns setup, and
> `volva` owns execution-host orchestration.

---

## Quick Start

```bash
# Recommended: ecosystem-managed install
stipe install cortina
stipe init
```

```bash
# Build from source
cargo install --git https://github.com/basidiocarp/cortina

# Operator surfaces
cortina policy
cortina status
cortina doctor
cortina statusline
```

---

## How It Works

```text
Claude Code or Volva         Cortina                         Ecosystem
────────────────────         ───────                         ─────────
hook envelope on stdin ─►    adapter layer            ─►     normalized signals
tool result + metadata ─►    classifier               ─►     Hyphae session data
change thresholds met   ─►   trigger logic            ─►     Rhizome export / doc ingest
```

1. Read host events: adapters parse Claude Code hooks and Volva hook-event payloads.
2. Normalize signals: the shared runtime maps host-specific envelopes into common event types.
3. Normalize usage edges: transcript-derived token and cost counters should converge on Septa's `usage-event-v1` contract before downstream summary layers.
4. Detect outcomes: identify failures, resolutions, self-corrections, and validation passes.
5. Write structured state: record session and feedback signals in Hyphae.
6. Trigger follow-up work: kick off Rhizome export or Hyphae doc ingest when thresholds are met.

---

## What Gets Captured

| Signal | Trigger | Stored as |
|--------|---------|-----------|
| Error | Bash exit code != 0 | `errors/active` |
| Resolution | Same command succeeds after failure | `errors/resolved` plus feedback signal |
| Self-correction | Edit after recent write to same file | `corrections` plus feedback signal |
| Validation pass | Build or test command succeeds | `build_passed` or `test_passed` signal |
| Test failure | Test runner with failures | `tests/failed` |
| Test fix | Test passes after failure | `tests/resolved` |
| Code changes | 5 or more edits plus successful build | Triggers Rhizome export |
| Doc changes | 3 or more doc edits | Triggers Hyphae ingest |
| Session end | SessionEnd or Stop event | `hyphae session end` with fallback summary |
| Compaction lifecycle | `PreCompact` event | `session/compaction-snapshot` plus normalized lifecycle envelope |
| Council lifecycle | council-style prompt submission | `session/council-lifecycle` |

---

## What Cortina Owns

- Host adapter boundary and lifecycle event intake
- Signal classification and scoped temp-state tracking
- Normalized usage-event producer boundary before downstream summaries
- Session outcome attribution
- Operator policy, status, and doctor surfaces

## What Cortina Does Not Own

- Long-term memory storage: handled by `hyphae`
- Code intelligence and graph extraction: handled by `rhizome`
- Installation and host registration: handled by `stipe`
- Full execution-host orchestration: handled by `volva`

---

## Key Features

- Adapter-first runtime: keeps host-specific intake separate from shared signal logic.
- Structured feedback signals: turn transient hook activity into reusable session data.
- Scoped operator views: report per-worktree policy, state, and hook health.
- Status line support: renders compact session summaries for Claude Code.
- Downstream triggers: launch Rhizome export and Hyphae doc ingest when local thresholds are met.

---

## Architecture

```text
cortina
├── src/adapters/   host-specific hook intake
├── src/events/     normalized signal types and classification
├── src/hooks/      lifecycle handlers and write paths
├── src/utils/      support code and scoped state helpers
├── cortina/src/    CLI entry point
└── docs/           boundary and behavior notes
```

```text
cortina adapter claude-code pre-tool-use
cortina adapter claude-code post-tool-use
cortina adapter claude-code session-end
cortina adapter volva hook-event
cortina status
cortina doctor
cortina statusline
```

---

## Documentation

- [docs/README.md](docs/README.md): repo-local docs index
- [docs/lamella-boundary.md](docs/lamella-boundary.md): ownership split between Lamella packaging and Cortina runtime behavior
- [docs/normalized-lifecycle-vocabulary.md](docs/normalized-lifecycle-vocabulary.md): normalized lifecycle categories, transferable fields, and fail-open rules

## Development

```bash
cargo build --release
cargo nextest run
cargo test
cargo clippy
cargo fmt
```

- Prefer `cargo nextest run` for the normal test loop.
- Keep `criterion` out of scope here until a concrete hot path is named.
- Use whole-command timing when runtime capture behavior feels slow, for
  example `time cargo run -- status`.
- Cortina links SQLite from the system on non-Windows targets and keeps the
  bundled SQLite fallback only on Windows.
- Command classification stays regex-free so the hook binary keeps a smaller
  dependency surface.

## Logging

Cortina writes diagnostic logs to stderr through Spore's shared logger so hook
stdout stays clean.

- Use `CORTINA_LOG` for repo-specific logging, for example
  `CORTINA_LOG=cortina=debug cortina status`.
- `RUST_LOG` still works as the broader Rust fallback, but `CORTINA_LOG` is the
  intended operator knob for this binary.
- Logging is separate from Cortina's normal runtime surfaces: hook payloads
  still flow through stdin/stdout, while shared tracing spans, subprocess
  diagnostics, and compatibility warnings stay on stderr.
- The runtime is intentionally fail-open. If a downstream bridge or adapter
  helper fails, Cortina warns, leaves the hook output intact, and keeps the
  host session moving.
- Most runtime diagnostics now flow through the shared tracing contract, but a
  few user-facing compatibility warnings still intentionally write straight to
  stderr with `eprintln!` so they appear even when structured logging is off.

## License

MIT
