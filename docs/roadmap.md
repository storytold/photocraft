# Roadmap

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-10 · **Change:** minor (Photomerge implementation status corrected; real-world quality validation remains open) · **Target:** Adobe Photoshop 2026

Forward-looking plan: milestones, the **Current focus** list, and what's next with estimates.
Where we stand today is in [`target-app-parity.md`](target-app-parity.md) (two numbers, by
dimension and area) and the ranked work list in [`gaps.md`](gaps.md); the one-page summary is
[`ROADMAP.md`](../ROADMAP.md). Measured numbers: [`scorecard.md`](scorecard.md)
(`cargo xtask scorecard`: performance budgets, corpus floors, per-area checklists, dead
preferences) and [`parity-checklist.md`](parity-checklist.md) (`cargo xtask parity`: menu wiring,
628/628, which says nothing about behaviour).

Status legend: ✅ done · 🟡 in progress · ⬜ not started. Hours are Opus 5.5 agent wall-clock hours
for one agent working sequentially (calibration in
[`target-app-parity.md`](target-app-parity.md#remaining-effort-and-calibration)).

**Stage: alpha** (ready for real work ~45%). Beta needs ~75% and reliable PSD: ~1,100–1,800 h.

## Alpha gate

The core workflows a typical Photoshop professional runs every day, checked end to end on the main
platform (macOS, 0.6.0), including saving and reopening the work. Any **no**, or a **partial**
that blocks the workflow, would make PhotoCraft pre-alpha.

| Core workflow | Works end to end? | Evidence | Hours to pass |
|---|---|---|---:|
| Photo retouch: open JPEG/raw, heal/clone/remove, adjustment layers, export JPEG, save and reopen | yes | Spot Healing, Healing, Clone, Patch, Remove tools; 16 adjustment layers; raw for DNG/CR2/NEF/ARW/RAF; JPEG export; MCP workflow tests (`automation/tests/agent_tasks.rs`). Quality complaints (#1092, #2104) and no CR3 don't stop the workflow | 0 |
| Composite: layers, masks, selections, blend modes, smart objects; save as PSD and `.pcraft` and reopen | yes | PSD round trip ≥ 169/170, 308/309, 258/256 corpus files; `.pcraft` atomic save and crash recovery. Multi-layer Smart Object conversion (#2604) and mask targeting (#2458) are partial but have workarounds | 0 |
| Cut-out: select a subject, refine the edge, mask, export PNG with transparency | partial (not blocking) | Object/Quick Selection, Select Subject (classical), Refine Edge dialog, layer masks, PNG export. Select and Mask is a dialog, not the workspace; no Refine Hair (#2671) | 0 (depth: 80–140 h, G7) |
| Design: type, shapes, layer styles, artboards; export PNG/JPEG; save and reopen | partial (not blocking) | Point/paragraph type, shapes with stroke controls, all 10 layer effects, artboards, Export As. One macOS report of a 60 s freeze on choosing the Type tool (#2518, not reproduced); no type on a path | 0 (fix #2518: 2–6 h) |
| Paint with a pen tablet: pressure brushes, layers, save and reopen | partial (not blocking) | Pressure, tilt, rotation on macOS (AppKit monitor); every Brush Settings section. Lag with big brushes (#2619) and feel gaps ([brush-parity.md](brush-parity.md)) | 0 (feel: 100–180 h, G4) |
| Exchange PSDs with Photoshop users: open a client PSD, edit, send a PSD back | partial (not blocking for alpha; blocking for beta) | Photoshop-authored oracle 133/256 render as Photoshop does; most exports open in Photoshop (smart objects verified), but some don't (#1281, #2469) | 0 for alpha; 150–250 h for beta (G1) |

**Result: passes.** Every core workflow completes end to end and the work saves and reopens in
PhotoCraft; the partial rows are depth and fidelity gaps with workarounds, not blockers. Ready for
real work is ~45%, inside the alpha band (~40–75%), but close to its floor.

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
| M8 PSD v2 | 🟡 | adjustments (incl. Selective Color, Color Lookup), fills, effects, patterns, text, shapes, smart objects, alpha channels; oracle floors 146/170 (io), 236/309 (psd-tools), 133/256 (Photoshop-authored) |
| M9 Text, vector, styles | 🟡 | type engine + Warp Text, shapes / pen / paths, all 10 effects on CPU and GPU (parity ≤1/255, 30/31 corpus effect files on the GPU) |
| M10 Smart features | 🟡 | classical Select Subject / Object, content-aware fill and scale, healing, auto-align / auto-blend; ML backend not started |
| M11 Automation + formats | 🟡 | MCP (headless + live bridge), batch, Image Processor, prefs over MCP; DoD test passes (10 agent tasks over MCP, `automation/tests/agent_tasks.rs`); since 2026-10-07 the CLI rejects unknown flags and answers `<subcommand> --help`, batch runs report same-name outputs instead of overwriting them, MCP `command_batch` steps can start background jobs, and headless saves never flatten over the opened file; JP2 / DICOM / DPX / C2PA pending |
| M12 Pro parity | 🟡 | CMYK / Lab / Indexed / Bitmap / Duotone, ICC + soft proofing, channels + Quick Mask, smart-object stack modes, artboards, layer comps; print, HDR, timeline pending; Photomerge is implemented, with real-world quality validation still open |

Releases: 0.1.0 (2026-10-02), 0.2.0 (2026-10-05, first signed and notarized), 0.3.0 (10-07),
0.5.0 (10-08), 0.6.0 (10-10).

## Current focus

Ranked; each item links to its gap entry. Pick from the top unless an issue is assigned to you.

1. **PSD that Photoshop trusts** ([G1](gaps.md#g1-psd-fidelity-and-photoshop-acceptance)):
   fix Photoshop refusing our files (#1281, #2469; text-layer case #2374 closed); add a
   Photoshop re-open check (FILE-216-4); Photoshop-oracle floor 133/256 → 200+. 150–250 h.
2. **Performance budgets** ([G2](gaps.md#g2-interactive-performance)): the 22 over-budget
   scenarios, the P12 GPU crash at 150 layers, brush and cursor lag (#2619, #2566). 200–350 h.
3. **User bug backlog** ([G3](gaps.md#g3-user-reported-bug-backlog)): 579 open issues, most from
   0.5/0.6 users; triage weekly, fix regressions first. 250–400 h.
4. **Brush and transform feel** ([G4](gaps.md#g4-brush-feel), [G5](gaps.md#g5-transform-handles-and-modifiers)):
   [`brush-parity.md`](brush-parity.md) and [`ui-parity.md`](ui-parity.md) rows marked partial
   or missing. 150–250 h.
5. **Missing tools** ([G6](gaps.md#g6-missing-tools)): 15 of Photoshop's 68. 60–100 h.

## What's next, with estimates

| Next milestone | Scope | Estimate |
|---|---|---:|
| 0.7 "files you can trust" | Current focus 1 plus JPEG Options, AVIF read, Photoshop PDF read | 200–320 h |
| 0.8 "fast enough" | Current focus 2; GPU tiling past the texture limit (#49); complex layout documents | 200–350 h |
| 0.9 "feels like Photoshop" | Current focus 4 and 5; transform context menu; floating panels; Color panel | 300–500 h |
| Beta | All of the above, type depth, Select and Mask / Content-Aware Fill workspaces, Hindi/Arabic/Vietnamese + RTL, bug backlog under 100 | 1,100–1,800 h total |
| Later, needs owner decisions | AI backend (#41), plug-in hosting (.8BF / UXP), scripting compatibility | 550–1,050 h |
**Measured numbers live in the [scorecard](scorecard.md)** (`cargo xtask scorecard`, kept current
by CI): performance scenarios against their budgets, corpus floors, per-area checklists with
done / partial / missing counts, and the number of settings that do nothing. Where the scorecard
and the estimates below disagree, the scorecard's numbers supersede them. As of 2026-10-05 it
counts 67 of 129 preferences that nothing reads; 3 of the 25 budgeted performance scenarios meet
their #209–#211 targets (first baseline, taken on a heavily loaded machine), and the 150-layer
nudge scenario crashes the GPU compositor; the open area issues' checklists (#203–#220) are
almost entirely missing or partial.

2026-10-07: UI Font Size now applies to interface text, including CJK fallbacks, independently
of display and canvas zoom (#532). The current preference audit drops from 59 to 58 unread
settings out of 135; see the regenerated scorecard.

2026-10-09: Output-preserving Radial Blur direct sampling measured on a local
i7-9750H, 12 workers, 24 MP RGBA8, Good quality, amount 1: Spin 66.10 → 16.43 s
(4.02×), Zoom 19.19 → 5.35 s (3.59×), one paired run each, exact output equality.
These are local measurements at amount 1; see [method and limits](radial-blur-performance.md).

2026-10-10: native selection distance transforms measured 7.59–10.57x faster on six
24–36 MP synthetic masks on an AWS c7i.4xlarge (16 workers, three paired release runs).
The transform also corrects f32 envelope errors past coordinate 4096, including nonzero
distances at selected pixels. The parallel path needs an additional four bytes per pixel;
see [measurements, correctness and limits](selection-distance-performance.md).

**Bottom line.** Two days after 0.2.0 we had merged ~96 PRs and closed ~48 issues, but **real
Photoshop parity is still well below 50%**. The biggest gaps are AI, missing tools, professional
workflow depth and the plug-in ecosystem. Most fixes since 0.2.0 have passed our tests but have
**not yet been validated by users**, and this week proved our tests miss what users hit:
on first public use they found broken basics (text selection offset, 214 shortcut failures, panels
resizing themselves, an immovable crop frame, folders that wouldn't collapse, lag on layout PSDs)
that all counted as "live" in `parity.md`.

**Overall.** Feature surface: roughly **60–70%** of what a typical Photoshop user touches exists in
some form. "A professional could switch for daily work": roughly **25–35%**. True 1:1 parity is
**many months** of focused work, and some areas need product decisions rather than effort.
Confidence: moderate — the next users of 0.2.x will move these numbers either way.

### By dimension

2026-10-10: [Shape stroke controls](shape-strokes.md) expose existing Solid/Dashed/Dotted,
custom dash/gap, offset, caps, joins, alignment, miter-limit and opacity capabilities in
Shape/Pen options and Properties. Properties stages a cached canvas preview before one
undoable edit. Gradient fill controls from #1053 remain open; the Line tool still draws a
filled bar. This extends UI reachability and does not claim Photoshop-authored stroke parity.

| Dimension | Measured / evidence (2026-10-05) | Grade | Notes |
|---|---|---|---|
| Menu wiring | 626/626 menu items dispatch a command (`parity.md`) | high but shallow | Says nothing about behaviour. |
| PSD fidelity (rendering) | Corpus oracle 115/170 (68%): 30 differ, 26 have no usable reference, 1 import error. 2026-10-07: io corpus 146/170, psd-tools corpus 236/309 (was 229: Advanced Blending knockouts), Photoshop oracles 132/258 | medium | Push to 170/170 under way (effects/strokes, multi-instance effects, 16/32-bit and colour modes, references for skipped files). |
| PSD round trip | 169/169 re-import identically; every adjustment layer and blend mode round-trips | high (within corpus) | Floors in `crates/io/tests/corpus.rs`; raise, never lower. Corpora: `cargo xtask corpus --all` (ours: https://github.com/storytold/photocraft-corpus). |
| OpenRaster read/write | 2026-10-09: 34 Krita 5.2.9-authored `.ora` files (27 blend modes, groups, pass-through, offsets, opacity, visibility, masks, 16-bit, gray) render within 3/255 of Krita's merged image (30) or differ for known reasons (4: Krita's lighter/darker-colour tie-break, Hard Mix at exactly 1, a linear-light 16-bit document); Krita re-opens our exports and renders them identically to its originals (33/34; Soft Light is written as `svg:soft-light`) | medium (one authoring app) | MyPaint and GIMP files untested. Masks are applied to pixels and layer styles dropped on save (reported). See [OpenRaster](ora.md). |
| Paint.NET import | 2026-10-09: 14/14 local PDN3 textures preserve editable layers exactly through `.pcraft` and render within 2.142/255 of full-size previews; a supplied 5.x document (1280×720, 11 layers) passes editing/history/native saves and matches its thumbnail within 0.418/255 mean; all 14 blend modes exercised synthetically | medium (limited corpus) | Import only; metadata omitted. More current-version files and full-size exports needed. See [PDN support](pdn.md). |
| Smart filters / text / effect shapes in PSDs | Measured on our Photoshop-authored set (https://github.com/storytold/photocraft-corpus, `corpus/photoshop`, 258 files): see the per-group floors in `crates/io/tests/corpus.rs` and `crates/engine/tests/photoshop_oracles.rs`. Smart objects and smart filters now survive PSD save and open (41 corpus files, 206 smart objects, round trip strict; Photoshop opens our exports with live filters); re-rendering Photoshop's smart filters with ours matches 5/30 (was 1/30) | low–medium | Remaining re-render gaps are filter maths (Gaussian/Motion Blur, Unsharp Mask, Emboss, Add Noise RNG) and bicubic placement. |
| Core editing (layers, masks, selections, adjustments, filters, transforms) | Broad engine coverage; many interaction bugs fixed after 0.2.0 (adjustment dialogs, Curves, crop, Move/Transform modifiers, gesture origin) | medium | Fixes not yet user-validated. |
| UI / UX polish | Shortcut audit 214 → 0 failures; dock, Layers rows and menus reworked; first visual-QA sweep found 14 defects (#147–#157). 2026-10-07: the keyboard-only shortcuts with no menu item (⌥[ ⌥] ⌥, ⌥. layer navigation, ⇧⌥[ ⇧⌥] to extend the selection, 1–0 for opacity and ⇧ for flow or fill, ⇧[ ⇧] hardness, ⌥⌘T to transform a copy and ⌥⇧⌘T to step and repeat), ⌥-click colour sampling with painting tools, double-click a Layers row for Layer Style, File › New from Clipboard, a centred main window and remembered Liquify settings (#352, #417, #350, #368, #419, #418). 2026-10-09: canvas zoom is physical at any display scale — 100% is one document pixel per physical display pixel, and every screen-space overlay, cursor, scrollbar, navigator and fit follows it (#1943) | low–medium | Needs recurring visual QA with realistic documents. |
| Tools | 2026-10-09: Pencil, Mixer Brush, Patch, Content-Aware Move, Vertical Type, Pattern Stamp and Rotate View are toolbar tools. Remaining missing tools include Red Eye, Art History Brush, Freeform/Curvature Pen, Add/Delete Anchor Point tools, single row/column marquee, Color Sampler, Perspective Crop, type masks and Frame | low–medium | Patch has a live healing preview. Content-Aware Move is partial: Move/Extend, Structure/Color and Sample All Layers exist; Transform On Drop and a live result preview remain missing (`TOOL-213-4` in the scorecard). Magic/Background Eraser added; live gradients in progress (#180). Magnetic Lasso added 2026-10-08 (live-wire edge tracing, Width/Contrast/Frequency, `select.magneticLasso`). Pattern Stamp added 2026-10-08 (S flyout, `paint.patternStamp`, Aligned / Impressionist). Rotate View (2026-10-09) turns the canvas camera around its centre without rewriting pixels; Reset View and Match Rotation copy the angle. Crop rotation (2026-10-09, #1792): dragging outside the crop frame turns it (⇧ 15° steps, angle readout), and ↵ rotates the document and crops in one undo step (`image.crop` `angle`); the frame turns over the image as in Photoshop's Classic Mode, and the options-bar Straighten is still missing. |
| Painting | Brush model and Brush Settings panel near Photoshop; .abr/.grd import; persistent presets; pen pressure/tilt on Windows, web, macOS and X11 | medium | Native Wayland pen input open (#79; a pen opens the window through Xwayland meanwhile); X11 pressure confirmed by a user, macOS not yet verified on tablet hardware. |
| Text / typography | Engine works; caret placement and size editing fixed; OpenType features, text-on-path editing, composer parity partial | medium-low | Measure with the Photoshop-authored set. |
| Colour management | Colour-managed canvas (document → monitor), embedded CMYK profiles, linear EXR/HDR, 16-bit float canvas | medium-high | Monitor profile follows only at launch. |
| Performance | 14k+ px on the GPU at ~⅓ the memory; adjustment preview 285 ms → 4–9 ms; font-size edits 297 ms → 4.6 ms; 2026-10-07: 30 MP TIFF open (banded, parallel strip/tile decode) Deflate 345 → 32 ms, LZW 428 → 43 ms, BigTIFF and every IFD readable; 2026-10-09: 4.2 MP Indexed Color, 256 colours, full-resolution CPU preview p50 3018 → 1061 ms (Ryzen AI 7 350, three measured runs; GPU upload excluded); 2026-10-10: Liquify 3500×2200, 800 px brush, visible 1:1 crop 3.52 → 4.03 ms p50 while refreshing ~9× as many pixels | medium-high on rasters | Complex layout documents still laggy (#125/#128); >16384 px GPU tiling in progress (#49). Liquify now displays source-resolution detail above 100% zoom; its ≤1 ms update target is not met (4.11 ms p95 on this Windows run; GPU upload excluded). Native Indexed Color previews run on one background worker with stale-result rejection; large palettes can still take about a second to compute. Web previews remain synchronous. |
| Stability | Never-crash lint series, crash guard, `panic_hunt` fuzzing in the gate | medium-high | No field crash data yet. |
| Camera RAW | DNG, CR2, Sony ARW (lossless + compressed), Nikon NEF (lossless + lossy compressed), RW2, uncompressed ORF; 2026-10-09: uncompressed Fujifilm RAF, Bayer and X-Trans (edge-directed X-Trans demosaic; 26 MP X-Trans develop 306 ms on 12 cores) | medium | CR3, compressed RAF, Nikon "lossy after split" and calibrated colour for non-DNG cameras (no Fuji colour calibration: neutral fallback) still open (#50). |
| AI / generative | none | ~0% | Deferred by decision (#41). |
| Ecosystem | Sandboxed WebAssembly plug-ins instead of .8BF; no ExtendScript/UXP/.atn; no Adobe Fonts/Libraries/cloud docs | low | By design for 8BF; scripting compatibility open. |
| Platforms | macOS (notarized), Windows, Linux (AppImage/deb/rpm/Flatpak bundle), web | medium-high | Flathub later (#173); Windows signing material pending. |
| Localisation | 2026-10-07: 10 UI languages; menu, `tl!`, blend mode, preference and brush-section coverage enforced by tests; live switching and scoped Preferences previews | medium | Engine errors/status messages still partly English; CJK web fonts, browser-locale detection, and RTL remain open. |

2026-10-08: Motion Blur adds adaptive FFT convolution for wide streaks while retaining the
row kernel merged in [#902](https://github.com/storytold/photocraft/pull/902) for shorter
streaks and unsupported cases. Native release medians of three paired runs on an Intel
i7-9750H with 12 Rayon workers, synthetic RGBA8 at 30° / distance 1000: 24 MP
22.594 → 9.785 s (2.31×), and 1.5 MP
2.455 → 0.592 s (4.15×).
Full-image comparisons differ by at most one U8 code level; distance 64/256 retain the
byte-identical row output. These measure decode/halo/filter/write/prune, excluding UI proxy
creation, compositing and upload; the UI remains synchronous. FFT work is scheduled within
a conservative 512 MiB working-set estimate, excluding stored Surface tiles and row fallback
memory. The crossover is a heuristic; images dominated by near-cutoff alpha can need extra
scalar work. These measurements do not establish Photoshop filter parity.

2026-10-10: Proximity Match search medians on an AWS c7i.4xlarge improved 1.54–3.82x
for four synthetic stroke regions (50–300 px holes), with identical source displacements.
Bounding the mask table to the hole and pruning losing SSD candidates reduced table storage
by about 90–93 percent there. Full 24–36 MP kernel inputs improved 4.60–6.24x, but normal
engine strokes already use a cropped region. See [method and limits](proximity-match-performance.md).

2026-10-09: the Remove tool is in the J flyout (`paint.remove`, a background job). A stroke
around an object also removes what it encloses. The fill is non-local patch completion with
texture features (A. Newson et al., IPOL 2017: `crates/algo/src/nonlocal.rs`), finished by a
best-patch copy and gradient-domain seam hiding. On 20 holes in public-domain photos
(`remove_quality`, `docs/development.md` › Remove Tool quality) the fill keeps 0.89 of the
original's texture (Wexler/PatchMatch completion as Content-Aware Fill uses it: 0.54). On a 24 MP
document a 100 px ring takes 120 ms and a 1000 px ring 2.5 s (`remove_bench`). It is not
generative: large objects over complex structure fill less convincingly than Photoshop's AI mode
(#41).

2026-10-08: the tools update above is checked against the current toolbar groups in
[`panels.rs`](../crates/ui-egui/src/panels.rs), the Pencil and tool-cycle tests in
[`pencil_tests.rs`](../crates/ui-egui/src/pencil_tests.rs), Patch's live-preview test in
[`patch_preview.rs`](../crates/ui-egui/src/patch_preview.rs), and Vertical Type's point/paragraph
creation and undo test in [`type_tool_tests.rs`](../crates/ui-egui/src/type_tool_tests.rs).
These implementations remove the tools from the missing list; they do not establish Photoshop
behavioural parity. The historical estimates and corpus measurements in this assessment are
unchanged; current measured floors remain in the scorecard.

2026-10-07: Camera Raw PSD mapping covers relative custom white balance, Light/Presence,
parametric and four point curves, HSL, Color Grading, sharpening/noise detail, grain and numeric
post-crop vignette controls. Two revisions of one supplied Photoshop ACR 18.4 PSD preserve their
filter descriptors without edits. The updated revision stays opaque because of active manual
Optics and unverified vignette style; a descriptor projection verifies its supported controls.
Synthetic 8/16/32-bit PSD → edit → `.pcraft` → PSD tests preserve the filter stack, mask and
undo/redo. Unmapped Camera Raw fields/versions remain opaque; Photoshop
acceptance of generated exports and pixel parity are still unverified. Corpus floors above are
unchanged.

2026-10-10: Select and Mask's Shift Edge now computes the exact octagonal footprint
with a square window extreme plus a dyadically built Manhattan ball. On a c7i.4xlarge
(16 vCPUs), full tiled CPU refinement at radius 64 / Shift Edge +100% takes
3.019 s -> 0.762 s at 24 MP (3.96x),
3.706 s -> 0.921 s at 36 MP (4.02x).
Three alternating paired release runs include guide sampling, guided filtering,
boundary distances, quantization and assembly. Every output mask matches exactly.
The dense fractional 24 MP radius-64 dilation kernel improves 36.00x;
circular soft-mask kernels improve 20.76-172.16x at 24-36 MP, radii 4-64.
Bounded halo tiles reduce the large-mask workspace allocation bound from 192/288 MB
to about 115/163 MB, including output and 16 workers' scratch, excluding the source.
See [method and results](octagonal-mask-performance.md).

### Where we're going (priority order)

1. **Ship the fixes:** cut 0.2.1 once the current batch lands, so users validate them.
2. **PSD fidelity to 170/170** with round trips, plus the Photoshop-authored reference set for smart
   filters, the text engine and effect shapes (https://github.com/storytold/photocraft-corpus, fetched by
   `cargo xtask corpus --all`); enforce floors in `corpus.rs`.
3. **Workflow acceptance tests:** 30–50 real tasks (retouch a portrait, social post with text and
   effects, composite with masks and adjustment layers, CMYK print prep…) scripted end to end and
   checked against Photoshop's output on every build. Their pass rate becomes the headline parity
   number.
4. **Tool coverage and workflow depth:** Pen variants (Direct Selection landed, #790) and
   Perspective Crop; finish Content-Aware Move's Transform On Drop and live result preview.
   Pencil, Mixer Brush, Patch, Vertical Type, Pattern Stamp and Rotate View are already available on the toolbar.
5. **Complex-document performance** (#125/#128) and GPU tiling beyond the texture limit (#49).
6. **Recurring visual QA** (`cargo run -p photocraft-engine --example designer_psd`) and fast
   turnaround on user reports (OS, document size, layer count, screenshot).
7. Later / needs decisions: generative AI backend (#41), scripting compatibility (ExtendScript /
   UXP / .atn), Flathub (#173), Wayland pen pressure (#79).

## Current focus (infrastructure before the long tail)

Landed on 2026-10-01:
- multi-layer selection, live smart objects + smart filters, alpha channels + Quick Mask;
- patterns, Warp, preferences, snapping, ~33 filters, the remaining core adjustments;
- Layer Comps and Artboards, GPU layer effects, Liquify / Puppet Warp / Perspective Warp;
- the PSD fidelity pass (oracle 102 → 111), the M11 MCP acceptance test, and the release pipeline
  (`docs/releasing.md`).

Next:
1. **Follow "Where we're going" in the assessment above.** 0.2.0 shipped signed and notarized on
   2026-10-05 (`docs/releasing.md`); Windows code-signing material still needs to be obtained.
2. **Fidelity**: the PSD oracle (113/170). Modern Brightness/Contrast and grayscale Levels
   curves; chisel-soft / stroke-emboss bevel shapes; Photoshop's 8-bit blend rounding; non-Normal
   modes in Lab documents; smart-filter maths against Photoshop's (re-render oracle 5/30).
3. **GPU**: only Multichannel documents and regions over the texture limit fall back to the CPU.
4. **Vanishing Point, Camera Raw / Lens Correction, Face-Aware Liquify** (needs a landmark model).
5. **Panels**: Patterns, Styles, Glyphs, Character/Paragraph Styles, Timeline; Custom Shape tool.
6. Print, Photomerge, Merge to HDR, video layers.
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
| 2026-10-10 | minor | Photomerge is implemented; real-world quality validation remains open |
| 2026-10-10 | minor | Alpha gate table (core-workflow gate from the progress-docs standard): passes, stage stays alpha |
| 2026-10-10 | major | Progress-docs standard: honest assessment and dated measurements moved to `target-app-parity.md`, Affinity notes to `file-format-parity.md`; new Current focus and estimates; `parity.md` renamed `parity-checklist.md` |
| 2026-10-08 | minor | Tools, Affinity import and performance measurements added |
| 2026-10-05 | major | Honest parity assessment added |
