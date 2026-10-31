# PhotoCraft roadmap

**Stage: alpha** · next: beta, ~23 points (ready for real work ~52% → ~75%, with reliable PSD) and ~1,350–2,200 h away

> **Last reviewed:** 2026-10-31 · **Last updated:** 2026-10-31 · **Change:** major (full re-measure at 0.18.0: tools 68 of 68, 16 of 40 measured performance budgets met on a new nightly baseline, 11 of the 12 key languages at 99%+ of 2,199 keys, ready for real work ~45% → ~52%, 2,351 open issues) · **Target:** Adobe Photoshop 2026

PhotoCraft's one-page summary of how close it is to Adobe Photoshop 2026 (27.11.0, measured from
the installed app on 2026-10-10) at version 0.18.0. Detail: [target-app parity](docs/target-app-parity.md)
(methodology, weights, evidence), [gaps](docs/gaps.md) (the ranked work list),
[roadmap](docs/roadmap.md) (milestones, current focus), [scorecard](docs/scorecard.md) and
[menu checklist](docs/parity-checklist.md) (both generated and measured),
[architecture](docs/architecture.md).

Hours are Opus 5.5 agent wall-clock hours for one agent working sequentially, calibrated from this
repository's first 1,004 merged PRs (see [calibration](docs/target-app-parity.md#remaining-effort-and-calibration)).
That calibration needs redoing: 3,471 PRs have merged, and recent ones are far smaller. About 80%
of the work parallelizes.

## Headline numbers

