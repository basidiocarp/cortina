# Lamella Boundary Cleanup

## Overview

Cortina should own host lifecycle event handling, signal detection, temp state, and ecosystem side effects such as Hyphae writes and Rhizome exports. Lamella should own plugin packaging, hook templates, skills, docs, and user-facing workflow guidance. Host-specific parsing belongs in Cortina adapters, not in the shared lifecycle handlers.

This page classifies the current Lamella scripts into three buckets:

- move now: active overlap with Cortina's shipped responsibility
- move later: plausible future Cortina work, but not required to finish the current boundary cleanup
- leave in Lamella: plugin UX, local policy, or continuous-learning behavior

## Decision Rule

Move a Lamella hook into Cortina when all of the following are true:

- it is triggered by a host lifecycle event such as `PreToolUse`, `PostToolUse`, `Stop`, or `SessionEnd`
- it derives reusable signals from tool input or output rather than enforcing local workflow policy
- it writes or should write ecosystem state to Hyphae or Rhizome
- the behavior should be host-agnostic after adapter normalization

Keep a hook in Lamella when it is primarily:

- plugin UX or installation policy
- local authoring guidance or reminders
- project-specific continuous-learning behavior
- formatting, linting, or interactive workflow assistance

Fail-open behavior is intentional across the boundary: if a downstream bridge fails, Cortina should warn and continue instead of turning hook execution into a hard stop.

## Move Now

These are active or near-active capture behaviors that already overlap with Cortina's runtime.

| Lamella file | Current status | Cortina target | Notes |
|---|---|---|---|
| `scripts/hooks/capture-errors.js` | Replaced in the shipped hook catalog by `cortina adapter claude-code post-tool-use` | `src/hooks/post_tool_use.rs` | Keep only as legacy reference until Lamella deletes the old helper. |
| `scripts/hooks/capture-corrections.js` | Replaced in the shipped hook catalog by `cortina adapter claude-code post-tool-use` | `src/hooks/post_tool_use.rs` | Keep only as legacy reference until Lamella deletes the old helper. |
| `scripts/hooks/capture-test-results.js` | Replaced in the shipped hook catalog by `cortina adapter claude-code post-tool-use` | `src/hooks/post_tool_use.rs` | Keep only as legacy reference until Lamella deletes the old helper. |
| `scripts/hooks/capture-code-changes.js` | Present in Lamella, not currently registered in `resources/hooks/hooks.json` | `src/hooks/post_tool_use.rs` | Cortina already tracks pending edits, triggers `rhizome export`, and runs `hyphae ingest-file`. Keep only until parity is confirmed, then remove from Lamella. |
| `scripts/hooks/session-end.js` | Active in `resources/hooks/hooks.json` under `SessionEnd` | `src/hooks/stop.rs` or a dedicated `session_end` hook module | Cortina already owns session-end summary storage. The remaining gap is adapter surface and hook registration shape, not business logic. |

## Move Later

These fit Cortina only if Cortina expands beyond the current capture boundary.

| Lamella file | Reason to defer |
|---|---|
| `scripts/hooks/session-start.js` | Useful lifecycle behavior, but it injects startup context and skill hints rather than capturing normalized signals. Move only if Cortina grows a `SessionStart` adapter role. |
| `scripts/hooks/capture-pr-reviews.js` | This is real signal capture, but it is narrower and more workflow-specific than the core error, test, correction, and session lifecycle path. It fits after the primary boundary cleanup is complete. |
| `scripts/hooks/evaluate-session.js` | This is tied to Lamella's continuous-learning product behavior. Move it only if Cortina becomes a broader event bus rather than a narrow lifecycle signal runner. |

## Leave In Lamella

These should stay in Lamella because they are packaging-time policy, local workflow nudges, or continuous-learning behavior rather than core capture runtime.

Scripts:

- `scripts/hooks/pre-write-doc-warn.js`
- `scripts/hooks/suggest-compact.js`
- `scripts/hooks/pre-compact.js`
- `scripts/hooks/post-edit-format.js`
- `scripts/hooks/post-edit-typecheck.js`
- `scripts/hooks/post-edit-console-warn.js`
- `scripts/hooks/comment-style-check.sh`
- `scripts/hooks/check-console-log.js`
- `resources/skills/core/continuous-learning/hooks/observe.sh`

Hook template behavior that should remain Lamella-owned:

- tmux blockers and reminders in `resources/hooks/hooks.json`
- `git push` review reminder in `resources/hooks/hooks.json`
- PR creation follow-up guidance in `resources/hooks/hooks.json`
- example build-complete messaging in `resources/hooks/hooks.json`

## Migration Order

### Phase 1

Route the active Lamella capture hooks to Cortina and delete the duplicate JavaScript once the shipped plugin templates no longer call them directly.

- `capture-errors.js`
- `capture-corrections.js`
- `capture-test-results.js`
- `session-end.js`

### Phase 2

Decide how Claude `SessionEnd` maps to Cortina.

- If `SessionEnd` payloads match the current stop adapter contract, keep the runtime in `src/hooks/stop.rs` and add a CLI alias.
- If `SessionEnd` needs distinct parsing, add a dedicated adapter event and hook module in Cortina.

### Phase 3

Retire the inactive Lamella `capture-code-changes.js` helper once the current Rust implementation remains the only code path for export and ingest thresholds.

### Phase 4

Revisit `session-start.js`, `capture-pr-reviews.js`, and `evaluate-session.js` only after the narrow lifecycle boundary is stable.

## Current Cortina Coverage

The current Cortina implementation already covers the core Lamella capture path:

- `src/hooks/post_tool_use.rs` replaces `capture-errors.js`, `capture-corrections.js`, and `capture-code-changes.js`
- `src/hooks/stop.rs` owns session-end summary storage
- `README.md` and `ROADMAP.md` already describe Cortina as the lifecycle signal runner and call out Lamella boundary cleanup explicitly

That means the remaining work is mostly adapter registration, template cleanup, and deletion of duplicate Lamella runtime code.
