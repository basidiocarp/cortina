# Cortina Roadmap

This page is the Cortina-specific backlog. The workspace [ROADMAP.md](../ROADMAP.md) keeps the ecosystem sequencing, and [MASTER-ROADMAP.md](../MASTER-ROADMAP.md) keeps the cross-repo summary.

## Recently Shipped

- Adapter-first event model.
- Claude-specific event envelope isolated behind an adapter boundary.
- Adapter-owned CLI surface and dispatch path.
- More portable temp-path handling.
- Hyphae session bridge for structured session start, reuse, end, and best-effort feedback signal emission.

## Next

### Structured outcome attribution

Emit richer structured outcome events, not just normalized capture or topic-based memory writes.

### Recall attribution

Attribute fixes, corrections, and successful tests back to prior recalls or sessions where possible.

### Session lifecycle tightening

Keep improving the `hyphae session start` and `hyphae session end` integration path so lifecycle capture is reliable and boring.

### Lamella boundary cleanup

Finish moving ecosystem lifecycle and capture-hook ownership out of Lamella and into Cortina.
See [docs/lamella-boundary.md](docs/lamella-boundary.md) for the current move-now and move-later split.

## Later

### Capture policy controls

Add configurable capture thresholds, deduping, and noise suppression.

### More adapters

Add broader lifecycle adapters if another host justifies real implementation work.

## Research

### Broader event bus role

Decide whether Cortina should stay narrowly focused on host adapters or become a broader event bus for the ecosystem.
