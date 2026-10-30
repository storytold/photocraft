# Backlog triage — storytold/photocraft

> Generated 2026-10-11 02:12 UTC by `scripts/pr-triage.py`. Do not edit by hand: regenerate with
> `scripts/pr-triage.py -o docs/backlog-triage.md`.
>
> A **point-in-time snapshot**: the backlog moves by dozens of items a day, and
> unlike `docs/scorecard.md` this is deliberately *not* enforced by CI — a
> freshness check here would fail constantly for no benefit. Regenerate it when
> you want a current view. `docs/backlog-shortcuts.md` is the hand-written
> companion and does not go stale.

## Headline

| | Count |
|---|---:|
| Open items | 942 |
| Open issues | 647 |
| Open PRs | 295 |
| PRs mergeable **and** green (merge now) | **26** (8%) |
| PRs green but conflicted (auto-rebase) | 32 |
| PRs with failing CI | 92 |
| PRs with no CI results at all | 114 |
| PRs with merge conflicts (`dirty`) | 107 (36%) |
| PRs still dirty with failing/absent CI | 31 |
| Draft PRs | 14 |
| Items with **no labels** | 929 (98%) |

> **114 PRs have no CI results**, 16 of them open for two days or more. Most are brand-new PRs whose checks have not reported yet; the older ones are usually forks with Actions disabled and need a push or a rebase to trigger a run.

The bottleneck is not writing fixes, it is *de-rotting and reviewing* the
queue. Work the lists in the order below: merge-now first (zero cost), then
auto-rebase (one command each), and only then look at failing CI.

## 1. Merge now — clean and green

These need no code and no rebase: `gh pr merge N` is a no-brainer for each.
Sorted smallest first, because merging a small PR often clears the conflict
on a bigger one behind it.

- **#1149** `XS` 1 files, +13 — README: install on Gentoo from the ::snakebyte overlay  
  `readme-gentoo` · GREEN · clean · opened 2026-10-08
- **#1164** `XS` 1 files, +78 — Design: bounded GPU stack prefix reuse for property edits  
  `codex/design-gpu-composite-reuse` · GREEN · clean · opened 2026-10-08
- **#2596** `XS` 1 files, +122 — Speed up large Content-Aware Scale operations  
  `codex/perf-211-content-aware-scale` · GREEN · clean · opened 2026-10-10
- **#2667** `XS` 1 files, +83 — raster: a shifted surface copies in one pass per row without pre-filled tiles  
  `perf/p14-large-move` · GREEN · clean · opened 2026-10-10
- **#2805** `XS` 1 files, +220 — Spot Healing: Content-Aware dab no longer scans every patch in full (#2771)  
  `perf/2771-content-aware-spot-heal` · GREEN · clean · opened 2026-10-10
- **#2417** `XS` 2 files, +173 — Speed up large feathered mask refreshes  
  `codex/fix-2377-mask-cache` · GREEN · clean · opened 2026-10-10
- **#2809** `XS` 2 files, +145 — Free Transform: themed rotate cursor and live angle/size/offset readout (#1767)  
  `fix-1767` · GREEN · clean · opened 2026-10-10
- **#2821** `XS` 2 files, +197 — Text fields: drag-selecting in a single-line field follows the pointer's x (#1946)  
  `fix/1946-singleline-drag` · GREEN · clean · opened 2026-10-10
- **#2847** `XS` 2 files, +13 — Docs: claim an issue with an "I'm working on this." comment (#2418)  
  `docs/claim-comment` · GREEN · clean · opened 2026-10-10
- **#1590** `S` 3 files, +169 — fix(format): prevent directory bundle saves from following unsafe symlinks  
  `fix/pcraft-directory-symlink-containment` · GREEN · clean · opened 2026-10-09
- **#2644** `S` 3 files, +61 — [macOS] Use CoreText for system font discovery  
  `codex/fix-2518-font-scan` · GREEN · clean · opened 2026-10-10
- **#2791** `S` 3 files, +193 — Unify the canvas-edge dividers  
  `ui-fixes` · GREEN · clean · opened 2026-10-10
- **#2870** `S` 3 files, +96 — perf: repeated Select Subject answers from the last mask (#2865)  
  `perf/2865-select-subject-repeat` · GREEN · clean · opened 2026-10-10
- **#2882** `S` 3 files, +115 — perf: Select › Modify › Feather/Smooth on 24 MP within budget (#2873)  
  `perf/2873-select-modify-feather-smooth` · GREEN · clean · opened 2026-10-10
- **#2065** `S` 4 files, +795 — Auto-Align Layers: register focus and exposure brackets by intensity when there is nothing to match  
  `feat/direct-registration` · GREEN · clean · opened 2026-10-09
- **#2652** `S` 4 files, +79 — Options bars: Cancel and Commit follow the options instead of sitting at the far right  
  `feat/option-bar-buttons` · GREEN · clean · opened 2026-10-10
- **#2754** `S` 4 files, +193 — PSD: parallel RLE for the merged image; layer channels decode and gather on all cores  
  `perf/psd-36mp` · GREEN · clean · opened 2026-10-10
- **#2044** `S` 5 files, +1428 — Auto-Blend Stack Images: fuse the merged layer per pyramid level by region energy, with halo control  
  `feat/stack-region-energy-fusion` · GREEN · clean · opened 2026-10-09
- **#2803** `S` 5 files, +299 — View: Pattern Preview tiles the document around the canvas (#1067)  
  `fix/1067-pattern-preview` · GREEN · clean · opened 2026-10-10
- **#2073** `S` 6 files, +2528 — Auto-Blend Stack Images: a depth map by depth from focus, for Lens Blur  
  `feat/depth-from-focus` · GREEN · clean · opened 2026-10-09
- **#2876** `S` 6 files, +157 — Status bar: buttons sit inside the bar with centred labels (#2823)  
  `fix/statusbar-buttons` · GREEN · clean · opened 2026-10-10
- **#2820** `M` 7 files, +213 — Layers: double-click a shape layer's thumbnail to pick its fill colour (#2812)  
  `fix/2812-shape-thumbnail-color` · GREEN · clean · opened 2026-10-10
- **#2263** `M` 9 files, +749 — Web: an embedding API (window.photocraft, postMessage, Photopea-compatible mode)  
  `embed-api` · GREEN · clean · opened 2026-10-10
- **#2811** `L` 17 files, +240 — select.cryptomatte: click an object, select its coverage  
  `feat/cryptomatte-select` · GREEN · clean · opened 2026-10-10
- **#2830** `L` 22 files, +291 — Options bar: Reset Tool and Reset All Tools (#2825)  
  `feat/2825-reset-tools` · GREEN · clean · opened 2026-10-10
- **#1171** `XL` 54 files, +3912 — Color Lookup: install LUT packs once, browse them in one list  
  `feat/lut-library` · GREEN · clean · opened 2026-10-08

```sh
gh pr merge 1149 --squash
gh pr merge 1164 --squash
gh pr merge 2596 --squash
gh pr merge 2667 --squash
gh pr merge 2805 --squash
gh pr merge 2417 --squash
gh pr merge 2809 --squash
gh pr merge 2821 --squash
gh pr merge 2847 --squash
gh pr merge 1590 --squash
gh pr merge 2644 --squash
gh pr merge 2791 --squash
gh pr merge 2870 --squash
gh pr merge 2882 --squash
gh pr merge 2065 --squash
gh pr merge 2652 --squash
gh pr merge 2754 --squash
gh pr merge 2044 --squash
gh pr merge 2803 --squash
gh pr merge 2073 --squash
gh pr merge 2876 --squash
gh pr merge 2820 --squash
gh pr merge 2263 --squash
gh pr merge 2811 --squash
gh pr merge 2830 --squash
gh pr merge 1171 --squash
```

## 2. Auto-rebase — green but conflicted

CI already passed; the only blocker is that main moved. `update-branch`
replays them on top of main; most come back clean with no human input.
Anything that still conflicts after that is a real conflict and needs a person.

- **#2258** `XS` 2 files, +253 — Bound shared numeric input before text layout  
  `codex/bounded-numeric-fields` · GREEN · dirty · opened 2026-10-10
- **#2617** `XS` 2 files, +46 — Rulers: keep tick marks and labels out of the origin corner  
  `fix/2600-ruler-corner-clipping` · GREEN · dirty · opened 2026-10-10
