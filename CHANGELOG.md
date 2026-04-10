# Changelog

All notable changes to Cortina are documented in this file.

## [Unreleased]

## [0.2.11] - 2026-04-09

### Changed

- **Lighter command classification**: The significant-command classifier now
  uses token and phrase matching instead of a direct `regex` dependency.
- **SQLite packaging policy**: Non-Windows builds use system SQLite, while
  Windows keeps the bundled fallback for portability.
- **Docs structure**: Cortina now has a central `docs/README.md` and plan
  index aligned to the lowercase docs layout.

## [0.2.9] - 2026-04-08

### Changed

- **Foundation alignment**: maintainer docs and boundary notes now describe
  Cortina's adapter-first ownership and evidence bridge more explicitly.
- **Test surface split**: Pre-tool-use regression coverage now lives in a
  focused test module instead of growing the hook implementation file further.

### Fixed

- **Evidence durability**: one-shot process exits no longer drop bounded
  Canopy evidence writes at shutdown.
- **Tracing and diagnostics**: adapter rewrites and helper subprocesses now
  preserve the shared tracing context and stderr at the failure points that
  operators actually need.

## [0.2.8] - 2026-04-08

### Fixed

- **Durable Canopy evidence attachment**: outcome evidence writes now complete
  synchronously with bounded retries, so one-shot CLI exits no longer drop
  Canopy evidence references on process shutdown.
- **Broader adapter and subprocess tracing**: rewrite and evidence paths now
  enter shared workflow, tool, and subprocess spans at the boundaries that tend
  to fail under real hook traffic.
- **Better child-process diagnostics**: Canopy and Hyphae helper subprocesses
  no longer black-hole stderr in production, while test runs still stay quiet.
- **Logging docs now match runtime reality**: the README now distinguishes
  shared structured tracing from the remaining intentional direct `eprintln!`
  compatibility warnings.

## [0.2.7] - 2026-04-08

### Changed

- **Shared logging rollout**: Cortina now initializes logging through Spore's
  app-aware `CORTINA_LOG` path instead of relying on generic runtime setup.
- **Lifecycle tracing**: Hook dispatch, Hyphae session flows, Hyphae background
  writes, and Canopy evidence bridge subprocesses now emit shared tracing spans
  with workspace-aware context for faster failure localization.

### Fixed

- **Operator guidance**: Docs now distinguish Cortina's debug logging from
  normal hook stdin/stdout behavior and stderr diagnostics.

## [0.2.6] - 2026-04-03

### Added

- **Volva adapter**: Cortina now includes a Volva adapter for backend
  integration.

### Changed

- **Adapter event routing**: Main event handling now routes adapter events
  through the updated integration path instead of the earlier narrower wiring.

## [0.2.4] - 2026-03-31

### Fixed

- **Statusline context meter**: `cortina statusline` now computes context usage
  from the latest assistant turn's live prompt footprint instead of cumulative
  session totals.

## [0.2.3] - 2026-03-31

### Changed

- **Statusline layout**: `cortina statusline` now renders a clearer two-line
  summary for context, tokens, cost, savings, git state, and workspace
  identity.

## [0.2.2] - 2026-03-31

### Fixed

- **Interactive stdin handling**: `cortina statusline` no longer blocks waiting
  for EOF on terminal stdin and now handles piped JSON input cleanly.

## [0.2.1] - 2026-03-31

### Added

- **Claude statusline command**: `cortina statusline` now reads Claude Code's
  statusline envelope and prints a compact session summary, with `--no-color`
  for plain sinks.

## [0.2.0] - 2026-03-31

### Added

- **Canopy evidence bridge**: Cortina can now attach best-effort outcome
  evidence to the active Canopy task for the current worktree.

### Changed

- **Strict identity-v1 runtime**: Session startup, stop handling, and Hyphae
  interaction now use structured project, worktree, and runtime identity.
- **Published Spore discovery**: Cortina now consumes the released Spore
  discovery surface for Hyphae, Canopy, and its own tool identity.

### Fixed

- **Stop-path attribution**: Outcome attribution now prefers exact session or
  identity matches instead of mirroring legacy project-scoped fallback
  memories.
