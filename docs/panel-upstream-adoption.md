# Panel upstream adoption, 2026-11-02

The shared host APIs were merged in storytold/craft-libs#470 at
`ea1fbeb05b94bb692b27b687adb2ac1e5abc640f`. PhotoCraft now consumes this immutable
upstream revision directly. The temporary vendored UI and its snapshot-only
build guard are removed from the full reference branch.

The reference merges PhotoCraft main `289dd0a87` (0.19.0) at `0555be175`.
Icons retain both the host source helper and upstream footer constants;
attribution retains both fixture categories. Documentation histories are retained
and the scorecard regenerated: 10 of 183 preferences are unread.

## Review units

PhotoCraft #10908 is the first consumer PR: only the dependency pin and lockfile,
based on current main. Workspace migration/persistence, compact/resize behavior,
and floating Tools remain subsequent changes. #10322 remains the full reference.
No PR was merged by this agent.

## Fresh verification

Dependency-only branch `aa5cd1dcf`: desktop all-target check, 43 docking tests,
fmt, layers (31 crates), wasm (24 crates plus codecs/heif) passed.

Full reference `0555be175`: desktop all-target check, 167 docking tests (2 ignored
benchmarks), 9 workspace integration tests, fmt, layers and wasm passed. WGPU
captures produced 444 PNGs across three Pro themes and scales 1, 1.25, 1.5 and 2;
seam assertions and capture tests passed. Representative 100% floating resize
and 125% below-document docking PNGs were inspected. Historical GIFs retain their
old provenance and are not evidence of this merge.

Strict UI Clippy in both branches stops in unchanged upstream IO files:
`psd_export/document.rs:106` (drop_non_drop) and `smart_map/import.rs:784`
(nonminimal_bool). Those files have no diff from current main. No lint suppression
or unrelated fix was added.

A broad name filter selected 851 UI tests, encountered failures including Color
Wheel and Curves, and was stopped. It has no complete result or current-main
baseline comparison; no full UI pass is claimed. No new runtime hot-path algorithm
was introduced in this adoption step; no fresh performance comparison is claimed.
The product EXE was not built or launched. Physical mixed-DPI/monitor behavior,
native focus/IME and GPU latency still need desktop acceptance.

Logs and PNGs are in the portable 2026-11-02 handoff. They are not bundled in this
feature PR. The previous source remains at codex/panel-before-v019-2026-11-02.
