# Normalized Lifecycle Vocabulary

Cortina treats host hook envelopes as adapter-specific input, then maps the
transferable parts into a narrower lifecycle vocabulary that downstream tools
can rely on without parsing host quirks.

## Categories

- `host`: execution-host lifecycle such as Volva hook phases
- `tool`: tool-result lifecycle such as bash outcomes or file edits
- `session`: session-adjacent lifecycle such as prompt submission
- `compaction`: explicit pre-compaction capture
- `council`: prompts or workflow steps that invoke council-like coordination
  and, when available, point back to the active task/worktree identity

## Status Values

- `requested`: a host requested work but it has not completed yet
- `captured`: Cortina observed and recorded the lifecycle event
- `started`: a lifecycle phase began
- `completed`: a lifecycle phase completed successfully
- `failed`: a lifecycle phase ended in failure

## Transferable Fields

The shared contract keeps only the fields other repos can safely reuse:

- `schema_version`
- `category`
- `status`
- `host`
- `event_name`
- `summary`
- `fail_open`
- optional identity fields such as `session_id`, `cwd`, `project_root`, and `worktree_id`
- task-linked council capture may also include `metadata.task_id` and
  `metadata.task_linked=true` when Cortina can resolve the active Canopy task
- optional event-specific fields such as `tool_name`, `trigger`, and `metadata`

## Boundaries

- Host-specific parsing stays in `src/adapters/`
- Normalized lifecycle types live in `src/events/normalized_lifecycle.rs`
- Shared cross-repo schema lives in `septa/cortina-lifecycle-event-v1.schema.json`
- Compaction and council capture stay in hook handlers, not in adapters

## Fail-Open Invariant

Lifecycle capture is intentionally fail-open across hosts. If parsing,
normalization, or downstream storage fails, Cortina logs a warning and allows
the host session to continue. The invariant is explicit in
`cortina/src/policy.rs` and should remain true for Claude Code and Volva paths.
