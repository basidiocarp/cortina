# Cortina Docs

The canonical Cortina docs are small on purpose. Keep the core lifecycle and
boundary story here, and leave workspace-wide strategy in the root
`docs/workspace/` tree.

Start with:

- [lamella-boundary.md](lamella-boundary.md): ownership split between Lamella
  packaging and Cortina runtime behavior
- [normalized-lifecycle-vocabulary.md](normalized-lifecycle-vocabulary.md):
  shared lifecycle categories, statuses, and fail-open boundary rules
- [plans/README.md](plans/README.md): active planning entrypoint

Then use the root docs for the broader repo view:

- [../README.md](../README.md): what Cortina captures and how the runtime works
- [../ROADMAP.md](../ROADMAP.md): Cortina-specific backlog
- [../CLAUDE.md](../CLAUDE.md): implementation guidance and contract notes
