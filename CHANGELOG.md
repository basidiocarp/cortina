# Changelog

## [0.1.0] - 2026-03-20

Initial release.

- PreToolUse hook: rewrites commands through Mycelium when available
- PostToolUse hook: captures errors, self-corrections, test failures, code changes, doc changes
- Stop hook: writes session summary to Hyphae with files changed, errors, and decisions
- Single binary, three subcommands (`pre-tool-use`, `post-tool-use`, `stop`)
- Replaces 5 JavaScript files and 2 shell scripts from Lamella/Mycelium