| | Value | Kind |
|---|---|---|
| **Feature breadth** | **~79%** | partly measured: menus 631/631, tools 68/68, panels 30/35, formats 15+7 of 31 |
| **Ready for real work** | **~52%** (47–57%) | estimated: weighted by dimension below |
| Mainstream practitioner | ~47% (42–52%) | estimated: weekly areas of a typical pro, with discounts ([method](docs/target-app-parity.md#mainstream-practitioner-47)) |
| Essentials user | ~63% (57–69%) | estimated: core features only ([method](docs/target-app-parity.md#essentials-user-63)) |
| Remaining to **beta** | **~1,350–2,200 h** | estimated |
| Remaining to **full parity** | **~2,600–4,500 h** | estimated; AI and plug-ins need owner decisions |

### Readiness by audience

| Audience | Ready % | Opus 5.5 agent-hours to ~95% | Work that dominates |
|---|---:|---:|---|
| Full target (ready for real work) | ~52% (additive weighted sum: 51.75%) | 2,600–4,500 | Everything below, plus AI tier, plug-in/scripting ecosystem, video, localization, hardware |
| Mainstream practitioner | ~47% | 1,900–3,100 | Depth in the 11 weekly areas (1,180–2,000), performance budgets, the bug backlog on core paths, PSD exchange with Photoshop |
| Essentials user | ~63% | 400–700 | GPU start failures, brush and transform responsiveness, discoverability (flyouts), AVIF/PDF open |

Hours calibrated as in [Remaining effort](docs/target-app-parity.md#remaining-effort-and-calibration) (2–9 h per checklist
item, 8–20 h per format or subsystem, from the repo's first 1,004 merged PRs); each tier is a subset
of the one above. About 80% parallelizes (independent crates and command modules); performance
architecture and the PSD writer are the serial parts.

**Why alpha.** Core workflows (open, layer, select, paint, adjust, filter, type, save) work end to
end, and breadth is high, but 2,351 issues are open (2026-10-31; ~1,760 once labelled duplicates,
needs-info, spam and already-fixed reports are set aside, 50 of them critical), 24 of the 40
measured performance budgets are missed (only 10 are met every night), and PSDs we write are not yet reliably accepted by
Photoshop (#1281). Beta needs ~75% ready and no blocking PSD gap. The
[alpha gate](docs/roadmap.md#alpha-gate) passes: all six core workflows (retouch, composite, cut-out,
design, paint, PSD exchange) complete end to end and save and reopen; four are partial in depth,
none blocked.

## By dimension

| Dimension | % ready | Hours to full | Doc |
|---|---:|---:|---|
| Features (depth) | 60% (breadth 79%) | 650–1,100 | [target-app-parity](docs/target-app-parity.md#feature-areas) |
| UI/UX fidelity (tools, handles, modifiers, brush feel, panels) | 45% | 500–850 | [ui-parity](docs/ui-parity.md), [brush-parity](docs/brush-parity.md), [context-menu-parity](docs/context-menu-parity.md) |
| File formats | 65% (presence ~80%) | 320–540 | [file-format-parity](docs/file-format-parity.md) |
| Hardware (GPU, pen, displays) | 55% | 70–130 | [hardware-parity](docs/hardware-parity.md) |
| Localization | 65% | 60–110 | [localization-parity](docs/localization-parity.md) |
| Performance | 45% (16/40 measured budgets met at the median, 10 every night) | 150–280 | [scorecard](docs/scorecard.md#performance) |
| Stability | 45% | 250–400 | [gaps](docs/gaps.md#g3-user-reported-bug-backlog) |
| Platforms | 80% | 30–60 | [scorecard](docs/scorecard.md#distribution) |
| Ecosystem / plug-ins / scripting | 15% | 250–450 | [gaps](docs/gaps.md#g16-plug-in-compatibility) |
| AI features | 10% | 300–600 | [gaps](docs/gaps.md#g15-ai-and-generative-features) |

## Features

| Area | % ready | Hours |
|---|---:|---:|
| Layers, masks, blend modes, smart objects, styles | 65% | 150–250 |
| Selections, Select and Mask, channels | 60% | 120–200 |
| Painting and brush engine | 60% | 150–250 |
| Retouching (healing, clone, patch, remove, content-aware) | 55% | 100–180 |
| Transform, warp, puppet, liquify | 60% | 100–180 |
| Adjustments and filters | 70% | 120–200 |
| Type | 45% | 120–200 |
| Vector shapes and paths | 60% | 80–140 |
| Colour management and modes | 70% | 60–100 |
| Camera Raw and computational photo | 50% | 120–200 |
| Automation, actions, scripting | 50% | 120–200 |
| File I/O | 65% | 320–540 |
| Print, export, Save for Web | 55% | 60–100 |
| Video / timeline, data-driven graphics | 30% | 100–200 |
| Workspace, panels, preferences | 50% | 120–200 |
| AI / generative | 10% | 300–600 |

Rows and evidence: [target-app-parity › Feature areas](docs/target-app-parity.md#feature-areas).

## Languages

Measured with `cargo xtask i18n-coverage` on 2026-10-31 (2,199 keys; the universe grew from 1,608, so
catalogs that were at 100% now miss a handful of new strings). `partial` everywhere because engine
messages are still English. Detail: [localization-parity](docs/localization-parity.md).

| Language | Code | UI strings | Status |
|---|---|---:|---|
| English | `en` | 100% | full |
| Simplified Chinese | `zh-hans` | 99.2% | partial |
| Spanish | `es` | 99.4% | partial |
| Hindi | `hi` | 14.7% | partial |
| Arabic | `ar` | 99.7% (RTL shell only) | partial |
| French | `fr` | 99.3% | partial |
| Portuguese (Brazil) | `pt-br` | 99.4% | partial |
| Indonesian | `id` | 99.2% | partial |
| Japanese | `ja` | 99.3% | partial |
| German | `de` | 99.1% | partial |
| Korean | `ko` | 99.2% | partial |
| Vietnamese | `vi` | 99.3% | partial |

Also shipped, each at 99%+: Traditional Chinese, Russian ([notes](docs/localization-ru.md)), Ukrainian, Czech, Polish, Greek, Dutch, Italian, Turkish, Serbian, Hungarian, Swedish, Finnish, Danish, Norwegian Bokmål (15).

## Upcoming

| Rank | Milestone | Estimate |
|---|---|---:|
| 1 | PSD that Photoshop trusts: Photoshop re-open check, #1281, Photoshop-oracle floor 190 → 230+ of 258, layer effects first (3 of 37 pass) | 150–250 h |
| 2 | Performance budgets: the 24 over-budget scenarios, worst first (150 full-canvas layers on the app's GPU budget, ~6 s) | 150–280 h |
| 3 | User bug backlog to under 100 open, critical and high first | 750–1,200 h |
| 4 | Brush and transform feel (brush-parity, ui-parity rows) | 150–270 h |
| 5 | The workspaces still shown as dialogs: Content-Aware Fill, Path Blur; Refine Hair | 60–110 h |

Detail and the next releases: [docs/roadmap.md](docs/roadmap.md).

## Progress log

| Date | Entry |
|---|---|
| 2026-10-31 | Full re-measure at 0.18.0: breadth ~79%, ready ~52%, alpha. Tools 68/68, P12's 150-layer GPU crash no longer seen on the nightly runner, performance baseline moved to that runner (16 of 40 budgets met at the median), 27 UI languages. Open issues 2,351 |
| 2026-10-30 | 0.18.0 |
| 2026-10-23 | Select and Mask becomes Photoshop's workspace (#7069, #8148) |
| 2026-10-17 | Object Finder classifies background by boundary coverage, retaining large enclosed foregrounds and rejecting surrounding backdrop (#6548); learned segmentation remains open (#2788) |
| 2026-10-10 | Full re-measure against Photoshop 2026 27.11.0 (installed bundle inspected): breadth ~76%, ready ~45%, alpha. Progress docs reorganized (`parity.md` → `parity-checklist.md`, `parity-estimate.md` merged into `target-app-parity.md`). 0.6.0 released. |
| 2026-10-09 | Remove tool, Rotate View, crop rotation; OpenRaster and Paint.NET; 39 Affinity documents in the corpus |
| 2026-10-08 | 0.5.0. Affinity import; Magnetic Lasso, Pattern Stamp, Mixer Brush, Pencil, Patch, Vertical Type on the toolbar |
| 2026-10-07 | 0.3.0. 10 UI languages; shortcut audit 214 → 0 failures; TIFF decode 10× faster |
| 2026-10-05 | 0.2.0, first signed and notarized release. Honest assessment: ready for real work 25–35% |
| 2026-10-03 | Menu breadth 625/625 |
| 2026-10-01 | Menu breadth 224 → 532 of 625 in a day |
| 2026-09-30 | First commit |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-31 | major | Re-measure at 0.18.0: breadth 76 → 79%, ready 45 → 52%, mainstream 43 → 47%, essentials 61 → 63%, beta 1,100–1,800 → 1,350–2,200 h, full 2,300–4,100 → 2,600–4,500 h (backlog growth outweighs what landed). Evidence in [target-app-parity](docs/target-app-parity.md#revision-history) |
| 2026-10-17 | minor | Object Finder retains large enclosed foregrounds (#6548) |
| 2026-10-13 | minor | Focus item 1 quoted the oracle floor as 133 of 256; it is 145 of 258 (`crates/io/tests/it/corpus.rs`, gaps.md G1). Open issues 579 → 953 (2026-10-13) |
| 2026-10-15 | minor | Russian catalog covers every dynamic UI label the other catalogs translate |
| 2026-10-11 | minor | Toolbox count 56 of 68 (12 missing) with the Convert Point tool (#3204) |
| 2026-10-10 | minor | Readiness by audience with hours per tier |
| 2026-10-10 | minor | Mainstream-practitioner and essentials-user numbers added |
| 2026-10-10 | minor | Alpha gate result (passes; stays alpha) in the stage explanation |
| 2026-10-10 | major | First version |
