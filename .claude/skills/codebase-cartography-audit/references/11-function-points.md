# Module 11 — Function Point Counting (IFPUG + Jones backfiring)

Size the delivered functionality in **function points (FP)** — a language- and
implementation-independent measure of how much the software *does* for its users
— and validate the number with a second, independently-derived estimate.
Function points complement LOC and complexity (Module 9): LOC measures *how much
code*, FP measures *how much capability*; the ratio between them (LOC/FP) is a
productivity and bloat signal.

Research base: **IFPUG** *Counting Practices Manual* (the formal method,
descended from **Allan Albrecht**'s 1979 work at IBM) and **Capers Jones**
(*Applied Software Measurement*, *Programming Productivity*) — the authority on
LOC-per-FP "backfiring" tables and on using two methods as a mutual checkpoint.

## 11.0-pre — Required output: FULL TABLES, and reproducibility

**Emit every FP figure as a full table — never collapse it to a prose one-liner.**
An LLM's instinct is to summarize ("IFPUG: EI59 EO25 EQ53 ILF18 EIF13, VAF 1.21");
that is *not acceptable* — it hides the counts and makes the estimate unauditable.
The report MUST contain, as literal tables:
1. **IFPUG function-type table** — one row per type (EI/EO/EQ/ILF/EIF) with
   `count | low/avg/high chosen | weight | FP`, and a UFP total row.
2. **GSC table** — all **14** characteristics, each with its 0–5 rating **and a
   one-line justification**, a ΣGSC row, and the VAF computation.
3. **Backfiring table per category** — `category | LOC | LOC/FP | FP`.
4. **Subtotals + total table** — System FP / DevOps-Test FP / Total, with shares.

**Reproducibility — why two analysts (or LLMs) diverge, and how to minimize it.**
Two correct-looking FP counts can differ 40–60%. Observed drivers, in priority
order — control each:
- **LOC-driven (largest):** the DevOps/Test backfiring inherits any LOC error. If
  the Module 9 subtotal doesn't reconcile with its components (phantom LOC from
  generated/vendored/config files), the equivalent FP is inflated 1:1. **Fix LOC
  first** (Module 9 reconciliation + exclusions) before backfiring.
- **Data-function granularity:** count **ILF/EIF as logical files (aggregate
  roots / logical groups), NOT one-per-table**. In PhotoCraft the
  document model's many structs are one ILF (the document) and ~20 raster codecs
  are a few EIF groups, not 20. Over-counting ILF/EIF is the #1 IFPUG inflator.
- **GSC subjectivity:** ΣGSC swings the VAF from 0.65 to 1.35. Rate conservatively,
  show every rating with a justification, and default mid-range (2–3) unless the
  characteristic is clearly high — don't stack 4s and 5s.
- **Backfiring is the more reproducible anchor** than hand-IFPUG (it's mechanical
  once LOC is right). When IFPUG and backfiring diverge, treat backfiring as the
  sanity bound and re-examine the IFPUG counts — the ±30% check "passing" does NOT
  mean the counts are right, only that they're not wildly off. "Re-examine" means
  audit the IFPUG basis (are ILFs table-counted? GSCs stacked high?), **not**
  silently scale IFPUG to match backfiring; if backfiring is high because of code
  bloat, say so rather than laundering it into the FP count.

## 11.0 Two FP subtotals: System vs DevOps/Test

Report function points in the **two categories from Module 9.3b** plus a total:

- **System FP** — count with **IFPUG** (below); this is delivered user
  functionality. Cross-check with backfiring of the System code LOC.
- **DevOps/Test FP** — test suites and build/deploy assets deliver no user
  functionality, so IFPUG does not apply; size them by **backfiring their LOC**
  (§11.3) and label the result **equivalent FP** (a size/effort proxy, not
  features).
- **Total FP = System + DevOps/Test.** Give both subtotals and the total; the
  DevOps/Test share is often large (a heavily-tested system can have more
  test-equivalent FP than product FP).

Everything below is the System (IFPUG) method; §11.3 backfiring is applied per
category.

## 11.1 Why two methods

