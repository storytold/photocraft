### IFPUG function-type table (System)

| Type | Name | Count | Complexity | Weight | FP | Basis |
|---|---|--:|---|--:|--:|---|
| EI | External Input | 824 | average | 4 | 3,296 | state-changing engine commands from the registry (907 commands, 12 parameter-only variants collapsed); see fp-transactions.md |
| EO | External Output | 28 | average | 5 | 140 | export, print, save-a-copy, preset/action save, package, contact sheet, render video, image assets; listed in fp-transactions.md |
| EQ | External Inquiry | 43 | average | 4 | 172 | read-only registry commands (info / list / get / inspect / printers / browse); listed in fp-transactions.md |
| ILF | Internal Logical File | 10 | average | 10 | 100 | logical files maintained: document (.pcraft incl. layers/channels/paths/history), preferences, shortcuts+workspaces, brush presets, swatches, gradients, patterns, layer styles, actions, tool/document/shape presets |
| EIF | External Interface File | 11 | average | 7 | 77 | logical groups read from elsewhere: PSD/PSB, flat raster codecs, HDR (EXR/DPX), camera raw/DNG, HEIF, Affinity, Krita/OpenRaster, Photoshop preset files (ABR/ASE/ACO/GRD/PAT/ATN/ASL), ICC profiles, fonts, plug-ins |
| **UFP** | | | | | **3,785** | |

### General System Characteristics

| # | Characteristic | DI (0-5) | Justification |
|--:|---|--:|---|
| 1 | Data communications | 2 | Data communications: local TCP control channel and MCP over stdio; no network service |
| 2 | Distributed data processing | 1 | Distributed data processing: none beyond sandboxed in-process WASM plug-ins |
| 3 | Performance | 4 | Performance: explicit budgets (perf/budgets.toml), 24-36 MP targets, GPU compositor, nightly perf runs |
| 4 | Heavily used configuration | 2 | Heavily used configuration: runs on the user's own machine; varied GPUs handled by wgpu |
| 5 | Transaction rate | 1 | Transaction rate: single-user interactive editing |
| 6 | Online data entry | 3 | Online data entry: all editing is interactive (dialogs, tools, canvas) |
| 7 | End-user efficiency | 4 | End-user efficiency: Photoshop-parity menus, shortcuts, panels, command palette are a stated goal |
| 8 | Online update | 3 | Online update: interactive updates of the document ILF with undo and atomic saves |
| 9 | Complex processing | 4 | Complex processing: imaging algorithms, ICC colour management, compositing, type layout (rated 4, not 5) |
| 10 | Reusability | 3 | Reusability: psd, codecs, cms are designed as publishable standalone crates |
| 11 | Installation ease | 3 | Installation ease: signed installers for macOS, Windows, Linux, FreeBSD, plus a web build |
| 12 | Operational ease | 2 | Operational ease: autosave and recovery; no operator role |
| 13 | Multiple sites | 3 | Multiple sites: five platforms incl. browser from one codebase |
| 14 | Facilitate change | 3 | Facilitate change: data-driven command registry, preferences, actions/scripting |
| | **ΣGSC** | **38** | |

VAF = 0.65 + 0.01 x 38 = **1.03**; AFP = UFP x VAF = 3,785 x 1.03 = **3,899**

System FP carried into the valuation: **3,899** (AFP).

### Backfiring - System

| Language | Physical LOC | LOC/FP | FP (raw physical) | FP (/ 1.3 physical->logical) | Ratio source |
|---|--:|--:|--:|--:|---|
| HTML | 30 | 40 | 1 | 1 | analyst-assumed markup ratio |
| Rust | 256,463 | 55 | 4,663 | 3,587 | analyst-assumed: no SPR/QSM ratio for Rust; nearest SPR peers C++ 55, Java 53 |
| TOML | 8 | 50 | 0 | 0 | analyst-assumed for TOML/YAML/shell/PowerShell/Python/other build assets |
| WebGPU Shading Language | 1,136 | 50 | 23 | 17 | analyst-assumed shader ratio |
| **System total** | **257,637** | | **4,687** | **3,605** | |

Sensitivity band (physical->logical 1.5 .. 1.0): 3,124 .. 4,687 FP; point estimate 3,605 FP.

### Backfiring - DevOps/Test

