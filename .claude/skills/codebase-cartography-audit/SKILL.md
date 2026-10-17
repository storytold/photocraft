---
name: codebase-cartography-audit
description: >-
  Full cartographic audit of the PhotoCraft Rust workspace (adapted from the .NET
  original for this codebase and this Linux cloud container). Produces a
  self-contained report directory with: a complete inventory split into System vs
  DevOps/Test code (inline #[cfg(test)] modules carved out of System; LOC /
  function-point / cost subtotals + total); five C4-PlantUML views (logical,
  runtime, dependencies, testing, build/devops/deploy) plus a metrics-annotated
  logical view, rendered to PNG and visually inspected for legibility; a
  maintainability-defect and code-smell analysis grounded in SOLID, Onion & Clean
  Architecture (checked against PhotoCraft's own L0-L6 layering), Fowler smells,
  enterprise patterns, the project's never-crash rules and the testing pyramid;
  tool-measured metrics (scc LOC, Rust McCabe complexity, clippy, jscpd
  duplication, test counts from `cargo test -- --list`, a Function Point count via
  IFPUG cross-checked with Capers Jones backfiring, and an economic valuation in
  staff-months and dollars from Capers Jones productivity rates and current median
  engineer salaries). All analysis helpers are std-only single-file Rust programs
  compiled with rustc (no Python/PowerShell/.NET). Use when onboarding to PhotoCraft,
  reviewing AI-written changes at scale, or planning a refactor and you want both a
  principle-linked defect backlog AND measured, diagrammed evidence.
---

# Codebase Cartography Audit (PhotoCraft edition)

Evidence-backed, principle-linked defect catalogue plus **measured metrics** (LOC,
cyclomatic complexity, lint, duplication, test counts, all from real tools), **five
C4 architecture diagrams** rendered to inspected PNGs, and a **single self-contained
report directory** with the diagrams embedded in a README.

Deliverable: a new directory (default `./codebase-audit-report/` at the repo root)
containing `README.md` (the report, with embedded diagram PNGs), `images/` (rendered
PNGs), `diagrams/` (PlantUML sources), and `metrics/` (raw tool output and helper
output). Every claim cites `file:line` or a reproducible command. Do not commit the
report unless the user asks.

**Read `references/00-photocraft-context.md` first.** It maps every lens onto this
codebase (crate layers, the command registry, the never-crash rules, how tests are
layered, and the function-point basis for a desktop editor).

## This machine (Linux cloud container) - toolchain, verified

`scripts/setup-tools.sh` installs and versions everything without touching the repo:

| Tool | How it is acquired | Used for |
|------|--------------------|----------|
| scc 3.5.0 | `go install github.com/boyter/scc/v3@v3.5.0` -> `~/go/bin/scc` | LOC per file/language |
| Graphviz | `apt-get install graphviz` | PlantUML layout |
| PlantUML 1.2025.4 | release jar -> `~/.local/share/plantuml/plantuml.jar` (bundles `<C4/...>`) | diagrams, offline |
| jscpd 4.0.5 | `npx -y jscpd@4.0.5` (on demand, never added to the repo) | clone blocks |
| clippy | rustup toolchain already present | lint |
| `metrics`, `sizing`, `testlist` | `rustc -O --edition 2021 scripts/<name>.rs` -> `~/.cache/cartography/bin` | rollup, FP + valuation, suite matrix |

`scripts/measure.sh [repo] [report]` then runs every measured step and writes raw
output to `<report>/metrics/` with the exact commands in `metrics/commands.txt` and
versions in `metrics/versions.txt`. Rough cost on a cold cache: scc seconds; jscpd
~4 min; clippy and the per-package test listing build the workspace into
`target/agent-audit` (built without debuginfo or incremental data, because a default debug build exceeded this session's disk allowance; still several GB and tens of minutes). Run `measure.sh` in the background
and do the reading-heavy stages meanwhile. Steps can be skipped with `SKIP_CLIPPY=1`,
`SKIP_TESTS=1`, `SKIP_JSCPD=1`; a skipped step is reported as skipped, never guessed.

## The inspection catalogue

| # | Module | File | Focus |
|---|--------|------|-------|
| 0 | PhotoCraft context | `references/00-photocraft-context.md` | how every lens maps here |
| 1 | SOLID (SRP, OCP, LSP, ISP, DIP) | `references/01-solid.md` | Martin |
| 2 | Onion Architecture | `references/02-onion-architecture.md` | Palermo |
| 3 | Clean Architecture & component rules | `references/03-clean-architecture.md` | Martin |
| 4 | Fowler code smells | `references/04-code-smells.md` | Fowler / Beck |
| 5 | Enterprise & domain patterns | `references/05-enterprise-patterns.md` | Fowler, Evans |
| 6 | Persistence, IoC & low-ceremony | `references/06-jeremy-miller.md` | Miller |
| 7 | AI-code anti-patterns | `references/07-ai-antipatterns.md` | (composite) |
| 8 | Testing & the automation pyramid | `references/08-testing.md` | Palermo, Martin, Cohn |
| 9 | Inventory, metrics & linting (tools) | `references/09-inventory-and-metrics.md` | measured |
| 10 | C4 diagrams (PlantUML) + rendering | `references/10-c4-plantuml-diagrams.md` | diagrammed |
| 11 | Function points (IFPUG + Jones backfiring) | `references/11-function-points.md` | sized |
| 12 | Economic valuation (effort, cost, replacement value) | `references/12-economic-valuation.md` | priced |

Modules 1-7 are written generically (some examples are .NET); apply them through the
Rust/PhotoCraft reading in Module 0.2.

## How to run the audit (ordered pipeline)

Do these in order; later stages consume earlier outputs. Fan the read-only
investigation stages out to parallel subagents (one per crate group) where useful.

1. **Scope & tools.** Run `scripts/setup-tools.sh`, then start
   `scripts/measure.sh . codebase-audit-report` in the background. Read `AGENTS.md`,
   `docs/architecture.md`, `xtask/src/layers.rs`. The report directory skeleton
   (`README.md`, `images/`, `diagrams/`, `metrics/`) is created by `measure.sh`.

2. **Map the architecture (Modules 2 & 3).** Reconstruct the crate graph from
   `metrics/cargo-metadata.json` (or the `Cargo.toml` `[dependencies]`) and compare it
   with the L0-L6 layering; read `metrics/layers.txt` (`cargo xtask layers`). Identify
   the domain core (`doc`, L2 services), the application layer (`engine`),
   infrastructure (`io`, `format`, `codecs`, `psd`, `gpu`, `plugins`) and delivery
   (`ui-egui`, `automation`, `apps/*`), and any dependency-direction violations,
   including ones the layer check cannot see (behaviour in the UI that bypasses
   commands).

3. **Measure (Modules 9 & 11 & 12).** From `metrics/`: `component-rollup.txt`
   (per-component LOC, types, fns, McCabe CC, CC/type, density, hottest file,
   reconciliation, Excluded bucket, >2k-LOC files, top-15 complex files),
   `clippy-by-lint.txt`, `jscpd/jscpd-report.json`, `test-suites.md`. Fill
   `templates/sizing-inputs.txt` into `metrics/sizing-inputs.txt` (IFPUG counts with
   their basis from the command registry, 14 GSCs with justifications, salary medians
   looked up live and cited) and run
   `~/.cache/cartography/bin/sizing metrics/sizing-inputs.txt metrics/category-language.tsv > metrics/sizing.md`.
   Classify all code into **System vs DevOps/Test** (done by `metrics`; inline
   `#[cfg(test)]` modules count as DevOps/Test) and report a **subtotal for each plus
   a total** for LOC, function points, and cost. For **cost**, the per-band Total
   column is coding-centric only - at the 0.32 full-lifecycle row the DevOps/Test and
   Total cells are "n/a (in System)" (never sum them; Module 12.2). Productivity bands
   span **0.32-3.0 FP/man-day**.

4. **Analyze defects & smells (Modules 0.2, 1, 4, 5, 6, 7).** Follow the risk - the
   high-LOC/high-complexity components and files from step 3 first. Sweep the cheap
   high-signal checks (never-crash violations from the clippy restriction lints,
   `unsafe` outside `tablet`, duplication, hard-coded colours/config, god modules),
   then the principle checks. Every finding: `file:line`, principle, why-it-hurts,
   named fix, effort/risk. **Report each defect once** under its most-specific
   principle and cross-reference the others.

5. **Assess the safety net (Module 8).** Use `metrics/test-suites.md` (counts from
   `cargo test -p <pkg> -- --list`, layer by evidence). Check its classification by
   reading a sample of each layer; correct and note any misfiled suite. Add what the
   runner listing cannot see: `panic_hunt` (ignored), `corpus`-feature tests, fuzz
   crates, perf budgets. Determine whether the risky code is covered. Thin coverage
   => the backlog leads with characterization tests.

6. **Diagram (Module 10).** Author the five views plus the metrics-annotated logical
   view with the C4-PlantUML library, each showing the **System vs DevOps/Test**
   split via separate `System_Boundary`s with subtotal labels. ASCII labels only.
   Render with
   `java -jar ~/.local/share/plantuml/plantuml.jar -tpng -o ../images codebase-audit-report/diagrams/*.puml`.
   **Read each rendered PNG and verify legibility**; fix source and re-render any that
   fail. Record that each was visually inspected.

7. **Assemble the report.** Write `codebase-audit-report/README.md` embedding the
   verified PNGs (`![](images/xx.png)`), followed by the executive summary,
   architecture map, metrics tables, prioritized findings, and the sequenced
   refactoring backlog. **Before delivering, run this self-check** - any missing item
   means the audit is not done: (i) the 5-row IFPUG function-type table; (ii) the
   14-row GSC table + VAF math; (iii) per-category backfiring tables; (iv) the printed
   `Σ(components)==subtotal` reconciliation lines and an Excluded (non-authored)
   bucket; (v) the per-band cost table with the 0.32-row cells marked "n/a (in
   System)"; (vi) System/DevOps-Test/Total for LOC, FP, and cost; (vii) a legibility
   verdict for each of the 6 PNGs you actually opened. Then delete
   `target/agent-audit` if disk is tight.

## Finding format (required for every defect)

```
### [SEVERITY] <short title>
- **Principle:** <e.g. DIP - Dependency Inversion (Module 1)>
- **Location:** crates/engine/src/foo_cmds.rs:120-148  (+ other sites if duplicated)
- **Evidence:** <the concrete smell - quote the code or describe precisely>
- **Why it hurts:** <maintainability/bug-surface/duplication cost>
- **Fix:** <the specific refactoring move, named where possible>
- **Effort / Risk:** <S/M/L, and whether tests exist to make it safe>
```

Severity: **Critical** (bug/security/data-loss, including any panic path reachable
from a file, command param or MCP input), **High** (blocks change / spreads widely),
**Medium** (localized), **Low** (cosmetic).

## Report structure (`codebase-audit-report/README.md`)

1. **Title + how-to-reproduce** - tools used (name+version from `metrics/versions.txt`
   and `setup-tools.sh`) and the commands (`metrics/commands.txt`), so every metric and
   diagram can be regenerated.
2. **Executive summary** - overall health, 3-5 systemic problems, single
   highest-leverage fix.
3. **Architecture diagrams** - the five embedded PNGs + the annotated logical PNG,
   each with a one-paragraph reading.
4. **Inventory & metrics** - language breakdown, per-component LOC/complexity/density
   rollup (with type count & avg CC/type), top-10 most-complex files, clippy summary
   by lint, duplication clones, the Function Point sizing, and the economic valuation.
   **Present LOC, function points, and cost each as System / DevOps-Test / Total**.
5. **Prioritized findings** - grouped by module, sorted by severity x spread.
6. **Testing assessment** - suite matrix, pyramid shape, coverage gaps,
   refactor-safety verdict.
7. **Refactoring backlog** - sequenced, "characterize with tests first" flagged
   wherever coverage is thin; tie items to `docs/gaps.md` entries where they overlap.

## Output fidelity rules (avoid these common LLM failure modes)

1. **Emit FULL tables, never prose summaries, for quantitative sections.** Paste the
   `sizing` and `metrics` Markdown verbatim; do not collapse it to "EI 300 EO 60 ...".
2. **Reconcile every subtotal with its parts.** `metrics` prints the
   `Σ(components) == subtotal` checks and the identity against the whole-repo scc
   total; a MISMATCH is a bug to fix before reporting.
3. **Exclude non-authored code** (generated tables, JSON/SVG/text data, docs) - the
   `metrics` Excluded bucket does this; review the >2k-LOC list and add exclusions in
   `classify()` if a large file is not authored source.
4. **Count ILF/EIF as logical files, not structs or formats-per-codec**; rate GSCs
   conservatively and show them.
5. **Do not present a full-lifecycle Total that sums System + DevOps/Test.**
   Headline = System full-lifecycle alone.
6. **Actually inspect each rendered PNG** (Read the image) before claiming it passed.
7. **Every objective number traces to a tool command**; qualitative findings each cite
   `file:line`. When IFPUG and backfiring disagree, backfiring is the sanity bound -
   re-examine the IFPUG basis, never scale it to match.

## Guiding stance

- **Evidence over opinion.** Every defect cites code; every number cites a command.
- **Objective counts come from tools, never inference.** LOC, types, complexity,
  test counts, lint, function points and cost each come from a command or a committed
  helper and are captured in `metrics/`. Count tests with the runner
  (`cargo test -- --list`), not by grepping `#[test]`.
- **Measure, then follow the risk.** The highest LOC x complexity component, touched
  by every feature, outranks a tidy leaf module.
- **Judge PhotoCraft by its own rules too.** `AGENTS.md` (never-crash, everything is a
  command, thin UI, no format/colour assumptions, wasm-clean, layering) is the local
  definition of done; violations of it are findings with the rule cited.
- **Diagrams must be readable.** A clipped, overlapping, or mislabelled PNG is a
  defect to fix, not ship.
- **Refactor behind tests.** Where the net is thin, the first backlog item is
  characterization tests.
- **Prefer removing over adding.** The best outcome is less code.
- **Never mutate the target repo** to gather metrics; tools live in `~/go/bin`,
  `~/.local/share/plantuml`, `~/.cache/cartography` and the npx cache; build output
  goes to `target/agent-audit` (gitignored).
- **Analysis helpers are single-file, std-only Rust** (`scripts/*.rs`, compiled with
  `rustc`): consistent with PhotoCraft's Rust-only rule and needing no extra runtime.
  Shell is used only for orchestration glue (`setup-tools.sh`, `measure.sh`).

See `PROMPT.md` for a portable single-paste version of this whole pipeline.