- **Lifecycle consistency**: Runtime session propagation and outcome persistence
  now align with the shared Hyphae and Cap timeline contract.

## [0.1.7] - 2026-03-29

### Added

- **Structured outcome attribution**: `PostToolUse`, `Stop`, and `SessionEnd`
  now share one durable outcome model for corrections, recoveries, validations,
  exports, and ingest events.

### Changed

- **Lifecycle module boundaries**: The hook runtime is now split into focused
  `events`, `utils`, `post_tool_use`, and `stop` modules with regression
  coverage.

### Fixed

- **Atomic lifecycle persistence**: Temp-state updates for sessions, outcomes,
  errors, edits, exports, and ingest queues are now locked and atomic.

## [0.1.6] - 2026-03-27

### Fixed

- **Structured session liveness**: Cached Hyphae sessions are now validated
  through `hyphae session status --id ...` instead of human-readable listings.
- **Scoped session cache checks**: Worktree-scoped sessions now reject
  mismatched scope data and restart cleanly when a competing active session
  exists.

## [0.1.5] - 2026-03-27

### Added

- **Validation outcome signals**: Successful build and test commands can now
  emit structured Hyphae `build_passed` and `test_passed` signals.

### Fixed

- **Scoped Hyphae sessions**: Cortina now starts Hyphae sessions with worktree
  scope so parallel workers do not collapse into one active session.
- **Stale session reuse**: Cached Hyphae session state is now checked against
  live session context before reuse.

## [0.1.3] - 2026-03-27

### Added

- **Hyphae session bridge**: Cortina can now start, reuse, and end Hyphae
  sessions around structured correction and recovery signals.

### Fixed

- **Failure-path cleanup**: Best-effort Hyphae session cleanup now preserves
  cached state on spawn and non-zero exit failures and avoids phantom session
  endings.

## [0.1.2] - 2026-03-26

### Changed

- **Shared event envelope parsing**: `pre-tool-use`, `post-tool-use`, and
  `stop` now read through one normalized event-envelope layer.
- **Explicit Claude adapter boundary**: Claude Code hook envelopes are now
  treated as adapter inputs instead of the internal event model.
- **Adapter-oriented CLI**: `cortina adapter claude-code ...` is now the
  preferred surface, while older flat commands remain as hidden compatibility
  aliases.
- **Shared adapter dispatch**: The binary entry point now routes host events
  through the adapter layer instead of wiring Claude-specific handlers directly.

### Fixed

- **Platform temp paths**: Cortina now uses the system temp directory instead
  of hardcoded `/tmp` paths.

## [0.1.1] - 2026-03-22

### Changed

- **Typed importance model**: Stringly typed importance values were replaced
  with an `Importance` enum across the public surface.
- **Transcript summary model**: `parse_jsonl_transcript` now returns a struct
  instead of mutating many reference arguments.
- **Clippy suppression docs**: `#[allow(clippy::unnecessary_wraps)]` sites now
  include reasons.

### Fixed

- **Regex caching**: Hot-path regexes now compile once through `OnceLock`
  instead of on every call.
- **Build-success precedence**: Cortina no longer misclassifies non-build
  commands because of `&&` and `||` precedence.
- **JSON parse visibility**: Hook handlers now log parse failures to stderr
  instead of silently returning.
- **Exit-code handling**: The previous `i32::MAX` sentinel has been replaced
  with `Option` handling.
- **cwd hash reuse**: Dedup helpers now reuse a single hash per event.

## [0.1.0] - 2026-03-20

### Added

- **Hook runtime**: Cortina shipped with `pre-tool-use`, `post-tool-use`, and
  `stop` flows in one Rust binary.
- **Mycelium rewrite bridge**: `pre-tool-use` can rewrite commands through
  Mycelium when it is available.
- **Structured capture**: `post-tool-use` records errors, self-corrections,
  test failures, code changes, and doc changes.
- **Session summaries**: `stop` writes session summaries to Hyphae with changed
  files, errors, and decisions.
- **Rust replacement path**: The initial release replaced the earlier mix of
  JavaScript and shell hook scripts with one compiled tool.
