# Roadmap

> **Last reviewed:** 2026-10-31 · **Last updated:** 2026-10-31 · **Change:** major (re-measure at 0.18.0: current focus, estimates, milestones and alpha-gate evidence refreshed; the stage stays alpha) · **Target:** Adobe Photoshop 2026

- UI.5: Panels can detach, join other tab groups, split, resize and redock through shared docking transactions. Custom layouts persist with workspaces, respect workspace locking, and retain collapse flags for intact native groups.

- UI.3: Shared icons, accessible icon buttons and panel tabs come from the pinned `craft-ui` library. Tab overflow stays within narrow panels; document tiling uses bounded shared geometry. Palettes and document commands remain app-owned. Existing strict-Clippy boolean/range warnings are corrected.

- UI.4: Compact toolbars keep colour and mode controls visible while tool rows scroll. Long tool-options rows scroll horizontally and reveal keyboard focus; offscreen geometry and interaction regressions cover all themes and 1×/1.5×/2× display scales.


Forward-looking plan: milestones, the **Current focus** list, and what's next with estimates.
Where we stand today is in [`target-app-parity.md`](target-app-parity.md) (two numbers, by
dimension and area) and the ranked work list in [`gaps.md`](gaps.md); the one-page summary is
[`ROADMAP.md`](../ROADMAP.md). Measured numbers: [`scorecard.md`](scorecard.md)
(`cargo xtask scorecard`: performance budgets, corpus floors, per-area checklists, dead
preferences) and [`parity-checklist.md`](parity-checklist.md) (`cargo xtask parity`: menu wiring,
631/631, which says nothing about behaviour).