- **#2174** `S` 3 files, +21 — docs: clarify task reading and performance reporting  
  `codex/2092-contributor-guidance` · GREEN · dirty · opened 2026-10-09
- **#2640** `S` 3 files, +73 — Brush HUD: wire Vary Round Brush Hardness and Brush Preview Color (#204)  
  `ui-204-brush-hud` · GREEN · dirty · opened 2026-10-10
- **#2300** `S` 4 files, +238 — Scroll the viewport while outlining with Polygonal Lasso  
  `codex/polygon-lasso-edge-scroll` · GREEN · dirty · opened 2026-10-10
- **#2543** `S` 4 files, +45 — Place: wire Always Create Smart Objects When Placing (#204)  
  `ui-204-smart-objects-place` · GREEN · dirty · opened 2026-10-10
- **#2587** `S` 4 files, +39 — Units & Rulers: wire the Type units preference (#204)  
  `ui-204-type-units` · GREEN · dirty · opened 2026-10-10
- **#344** `S` 5 files, +235 — Dock: visible splitter grips, exact drags, double-click reset (#295)  
  `fix/dock-splitters-295` · GREEN · dirty · opened 2026-10-06
- **#968** `S` 6 files, +436 — Use native brush and pipette cursors with low-latency presentation  
  `codex/native-pointer-feedback` · GREEN · dirty · opened 2026-10-08
- **#2658** `S` 6 files, +60 — General: wire Beep When Done for finished background jobs (#204)  
  `ui-204-beep-when-done` · GREEN · dirty · opened 2026-10-10
- **#2668** `S` 6 files, +110 — Type: wire Font Preview Size for the font menu samples (#204)  
  `ui-204-font-preview` · GREEN · dirty · opened 2026-10-10
- **#2577** `M` 7 files, +62 — Enhanced Controls: wire Zoom with Trackpad Pinch (#204)  
  `ui-204-pinch-zoom` · GREEN · dirty · opened 2026-10-10
- **#2854** `M` 8 files, +358 — Dropdowns: press, drag onto an option and release to choose it (#2735)  
  `fix-dropdown-press-drag` · GREEN · dirty · opened 2026-10-10
- **#2880** `M` 8 files, +692 — Speed up Tilt-Shift with existing per-level halo trimming  
  `codex/tilt-shift-optimization` · GREEN · dirty · opened 2026-10-10
- **#527** `M` 10 files, +149 — Fix New Document resolution and add size presets  
  `codex/new-document-resolution` · GREEN · dirty · opened 2026-10-07
- **#1911** `M` 11 files, +888 — No windows ink pressure issue + responsive stylus right click  
  `wintab-issue` · GREEN · dirty · opened 2026-10-09
- **#2516** `M` 14 files, +680 — Addition : Brush shape outline matching alpha + last brush persistance  
  `brush-shape-outline-matching-alpha` · GREEN · dirty · opened 2026-10-10
- **#2685** `M` 14 files, +1795 — Five exact-output speedups: dense Extrude 160s to 0.39s  
  `perf/five-algorithms-20261010` · GREEN · dirty · opened 2026-10-10
- **#2589** `L` 17 files, +258 — Preferences › Interface: Photoshop's Appearance / Presentation / Options layout (#2532)  
  `fix/2532-language-on-top` · GREEN · dirty · opened 2026-10-10
- **#2515** `L` 20 files, +283 — File › Save: a flat PNG saves over itself, a flat JPEG asks JPEG Options (#2223)  
  `fix/save-flat-in-place` · GREEN · dirty · opened 2026-10-10
- **#2673** `L` 20 files, +71 — Addition : Unsaved Changes Prompt Options  
  `add-unsaved-changes-optional-dropdown` · GREEN · dirty · opened 2026-10-10
- **#2445** `L` 25 files, +226 — Favorite themes from the appearance button's right-click menu  
  `feat/theme-favorites` · GREEN · dirty · opened 2026-10-10
- **#2723** `L` 27 files, +831 — Crop tool: New / Delete Crop Preset saves and removes user presets (#1919)  
  `fix/crop-user-presets` · GREEN · dirty · opened 2026-10-10
- **#2432** `L` 28 files, +585 — Restore macOS save format selection and expose writable exports  
  `codex/save-export-formats` · GREEN · dirty · opened 2026-10-10
- **#2764** `L` 29 files, +731 — Arrow-key nudges: a run of nudges is one History state, with Tools preferences (#2259)  
  `feat/bundle-nudges-2259` · GREEN · dirty · opened 2026-10-10
- **#2513** `L` 37 files, +2687 — Add initial pure-Rust JPEG 2000 import and export  
  `feat/jpeg2000-20261010` · GREEN · dirty · opened 2026-10-10
- **#2345** `XL` 44 files, +1438 — Add startup update prompts and platform installer downloads  
  `fix/2316-software-updates` · GREEN · dirty · opened 2026-10-10
- **#2878** `XL` 48 files, +2481 — Generative Fill through a provider interface, with Google Gemini as the first backend (#41)  
  `feat/genai-fill` · GREEN · dirty · opened 2026-10-10
- **#2879** `XL` 49 files, +2547 — Options bar: a Generative Fill… button after Select and Mask…  
  `feat/genai-fill-button` · GREEN · dirty · opened 2026-10-10
- **#1522** `XL` 62 files, +1828 — Add opt-in native AVIF import and export  
  `avif-import-export` · GREEN · dirty · opened 2026-10-09
- **#2534** `XL` 75 files, +2879 — Modular side panel: add and close tabs and panels, inspector style side panel  
  `feat/panel-modules` · GREEN · dirty · opened 2026-10-10
- **#2464** `XL` 200 files, +372 — Tests: one integration-test binary per crate (#2204)  
  `chore/test-binaries` · GREEN · dirty · opened 2026-10-10

```sh
gh pr update-branch 2258
gh pr update-branch 2617
gh pr update-branch 2174
gh pr update-branch 2640
gh pr update-branch 2300
gh pr update-branch 2543
gh pr update-branch 2587
gh pr update-branch 344
gh pr update-branch 968
gh pr update-branch 2658
gh pr update-branch 2668
gh pr update-branch 2577
gh pr update-branch 2854
gh pr update-branch 2880
gh pr update-branch 527
gh pr update-branch 1911
gh pr update-branch 2516
gh pr update-branch 2685
gh pr update-branch 2589
gh pr update-branch 2515
gh pr update-branch 2673
gh pr update-branch 2445
gh pr update-branch 2723
gh pr update-branch 2432
gh pr update-branch 2764
gh pr update-branch 2513
gh pr update-branch 2345
gh pr update-branch 2878
gh pr update-branch 2879
gh pr update-branch 1522
gh pr update-branch 2534
gh pr update-branch 2464
```

## 3. Failing CI — needs a human

- **#2178** `XS` 1 files, +9 — Present each canvas frame with at most one queued frame (low-latency surface, split from #968)  
  `feat/low-latency-surface` · FAILING · unstable · opened 2026-10-09
- **#2324** `XS` 1 files, +43 — feat(toolbar): select grouped tools using press-drag-release  
  `feat/2301-toolbar-drag-flyout-s20261010-c9f31d` · FAILING · unstable · opened 2026-10-10
- **#2887** `XS` 1 files, +61 — fix(canvas): update Move transform-control cursors on first hover  
  `fix/2871-move-transform-hover-s20261010T2149Z-b7d3e6` · FAILING · unstable · opened 2026-10-10
- **#2909** `XS` 1 files, +73 — fix(transform): commit Free Transform on an outside click  
  `fix/2851-transform-click-outside-20261011-d92c` · FAILING · unstable · opened 2026-10-10
- **#2911** `XS` 1 files, +6 — fix(clipboard): retain transparent selection frames on copy (#2598)  
  `fix/2598-copy-selection-frame-20261011-a7f4` · FAILING · unstable · opened 2026-10-10
- **#2923** `XS` 1 files, +22 — Raw: lossless-JPEG tiles respect the allocation limit (#2897)  
  `fix/raw-ljpeg-tile-limit` · FAILING · unstable · opened 2026-10-10
- **#2931** `XS` 1 files, +81 — PSD: bound parse-time merged-image ZIP validation by the decode budget (#2898)  
  `fix-2898` · FAILING · unstable · opened 2026-10-10
