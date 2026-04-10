# Cortina Agent Notes

## Purpose

Cortina owns lifecycle capture and session attribution at the hook boundary. Work here should keep host-specific parsing in adapters, shared capture logic in hooks and events, and persistence behind the Hyphae client boundary. Cortina observes and records; it should not become a policy engine for sibling runtimes.

---

## Source of Truth

- `src/adapters/`: host-specific hook-envelope parsing.
- `src/hooks/`: event-specific capture logic.
- `src/events/`: normalized outcome and lifecycle types.
- `src/utils/`: scoped state, identity, and Hyphae client helpers.
- `src/policy.rs`: thresholds and capture-policy settings.
- `../septa/`: authoritative schema and fixture for `session-event-v1`.

If Cortina's structured session payload changes, update `../septa/` first.

---

## Before You Start

Before writing code, verify:

1. **Owning layer**: keep host parsing in adapters, shared capture logic in hooks, and structured writes in the Hyphae boundary.
2. **Contracts**: if the Hyphae session payload changes, read `../septa/README.md` first.
3. **Failure policy**: preserve best-effort behavior so hook failures do not stop the outer tool.
4. **Identity model**: keep worktree-scoped and session-scoped identity logic aligned.

---

## Preferred Commands

Use these for most work:

```bash
cargo build --release
cargo test
```

For targeted work:

```bash
cargo test --ignored
cargo test hook
cargo clippy
cargo fmt --check
```

---

## Repo Architecture

Cortina is healthiest when capture, classification, and persistence stay in separate layers.

Key boundaries:

- `src/adapters/`: host-specific intake only.
- `src/hooks/`: lifecycle handling and event wiring.
- `src/events/`: shared normalized outcome model.
- `src/utils/hyphae_client.rs`: structured write boundary.

Current direction:

- Keep capture best-effort instead of blocking the outer runtime.
- Keep adapter-specific parsing separate from shared event logic.
- Keep session identity scoped enough that parallel worktrees do not collapse together.

---

## Working Rules

- Do not turn hook failures into hard stops unless the repo already treats them that way.
- Do not mix host-specific parsing into shared lifecycle handlers.
- Treat session-shape changes as contract work and update `../septa/` in the same change.
- Test fallback behavior when Hyphae is unavailable, not just the happy path.

---

## Multi-Agent Patterns

For substantial Cortina work, default to two agents:

**1. Primary implementation worker**
- Owns the touched adapter, hook, event, or helper layer
- Keeps the write scope inside Cortina unless a real contract update requires `../septa/`

**2. Independent validator**
- Reviews the broader shape instead of redoing the implementation
- Specifically looks for adapter leakage, session-identity drift, broken best-effort behavior, and missing contract updates

Add a docs worker when `README.md`, `CLAUDE.md`, `AGENTS.md`, or public docs changed materially.

---

## Skills to Load

Use these for most work in this repo:

- `basidiocarp-rust-repos`: repo-local Rust workflow and validation habits
- `systematic-debugging`: before fixing unexplained hook or session-capture failures
- `writing-voice`: when touching README or docs prose

Use these when the task needs them:

- `test-writing`: when lifecycle behavior changes need stronger coverage
- `basidiocarp-workspace-router`: when the change may spill into `septa`, `hyphae`, or `volva`
- `tool-preferences`: when exploration should stay tight

---

## Done Means

A task is not complete until:

- [ ] The change is in the right adapter, hook, event, or helper layer
- [ ] The narrowest relevant validation has run, when practical
- [ ] Related schemas, fixtures, or docs are updated if they should move together
- [ ] Any skipped validation or follow-up work is stated clearly in the final response

If validation was skipped, say so clearly and explain why.