| Language | Physical LOC | LOC/FP | FP (raw physical) | FP (/ 1.3 physical->logical) | Ratio source |
|---|--:|--:|--:|--:|---|
| Autoconf | 309 | 50 | 6 | 5 | analyst-assumed for TOML/YAML/shell/PowerShell/Python/other build assets |
| Docker ignore | 18 | 50 | 0 | 0 | analyst-assumed for TOML/YAML/shell/PowerShell/Python/other build assets |
| Dockerfile | 48 | 50 | 1 | 1 | analyst-assumed for TOML/YAML/shell/PowerShell/Python/other build assets |
| Powershell | 257 | 50 | 5 | 4 | analyst-assumed for TOML/YAML/shell/PowerShell/Python/other build assets |
| Python | 210 | 50 | 4 | 3 | analyst-assumed for TOML/YAML/shell/PowerShell/Python/other build assets |
| Rust | 200,478 | 55 | 3,645 | 2,804 | analyst-assumed: no SPR/QSM ratio for Rust; nearest SPR peers C++ 55, Java 53 |
| Shell | 933 | 50 | 19 | 14 | analyst-assumed for TOML/YAML/shell/PowerShell/Python/other build assets |
| TOML | 2,669 | 50 | 53 | 41 | analyst-assumed for TOML/YAML/shell/PowerShell/Python/other build assets |
| XML | 33 | 50 | 1 | 1 | analyst-assumed for TOML/YAML/shell/PowerShell/Python/other build assets |
| YAML | 597 | 50 | 12 | 9 | analyst-assumed for TOML/YAML/shell/PowerShell/Python/other build assets |
| **DevOps/Test total** | **205,552** | | **3,747** | **2,882** | |

Sensitivity band (physical->logical 1.5 .. 1.0): 2,498 .. 3,747 FP; point estimate 2,882 FP.

### System reconciliation (IFPUG vs backfiring)

| Method | System FP |
|---|--:|
| IFPUG (AFP) | 3,899 |
| Backfiring (point) | 3,605 |
| Divergence | -7.5% |
| Verdict (±30%) | agree |

System physical LOC per IFPUG FP: 66

### Function points: subtotals and total

| Category | Method | FP | Share |
|---|---|--:|--:|
| System | IFPUG AFP | 3,899 | 57.5% |
| DevOps/Test | backfired equivalent FP | 2,882 | 42.5% |
| **Total** | | **6,780** | 100% |

### Salary inputs

| Source | Annual base (USD) | Note |
|---|--:|---|
| bls | $133,080 | US BLS Occupational Outlook Handbook, software developers median annual wage May 2024 (cited via scaler.com; secondary sites report $135,980 for May 2025), accessed 2026-10-14 |
| payscale | $98,101 | PayScale software engineer average base pay, self-reported (cited via scaler.com; not verified on payscale.com), accessed 2026-10-14 |
| salarycom | $128,709 | Salary.com Software Engineer III US median, Oct 2025 (from a non-official mirror of the Salary.com page), accessed 2026-10-14 |

Blended median base = median of the sources = **$128,709**; burden 1.5x-1.75x; fully-loaded day rate = base x burden / 260 = **$743 - $866**.

### Effort by productivity band

| FP/man-day | ~FP/staff-month | Level | ~System LOC/man-day | System man-months | DevOps/Test man-months | Total man-months (coding-centric) |
|--:|--:|---|--:|--:|--:|--:|
| 0.32 | 7 | Jones full-lifecycle baseline (headline) | 21 | 554 | n/a (in System) | n/a (in System) |
| 0.5 | 11 | below baseline (large / high-ceremony) | 33 | 354 | 262 | 616 |
| 1 | 22 | coding-centric / well-run | 66 | 177 | 131 | 308 |
| 2 | 44 | high-performing small team | 132 | 89 | 65 | 154 |
| 3 | 66 | elite / best-in-class | 198 | 59 | 44 | 103 |

### Cost by productivity band (fully-loaded, burden 1.5x - 1.75x)

| FP/man-day | System | DevOps/Test | Total (coding-centric) |
|--:|---|---|---|
| 0.32 | $9,046,487 - $10,554,235 | n/a (in System) | n/a (in System) |
| 0.5 | $5,789,752 - $6,754,710 | $4,279,995 - $4,993,327 | $10,069,746 - $11,748,037 |
| 1 | $2,894,876 - $3,377,355 | $2,139,997 - $2,496,663 | $5,034,873 - $5,874,019 |
| 2 | $1,447,438 - $1,688,678 | $1,069,999 - $1,248,332 | $2,517,437 - $2,937,009 |
| 3 | $964,959 - $1,125,785 | $713,332 - $832,221 | $1,678,291 - $1,958,006 |

**Headline (System, full-lifecycle 0.32 FP/day): $9,046,487 - $10,554,235.** Coding-centric best case (System + DevOps/Test at 3 FP/day): $1,678,291 - $1,958,006. Range across all System bands: $964,959 - $10,554,235.

### Schedule sanity check

| Quantity | Value |
|---|--:|
| System FP | 3,899 |
| Exponent (mid-to-large commercial desktop product (Jones: ~0.4 mid-sized, ~0.45 large systems)) | 0.42 |
| Calendar months = FP^exp | 32.2 |
| Staff-months at 0.32 FP/day | 554 |
| Implied average team | 17.2 |

Replacement cost, not market value; order-of-magnitude only. Productivity: Capers Jones, "Software Economics and Function Point Metrics" (IFPUG, 2017) for ~7 FP/staff-month full lifecycle; Steve McConnell, "Software Estimation" (2006) for the 10x productivity spread.
