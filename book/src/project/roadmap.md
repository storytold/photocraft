# Roadmap

The live product roadmap, milestone definitions, parity measurements, and current focus are maintained in [`docs/roadmap.md`](https://github.com/storytold/photocraft/blob/main/docs/roadmap.md). Menu coverage is generated into [`docs/parity-checklist.md`](https://github.com/storytold/photocraft/blob/main/docs/parity-checklist.md) by `cargo xtask parity`.

Security hardening is cross-cutting rather than a replacement for those product milestones. The current security sequence is:

1. threat model and reporting policy;
2. authenticated control sessions;
3. capability-scoped MCP/control access;
4. workspace-rooted filesystem handles;
5. request, batch, render, and connection budgets;
6. broader parser fuzzing and malicious-input regression corpora;
7. dependency policy and release SBOM/provenance.

Only the documentation foundation in item 1 is established by this book. Bearer-token authentication of control connections (part of item 2), workspace-rooted filesystem handles (item 4) and request, batch, preview and connection budgets (item 5) are now implemented; the other controls remain proposed until implemented and tested.
