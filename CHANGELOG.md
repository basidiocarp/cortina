# Changelog

## [Unreleased]

### Fixed

- **Platform temp paths**: Cortina now uses the system temp directory for tracking files instead of hardcoded `/tmp` paths, which removes a Windows portability blocker for error, edit, export, and ingest state.

### Changed

- **Shared event envelope parsing**: `pre-tool-use`, `post-tool-use`, and `stop` now read through one normalized event-envelope layer instead of each command manually traversing raw JSON.
- **Explicit Claude adapter boundary**: Cortina now treats the current Claude Code hook envelope as an adapter input rather than the core internal event model, while keeping the existing CLI and output compatibility.

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
