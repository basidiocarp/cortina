# Changelog

## [Unreleased]

## [0.1.6] - 2026-03-27

### Fixed

- **Structured Hyphae session liveness**: Cortina now validates cached Hyphae sessions through `hyphae session status --id ...` instead of parsing human-readable session listings.
- **Scoped session cache checks**: cached worktree-scoped sessions now reject mismatched scope data and restart cleanly when another active session exists in the same project.

## [0.1.5] - 2026-03-27

### Added

- **Validation outcome signals**: successful build and test commands can now emit structured Hyphae `build_passed` and `test_passed` signals.

### Fixed

- **Scoped Hyphae sessions**: Cortina now starts Hyphae sessions with a worktree scope so parallel workers in one project do not share a single active session.
- **Stale session cache reuse**: cached Hyphae session state is now checked against live session context before reuse.

## [0.1.3] - 2026-03-27

### Added

- **Hyphae session bridge**: Cortina can now start, reuse, and end Hyphae sessions around structured correction and recovery signals instead of only writing ad hoc session memories.

### Fixed

- **Failure-path session handling**: Best-effort Hyphae session cleanup now preserves cached state on spawn and non-zero exit failures, avoids phantom session endings when no state exists, and has regression coverage for those failure paths.

## [0.1.2] - 2026-03-26

### Fixed

- **Platform temp paths**: Cortina now uses the system temp directory for tracking files instead of hardcoded `/tmp` paths, which removes a Windows portability blocker for error, edit, export, and ingest state.

### Changed

- **Shared event envelope parsing**: `pre-tool-use`, `post-tool-use`, and `stop` now read through one normalized event-envelope layer instead of each command manually traversing raw JSON.
- **Explicit Claude adapter boundary**: Cortina now treats the current Claude Code hook envelope as an adapter input rather than the core internal event model, while keeping the existing CLI and output compatibility.
- **Adapter-oriented CLI**: `cortina adapter claude-code ...` is now the preferred command surface, while the old flat Claude hook commands remain as hidden compatibility aliases.
- **Shared adapter dispatch**: The binary entrypoint now routes host events through the adapter layer instead of wiring Claude-specific handlers directly in `main`.

## [0.1.1] - 2026-03-22

### Fixed

- **Regex caching**: 41 patterns now compiled once via `OnceLock` instead of per-call. Eliminates repeated compilation in `has_error`, `is_build_command`, `is_significant_command`.
- **Build-success logic**: Fixed operator precedence bug — `&&` was binding tighter than `||`, causing export triggers on non-build commands when exit code was absent.
- **JSON parse errors**: All three hook handlers now log parse failures to stderr instead of silently returning with a no-op.
- **Exit code sentinel**: Replaced `i32::MAX` fallback with `Option` via `.ok()` for out-of-range exit codes.
- **cwd_hash dedup**: Hash computed once per event and passed to helpers, down from 4 syscalls per invocation.

### Changed

- **Importance enum**: Replaced stringly-typed `&str` importance parameter with typed `Importance` enum (Low, Medium, High).
- **TranscriptSummary struct**: `parse_jsonl_transcript` returns a struct instead of taking 6 mutable reference parameters. Renamed `errors_resolved` to `errors_encountered` (correct semantics).
- **Allow reasons**: Added `reason` attributes to all `#[allow(clippy::unnecessary_wraps)]` suppressions.

## [0.1.0] - 2026-03-20

Initial release.

- PreToolUse hook: rewrites commands through Mycelium when available
- PostToolUse hook: captures errors, self-corrections, test failures, code changes, doc changes
- Stop hook: writes session summary to Hyphae with files changed, errors, and decisions
- Single binary, three subcommands (`pre-tool-use`, `post-tool-use`, `stop`)
- Replaces 5 JavaScript files and 2 shell scripts from Lamella/Mycelium
