# Portable Cartography-Audit Prompt (standalone, PhotoCraft edition)

Paste into any capable coding-agent session in the PhotoCraft repo when you can't load
the skill. It compresses the full pipeline: map -> measure -> analyze -> diagram ->
assemble. The scripts it names live in `.claude/skills/codebase-cartography-audit/`.

---

You are performing a **full cartographic audit** of the PhotoCraft Rust workspace.
Produce a **self-contained report directory** `codebase-audit-report/` with embedded,
rendered architecture diagrams and measured metrics. Read `AGENTS.md`,
`docs/architecture.md` and the skill's `references/00-photocraft-context.md` first.
Do not mutate the repo (no new dependencies, no tool installs inside it); use
`CARGO_TARGET_DIR=target/agent-audit`; exclude `target`, `corpus`, `.git`, `.claude`
and the report directory from every metric. Don't commit the report unless asked.

**Output-fidelity rules (do not skip - these are the usual failure modes):**
(a) Emit **full tables**, not prose summaries - paste the `sizing` and `metrics`
Markdown verbatim (IFPUG table, all 14 GSCs with justifications + VAF math,
per-category backfiring tables, subtotals+total). (b) **Reconcile every subtotal**:
`metrics` prints `Σ(components) == subtotal` per category and against the whole-repo
scc total; a MISMATCH must be fixed, not reported. (c) **Exclude non-authored code**
(generated Rust, JSON/SVG/text data, docs); review every file >2k LOC. (d) Count
**ILF/EIF as logical files** (the document is one ILF; codecs are a few EIF groups),
rate GSCs conservatively (default 2-3) and show them. (e) **Never sum System +
DevOps/Test at the full-lifecycle rate** - headline is System-only. (f) **Actually
open/Read each rendered PNG** before claiming it passed.

Run this ordered pipeline:

1. **Tools + measurement.** Run `scripts/setup-tools.sh` (installs scc 3.5.0 via
   `go install`, Graphviz via apt, PlantUML 1.2025.4 jar, checks jscpd 4.0.5 via npx,
   compiles the std-only Rust helpers `metrics`, `sizing`, `testlist` with `rustc`
   into `~/.cache/cartography/bin`). Start
   `scripts/measure.sh . codebase-audit-report` in the background: it runs scc,
   `metrics` (System / DevOps-Test / Excluded rollup with inline `#[cfg(test)]`
   modules carved out of System, Rust McCabe CC, types, fns, density, reconciliation,
   >2k-LOC list, top-15 complex files), `cargo xtask layers`, `cargo metadata`,
   clippy with pedantic + never-crash restriction lints grouped by lint id, jscpd
   over `crates apps xtask`, and `cargo test -p <pkg> -- --list` (and `--ignored`)
   for every package -> `testlist` suite matrix. Every command lands in
   `metrics/commands.txt`, versions in `metrics/versions.txt`.

2. **Architecture map.** Rebuild the crate graph from `metrics/cargo-metadata.json`
   and compare it with the L0-L6 layering (`xtask/src/layers.rs`,
   `metrics/layers.txt`). Do dependencies point inward to `doc`/L2 and through
   `engine`? Where does behaviour bypass the command registry (UI logic that should
   be a command)? Cycles?