Status legend: ✅ done · 🟡 in progress · ⬜ not started. Hours are Opus 5.5 agent wall-clock hours
for one agent working sequentially (calibration in
[`target-app-parity.md`](target-app-parity.md#remaining-effort-and-calibration)).

**Stage: alpha** (ready for real work ~52%). Beta needs ~75% and reliable PSD: ~1,350–2,200 h.

## Alpha gate

The core workflows a typical Photoshop professional runs every day, checked end to end on the main
platform (macOS, 0.6.0), including saving and reopening the work. Any **no**, or a **partial**
that blocks the workflow, would make PhotoCraft pre-alpha. The evidence column was refreshed for
0.18.0 on 2026-10-31 from tests, corpus runs and issues; the workflows were not walked by hand again.

| Core workflow | Works end to end? | Evidence | Hours to pass |
|---|---|---|---:|
| Photo retouch: open JPEG/raw, heal/clone/remove, adjustment layers, export JPEG, save and reopen | yes | Spot Healing, Healing, Clone, Patch, Remove tools; 16 adjustment layers; raw for DNG/CR2/NEF/ARW/RAF; JPEG export; MCP workflow tests (`automation/tests/it/agent_tasks.rs`). Quality complaints (#1092, #2104) and no CR3 don't stop the workflow | 0 |
| Composite: layers, masks, selections, blend modes, smart objects; save as PSD and `.pcraft` and reopen | yes | PSD round trip 169/170, 308/309, 258/258 corpus files; `.pcraft` atomic save and crash recovery; Multi-layer Smart Object conversion (#2604) and mask targeting (#2458) landed | 0 |
| Cut-out: select a subject, refine the edge, mask, export PNG with transparency | partial (not blocking) | Object/Quick Selection, Select Subject (classical), Photoshop's Select and Mask workspace, layer masks, PNG export. No Refine Hair (#2671), Object Aware or learned segmentation (#2788) | 0 (depth: 60–110 h, G7) |
| Design: type, shapes, layer styles, artboards; export PNG/JPEG; save and reopen | partial (not blocking) | Point/paragraph/area type, type on a path (rigid glyphs), shapes with stroke controls, all 10 layer effects, artboards, Export As. The Type tool's macOS freeze (#2518) closed with the font scan moved off the UI thread (#3021) | 0 |
| Paint with a pen tablet: pressure brushes, layers, save and reopen | partial (not blocking) | Pressure, tilt, rotation on macOS (AppKit monitor); every Brush Settings section. Lag with big brushes (#2619) and feel gaps ([brush-parity.md](brush-parity.md)) | 0 (feel: 100–180 h, G4) |
| Exchange PSDs with Photoshop users: open a client PSD, edit, send a PSD back | partial (not blocking for alpha; blocking for beta) | Photoshop-authored oracle 190/258 render as Photoshop does (effects 3/37); most exports open in Photoshop (smart objects verified), but some don't (#1281) | 0 for alpha; 150–250 h for beta (G1) |

**Result: passes.** Every core workflow completes end to end and the work saves and reopens in
PhotoCraft; the partial rows are depth and fidelity gaps with workarounds, not blockers. Ready for
real work is ~52%, inside the alpha band (~40–75%).

## Milestones

| M | Status | Where we are |
|---|---|---|
| M0 Skeleton | ✅ | workspace, xtask (layers / wasm / ci / stats / corpus / parity), CI workflow |
| M1 Foundation | ✅ | geom, color (27 blend modes), raster (COW tiles, any depth), doc, ops, cms (ICC) |
| M2 PSD v1 | ✅ | photocraft-psd: 134/135 real files byte-exact round trip |
| M3 Viewer app | ✅ | egui shell (Pro / Studio / Classic themes), 13+ codecs, native and web (trunk) builds |
| M4 Native format + engine | ✅ | ~700 commands, `.pcraft` (incremental, autosave, crash recovery), CLI, persistent preferences |
| M5 GPU compositor | 🟡 | wgpu compositor drives the canvas, layer effects, vector masks, artboards, pattern fills and every clip case included (≤1/255 vs CPU); Multichannel documents fall back to the CPU |
| M6 Paint + select | 🟡 | brush engine, all selection tools, multi-layer selection, snapping + smart guides, free transform + warp; stylus pressure on Windows (WM_POINTER via winit), the web (Pointer Events, with tilt/twist), macOS (AppKit event monitor: pressure, tilt, rotation, eraser end) and Linux X11 (XInput2 raw valuators); on Linux Wayland a pen opens the window through Xwayland (native tablet-v2 still open, #79) |
| M7 Adjust + filters | 🟡 | 16 adjustment layers + destructive-only adjustments, 70+ filters incl. Blur Gallery, Actions record/replay, Fade |
| M8 PSD v2 | 🟡 | adjustments (incl. Selective Color, Color Lookup), fills, effects, patterns, text, shapes, smart objects, alpha channels; oracle floors 148/170 (io), 249/309 (psd-tools), 190/258 (Photoshop-authored), all measured exactly at the floor on 2026-10-31 |
| M9 Text, vector, styles | 🟡 | type engine + Warp Text, shapes / pen / paths, all 10 effects on CPU and GPU (parity ≤1/255, 30/31 corpus effect files on the GPU) |
| M10 Smart features | 🟡 | classical Select Subject / Object, content-aware fill and scale, healing, auto-align / auto-blend; ML backend not started |
| M11 Automation + formats | 🟡 | MCP (headless + live bridge), batch, Image Processor, prefs over MCP; DoD test passes (10 agent tasks over MCP, `automation/tests/it/agent_tasks.rs`); since 2026-10-07 the CLI rejects unknown flags and answers `<subcommand> --help`, batch runs report same-name outputs instead of overwriting them, MCP `command_batch` steps can start background jobs, and headless saves never flatten over the opened file; DPX and Cineon read and write, DICOM read; JP2 and C2PA pending |
| M12 Pro parity | 🟡 | CMYK / Lab / Indexed / Bitmap / Duotone, ICC + soft proofing, channels + Quick Mask, smart-object stack modes, artboards, layer comps; print, HDR, timeline pending; Photomerge is implemented, with real-world quality validation still open |

Releases: 0.1.0 (2026-10-02), 0.2.0 (2026-10-05, first signed and notarized), 0.3.0 (10-07),
0.5.0 (10-08), 0.6.0 (10-10), 0.7.0 (10-10), 0.8.2 (10-12), 0.9.0 (10-14), 0.10.0 (10-17),
0.11.1 (10-20), 0.12.0 (10-21), 0.13.0 and 0.14.0 (10-22), 0.15.1 (10-23), 0.16.0 (10-25),
0.17.0 (10-28), 0.18.0 (10-30).

## Current focus

Ranked; each item links to its gap entry. Pick from the top unless an issue is assigned to you.

1. **PSD that Photoshop trusts** ([G1](gaps.md#g1-psd-fidelity-and-photoshop-acceptance)):
   fix Photoshop refusing our files (#1281); add a Photoshop re-open check (FILE-216-4);
   Photoshop-oracle floor 190/258 → 230+, starting with layer effects (3/37, half of the 68
   failures), then CMYK and Lab adjustments. 150–250 h.
2. **Performance budgets** ([G2](gaps.md#g2-interactive-performance)): the 24 over-budget
   scenarios of the 2026-10-31 baseline, worst first (P60/P61, 150 full-canvas layers on the app's
   GPU budget, ~6 s), then 12 MP duplicate and paste, then layer property edits; big-brush and
   cursor lag (#2619, #1343). 150–280 h.
   Local adaptive [Unsharp Mask optimisation](unsharp-mask-performance.md) improves native
   threshold-zero 24 MP RGBA CPU application by 37–102x at radii 300–1000, with measured
   rounding differences and an exact fallback; no published budget is closed by that measurement.
3. **User bug backlog** ([G3](gaps.md#g3-user-reported-bug-backlog)): 2,351 open issues, ~1,760
   actionable, 50 critical and 182 high; triage weekly, fix regressions first. 750–1,200 h.
4. **Brush and transform feel** ([G4](gaps.md#g4-brush-feel), [G5](gaps.md#g5-transform-handles-and-modifiers)):
   [`brush-parity.md`](brush-parity.md) and [`ui-parity.md`](ui-parity.md) rows marked partial
   or missing. 150–270 h.
5. **Workspaces and tool depth** ([G7](gaps.md#g7-workspaces-shown-as-parameter-dialogs),
   [G6](gaps.md#g6-tool-depth)): every toolbar tool exists (68 of 68); Content-Aware Fill and Path
   Blur are still dialogs, and 9 tool checklist rows are partial. 75–160 h.

## What's next, with estimates

| Next milestone | Scope | Estimate |
|---|---|---:|
| "Files you can trust" | Current focus 1 plus AVIF read and Photoshop PDF read | 200–320 h |
| "Fast enough" | Current focus 2; complex layout documents | 150–280 h |
| "Feels like Photoshop" | Current focus 4 and 5; press-and-hold toolbar flyouts | 225–430 h |
| Beta | All of the above, type depth, Hindi and full RTL, bug backlog under 100 | 1,350–2,200 h total |
| Later, needs owner decisions | AI backend (#41), plug-in hosting (.8BF / UXP), scripting compatibility | 550–1,050 h |

## Milestone definitions

Each milestone has a **definition of done (DoD)** and must leave `main` green on all Tier-1 platforms plus a wasm build.

| M | Name | Scope (key items) | DoD / acceptance |
|---|---|---|---|
| **M0** | Skeleton | Workspace, all crate stubs, lints, `xtask` (layers check, ci), CI matrix, testkit | `cargo test --workspace` green; `cargo check --target wasm32-unknown-unknown -p photocraft-engine -p photocraft-ui-egui` green; layering check passes |
| **M1** | Foundation | geom, color (formats, blend math for all 27 modes, sRGB/linear), raster (sparse COW tiles, U8/U16/F32), doc (full type model incl. CMYK/Lab/adjust/smart), ops (history) | Property tests (proptest) on tile COW and history; blend-mode reference tests against published formulas |
| **M2** | PSD v1 | `photocraft-psd`: header, resources, layer records, channel data (raw/RLE/ZIP/ZIP+pred), masks, groups (lsct), unicode names, unknown-block passthrough, merged image, PSB; writer | Round-trip byte-stability tests; synthetic PSD generator tests; fuzz target; ≥150 unit tests |
| **M3** | Viewer app | codecs (png/jpeg/tiff/webp/gif/bmp/tga/pnm/qoi/exr/hdr, all read+write), CPU compositor, egui shell (menu from registry, canvas pan/zoom, layers panel, history), open/save, native + web build | Opens PNG/JPEG/PSD, shows layers, toggles visibility, undo/redo, saves PNG/PSD; web build loads a file from the browser |
| **M4** | Native format + engine | `.pcraft` bundle, command registry with schemas, jobs/cancellation, snapshots via arc-swap, CLI (`convert`, `run`, `inspect`) | CLI parity tests: every GUI command also runs headless |
| **M5** | GPU compositor | wgpu planner backend, tile residency, parity tests vs CPU (all blend modes), viewport on GPU, mips | GPU vs CPU ≤1/255; pan/zoom 60 fps on a 100 MP / 20-layer document |
| **M6** | Paint + select | platform input (pen: macOS/Windows/Linux/web), brush engine v1, eraser, marquee/lasso/wand, move, free transform, crop | Brush latency <1 frame; selection-restricted paint tests |
| **M7** | Adjust + filters | 16 adjustment layers, first 30 filters, schema-generated dialogs + live preview, actions record/replay | Golden tests at 8/16/32f; action replay determinism |
| **M8** | PSD v2 | adjustment layers, fill layers, lfx2 effects (descriptor parser), text (EngineData) preserve+render, smart objects, vector masks | Composite-oracle pass rate ≥90% on corpus |
| **M9** | Text, vector, styles | parley text layers, shapes/pen/paths, all 10 layer effects on GPU | Visual goldens; PSD text round-trip |
| **M10** | Smart features | `ml` (ort native / ort-web on the web), Select Subject/Object/Sky, Remove BG, Remove tool, content-aware fill, healing, AI denoise, RAW develop | Quality benchmarks on a public dataset; timing budgets |
| **M11** | Automation + formats | MCP server, batch, scripting, remaining formats (JP2, DICOM, DPX…), C2PA | An agent completes 10 scripted edit tasks via MCP |
| **M12** | Pro parity | CMYK/Lab UI, print, HDR display/merge, Photomerge quality validation, timeline, layer comps, artboards, symmetry, neural filters | `xtask parity` ≥ 90% of Photoshop menu checklist |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-31 | major | Re-measure at 0.18.0: stage stays alpha, ready ~52%, beta 1,350–2,200 h; alpha-gate evidence, milestones, releases, focus and next steps refreshed; version-numbered next milestones (0.7–0.9) renamed, those versions have shipped |
| 2026-10-11 | minor | Linked adaptive Unsharp Mask performance and quality evidence; milestones, alpha gate and estimates unchanged |
| 2026-10-10 | minor | Photomerge is implemented; real-world quality validation remains open |
| 2026-10-10 | minor | Alpha gate table (core-workflow gate): passes, stage stays alpha |
| 2026-10-10 | major | Progress docs restructured: honest assessment and dated measurements moved to `target-app-parity.md`, Affinity notes to `file-format-parity.md`; new Current focus and estimates; `parity.md` renamed `parity-checklist.md` |
| 2026-10-08 | minor | Tools, Affinity import and performance measurements added |
| 2026-10-05 | major | Honest parity assessment added |
