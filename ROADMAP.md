# Cortina Roadmap

This page is the Cortina-specific backlog. The workspace [ROADMAP.md](../ROADMAP.md) keeps the ecosystem sequencing, and [MASTER-ROADMAP.md](../MASTER-ROADMAP.md) keeps the cross-repo summary.

## Recently Shipped

- Adapter-first event model.
- Claude-specific event envelope isolated behind an adapter boundary.
- Adapter-owned CLI surface and dispatch path.
- More portable temp-path handling.
- Hyphae session bridge for structured session start, reuse, end, and best-effort feedback signal emission.
- Structured outcome attribution across `PostToolUse`, `Stop`, and `SessionEnd`.
- Recall and session attribution tied to scoped Hyphae sessions instead of ad hoc local state.
- Lifecycle persistence hardening with locked, atomic state updates for sessions, outcomes, edit tracking, and pending export or ingest queues.
- Lifecycle module split across focused `events`, `utils`, `post_tool_use`, and `stop` submodules with extracted regression tests.

## Next

### Lamella boundary cleanup

Finish moving ecosystem lifecycle and capture-hook ownership out of Lamella and into Cortina, with Cortina as the default runtime and Lamella acting as packaging, templates, and fallback glue.
See [docs/lamella-boundary.md](docs/lamella-boundary.md) for the current move-now and move-later split.

### Capture policy controls

Add explicit policy around what gets captured, when it is deduped, and when noisy or low-value signals should be suppressed instead of stored.

### Session and outcome policy refinement

Keep the scoped Hyphae session and outcome model boring by tightening policy around retries, partial failures, fallback behavior, and attribution windows.

## Later

### More adapters

Add broader lifecycle adapters only if another host justifies real implementation work and cannot be handled as a thin packaging or template layer.

## Research

### Broader event bus role

Decide whether Cortina should stay narrowly focused on host adapters or become a broader event bus for the ecosystem.