3. **Sizing (function points + valuation).** Copy `templates/sizing-inputs.txt` to
   `metrics/sizing-inputs.txt` and fill it: (A) **IFPUG** counts with a written basis -
   EI from state-changing commands (`cargo run -q -p photocraft-cli -- commands --json`,
   families collapsed), EQ read-only commands/queries, EO export writers/derived
   outputs, ILF logical files PhotoCraft maintains, EIF foreign formats/ICC/fonts/
   plug-ins, weighted average; 14 GSCs. (B) Backfiring ratios (Rust 55 analyst-assumed,
   peers C++ 55/Java 53), physical->logical factor. (C) Salaries looked up live (US
   BLS, PayScale, Salary.com; note aggregators), each with figure and access date;
   burden 1.5-1.75x; schedule exponent. Run
   `~/.cache/cartography/bin/sizing metrics/sizing-inputs.txt metrics/category-language.tsv > metrics/sizing.md`.
   It prints IFPUG + GSC/VAF, backfiring per category with a sensitivity band, the
   ±30% reconciliation, System/DevOps-Test/Total FP, effort and cost for 0.32 / 0.5 /
   1 / 2 / 3 FP/day (0.32 row DevOps/Test and Total = "n/a (in System)"), the
   headline, and the schedule check. Cite Capers Jones ("Software Economics and
   Function Point Metrics", IFPUG 2017; ~7 FP/staff-month full lifecycle) and Steve
   McConnell ("Software Estimation") for the 10x spread. Replacement cost, not market
   value. If IFPUG and backfiring diverge >30%, re-examine the IFPUG basis; never scale
   it to match.

4. **Defects & smells.** Following the highest LOC x CC components first, audit:
   SOLID; Onion/Clean dependency direction; Fowler smells (long function, large
   module, primitive obsession, data clumps, duplication from jscpd, dead code,
   feature envy, shotgun surgery); enterprise/domain patterns (command, memento/undo,
   unit of work in saves, value objects, anemic `doc`); PhotoCraft's own rules as
   Critical when broken: panics reachable from input (`unwrap`/`expect`/`panic!`/
   unchecked indexing/arithmetic, unbounded recursion), `unsafe` outside
   `crates/tablet/src/macos.rs`, sRGB/`u8`-only assumptions, hard-coded colours, UI
   behaviour not behind a command, wasm-breaking code. Every finding: `file:line`,
   principle, why-it-hurts, named refactoring, effort/risk (S/M/L). Rank by severity x
   spread. Severity: **Critical** (bug/security/data-loss/panic on input), **High**
   (blocks change / spreads widely), **Medium** (localized), **Low** (cosmetic).

5. **Testing.** From `metrics/test-suites.md` (runner counts; layer by evidence:
   acceptance = spawns app binary / control socket / full egui shell; integration =
   GPU, real files/corpora, io round trips; unit = in-memory), spot-check each layer
   and correct misfiles. Add what the default listing misses: `#[ignore]` tests
   (`panic_hunt`), `corpus`-feature tests, fuzz crates, perf budgets. Report pyramid
   shape, whether the hottest code is covered, boundary tests at file/GPU/plug-in/
   control seams, flakiness sources (time, randomness, sleeps, GPU variance). Thin
   coverage => backlog leads with characterization tests.

6. **Diagrams (PlantUML -> PNG, inspected).** Under `diagrams/`: (1) logical C4
   component (crates by layer), (2) runtime container (desktop app, control server,
   CLI, MCP, web/WASM, plug-ins, GPU, files, fonts/ICC), (3) dependencies (crate graph,
   L0-L6 bands, violations/cycles), (4) testing (suites by layer, pyramid), (5)
   build/devops/deploy (`.github/workflows`, xtask, packaging, release). Plus a
   logical-annotated view with LOC and CC per box from `metrics/component-rollup.txt`.
   `!include <C4/...>`, ASCII labels only, System vs DevOps/Test as separate
   `System_Boundary`s with subtotal labels. Render:
   `java -jar ~/.local/share/plantuml/plantuml.jar -tpng -o ../images codebase-audit-report/diagrams/*.puml`.
   **Read each PNG** - no clipping, no overlap, correct arrows, readable annotations,
   sane aspect ratio; fix and re-render failures (group L0 crates or split wide views).

7. **Assemble `codebase-audit-report/README.md`**: how-to-reproduce (tools+versions+
   commands); executive summary (3-5 systemic problems + single highest-leverage
   fix); the embedded PNGs (`![](images/xx.png)`) each with a reading; inventory &
   metrics - **LOC, function points and cost each as System / DevOps-Test / Total** -
   plus the per-component rollup, top-10 complex files, clippy by lint, duplication
   clones; prioritized findings; testing assessment (suite matrix); sequenced
   refactoring backlog flagging where characterization tests come first, linked to
   `docs/gaps.md` where they overlap.

**Self-check before delivering** (any missing item = not done): the 5-row IFPUG
table; the 14-row GSC table + VAF math; per-category backfiring tables; the printed
`Σ(components)==subtotal` reconciliation + the Excluded bucket; the per-band cost
table with 0.32-row cells marked "n/a (in System)"; System/DevOps-Test/Total for LOC,
FP, and cost; and a legibility verdict for each of the 6 PNGs you actually opened.

Deliver the path to the finished report directory and a short summary of the worst
findings.