A single FP number is easy to get wrong (miscount a file type, mis-weight). The
discipline is to compute FP **two independent ways** and confirm they agree
within a tolerance (**±30% is this audit's chosen "same
ballpark" threshold** for an estimate-grade count). If they diverge more,
something is wrong — re-examine the component counts or the LOC baseline.
Agreement validates the estimate; the gap itself is diagnostic (a large
backfiring-over-IFPUG gap usually means generated or duplicated code inflating LOC
without delivering functionality). Note ±30% is a working threshold, not a
guarantee: Jones reports backfiring accuracy for individual projects can vary
±50% or worse, so divergence inside the band is *consistency*, not proof.

## 11.2 Method A — IFPUG Function Point Analysis

Count five **function types**, classify each as low/average/high complexity, and
weight it. For an estimate, count at **average** unless you have data.

| Function type | Low | Avg | High | What it is |
|---------------|----:|----:|-----:|-----------|
| External Input (EI) | 3 | **4** | 6 | user/interface data-in that changes state (a create/update command) |
| External Output (EO) | 4 | **5** | 7 | data-out with derived/calculated content (report, export, statement) |
| External Inquiry (EQ) | 3 | **4** | 6 | data-out that is pure retrieval, no derivation (a lookup/query) |
| Internal Logical File (ILF) | 7 | **10** | 15 | a logical group of data **this app maintains** (an aggregate/entity group) |
| External Interface File (EIF) | 5 | **7** | 10 | a logical group of data **another app maintains** that this one reads |

**Unadjusted FP (UFP)** = Σ (count × weight).

**Value Adjustment Factor (VAF)** from the 14 General System Characteristics,
each rated 0–5 (Degree of Influence): data communications, distributed
processing, performance, heavily-used config, transaction rate, online data
entry, end-user efficiency, online update, complex processing, reusability,
installation ease, operational ease, multiple sites, facilitate change.

```
VAF = 0.65 + 0.01 × ΣGSC          (ranges 0.65 … 1.35)
Adjusted FP (AFP) = UFP × VAF
```

**Rate the 14 GSCs conservatively and SHOW them.** Emit all 14 as a table with
each 0–5 rating and a one-line justification; do not report VAF as a bare number.
Default to **2–3** unless a characteristic is clearly high — piling on 4s/5s
inflates VAF toward 1.35 and is the second-biggest IFPUG divergence source between
analysts. Note: IFPUG CPM 4.3+/ISO 20926 make VAF **optional** and benchmark on
**UFP**; if you carry AFP into Module 12, say so and keep it consistent.

**Deriving the counts from code (estimate-grade heuristics):**
- **EI** ← state-changing transactions. In PhotoCraft: engine commands that modify
  the document/session (registry: `photocraft-cli commands --json`). Count the
  *business transaction*, not every variant: collapse families that differ only by
  a parameter and say how.
- **EQ** ← retrieval queries/lookups (read-only commands, info/histogram/layer-list
  reads, MCP read tools).
- **EO** ← outputs with **derived/calculated** content only (reports, statements,
  exports, dashboards). A plain list/grid of stored rows is an EQ, not an EO —
  don't double-file the same read as both.
- **ILF** ← logical data groups the app **maintains**, counted as **aggregate
  roots / logical files, emphatically NOT one-per-struct**. PhotoCraft: the document
  (`.pcraft`), preferences, preset libraries, actions, workspaces - a handful.
  This is the single biggest IFPUG inflator — if your ILF count approaches the
  table count, you are counting wrong.
- **EIF** ← logical groups of data owned by **another** app that this one reads,
  grouped logically: PSD/PSB, flat raster codecs, camera raw, HEIF, Affinity,
  Krita/OpenRaster, ICC profiles, fonts, plug-ins. See Module 0.3.

**Sanity bounds (flag if exceeded):** ILF+EIF together are usually a small
multiple of the number of aggregate roots, and total data-function FP (ILF+EIF)
rarely exceeds transaction-function FP (EI+EO+EQ) for a line-of-business app. If
data functions dominate, you're table-counting — regroup.

Document the basis for every count (grep results, handler lists) so the estimate
is auditable.

## 11.3 Method B — Capers Jones backfiring (LOC → FP)

Backfiring converts measured LOC to FP using Jones/SPR empirical median
**LOC-per-FP** ratios by language:

| Language | ~LOC/FP | source | | Language | ~LOC/FP | source |
|----------|--------:|--------|-|----------|--------:|--------|
| C# | 54 | SPR table | | JavaScript/TS | 47 | SPR table |
| Java | 53 | SPR table | | Python | ~42 | analyst-assumed* |
| C++ | 55 | SPR table | | SQL | 13–21 | SPR (varies) |
| Markup (Razor/HTML) | ~40 | analyst-assumed* | | Go | ~45 | analyst-assumed* |
| **Rust** | **~55** | analyst-assumed* (peers C++ 55, Java 53) | | WGSL shaders | ~50 | analyst-assumed* |

*Ratios marked analyst-assumed do not trace to a specific SPR/QSM edition (Python/
Go/Rust post-date the classic SPR table; QSM medians differ) — cite the exact table you
use and widen the sensitivity band for these.

```
FP_backfired = Σ over languages ( LOC_lang / LOCperFP_lang )
```

**Physical vs logical LOC.** The SPR ratios are calibrated to *logical statements*;
scc counts *physical* code lines, which for Rust (rustfmt, one chained call per line) run ~1.2–1.5× logical — so raw
backfiring from scc over-states FP by 20–50%. Either apply a documented physical→
logical factor or use physical-LOC-calibrated ratios (QSM), and say which; keep it
consistent across categories. Always report a sensitivity band, not a single
number — Jones himself cautions backfiring is approximate.

**Run per category (Module 11.0), not "production only":** backfire the System-code
LOC (cross-checks IFPUG) *and* the DevOps/Test-code LOC (its primary equivalent-FP
size) separately; exclude only generated/vendored code from both.

## 11.4 Reconciliation & interpretation

- Report both numbers, the % divergence, and a verdict (agree ≤30% / review).
- **LOC per delivered FP** = productionLOC ÷ AFP — a bloat/productivity signal.
  Much higher than the language's Jones ratio ⇒ generated/duplicated code or
  over-engineering (cross-link Modules 4, 7).
- FP gives a size-normalized denominator for other metrics: **defects per FP**,
  **cost per FP**, **FP per developer-month** — useful for planning the refactor
  backlog and for comparing this system to industry baselines.
- Keep it estimate-grade and say so; a certified IFPUG count is a manual
  exercise. The value here is a defensible order-of-magnitude size, validated two
  ways.

## 11.5 Implementation

Computed by **`scripts/sizing.rs`** (std-only Rust, compiled by `setup-tools.sh`).
Inputs: the analyst's counts, GSCs, ratios and salaries in
`metrics/sizing-inputs.txt` (template: `templates/sizing-inputs.txt`) and the
per-category, per-language LOC that `metrics` wrote to `metrics/category-language.tsv`.
It prints the IFPUG table, the GSC table + VAF, the per-category backfiring tables +
sensitivity band, the System reconciliation, the **two subtotals + total** (§11.0)
and the Module 12 valuation, all as ready-to-paste Markdown in `metrics/sizing.md`;
**the README embeds that output verbatim — do not re-summarize it in prose.** State which figure feeds Module 12 — modern IFPUG (CPM 4.3+)
and ISO 20926 treat VAF/AFP as optional and often benchmark on **UFP**; if you
carry AFP through, say so consistently.

**Inspection prompt**
> Size the application in function points, in two categories (Module 9.3b) and two
> methods. (A) IFPUG for the SYSTEM: derive EI/EO/EQ/ILF/EIF counts (command
> handlers, queries, reports, app-owned aggregates, external data sources —
> document each basis), weight at average, estimate the 14 GSCs, compute
> UFP → VAF → AFP. (B) Backfiring per category: System LOC → FP (cross-checks
> IFPUG); DevOps/Test LOC → **equivalent FP** (its primary size), with a
> sensitivity band and the physical-vs-logical caveat. Report: System
> reconciliation (% divergence, agree within ±30%?), then **System FP + DevOps/Test
> FP + Total** and the System LOC-per-FP ratio. Run `sizing` and cite it.