- **#2988** `XS` 1 files, +113 — control: allow requests that operate an open Camera Raw dialog  
  `codex/camera-raw-control-continuation` · FAILING · unstable · opened 2026-10-10
- **#2997** `XS` 1 files, +21 — Tests: panic_hunt reruns a timed-out command before calling it hung (#2994)  
  `fix/2994-panic-hunt-font-scan` · FAILING · unstable · opened 2026-10-11
- **#3006** `XS` 1 files, +63 — fix(shortcuts): reject malformed bindings and correctly display Plus  
  `fix/reject-malformed-shortcuts-rebased-20261011` · FAILING · unstable · opened 2026-10-11
- **#3018** `XS` 1 files, +51 — Tests: the marquee ellipse check walks several dash periods of the curve  
  `fix/marquee-ants-flaky-check` · FAILING · unstable · opened 2026-10-11
- **#1988** `XS` 2 files, +87 — fix(psd): honor Smooth interpolation for gradient opacity stops  
  `fix/1954-gradient-overlay-smooth-s20261009T170500Z-8a3a71` · FAILING · unstable · opened 2026-10-09
- **#2358** `XS` 2 files, +435 — Linux/BSD: a second launch opens its files in the running window (#1540)  
  `single-instance` · FAILING · unstable · opened 2026-10-10
- **#2792** `XS` 2 files, +107 — CLI: batch plays an action from an Actions set with the whole set loaded (#2786)  
  `fix/2786-cli-batch-action-set` · FAILING · unstable · opened 2026-10-10
- **#2884** `XS` 2 files, +89 — perf: duplicating a layer recomposites only where the copy draws (#2883)  
  `perf/2883-duplicate-layer-damage` · FAILING · unstable · opened 2026-10-10
- **#2902** `XS` 2 files, +131 — perf: keep resident GPU tiles and convert 32-bit tiles directly for brush dabs on large documents (#2900)  
  `perf/2900-brush-dab-large-docs` · FAILING · unstable · opened 2026-10-10
- **#2906** `XS` 2 files, +231 — Layers: Combine Shapes merges the selected shape layers (#2400)  
  `fix/combine-shapes-layers` · FAILING · unstable · opened 2026-10-10
- **#3005** `XS` 2 files, +69 — feat(crop): nudge pending crop rectangles with arrow keys (#1919)  
  `feat/crop-arrow-nudge-rebased-20261011` · FAILING · unstable · opened 2026-10-11
- **#3012** `XS` 2 files, +51 — Free Transform: ⌘-drag of a corner scales editable type instead of distorting it (#3010)  
  `fix/3010-type-transform-modifiers` · FAILING · unstable · opened 2026-10-11
- **#2317** `S` 3 files, +38 — feat(ui): allow disabling Studio canvas background dots  
  `feat/2264-toggle-canvas-dots-s20261010-b4f92d` · FAILING · unstable · opened 2026-10-10
- **#2890** `S` 3 files, +42 — fix(brush): interpolate pen pressure across Shift-click connecting strokes  
  `fix/2827-shift-click-pressure-ramp-s20261010T2149Z-b7d3e6` · FAILING · unstable · opened 2026-10-10
- **#2950** `S` 3 files, +95 — Layers: right-click an effect row for the layer style menu (#2846)  
  `fix/effects-context-menu` · FAILING · unstable · opened 2026-10-10
- **#2960** `S` 3 files, +91 — Type: Edit › Paste, Copy and Select › All work on the text being typed (#2893)  
  `fix/2893-type-paste` · FAILING · unstable · opened 2026-10-10
- **#2962** `S` 3 files, +90 — Use each XRandR output's ICC profile in the screen picker  
  `fix/x11-eyedropper-icc` · FAILING · unstable · opened 2026-10-10
- **#2990** `S` 3 files, +125 — Pixel grid: fade the line colour across mid-tones instead of flipping at one luma  
  `fix/pixel-grid-line-fade` · FAILING · unstable · opened 2026-10-10
- **#2992** `S` 3 files, +5 — Revert SVG nesting depth check (#1901)  
  `revert/svg-nesting-depth-check` · FAILING · unstable · opened 2026-10-10
- **#3098** `S` 3 files, +424 — Optimise computed brush coverage and native raster row writes  
  `codex/large-brush-performance` · FAILING · unstable · opened 2026-10-11
- **#1733** `S` 4 files, +86 — fix(ui): Delete and Backspace delete layers and can be assigned as shortcuts (#1621)  
  `buivanvu94/fix-1621-delete-layer-keys` · FAILING · unstable · opened 2026-10-09
- **#2654** `S` 4 files, +153 — Double-clicking a mask thumbnail in the Layers panel now opens that mask's settings  
  `feat/double-click-mask-for-settings` · FAILING · unstable · opened 2026-10-10
- **#2703** `S` 4 files, +129 — feat(file-info): display read-only camera metadata from EXIF and XMP  
  `feat/2424-exif-camera-file-info-s20261010T170100Z-7ca5` · FAILING · unstable · opened 2026-10-10
- **#2800** `S` 4 files, +102 — Droplets: carry the actions their Play Action steps call (#2794)  
  `fix/2794-droplet-called-actions` · FAILING · unstable · opened 2026-10-10
- **#2907** `S` 4 files, +371 — perf: marquee selections on large documents are built row by row (#2888)  
  `perf/2888-marquee-selection` · FAILING · unstable · opened 2026-10-10
- **#2917** `S` 4 files, +136 — perf: pasting a layer recomposites only where it draws (#2901)  
  `perf/2901-paste-damage` · FAILING · unstable · opened 2026-10-10
- **#2948** `S` 4 files, +247 — Dropdowns: type a letter to jump to an item, Enter to choose (#1481)  
  `fix/dropdown-type-ahead` · FAILING · unstable · opened 2026-10-10
- **#2967** `S` 4 files, +41 — PSD: don't inflate an over-budget ZIP merged image at parse time (#2926)  
  `fix/2926-psd-zip-validate-budget` · FAILING · unstable · opened 2026-10-10
- **#3035** `S` 4 files, +26 — Marquee: Single Row/Column Marquee History step names (#3031)  
  `fix/3031-single-marquee-history` · FAILING · unstable · opened 2026-10-11
- **#3073** `S` 4 files, +54 — Artboards: a board moved past the canvas origin grows the canvas left/up (#3070)  
  `fix/3070-artboard-negative-origin` · FAILING · unstable · opened 2026-10-11
- **#2495** `S` 5 files, +139 — CLI: PHOTOCRAFT_CONTROL_PORT as mcp/serve fallback (#2491)  
  `feat/cli-control-port-env` · FAILING · unstable · opened 2026-10-10
- **#2507** `S` 5 files, +321 — MCP bridge: reconnect before idle sockets die, retry reads, actionable timeouts (#2487)  
  `fix/bridge-idle-timeout` · FAILING · unstable · opened 2026-10-10
- **#2758** `S` 5 files, +645 — Layers panel: load a fallback font for layer names in other scripts (#2275)  
  `fix-2275` · FAILING · unstable · opened 2026-10-10
- **#2822** `S` 5 files, +115 — Tools: remember tool options between launches (#2814)  
  `fix/2814-persist-tool-options` · FAILING · unstable · opened 2026-10-10
- **#2952** `S` 5 files, +388 — Text fields: right-click edit menu (#2921)  
  `feat/2921-text-field-context-menu` · FAILING · unstable · opened 2026-10-10
- **#2961** `S` 5 files, +270 — Use Fontconfig for Linux font discovery (#2330)  
  `fix/linux-fontconfig-2330` · FAILING · unstable · opened 2026-10-10
- **#3025** `S` 5 files, +252 — Transform: Distort, Perspective and Warp keep shape layers live (#3009)  
  `fix/3009-shape-distort-warp` · FAILING · unstable · opened 2026-10-11
- **#3072** `S` 5 files, +163 — Actions: a command that runs other commands is recorded once (#3067)  
  `fix/3067-nested-journal` · FAILING · unstable · opened 2026-10-11
- **#2715** `S` 6 files, +565 — Improve zoomed Liquify preview detail  
  `codex/perf-liquify-fullres` · FAILING · unstable · opened 2026-10-10
