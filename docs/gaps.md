# Where PhotoCraft falls short of Photoshop

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-10 · **Change:** major (first version: every known shortfall ranked, from the 2026-10-10 re-measure) · **Target:** Adobe Photoshop 2026

Every known shortfall against Photoshop 2026 (27.11.0), one entry each, ranked by user impact
on the way to beta. This is the work list agents pick from: take the highest-ranked entry that
nobody is working on, and when you close part of one, update its evidence and estimate here and
the matching parity doc. Numbers come from [`target-app-parity.md`](target-app-parity.md);
checklist ids (`TOOL-…`, `FILE-…`, `UI-…`) are rows in [`scorecard.md`](scorecard.md); `#n` are
GitHub issues.

No gap blocks alpha: the [alpha gate](roadmap.md#alpha-gate) passes. "Blocks beta" marks what
stands between alpha and beta.

Hours are Opus 5.5 agent-hours, one agent working sequentially, including tests. **Human** marks
what an agent can't finish alone.

## Summary

| # | Gap | Kind | Impact | Estimate | Parity doc |
|---|---|---|---|---:|---|
| G1 | PSD fidelity and Photoshop acceptance | file format | blocks beta | 150–250 h | [file-format-parity](file-format-parity.md) |
| G2 | Interactive performance | performance | blocks beta | 200–350 h | [scorecard](scorecard.md#performance) |
| G3 | User-reported bug backlog | stability, UI | blocks beta | 250–400 h | [target-app-parity](target-app-parity.md) |
| G4 | Brush feel | UI / painting | high | 100–180 h | [brush-parity](brush-parity.md) |
| G5 | Transform, handles and modifiers | UI | high | 60–110 h | [ui-parity](ui-parity.md) |
| G6 | Missing tools (14) | features | high | 60–100 h | [ui-parity](ui-parity.md#tools) |
| G7 | Workspaces shown as parameter dialogs | features | high | 80–140 h | [target-app-parity](target-app-parity.md#feature-areas) |
| G8 | Type depth | features | high | 120–200 h | [target-app-parity](target-app-parity.md#feature-areas) |
| G9 | Panels, preferences and workspace | UI | medium-high | 150–250 h | [ui-parity](ui-parity.md#panels-and-workspace) |
| G10 | Missing and partial file formats | file format | medium | 120–220 h | [file-format-parity](file-format-parity.md) |
| G11 | Actions and scripting | ecosystem | medium | 120–200 h | [target-app-parity](target-app-parity.md) |
| G12 | Pen tablets and display hardware | hardware | medium | 80–150 h | [hardware-parity](hardware-parity.md) |
| G13 | Languages: Hindi, Arabic, Vietnamese, RTL, engine messages | localization | medium | 80–140 h + human | [localization-parity](localization-parity.md) |
| G14 | Camera Raw depth | features, formats | medium | 120–200 h | [file-format-parity](file-format-parity.md#camera-raw) |
| G15 | AI and generative features | AI | medium (high for some users) | 300–600 h + human | [target-app-parity](target-app-parity.md) |
| G16 | Plug-in compatibility (.8BF, UXP) | ecosystem | medium | 250–450 h + human | [target-app-parity](target-app-parity.md) |
| G17 | Platforms and distribution | platforms | low-medium | 30–60 h + human | [scorecard](scorecard.md#distribution) |

## Feature gaps

### G1. PSD fidelity and Photoshop acceptance
- **Missing:** PSDs we write are not always accepted by Photoshop, and Photoshop-authored files
  don't always render as Photoshop renders them.
- **Evidence:** corpus floors (`crates/io/tests/corpus.rs`, scorecard): Photoshop-authored
  oracles 133/256 (52%), io corpus 146/170 (86%), psd-tools 236/309 (76%); round trips ~100%
  within the corpus. Smart-filter re-render matches 5/30. User reports: a PSD with text layers
  re-saved unmodified would not reopen (#2374, closed 2026-10-10), Photoshop 26.6 cannot reopen our files (#1281), "PSD not
  opening in Photoshop" (#2469), a CS2-era smart object unavailable (#2465), PSB smart object
  blank (#1764), perspective placed layers flattened (#579), PSB > 4 GB fails on Windows (#375).
  No test opens our exports in a third-party editor (FILE-216-4).
- **Impact:** PSD is the format professionals exchange. Beta requires it to be reliable.
- **Estimate:** 150–250 h. **Human:** opening exports in Photoshop on the owner's Mac (or a
  scripted check there) is the only real oracle.

### G6. Missing tools
- **Missing (14 of 68):** Single Row Marquee, Single Column Marquee, Perspective Crop,
  Frame, Color Sampler (#1046), Color Replacement (engine command exists), Art History Brush,
  Freeform Pen and Curvature Pen (#1054), Add Anchor Point, Delete Anchor Point, Convert Point
  (only as Pen + ⌥), Horizontal Type Mask, Vertical Type Mask.
- **Evidence:** `Tool` enum (`crates/ui-egui/src/state.rs`) against the 67 `<tool>` entries of
  Photoshop's `Default Keyboard Shortcuts.kys` plus Remove, Triangle and Curvature Pen.
- **Impact:** users look for them by name and shortcut; several are daily tools for designers.
- **Estimate:** 60–100 h (2–9 h each, per the calibration).

### G7. Workspaces shown as parameter dialogs
- **Missing:** Select and Mask workspace with view modes, Refine Edge brush and outputs
  (TOOL-214-1, Refine Hair #2671); Content-Aware Fill workspace with a paintable sampling area
  (TOOL-214-15); Vanishing Point plane editor (TOOL-214-14); Content-Aware Scale through the
  transform box (TOOL-214-7); Puppet Warp mesh following the outline (TOOL-214-6); Content-Aware
  Move's Transform On Drop (TOOL-213-4).
- **Evidence:** scorecard rows above, all partial or missing.
- **Impact:** these are the tools retouchers and compositors use daily; a slider dialog is not a
  substitute.
- **Estimate:** 80–140 h.

### G8. Type depth
- **Missing:** type on a path (TYPE-218-1), paragraph direction UI (TYPE-218-2), complex-script
  fallback fonts and golden tests (TYPE-218-3), variable-font axes (TYPE-218-4), OpenType panel
  UI (TYPE-218-5), manual and optical kerning with PSD round trip (TYPE-218-6, #206), live Warp
  Text preview (TYPE-218-7, #2689), triple/quadruple click (TYPE-218-8), "rasterize and continue"
  for distort/perspective on type (TYPE-218-9), text free transform (#2630).
- **Evidence:** scorecard Type: 0 done, 3 partial, 6 missing. The Type tool froze the app for
  60 s on one macOS machine (#2518).
- **Estimate:** 120–200 h.

### G11. Actions and scripting
- **Missing:** document-independent action recording (AUTO-219-1), a full Actions panel with
  step toggles and .atn import (AUTO-219-2), replay tests (AUTO-219-4), Batch recursion and
  naming (AUTO-219-5), ExtendScript/UXP compatibility.
- **Ahead of Photoshop:** CLI, MCP and the JSON control channel drive every command.
- **Estimate:** 120–200 h. **Human:** whether to support a JavaScript scripting dialect at all
  (AGENTS.md forbids JS in the app).

### G14. Camera Raw depth
- **Missing:** CR3 (detected, unsupported), compressed RAF, lossy DNG (FILE-215-11), .xmp
  sidecars, masking and calibration in the Camera Raw dialog, per-camera colour for non-DNG
  cameras, lens profiles.
- **Evidence:** `crates/raw/src/lib.rs:219` (CR3), scorecard FILE-215-11/12, #50.
- **Estimate:** 120–200 h.

### G15. AI and generative features
- **Missing:** Generative Fill and Expand, Generative Upscale, Neural Filters, Sky Replacement,
  Super Zoom, ML Select Subject/Sky. There is no ML runtime (AUTO-219-7). Classical substitutes
  exist (GrabCut/max-flow selection, Remove tool by non-local patch completion).
- **Evidence:** Photoshop's `Required/UXP` ships the Neural Filters gallery and its `.kys` has
  a Neural Filters taskspace; users ask for local models (#2313, #1996, #2682).
- **Estimate:** 300–600 h. **Human:** owner decision #41 (model licences, local vs remote).

### G16. Plug-in compatibility
- **Missing:** hosting Photoshop .8BF filters (#2647), UXP panels, CEP. PhotoCraft has its own
  sandboxed WebAssembly plug-in ABI v1 (`docs/plugins.md`), but ABI v2 (panels, progress, other
  layers) is missing (AUTO-219-6).
- **Estimate:** 250–450 h. **Human:** .8BF needs native code under the isolated-`unsafe` rule.

## UI / UX gaps

### G3. User-reported bug backlog
- **Missing:** fixes for 579 open issues (2026-10-10), filed mostly by 0.5/0.6 users. By title:
  ~150 UI/panels/menus/shortcuts, 94 file/format, 87 layers, 54 painting, 54 selection,
  46 type, 41 performance, 34 transform/move, 25 Linux/Wayland, 24 Windows, 20 macOS.
- **Impact:** each is a place where a Photoshop user hits something different from what they
  expect; together they are why ready-for-real-work is ~45% while breadth is ~76%.
- **Estimate:** 250–400 h (many overlap with other gaps; count each fix once).

### G4. Brush feel
- **Missing:** lag with big brushes (#2619, #1994) and under the cursor (#2566, #1343); jumpy
  pressure on Windows (#2046) and tap sensitivity (#1933); stair-stepped line weights (#1776),
  interpolation and smoothing problems (#2138), edge falloff (#2149), pixelated cursor vs
  Photoshop (#1433), brush rotation not driven by pen rotation (#1301); live preview only for
  Brush and Eraser (TOOL-213-7); no image file as a brush tip (TOOL-213-12); per-control pressure
  curves (TOOL-213-11); 4 of Photoshop's symmetry modes.
- **Detail:** [`brush-parity.md`](brush-parity.md).
- **Estimate:** 100–180 h. **Human:** checking feel on real tablets.

### G5. Transform, handles and modifiers
- **Missing:** right-click menu during Free Transform (TOOL-214-5); commit/cancel buttons in the
  options bar (#2442); H/V skew fields and the full interpolation list; transform keeps layer
  order and blend mode in its preview (#2439); Hand tool interrupting a transform (#2302);
  blend mode/opacity edits during a transform (#1384); locked layers refused before the drag
  (#2585); ⌘-drag temporary Move without box or snapping (#2681), Auto-Select box follows the
  previous layer (#2666); ⌘-arrow layer nudge (#2474); path-point nudge; polygonal lasso Shift
  angles (#2352); autoscroll at the edge while selecting (#2231); hold-a-key spring-loaded tools
  (#2672); double-click Zoom tool to 100% (#2508); wheel and pinch behaviour (#2485, #2463,
  #1989); toolbar press-and-hold flyouts (#2301); tool-key memory in a group (#2608).
- **Detail:** [`ui-parity.md`](ui-parity.md).
- **Estimate:** 60–110 h.

### G9. Panels, preferences and workspace
- **Missing:** 46 of 149 preferences do nothing (scorecard prefs audit); panel groups can't
  float or tear off into OS windows (UI-217-5, #1828, #1213, #2305); Color panel lacks wheel and
  slider modes (UI-217-17, #2569); UI scale presets (UI-217-9, #2492); more themes (UI-217-11,
  #2661); Stamp Visible (UI-217-22); layer clipboard
  (UI-217-23, #2670); tool-specific context menus beyond brushes, selections and Pen
  ([context-menu-parity.md](context-menu-parity.md)); Libraries, Comments, Contextual Task Bar
  panels.
- **Estimate:** 150–250 h.

### G2. Interactive performance
- **Missing:** 22 of 25 budgeted scenarios are over budget (baseline 2026-10-05, a heavily
  loaded machine): layer move with styles 321 ms p50 against 25 ms, opacity change 276 ms
  against 20 ms, paste 12 MP 1.2 s against 100 ms, layer move on 15000×10000 16-bit 2.7 s,
  Content-Aware Scale 24 MP 954 s against 3 s, Select Subject 559 ms against 100 ms; the
  150-layer nudge crashes the GPU compositor (P12); a 50 MP PSB with feathered masks redraws in
  ~10 s (#2377); menus slow with thousands of layers (#2460); old machines lag (#2687).
- **Estimate:** 200–350 h. Serial in places (compositor architecture).

## File-format gaps

### G10. Missing and partial file formats
- **Missing:** JPEG Options dialog (FILE-215-19), Photoshop PDF read and write and multi-page PDF
  (FILE-215-10, #2541), AVIF read (#463) and default-build AVIF write, JPEG XL, JPEG 2000 (#1047),
  DPX/Cineon (#1049), DICOM, EPS/DCS, PCX, Pixar, IFF, WBMP, MPO/JPS, PICT, Scitex CT, Photoshop
  Raw; layered TIFF extra alpha channels (FILE-215-7); spot plates (FILE-215-17); .atn, .asl,
  .shc, .csh presets (FILE-215-15); TGA/PNG alpha channels on open (#2225); C2PA (#1050).
- **Detail:** [`file-format-parity.md`](file-format-parity.md).
- **Estimate:** 120–220 h.

## Hardware gaps

### G12. Pen tablets and display hardware
- **Missing:** tilt and rotation on Windows (pressure only, via WM_POINTER; no WinTab); native
  Wayland tablet-v2 (#79; the pen does not click on GNOME 50, #2622, KDE #2297); eraser end on
  flipped Wacom pens (#2285); HDR/EDR output (Photoshop has "Precise Color Management for HDR
  Display"); monitor profile detection off macOS and live profile changes; Touch Bar (macOS
  Intel only, low value); 3D mice.
- **Detail:** [`hardware-parity.md`](hardware-parity.md).
- **Estimate:** 80–150 h. **Human:** tablets and HDR displays to test on.

## Localization gaps

### G13. Hindi, Arabic, Vietnamese, RTL and engine messages
- **Missing:** catalogs for Hindi, Arabic and Vietnamese (3 of the 12 key languages); right-to-
  left UI layout; Devanagari/Thai/Khmer fallback fonts; engine error and status messages in
  English in every language; browser-locale detection and CJK fonts on the web; Japanese IME on
  Windows (#590); installed Japanese fonts missing from the font list (#2330); Arabic layer names
  as tofu (#1408, #1693).
- **Detail:** [`localization-parity.md`](localization-parity.md).
- **Estimate:** 80–140 h. **Human:** native-speaker review of every catalog (none is recorded).

## Platform gaps

### G17. Platforms and distribution
- **Missing:** Windows code signing (material not obtained), Windows ARM64 (DIST-220-2), in-app
  update check (DIST-220-1, #2650, #2375), in-app bug report (DIST-220-3), Flathub (#173),
  starting without a GPU adapter (#2519, #2412), Windows 7 (#2310, out of scope), Intel Macs with
  OpenCore (#1859).
- **Estimate:** 30–60 h. **Human:** signing certificates.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-10 | minor | Checked against the alpha gate (no alpha blockers); #2374 marked closed |
| 2026-10-10 | major | First version, from the full re-measure against Photoshop 2026 27.11.0 and the open issues |
