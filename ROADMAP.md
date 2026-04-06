# Cortina Roadmap

This page is the Cortina-specific backlog. The workspace [ROADMAP.md](../docs/ROADMAP.md) keeps the ecosystem sequencing and cross-repo priorities.

## Recently Shipped

- Cortina now has an adapter-first event model instead of a Claude-shaped core. Claude-specific envelopes live behind an adapter boundary, which keeps the runtime flexible as more hosts appear.
- The CLI surface follows the same boundary. Operators can inspect lifecycle policy, state, and health through Cortina itself instead of piecing behavior together from hooks and local files.
- Hyphae session bridging is in place. Cortina can now start, reuse, and end scoped sessions while emitting best-effort feedback signals and structured outcome attribution.
- Lifecycle persistence is much harder to knock over. Session state, outcome tracking, edit tracking, and export or ingest queues now use locked, atomic updates with extracted regression coverage.
- Capture policy controls are no longer implicit. Deduping windows, correction windows, thresholds, attribution grace, and fallback session behavior all live behind an explicit policy surface.

## Next

### Lamella boundary cleanup

Cortina needs to finish absorbing lifecycle and capture-hook ownership from Lamella. Lamella should package workflows and templates; Cortina should own the runtime behavior that captures, normalizes, and forwards session events. See [docs/lamella-boundary.md](docs/lamella-boundary.md) for the current split.

### Session and outcome policy refinement

The scoped Hyphae session model is in place, but it still needs edge-case cleanup around retries, partial failures, and attribution windows. The near-term goal is to make outcome policy predictable enough that other tools can trust it as infrastructure.

## Later

### More adapters

Cortina can support broader host coverage, but only when another runtime justifies real implementation work. Thin packaging or template layers should stay outside Cortina whenever they can.

## Research

### Broader event bus role

The open question is whether Cortina should remain a narrow host-adapter runtime or become a broader event bus for the ecosystem. That decision depends on whether cross-tool event routing starts to matter more than adapter fidelity.