- **#2840** `S` 6 files, +147 — Free Transform: right-click menu with the transform modes, rotate and flip (#2832)  
  `feat/2832-transform-context-menu` · FAILING · unstable · opened 2026-10-10
- **#2845** `S` 6 files, +106 — Channels: Edit › Transform works on a targeted alpha channel (#2835)  
  `fix/2835-transform-alpha-channel` · FAILING · unstable · opened 2026-10-10
- **#2984** `S` 6 files, +41 — Type menu: Anti-Alias › None first and checked; checks follow the edited layer (#2983)  
  `fix/2983-type-menu-checks` · FAILING · unstable · opened 2026-10-10
- **#2852** `M` 7 files, +253 — Move tool: drag a marquee from an empty spot to select the layers it touches (#2844)  
  `feat/2844-move-box-select-layers` · FAILING · unstable · opened 2026-10-10
- **#2963** `M` 7 files, +342 — Support automatic monitor ICC profiles on Linux X11  
  `feat/linux-auto-display-icc` · FAILING · unstable · opened 2026-10-10
- **#2975** `M` 7 files, +84 — PSD: cap the layer tree in psd_to_document; layered TIFF checks group depth (#2940)  
  `fix/2940-psd-group-depth-preflight` · FAILING · unstable · opened 2026-10-10
- **#2922** `M` 8 files, +157 — Quick Mask: an empty start and Select All stay apart (#2743)  
  `fix-2743` · FAILING · unstable · opened 2026-10-10
- **#2969** `M` 8 files, +311 — Type tool: right-click menu while editing text (#2968)  
  `feat/2968-type-edit-context-menu` · FAILING · unstable · opened 2026-10-10
- **#2981** `M` 8 files, +237 — Type: wire Show Font Names in English (#204)  
  `fix/font-names-english` · FAILING · unstable · opened 2026-10-10
- **#3032** `M` 8 files, +431 — Transform Path: Distort, Perspective and Warp apply to paths (#3027)  
  `fix/3027-path-distort` · FAILING · unstable · opened 2026-10-11
- **#1169** `M` 10 files, +834 — GPU composite: reuse unchanged lower-stack chunks for property edits  
  `codex/gpu-composite-reuse` · FAILING · unstable · opened 2026-10-08
- **#3008** `M` 10 files, +151 — Type: ask to rasterize before Warp on live type (#3004)  
  `fix/3004-type-rasterize-prompt` · FAILING · unstable · opened 2026-10-11
- **#3029** `M` 11 files, +466 — Fix canvas context menus closing immediately on trackpad taps  
  `fix/canvas-context-menu-2849` · FAILING · unstable · opened 2026-10-11
- **#1955** `M` 12 files, +361 — fix(psd): Improve interoperability and decoding  
  `fix/psd-interoperability` · FAILING · unstable · opened 2026-10-09
- **#2999** `M` 13 files, +497 — Animated GIF: every frame loads into video layers (#572)  
  `feat/gif-animation-572` · FAILING · unstable · opened 2026-10-11
- **#2987** `M` 15 files, +4442 — Speed up large-radius Unsharp Mask with guarded FFT convolution  
  `codex/unsharp-mask-fft-pr` · FAILING · unstable · opened 2026-10-10
- **#3015** `L` 17 files, +125 — i18n: translate the Select and Mask dialog labels (#3014)  
  `fix/3014-i18n-select-and-mask` · FAILING · unstable · opened 2026-10-11
- **#2919** `L` 18 files, +1565 — Camera Raw: Shadows/Highlights inside raw development, Whites/Blacks and the D4 look measured from Photoshop  
  `camera-raw-tone` · FAILING · unstable · opened 2026-10-10
- **#2354** `L` 19 files, +221 — HEIF: decode with heic-decoder instead of heic-rs (correct iPhone colours, 62/63 conformance)  
  `heif/heic-decoder-rust` · FAILING · unstable · opened 2026-10-10
- **#2451** `L` 20 files, +510 — Image Size: Photoshop's resampling, measured  
  `resample-photoshop` · FAILING · unstable · opened 2026-10-10
- **#2517** `L` 21 files, +239 — app.save: encode on the save worker, not the UI thread (#2488)  
  `fix/app-save-background` · FAILING · unstable · opened 2026-10-10
- **#2957** `L` 21 files, +3158 — raw: decode compressed Olympus ORF/ORI and legacy packed layouts  
  `codex/packed-olympus-orf` · FAILING · unstable · opened 2026-10-10
- **#3007** `L` 21 files, +128 — Select and Mask: Output To New Document, with or without a layer mask (#2998)  
  `fix/2998-select-mask-new-document` · FAILING · unstable · opened 2026-10-11
- **#3030** `L` 21 files, +196 — Type: scan system fonts off the UI thread (#3021)  
  `fix/3021-font-scan-off-ui-thread` · FAILING · unstable · opened 2026-10-11
- **#2996** `L` 22 files, +168 — Type: variable font axis sliders; optical size follows the type size (#2985)  
  `fix/2985-variable-font-axes` · FAILING · unstable · opened 2026-10-11
- **#3039** `L` 22 files, +337 — Paths: Stroke Path with any brush-based tool and Simulate Pressure (#2978)  
  `fix/2978-stroke-path-tools` · FAILING · unstable · opened 2026-10-11
- **#2866** `L` 23 files, +269 — Actions: toggle, reorder and delete single steps (#2855)  
  `feat/2855-actions-step-toggle` · FAILING · unstable · opened 2026-10-10
- **#2581** `L` 24 files, +252 — app.open: open directory .pcraft through the automation read root (#2490)  
  `feat/pcraft-open` · FAILING · unstable · opened 2026-10-10
- **#2970** `L` 24 files, +684 — Select and Mask: live preview in Photoshop's view modes (#1754)  
  `fix/1754-select-mask-view-modes` · FAILING · unstable · opened 2026-10-10
- **#2376** `L` 25 files, +2397 — Content-Aware Fill workspace, as Photoshop's  
  `content-aware-workspace` · FAILING · unstable · opened 2026-10-10
- **#2977** `L` 26 files, +1035 — Improve canvas menus, layer picking, Zoom, trackpad pan, and gradients  
  `fix/five-canvas-improvements-20261011` · FAILING · unstable · opened 2026-10-10
- **#2979** `L` 28 files, +697 — Layers: Pattern Fill dialog for new and edited pattern fill layers (#2908)  
  `fix/pattern-fill-dialog` · FAILING · unstable · opened 2026-10-10
- **#2989** `L` 28 files, +1493 — Select and Mask: Refine Edge Brush (#2972)  
  `fix/2972-refine-edge-brush` · FAILING · unstable · opened 2026-10-10
- **#2949** `L` 29 files, +1222 — Layer Style: contour picker and Contour Editor (#2912)  
  `feat/2912-layer-style-contours` · FAILING · unstable · opened 2026-10-10
- **#2915** `L` 31 files, +417 — Tools: Frame tool (#2914)  
  `feat/2914-frame-tool` · FAILING · unstable · opened 2026-10-10
- **#2862** `L` 32 files, +469 — Tools: Artboard tool (#2858)  
  `feat/2858-artboard-tool` · FAILING · unstable · opened 2026-10-10
- **#2954** `L` 32 files, +139 — Rulers: remember View › Rulers for new documents; guides as f64 (#2918)  
  `feat/2918-rulers-remembered-guides-f64` · FAILING · unstable · opened 2026-10-10
- **#3095** `L` 32 files, +630 — Content-Aware Move: Transform On Drop (#3071)  
  `feat/3071-cam-transform-on-drop` · FAILING · unstable · opened 2026-10-11
- **#3046** `L` 35 files, +687 — Artboard tool: '+' buttons add a board beside the selected one; ⇧ keeps proportions (#3033)  
  `feat/2858-artboard-plus-buttons` · FAILING · unstable · opened 2026-10-11
- **#2856** `L` 37 files, +669 — Tools: Freeform Pen and Curvature Pen (#1054)  
  `feat/1054-freeform-curvature-pen` · FAILING · unstable · opened 2026-10-10
- **#2864** `L` 38 files, +876 — Tools: Perspective Crop (#1043)  
  `feat/1043-perspective-crop` · FAILING · unstable · opened 2026-10-10
