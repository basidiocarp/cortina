# Lamella Hook Migration Plan

## Objective

Move the ecosystem-level, stateful Lamella hooks into Cortina so Cortina becomes the runtime for session lifecycle and capture logic, while Lamella remains the distribution layer for workflow, policy, formatting, and plugin-specific hooks.

## Scope

### In scope
- Port `capture-errors.js`, `capture-corrections.js`, `capture-test-results.js`, and `capture-code-changes.js` into Cortina’s Rust hook handlers.
- Port `suggest-compact.js` and `pre-compact.js` into Cortina.
- Update Lamella’s default hook wiring to call Cortina for migrated behavior.
- Keep behavior parity for current log messages and Hyphae writes where practical.

### Optional in scope
- Port `capture-pr-reviews.js` into Cortina if PR review memory is considered part of the core ecosystem signal model.

### Out of scope
- `session-start.js`
- `session-end.js`
- `post-edit-format.js`
- `post-edit-typecheck.js`
- `post-edit-console-warn.js`
- `check-console-log.js`
- `comment-style-check.sh`
- `pre-write-doc-warn.js`

These remain Lamella responsibilities because they are repo policy, editor ergonomics, or startup UX rather than core event processing.

## Source Context

- Lamella hook wiring: `lamella/resources/hooks/hooks.json`
- Lamella hook scripts: `lamella/scripts/hooks/`
- Cortina hook entry points: `cortina/src/hooks/pre_tool_use.rs`, `cortina/src/hooks/post_tool_use.rs`, `cortina/src/hooks/stop.rs`

## Plan

### Phase 1: Define the migration boundary

1. Write a hook ownership matrix mapping every Lamella hook script to one of:
   - migrate to Cortina
   - keep in Lamella
   - retire because Cortina already replaces it
2. Decide whether `capture-pr-reviews.js` moves in the first pass or a follow-up pass.
3. Freeze the target Cortina command surface:
   - `cortina pre-tool-use`
   - `cortina post-tool-use`
   - `cortina pre-compact`
   - `cortina stop`

### Phase 2: Add shared session state in Cortina

1. Introduce a single per-session state store for:
   - recent errors
   - recent edits
   - pending changed files
   - pending documentation files
   - tool-call count
   - compaction markers
2. Replace scattered `/tmp` JSON files with one schema owned by Cortina.
3. Add cleanup rules and tests for stale state.

### Phase 3: Port lifecycle hooks

1. Add a new `pre_compact` hook module in Cortina.
2. Port `pre-compact.js` behavior:
   - log compaction event
   - append compaction marker to the active session file when applicable
3. Port `suggest-compact.js` behavior into Cortina’s pre-tool lifecycle:
   - increment per-session tool count
   - emit threshold-based compaction suggestions
4. Keep environment-controlled thresholds compatible with the current hook.

### Phase 4: Finish PostToolUse consolidation

1. Verify `post_tool_use.rs` fully covers:
   - errors and resolutions
   - self-corrections
   - test failures and fixes
   - changed-file export thresholds
2. Fill any gaps before removing Lamella JS capture hooks.
3. If adopted now, add PR review capture as a separate Cortina post-tool module.

### Phase 5: Switch Lamella defaults

1. Update `lamella/resources/hooks/hooks.json` to call Cortina for migrated responsibilities.
2. Remove or disable redundant Lamella capture hook entries.
3. Keep Lamella-only hooks unchanged.
4. Ensure installs still degrade safely when Cortina is missing.

### Phase 6: Docs and rollout

1. Update Cortina README to describe the broader “session runtime” role.
2. Update Lamella docs to distinguish:
   - Cortina-owned lifecycle/capture hooks
   - Lamella-owned workflow/policy hooks
3. Update Stipe init/setup flows so the default installed hook graph uses Cortina where available.

## Verification

- `cargo test` passes in `cortina/`
- New unit tests cover session state load/save/cleanup
- Hook fixture tests confirm equivalent behavior for:
  - error capture
  - correction capture
  - test result capture
  - export threshold handling
  - compaction suggestion thresholds
  - pre-compact logging
- Manual end-to-end check:
  - Lamella hook config points to Cortina for migrated hooks
  - Hyphae still receives the expected topics
  - compaction suggestions still appear at the same thresholds

## Risks

- Scope creep: Cortina can become “all hooks in Rust” unless the boundary stays explicit.
- Behavior drift: user-facing logs and temp-file semantics may change unless parity is tested.
- Install complexity: Lamella and Stipe must agree on when Cortina is required vs optional.

## Done Criteria

- Lamella no longer uses separate JS capture hooks for logic that Cortina owns.
- Cortina owns ecosystem signal capture plus compaction lifecycle handling.
- Lamella remains the home for formatting, policy, and startup UX hooks.
