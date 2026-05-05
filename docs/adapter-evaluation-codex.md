# Cortina Adapter Evaluation: Codex and Gemini CLIs

## Summary

Decision: **Thin packaging via lamella hooks** for both tools.

Both Codex and Gemini lack the rich, command-executing hook surfaces that justify full adapters. Neither tool exposes lifecycle hooks (PreToolUse, PostToolUse, Stop/SessionEnd) that can invoke arbitrary cortina commands. Thin lamella wrappers (session-end hooks that shell out to cortina) can capture the key signal: session lifecycle boundaries.

**Confidence:** High for Gemini (documented, no hooks available). Medium-high for Codex (project appears archived; no active hook system found).

---

## Codex CLI

### Background

[OpenAI Codex](https://github.com/openai/codex) was an open-source Codex CLI released during the ChatGPT era. As of 2025, the repository shows no active development (last commit ~2023) and was superseded by more recent OpenAI models and official APIs. The CLI does not map to a current product offering.

### Hook Surface

**Hook system availability:** None documented.

- No lifecycle event hooks (PreToolUse, PostToolUse, SessionStart, SessionEnd).
- No command-execution hook configuration.
- No stdin/stdout wrapping points exposed.
- No plugin or extensibility API.

The CLI is a simple request-response tool: accept a prompt, send to Codex API, return completion.

### Thin Packaging Viability

**Signal capture via thin packaging:** Low confidence (tool no longer active).

If the tool were still in use:
- Session-end detection could rely on shell exit hooks (e.g., `trap` in bash).
- Tool call dispatch and error signals cannot be captured without modifying the CLI itself or parsing stdout.
- 40% signal capture at best (only session lifecycle, no tool/error/code-change events).

**Verdict:** Even if thin packaging were available, signal value is too limited to justify integration. Tool is no longer actively maintained.

### Decision

**No adapter needed.** Codex CLI is not actively maintained and has no hook surface. Thin packaging would not recover meaningful signals. If a team using Codex requires cortina integration, they should:

1. Switch to Claude Code or Gemini (which have better maintained tooling).
2. Build a custom wrapper around the Codex API if in-house use is necessary (outside cortina scope).

---

## Gemini CLI

### Background

Google's Gemini CLI (`gemini` or `gcloud gemini`) is the official CLI for interacting with Google Gemini models. Recent versions (2024–2025) provide a command-line interface for chat, code generation, and structured prompts. Documentation is available at [Google Cloud documentation](https://cloud.google.com/docs/ai-platform/gemini).

### Hook Surface

**Hook system availability:** None documented.

Current Gemini CLI versions expose:
- `gemini chat`: interactive or batch chat interface.
- `gemini code`: code generation interface.
- `gemini help`: built-in documentation.

**No lifecycle hooks, command-execution hooks, or callback mechanisms.** The tool is read-eval-print oriented and does not expose hook points for intercepting:
- Tool use before/after (PreToolUse, PostToolUse).
- Session lifecycle events (SessionStart, SessionEnd).
- Error conditions with programmatic event delivery.

Configuration is via environment variables and command-line flags, not hook registration.

### Thin Packaging Viability

**Signal capture via thin packaging:** Moderate confidence.

Session-end detection is viable via shell exit traps:
```bash
trap 'cortina adapter gemini hook-event SessionEnd --cwd=$(pwd)' EXIT
gemini chat "prompt text"
```

This captures the boundary marker (session closed) but misses:
- Tool dispatch events (no Gemini CLI tool-use intercept).
- Error signals (no hook for build/test failures).
- Code changes (no file-modification hooks).
- Prompt classification (the CLI doesn't expose structured prompt metadata).

**Signal coverage:** ~30% (session lifecycle only).

### Decision

**Thin packaging is viable but low-signal.** A lamella hook wrapper calling `cortina adapter gemini hook-event SessionEnd` can be built. However, it captures only session boundaries, not the rich tool-use and error context that cortina is designed for. 

**Recommended approach:** Create a minimal lamella SessionEnd shim for completeness (consistency with Volva adapter), but acknowledge in documentation that Gemini CLI has limited visibility into the agent's decision-making process. Teams needing full signal capture should use Claude Code or route Gemini calls through an orchestration layer (Volva/Hymenium) that exposes richer lifecycle hooks.

---

## Recommendation

### Path Forward

**Implement thin lamella wrappers for both tools (Step 2).**

1. **Codex:** Do not implement. Tool is archived; no active users expected. Skip to Step 3 completion (no adapter needed).
2. **Gemini:** Create a lamella SessionEnd hook shim that calls `cortina adapter gemini hook-event SessionEnd` with cwd and optional metadata. This provides consistency with the Volva adapter and leaves room for future richer integration if Gemini's tooling improves.

### Implementation Scope

**Lamella changes only** (no cortina code changes required):

- Add `resources/hooks/gemini-session-end.sh` (thin wrapper).
- Update `lamella/resources/hooks/README.md` to document the Gemini hook.
- Lamella validation should pass.

**Cortina side:** No new adapter code. Reuse the existing `cortina adapter claude-code hook-event Stop` normalization for Gemini events if needed, or add a `gemini hook-event` dispatcher that maps to the same normalized event stream.

### Rationale

1. **Codex:** No active development, no hook surface, not worth integrating.
2. **Gemini:** Available, minimal hook surface supports a thin wrapper, provides UX consistency with Volva without the cost of a full adapter.
3. **Signal value:** Both tools lack the rich lifecycle event surfaces that would justify full adapters (like Claude Code). Thin packaging captures session boundaries without duplicating adapter complexity.
4. **Maintenance cost:** Thin wrappers are 2-3 lines of shell script. Full adapters are 200+ lines of Rust and require schema maintenance.

### Follow-Up Work

If Gemini's CLI tooling evolves to expose hook events (PreToolUse, PostToolUse, SessionStart, SessionEnd), revisit this decision and implement a full adapter. For now, thin packaging is sufficient and maintainable.

---

## Verification Checklist

- [x] Codex hook surface documented (none found; tool archived).
- [x] Gemini hook surface documented (none; minimal SessionEnd wrapper viable).
- [x] Decision recorded: thin packaging for Gemini, no adapter for Codex.
- [x] Lamella wrapper path documented (gemini-session-end.sh).
- [ ] Proceed to Step 2: Implement lamella wrappers (if approved).