- **#2861** `L` 39 files, +591 — Tools: Horizontal and Vertical Type Mask (#1055)  
  `feat/1055-type-mask-tools` · FAILING · unstable · opened 2026-10-10
- **#3087** `L` 39 files, +877 — Freeform Pen: Alt-clicks draw straight segments; Cmd/Ctrl is Direct Selection with the new pens (#3085)  
  `feat/1054-freeform-pen-alt` · FAILING · unstable · opened 2026-10-11
- **#2945** `L` 40 files, +2398 — Open GIMP XCF files: layers, groups, masks, blend modes, channels and selection  
  `feat/xcf-read` · FAILING · unstable · opened 2026-10-10
- **#3069** `L` 40 files, +656 — Type Mask: Shift/Alt at commit add to or subtract from the selection; no Layers row for the working text (#3036)  
  `feat/3036-type-mask-modes` · FAILING · unstable · opened 2026-10-11
- **#3097** `XL` 52 files, +2932 — Adopt shared docking while preserving panel controls and workspaces  
  `codex/ui-suite-integration-20261010` · FAILING · unstable · opened 2026-10-11

## 4. No CI results

CI never reported for these. Most are brand-new PRs whose checks have not
finished; the older ones are usually forks with Actions disabled. Push an
empty commit or rebase to trigger a run before reviewing — a PR with no CI
is not a PR you can merge, whatever its diff looks like. Some of these are
also conflicted, in which case the rebase triggers CI and clears both.

- **#454** `XS` 1 files, +85 — docs: add prominent platform download and installation instructions  
  `codex/readme-install-links` · no-checks · unstable · opened 2026-10-07
- **#2107** `XS` 1 files, +161 — Publish Flatpak repository through GitHub Pages  
  `ci/flatpak-pages` · no-checks · unstable · opened 2026-10-09
- **#2219** `XS` 1 files, +11 — Add OmaStore manifest  
  `add-omastore-manifest` · no-checks · unstable · opened 2026-10-10
- **#2572** `XS` 1 files, +2835 — Add Portugal translation  
  `main` · no-checks · unstable · opened 2026-10-10
- **#2797** `XS` 1 files, +1 — Linux: match the desktop scale instead of the panel's DPI  
  `linux-x11-scale-factor` · no-checks · unstable · opened 2026-10-10
- **#2816** `XS` 1 files, +215 — i18n(ru): Add missing translations  
  `main` · no-checks · dirty · opened 2026-10-10
- **#2863** `XS` 1 files, +8 — CI: run panic_hunt in the Linux test job  
  `ci-panic-hunt` · no-checks · unstable · opened 2026-10-10
- **#2932** `XS` 1 files, +16 — Toolbar: Pro themes draw tool glyphs at two thirds of the button (#2665)  
  `fix/tool-icon-size-2665` · no-checks · unstable · opened 2026-10-10
- **#2933** `XS` 1 files, +72 — CI: cut wall time, starting with the Windows test job  
  `ci-wallclock` · no-checks · unstable · opened 2026-10-10
- **#2986** `XS` 1 files, +1 — docs: add repo size badge to README  
  `main` · no-checks · unstable · opened 2026-10-10
- **#3084** `XS` 1 files, +1 — fix(ui): reduced suffix spacing width in the value field to prevent overflow (#3040)  
  `fix/3040-zoom-label-overflow` · no-checks · unstable · opened 2026-10-11
- **#2106** `XS` 2 files, +72 — Add semantic-release automation on main  
  `ci/semantic-release` · no-checks · unstable · opened 2026-10-09
- **#2363** `XS` 2 files, +158 — Fix/camera raw dehaze  
  `fix/camera-raw-dehaze` · no-checks · unstable · opened 2026-10-10
- **#2405** `XS` 2 files, +25 — F2 renames the active layer in place (#2366)  
  `f2-rename-layer` · no-checks · unstable · opened 2026-10-10
- **#2656** `XS` 2 files, +310 — Control channel: input methods reject bad params instead of using defaults (#2449)  
  `fix-2449` · no-checks · dirty · opened 2026-10-10
- **#2738** `XS` 2 files, +105 — Test Move layer edges across bit depths and view modes  
  `test/move-layer-edges-regressions` · no-checks · unstable · opened 2026-10-10
- **#2886** `XS` 2 files, +149 — fix(engine): preserve Frame from Layers undo under history limits  
  `fix/frame-history-limits` · no-checks · unstable · opened 2026-10-10
- **#2916** `XS` 2 files, +58 — Add built-in rainbow gradient presets  
  `feat/rainbow-gradient-presets` · no-checks · unstable · opened 2026-10-10
- **#2920** `XS` 2 files, +4 — docs: clarify validation details in PR descriptions  
  `docs/clarify-pr-reporting-2094` · no-checks · dirty · opened 2026-10-10
- **#2929** `XS` 2 files, +125 — fix(linux): preserve customized AppImage desktop launchers  
  `fix/apprun-preserve-custom-launcher-2905` · no-checks · unstable · opened 2026-10-10
- **#2980** `XS` 2 files, +58 — Inspect: report each layer's vector mask  
  `squad/inspect-vector-mask` · no-checks · unstable · opened 2026-10-10
- **#3023** `XS` 2 files, +53 — fix(cli): reject repeated automation read roots  
  `fix/cli-reject-repeated-roots-2319` · no-checks · unstable · opened 2026-10-11
- **#605** `S` 3 files, +207 — fix(type): preserve IME composition boundaries and event order  
  `codex/ime-event-order-contribution` · no-checks · dirty · opened 2026-10-07
- **#1737** `S` 3 files, +17 — ui: remove the title-bar Discord button  
  `discord-to-help-menu` · no-checks · unstable · opened 2026-10-09
- **#1999** `S` 3 files, +462 — Fix Free Transform preview layer order and frame timing  
  `fix/transform-preview-layer-order` · no-checks · dirty · opened 2026-10-09
- **#2186** `S` 3 files, +83 — ci: Add RISC-V Linux build  
  `feat/riscv-build` · no-checks · dirty · opened 2026-10-10
- **#2461** `S` 3 files, +76 — Fix false contours in the pixel grid at high zoom  
  `main` · no-checks · unstable · opened 2026-10-10
- **#2597** `S` 3 files, +753 — fix(linux): update stale KDE global menu properties  
  `fix/linux-global-menu-stale-properties` · no-checks · dirty · opened 2026-10-10
- **#2795** `S` 3 files, +140 — Add Shift+scroll snap zoom to Photoshop zoom ladder levels  
  `shift-scroll-snap-zoom` · no-checks · unstable · opened 2026-10-10
- **#2982** `S` 3 files, +66 — Edit > Copy and Edit > Paste work on the text while editing type (#2976)  
  `fix/2976-menu-paste-into-type` · no-checks · unstable · opened 2026-10-10
- **#2053** `S` 4 files, +417 — fix(gpu): evaluate hard gradient stops exactly (#974)  
  `pr/gpu-hard-stops-974` · no-checks · dirty · opened 2026-10-09
- **#2359** `S` 4 files, +61 — fix(i18n): translate file-open progress and layer effects  
  `main` · no-checks · dirty · opened 2026-10-10
- **#2510** `S` 4 files, +332 — Smart Objects: linked smart objects follow their files while open  
  `smart-objects-follow-files` · no-checks · unstable · opened 2026-10-10
- **#2973** `S` 4 files, +79 — Layers: CMD/CTRL + Click new layer adds layer below current.  
  `main` · no-checks · unstable · opened 2026-10-10
- **#3090** `S` 4 files, +325 — Layers: clip groups follow New Layer, Arrange, Reorder and Delete (#2203)  
  `fix/2203-clip-groups-follow-layers` · no-checks · unstable · opened 2026-10-11
- **#3101** `S` 4 files, +215 — Web: opt-in embedding bridge for host pages (?host=parent)  
  `nextcloud-embedding-bridge` · no-checks · unstable · opened 2026-10-11
- **#608** `S` 5 files, +229 — fix(text): resolve PostScript faces beyond family-name guesses  
  `codex/font-postscript-contribution` · no-checks · dirty · opened 2026-10-07
- **#2379** `S` 5 files, +282 — Brush rendering: rayon parallelism, fast-path rasterization, tail throttle  
  `brush-render-perf` · no-checks · unstable · opened 2026-10-10
- **#2402** `S` 5 files, +145 — Type: ⇧⌘> / ⇧⌘< change the type size (and work on Nordic/German layouts)  
  `type-size-shortcuts` · no-checks · dirty · opened 2026-10-10
- **#3026** `S` 5 files, +233 — Color: honour the EXIF sRGB tag on open; wire Ignore EXIF Profile Tag (#204)  
  `prefs/ignore-exif-profile-tag` · no-checks · unstable · opened 2026-10-11
- **#1992** `S` 6 files, +273 — Keep imported PSD text layers editable  
  `fix/psd-text-editing` · no-checks · dirty · opened 2026-10-09
- **#2175** `S` 6 files, +302 — fix(layers): protect locks and allow final-layer deletion  
  `fix/delete-layer-shortcut` · no-checks · unstable · opened 2026-10-09
- **#2318** `S` 6 files, +837 — Add cargo xtask doctor: a fresh checkout now tells you what's missing  
  `pr/repo-prerequisites` · no-checks · dirty · opened 2026-10-10
- **#2859** `S` 6 files, +749 — Refine gradient stop controls in Properties and Gradient Editor  
  `ui/gradient-properties` · no-checks · unstable · opened 2026-10-10
- **#2953** `S` 6 files, +40 — Preferences: wire Tools › Show Transformation Values (#204)  
  `prefs/show-transformation-values` · no-checks · unstable · opened 2026-10-10
- **#621** `M` 7 files, +523 — Add a Nix flake: release package and dev shell with debugging tools  
  `main` · no-checks · dirty · opened 2026-10-07
- **#1300** `M` 7 files, +37 — [ci] Update GitHub Actions to latest major release  
  `feature/update-github-actions` · no-checks · unstable · opened 2026-10-08
- **#2169** `M` 7 files, +63 — Load the Arabic craft fonts  
  `arabic-font-loading` · no-checks · unstable · opened 2026-10-09
- **#2182** `M` 7 files, +57 — chore: bump github actions to fix Node 20 deprecation warnings  
  `chore/update-actions` · no-checks · unstable · opened 2026-10-10
- **#2270** `M` 7 files, +212 — Security: pin and verify release build downloads  
  `codex/photocraft-release-security` · no-checks · unstable · opened 2026-10-10
- **#2505** `M` 7 files, +2841 — ADDED ARABIC TRANSLATION  
  `main` · no-checks · unstable · opened 2026-10-10
- **#3011** `M` 7 files, +440 — Installed ICC profiles in the profile menus, and Mirror in Print  
  `moiz/icc-profiles-mirror` · no-checks · unstable · opened 2026-10-11
- **#3037** `M` 7 files, +148 — fix(ui): correct the clipping-mask indicator orientation  
  `fix/clipping-mask-indicator` · no-checks · unstable · opened 2026-10-11
- **#374** `M` 8 files, +412 — Packaging: Arch Linux AUR packages (photocraft, photocraft-bin) and publish workflow  
  `packaging/arch-aur` · no-checks · unstable · opened 2026-10-06
- **#2592** `M` 8 files, +285 — Preserve HDR range in float adjustments and stack statistics  
  `fix/hdr-adjustment-range` · no-checks · dirty · opened 2026-10-10
- **#2836** `M` 8 files, +339 — Fix Kurdish/Arabic RTL layer names (tofu + word order)  
  `fix/rtl-layer-names-kurdish` · no-checks · unstable · opened 2026-10-10
- **#2555** `M` 9 files, +251 — Channels: keep RGB off while an alpha or mask is shown, and select from that channel.  
  `daily-usage` · no-checks · dirty · opened 2026-10-10
- **#2646** `M` 9 files, +2588 — Add Persian (fa) UI translation and lazy Persian/Arabic font fallback  
  `add-persian-translation` · no-checks · unstable · opened 2026-10-10
- **#2575** `M` 11 files, +1428 — Perf nightly: macOS, Linux and Windows runners, one baseline per machine  
  `perf-nightly-runner-matrix` · no-checks · dirty · opened 2026-10-10
- **#1827** `M` 12 files, +7 — Removed 22 unit radius that doesn't exist in other Craft app icons  
  `icon-corner-radius-consistency` · no-checks · unstable · opened 2026-10-09
- **#2371** `M` 12 files, +998 — Raw: Fujifilm lossless compressed RAF decodes natively, X-Trans and Bayer, 14/16-bit (#50)  
  `feat/fuji-lossless-compressed-raf` · no-checks · dirty · opened 2026-10-10
- **#1257** `M` 13 files, +861 — Adobe assets: new crate; read and write Photoshop action files (.atn) (#219)  
  `feat/atn-reader` · no-checks · dirty · opened 2026-10-08
- **#2580** `M` 13 files, +341 — perf: Reduce allocations and redundant work in image processing  
  `perf/image-processing` · no-checks · dirty · opened 2026-10-10
- **#2636** `M` 13 files, +2403 — Add Vietnamese localization  
  `feat/vietnamese-localization` · no-checks · unstable · opened 2026-10-10
- **#2824** `M` 14 files, +720 — Pen tool: Path editing and cursor updates  
  `origin/feat/pen-updates-add-remove-direction` · no-checks · dirty · opened 2026-10-10
- **#2660** `M` 15 files, +196 — i18n test: scan past small test modules; translate what it was missing  
  `contrib/i18n-coverage-scan` · no-checks · dirty · opened 2026-10-10
- **#2956** `M` 15 files, +681 — Add FITS read/write (astronomy images)  
  `fits-codec` · no-checks · unstable · opened 2026-10-10
- **#2885** `L` 16 files, +1336 — Accelerate large canvas brushes with tiled rasterization and GPU compute  
  `codex/large-canvas-gpu-brush` · no-checks · unstable · opened 2026-10-10
- **#3003** `L` 16 files, +861 — feat: comprehensive core tools overhaul & engine reliability (Phase 1)  
  `feat/core-tools-phase1-reliability` · no-checks · unstable · opened 2026-10-11
- **#3013** `L` 16 files, +1369 — Print on Windows: printer list, driver Print Settings, GDI printing  
  `moiz/windows-printing-v2` · no-checks · unstable · opened 2026-10-11
- **#2416** `L` 17 files, +1064 — Fix desktop save paths and preserve linked Smart Object edits  
  `fix/portable-document-paths` · no-checks · dirty · opened 2026-10-10
- **#408** `L` 19 files, +1441 — [DO NOT MERGE] WIP: Color panel modes (#294)  
  `wip/color-panel-modes-294` · no-checks · dirty · opened 2026-10-06
- **#3017** `L` 19 files, +1652 — Print through CUPS over IPP and the print portal: File › Print works in the Flatpak  
  `feat/print-cups-portal` · no-checks · dirty · opened 2026-10-11
- **#1920** `L` 21 files, +952 — feat(android): add Android runner with mobile UI, multi-arch CI, and input support  
  `feature/android-support` · no-checks · dirty · opened 2026-10-09
- **#2692** `L` 21 files, +472 — File Info shows Camera Data from EXIF/XMP (#2424)  
  `fix-2424` · no-checks · dirty · opened 2026-10-10
- **#2801** `L` 21 files, +1384 — Android native  
  `android-native` · no-checks · dirty · opened 2026-10-10
- **#1315** `L` 23 files, +505 — Eyedropper: add live sampled-color preview  
  `feat/color-sampling-preview` · no-checks · dirty · opened 2026-10-08
- **#2269** `L` 23 files, +976 — Security: harden document exports and automation inputs  
  `codex/photocraft-app-security` · no-checks · unstable · opened 2026-10-10
- **#884** `L` 24 files, +404 — Font menu: each font drawn in its own face, recently used fonts first (#540)  
  `issue-540-font-previews` · no-checks · dirty · opened 2026-10-08
- **#2892** `L` 24 files, +232 — Themes: Photoshop's light-gray and light Pro brightness levels (#2661)  
  `feat/light-gray-themes-2661` · no-checks · dirty · opened 2026-10-10
- **#730** `L` 25 files, +1624 — Optional generative AI: Generate, Generative Fill, Expand Canvas and Variations through a user-supplied OpenAI key (#41)  
  `feat/generative-ai-openai` · no-checks · dirty · opened 2026-10-07
- **#2683** `L` 26 files, +1080 — Modernize New Document with social formats and centered layout  
  `feat/modern-new-document` · no-checks · dirty · opened 2026-10-10
- **#2191** `L` 28 files, +308 — Add native macOS Sparkle updater  
  `macos-sparkle-updater` · no-checks · dirty · opened 2026-10-10
- **#2995** `L` 28 files, +1549 — Add AI Select Subject (BiRefNet-lite via tract) and Magic Wand Sample Size  
  `feat/ai-select-subject` · no-checks · unstable · opened 2026-10-10
- **#3104** `L` 28 files, +861 — Layers: the New Layer dialog (Layer › New › Layer…, ⇧⌘N)  
  `feat/new-layer-dialog` · no-checks · unstable · opened 2026-10-11
- **#2559** `L` 29 files, +1824 — Color panel modes: Hue/Brightness Cube, Color Wheel and slider sets (#294)  
  `color-panel-modes-294` · no-checks · dirty · opened 2026-10-10
- **#2839** `L` 29 files, +462 — Fix Zoom and Eyedropper cursors and retain the previous sampling color  
  `fix/canvas-cursors-pr` · no-checks · unstable · opened 2026-10-10
- **#2129** `L` 30 files, +2149 — Add searchable Documentation and personal notes  
  `feature/help-searchbar` · no-checks · dirty · opened 2026-10-09
- **#2649** `L` 30 files, +898 — Add local AI background removal to layer context menus  
  `background-removal` · no-checks · unstable · opened 2026-10-10
- **#2930** `L` 30 files, +1182 — Add Photoshop-style guide layout dialog and live preview  
  `feat/guide-layout-live-preview` · no-checks · unstable · opened 2026-10-10
- **#2477** `L` 32 files, +1316 — Add floating panel docking and precise drop previews  
  `codex/panel-docking` · no-checks · dirty · opened 2026-10-10
- **#499** `L` 33 files, +387 — Use vector filmstrip Pc logo for PhotoCraft  
  `codex/photocraft-filmstrip-logo` · no-checks · dirty · opened 2026-10-07
- **#1610** `L` 33 files, +1905 — Art History Brush in the Y-group flyout  
  `fix/art-history-brush` · no-checks · dirty · opened 2026-10-09
- **#3022** `L` 34 files, +922 — Reuse pixel layer masks with Copy/Paste and thumbnail drag  
  `feat/mask-transfer-2928` · no-checks · unstable · opened 2026-10-11
- **#2753** `L` 36 files, +909 — Color Sampler tool (#1046)  
  `feat/1046-color-sampler` · no-checks · dirty · opened 2026-10-10
- **#2719** `L` 37 files, +632 — Color Replacement tool on the toolbar (#1045)  
  `fix-1045` · no-checks · dirty · opened 2026-10-10
- **#557** `L` 40 files, +2796 — Support the global menu on Linux, closes #383  
  `appmenu-global-menu` · no-checks · dirty · opened 2026-10-07
- **#2372** `L` 40 files, +2352 — Add Windows MSI auto-updates in About  
  `add-windows-auto-updates` · no-checks · dirty · opened 2026-10-10
- **#2815** `L` 40 files, +779 — Add a contextual task bar for background removal  
  `feat/contextual-background-taskbar` · no-checks · unstable · opened 2026-10-10
- **#2760** `XL` 44 files, +1028 — TGA: targa options asks for 24 or 32 bits/pixel, same as in Photoshop  
  `tga-targa-options` · no-checks · unstable · opened 2026-10-10
- **#2011** `XL` 51 files, +2262 — Add PDF page selection, per-page tabs, and binder export  
  `pdf-binder-workflow` · no-checks · dirty · opened 2026-10-09
- **#2631** `XL` 53 files, +4212 — Add Arabic text support: visual RTL editing, paragraph direction, kashida and digit shapes  
  `arabic-support` · no-checks · dirty · opened 2026-10-10
- **#2058** `XL` 54 files, +2121 — fix(ui): honor pixel aspect and View overlay controls (#1119)  
  `pr/view-pixel-aspect-1119` · no-checks · dirty · opened 2026-10-09
- **#2868** `XL` 58 files, +1508 — Open JPEG XL files: optional photocraft-jxl crate (jxl-oxide) behind the jxl feature  
  `feat/jxl-read` · no-checks · dirty · opened 2026-10-10
- **#2751** `XL` 61 files, +3422 — Integrate PDF import with thumbnail picker and Smart Object placement  
  `fix/pdf-import-2541` · no-checks · unstable · opened 2026-10-10
- **#2453** `XL` 62 files, +7590 — Google Fonts: install fonts from inside PhotoCraft (Find More tab, Resolve Missing Fonts download)  
  `feat/google-fonts` · no-checks · dirty · opened 2026-10-10
- **#1199** `XL` 63 files, +9481 — Add optional local models for subject matting and object selection  
  `feat/optional-local-selection-models` · no-checks · unstable · opened 2026-10-08
- **#1995** `XL` 64 files, +767 — Multithreaded web build: WebAssembly threads with a rayon Web Worker pool  
  `wasm-threads` · no-checks · dirty · opened 2026-10-09
- **#2028** `XL` 88 files, +6442 — Add native scratch storage and bounded PSB/EXR viewport streaming  
  `codex/scratch-disks` · no-checks · dirty · opened 2026-10-09
- **#573** `XL` 112 files, +36099 — Migrate PhotoCraft UI localization to Fluent and complete Simplified Chinese  
  `codex/translation-work` · no-checks · dirty · opened 2026-10-07
- **#2230** `XL` 114 files, +3953 — Add Camera Raw auto controls and licensed camera calibration  
  `main` · no-checks · dirty · opened 2026-10-10
- **#2295** `XL` 212 files, +15869 — Add GitHub update checks  
  `feat/update-check` · no-checks · dirty · opened 2026-10-10
- **#2475** `XL` 221 files, +62652 — Fix Windows Ink pen barrel-button input  
  `codex/wacom-barrel-input` · no-checks · dirty · opened 2026-10-10
- **#606** `XL` 224 files, +64901 — fix(macos): recover first Korean IME fallback at the native boundary  
  `codex/macos-first-hangul-contribution` · no-checks · unstable · opened 2026-10-07

## 5. Everything else (dirty *and* not green)

Cheapest first. Many of these are worth closing rather than resurrecting:
if the author has not answered in a week and CI is red, ask once, then close
with a pointer to re-open when it is rebased.

- **#1991** `XS` 1 files, +218 — feat(actions): import and export reusable Actions JSON files  
  `feat/1951-actions-json-import-export-s20261009T170500Z-8a3a71` · FAILING · dirty · opened 2026-10-09
- **#3091** `XS` 2 files, +35 — Actions: Last Filter is recorded as the filter it repeated (#3089)  
  `fix/3089-last-filter-records-filter` · PENDING · unstable · opened 2026-10-11
- **#3111** `XS` 2 files, +41 — fix(paint): respect Lock All on layer masks (#3094)  
  `fix/brush-mask-lock-3094` · PENDING · unstable · opened 2026-10-11
- **#2964** `S` 4 files, +288 — Tools: wire Enable Flick Panning for the Hand tool (#204)  
  `fix/flick-panning` · FAILING · dirty · opened 2026-10-10
- **#1060** `S` 6 files, +271 — Install PhotoCraft with Homebrew: brew install --cask storytold/tap/photocraft (#342, #592, #1033)  
  `issue-342-homebrew-cask` · FAILING · dirty · opened 2026-10-08
- **#2894** `M` 7 files, +64 — scorecard: twenty rows describe work that is already in the tree  
  `scorecard/mask-transform-rows-are-done` · FAILING · dirty · opened 2026-10-10
- **#2441** `M` 8 files, +171 — Actions: record layer selections by name and display the target  
  `fix/action-layer-targets` · FAILING · dirty · opened 2026-10-10
- **#3106** `M` 8 files, +317 — Layers panel: Shift+Plus/Minus cycle blend modes; effects dropped on the Trash are deleted (#3102)  
  `fix/3102-blend-keys-fx-trash` · PENDING · unstable · opened 2026-10-11
- **#3112** `M` 8 files, +363 — Shortcuts: Ö / # are the brush keys on a German layout (#3109)  
  `fix/3109-german-brush-keys` · PENDING · unstable · opened 2026-10-11
- **#2705** `M` 9 files, +131 — fix(engine): allow targeting layer mask and vector mask via channel.target (#2458)  
  `fix/channel-target-mask-2458` · PENDING · unstable · opened 2026-10-10
- **#2131** `M` 10 files, +368 — Releasing a brush stroke no longer freezes: the commit takes the pixels the live stroke already drew (P32: 136 ms → 0.8 ms)  
  `perf/brush-commit-live-stroke` · FAILING · dirty · opened 2026-10-09
- **#2135** `M` 10 files, +1470 — raw: fit non-DNG colour to the camera's embedded JPEG  
  `raw-colour-look` · FAILING · dirty · opened 2026-10-09
- **#3100** `M` 11 files, +429 — Numeric fields: typed units, scrubby labels, double-click reset (#3099)  
  `fix/3099-numeric-field-units-scrub` · PENDING · unstable · opened 2026-10-11
- **#2137** `M` 12 files, +308 — Fix Affinity import review regressions  
  `fix/issue-2035-affinity-feedback` · FAILING · dirty · opened 2026-10-09
- **#2184** `M` 13 files, +1185 — Preserve editable layer masks across Copy and Paste  
  `codex/mask-clipboard` · FAILING · dirty · opened 2026-10-10
- **#3019** `M` 13 files, +1770 — Fix Shape Stroke controls, popup interactions and paint colours  
  `codex/shape-stroke-ui-polish` · PENDING · unstable · opened 2026-10-11
- **#354** `L` 16 files, +1735 — Use native brush cursors and endpoint-aligned desktop motion  
  `cursor-input` · FAILING · dirty · opened 2026-10-06
- **#2971** `L` 16 files, +371 — File Handling: wire Image Previews to embed a PSD thumbnail (#204)  
  `fix/psd-image-previews` · FAILING · dirty · opened 2026-10-10
- **#856** `L` 21 files, +1358 — Android: add an experimental NativeActivity app and Nix APK build  
  `contrib/android-base` · FAILING · dirty · opened 2026-10-07
- **#1312** `L` 22 files, +696 — Add issue-reporting dropdown with reusable templates and safe diagnostic prefilling  
  `feature/issue-reporting` · FAILING · dirty · opened 2026-10-08
- **#424** `L` 25 files, +932 — Shrink native releases and split architecture and CLI downloads  
  `codex/shrink-native-releases` · FAILING · dirty · opened 2026-10-06
- **#857** `L` 28 files, +2202 — Android: add automatic pen-aware palm rejection  
  `contrib/android-palm-rejection` · FAILING · dirty · opened 2026-10-07
- **#3103** `L` 28 files, +681 — Content-Aware Scale: transform box with live preview (#3096)  
  `fix/3096-cas-transform-box` · PENDING · unstable · opened 2026-10-11
- **#858** `L` 30 files, +2367 — Android: enable immersive system bars with transient edge-swipe access  
  `contrib/android-immersive` · FAILING · dirty · opened 2026-10-07
- **#3108** `L` 30 files, +611 — Curves: pencil mode, Smooth and Edit points (#3088)  
  `fix/3088-curves-pencil` · PENDING · unstable · opened 2026-10-11
- **#2756** `L` 36 files, +685 — Add Anchor Point and Delete Anchor Point tools (#1044)  
  `fix-1044` · FAILING · dirty · opened 2026-10-10
- **#1122** `XL` 43 files, +1237 — Deep stacking: sample-level merge (Nuke DeepMerge) and File › Open as Deep  
  `feat/deep-exr-stack` · PENDING · unstable · opened 2026-10-08
- **#1646** `XL` 43 files, +1391 — feat: enable AVIF export in Save As, Export As and CLI  
  `feat/avif-export` · FAILING · dirty · opened 2026-10-09
- **#2872** `XL` 49 files, +2071 — Add 13 image, asset and document export format groups  
  `codex/additional-export-formats` · FAILING · dirty · opened 2026-10-10
- **#2282** `XL` 61 files, +2878 — Add shared 4 GiB RAM and 8 GiB disk undo cache  
  `codex/disk-backed-undo` · FAILING · dirty · opened 2026-10-10
- **#2283** `XL` 81 files, +5483 — Preserve documents and undo history across crashes  
  `codex/crash-recovery-hardening` · FAILING · dirty · opened 2026-10-10

## Open issues by area

| Area | Issues | PRs |
|---|---:|---:|
| layers-panels-ui | 103 | 45 |
| other | 97 | 28 |
| file-format | 80 | 36 |
| selection-mask | 57 | 33 |
| brush-paint | 52 | 17 |
| type-font | 51 | 31 |
| transform | 33 | 21 |
| automation-mcp | 31 | 15 |
| install-packaging | 26 | 19 |
| crash-stability | 24 | 5 |
| input-clipboard | 24 | 8 |
| tablet-pen | 18 | 9 |
| performance | 18 | 8 |
| ai-generative | 11 | 5 |
| print-export | 5 | 1 |
| docs | 5 | 3 |
| localization | 4 | 7 |
| gpu-render | 4 | 2 |
| video | 2 | 0 |
| ci-build | 2 | 2 |

A big PR count next to a big issue count is a queue problem, not a code
problem: the fixes exist and are waiting on review.

## Most-discussed open issues

Comment count is a rough proxy for how many people hit the same thing.
An issue with eight comments is usually eight users, not one argument.

| Issue | Comments | Age (d) | Area | Title |
|---|---:|---:|---|---|
| #341 | 13 | 4 | other | Is it really a clean room implementation if code is generated by LLM? |
| #2717 | 9 | 0 | file-format | [Bug] Photoshop 2023 cannot open PSD/PSB re-saved from complex imported PSB, even without modifications |
| #386 | 9 | 4 | input-clipboard | Linux Wayland: dropping files on the window does nothing (winit 0.30 has no Wayland drag and drop) |
| #759 | 8 | 3 | tablet-pen | There is no pressure sensitivity for Wacom and XP-Pen tablets |
| #41 | 8 | 6 | ai-generative | Generative AI editing: Generative Fill, Generative Expand, AI-assisted tools |
| #1461 | 7 | 2 | crash-stability | WGPU crash on Arch Linux |
| #1343 | 7 | 2 | performance | Mouse cursor lag — SurfaceConfig::HIGH_THROUGHPUT is wrong default for an image   editor |
| #1015 | 7 | 2 | performance | GPU canvas: very large documents run the GPU out of memory, then the CPU fallback reads every pixel (300000² hangs the app) |
| #2138 | 6 | 1 | brush-paint | Brush interpolation and smoothing problems |
| #1606 | 6 | 1 | file-format | Affinity import: effects, adjustments, brushes, master pages and other remaining gaps |
| #309 | 6 | 4 | localization | Add localization support |
| #2959 | 5 | 0 | transform | there are no flip vertically/horizontally options |
| #2034 | 5 | 1 | input-clipboard | If there's an image in the clipboard then New Document's initial size should match it |
| #2012 | 5 | 1 | other | Be more honest. |
| #1761 | 5 | 1 | brush-paint | Brush affects a larger area than its cursor outline (Spot Healing, Blur) at 218% zoom |
| #613 | 5 | 3 | performance | Windows MSIs: Start Menu shortcuts point at the installer-cache icon, and the executables have no embedded icon (PhotoCraft, VectorCraft, EffectCraft 0.4.0) |
| #600 | 5 | 3 | crash-stability | The program on my Late 2013 iMac closes without even opening. v3.0 |
| #2619 | 4 | 0 | brush-paint | Lagg when using big brushes |
| #2255 | 4 | 0 | input-clipboard | Cannot drag and drop files |
| #1952 | 4 | 1 | crash-stability | [Bug] Only black window |
| #1399 | 4 | 2 | other | PhotoCraft randomly flashing |
| #1213 | 4 | 2 | layers-panels-ui | Unable to pull tabs out into a separate panel |
| #1186 | 4 | 2 | input-clipboard | Not even support basic drag and drop image to be a layer and open it as a new document instead |
| #1173 | 4 | 2 | type-font | Text tool destroys text if selected, or selected all (with ctrl+a) |
| #1067 | 4 | 2 | other | Pattern Preview does not display a seamless four-way repeating preview |

