# Module 12 — Economic Valuation (effort, cost, replacement value)

Turn the function-point size (Module 11) into **effort (staff-months)**, a
**build cost / replacement-investment range ($)**, and a **schedule sanity
check** — so the audit can state what the software would cost to rebuild at
market rates. This frames the maintainability debt in money: a system that costs
$X to replace but is expensive to change has quantifiable technical debt.

Be explicit about what this is: an **estimate of cost-to-build / replacement
value**, derived from published productivity and salary research. It is **not** a
business-value or market appraisal, and not a certified estimate. Present it as
order-of-magnitude with an honest range.

## 12.1 Effort — MULTIPLE productivity bands (Capers Jones + Steve McConnell)

Effort = **Function Points ÷ productivity rate**. Productivity varies **more than
10×** with project size, domain, ceremony, and team skill, so a single rate is
misleading — always report a **band of several levels**, expressed in **FP per
man-day** (convert with ~22 man-days/month, ~260 workdays/year). A defensible
band uses **four levels from 0.5 → 3.0 FP/man-day**:

Include a band row **at the full-lifecycle baseline** so the recommended headline
is a row you can read off (not a rate outside the band):

| FP/man-day | ≈ FP/staff-month | Level |
|-----------:|-----------------:|-------|
| 0.32 | ~7 | **Jones full-lifecycle baseline (recommended headline)** |
| 0.50 | ~11 | below baseline (large / high-ceremony) |
| 1.00 | ~22 | coding-centric / well-run |
| 2.00 | ~44 | high-performing small team |
| 3.00 | ~66 | elite / best-in-class small project |

Citations (**put both in the report**):
- **Capers Jones**, *"Software Economics and Function Point Metrics," IFPUG (2017)*;
  *"Applied Software Measurement," 3rd ed.*: full-lifecycle average ≈ **7
  FP/staff-month (~0.32 FP/day)**; coding-only ≈ 25 FP/month (~1.1 FP/day);
  >10 FP/staff-month good, <7 poor, some Agile >15. Jones' *low end* for large /
  high-compliance work is well below 7 FP/month, so treat 0.5 FP/day as "below
  baseline," not Jones' floor.
- **Steve McConnell**, *"Software Estimation: Demystifying the Black Art"*; *"Code
  Complete"*: cite for the **order-of-magnitude productivity spread** by size/
  domain/team (his central, well-supported claim). Do **not** attribute a "1 FP/day
  nominal" figure to McConnell — he reports productivity in LOC/staff-month by
  project size (ISBSG/Putnam), not FP/day; the ~1 FP/day figure is nearer Jones'
  coding-only/agile rate.

Tie the FP/day rates back to **LOC per man-day** using the measured LOC/FP from
Module 11.4 (e.g. at ~56 LOC/FP, 0.32–3.0 FP/day ≈ 18–168 LOC/day). This makes the
productivity assumption concrete and shows why "125 LOC/day vs 10 LOC/day" swings
the cost by an order of magnitude. Apply every band to the System FP and the
DevOps/Test FP separately (Module 11.0) — except the **0.32 full-lifecycle row**,
where DevOps/Test is marked "n/a (in System)" in both the effort and cost tables
(§12.2), because Jones' lifecycle rate already includes writing the system's tests.

**Two anchors to call out explicitly**, because they differ ~3× and readers
conflate them:
- **Full-lifecycle replacement (recommended headline)** — Jones' 7 FP/staff-month
  (0.32 FP/day) counts requirements→design→code→test→docs→PM.
- **Coding-centric best case** — elite team, coding-dominated (~3 FP/day).

## 12.2 Cost — median salary surveys × burden

Fully-loaded monthly cost per engineer = **annual base salary × burden ÷ 12**
(or day rate = annual × burden ÷ 260 workdays).

- **Base salary:** use current medians from named surveys and cite each with its
  figure and access date. Anchor on generalist-median sources (PayScale,
  Salary.com) and a government source (US BLS software developer), and note the
  higher aggregator numbers (Indeed, ZipRecruiter, Glassdoor) for range. Pick a
  blended median base and say how you blended it. **Look these up live** (web
  search) rather than hardcoding stale numbers — salaries move.
- **Burden multiplier:** 1.4–2.0 fully-loaded (benefits, payroll tax, overhead,
  facilities, tooling). Default **1.5× (lean)** to **1.75× (typical)**.

Cost = effort × fully-loaded rate. Report cost **for every productivity band**
(not a single trio) AND **split into the two categories from Module 9.3b —
System, DevOps/Test, and Total** — using each category's FP subtotal (Module
11.0). Each band row shows System $, DevOps/Test $, and Total $.

**Avoid double-counting — and never present a full-lifecycle Total that sums both
categories.** Capers Jones' full-lifecycle rate (7 FP/staff-month) *already
includes writing the system's tests*. So:
- The **System full-lifecycle figure is the recommended headline replacement
  cost** (it already covers testing effort). Report it alone as the headline.
- A per-band **Total column is coding-centric only** (each category sized as
  separate code-production effort at that FP/day rate) — valid because at
  coding rates the two bodies of code are distinct build efforts. Label the Total
  column "coding-centric" so no one reads it as a lifecycle cost.
- **Do NOT** compute a Total at the 0.32 full-lifecycle row by adding System +
  DevOps/Test — that double-counts test-writing (it's already inside the System
  lifecycle figure). At the 0.32 row, show the System headline and leave the
  DevOps/Test full-lifecycle cell blank or marked "n/a (in System)".

Then give the coding-centric best case and a single headline range.

## 12.3 Schedule sanity check

Capers Jones' rule of thumb: **calendar months ≈ FunctionPoints ^ exponent**,
where the exponent rises with project class/ceremony — roughly **0.32** for small
agile, **~0.4** mid-sized, up to **~0.45** for large military/systems work. Pick
the exponent to match the audited system's class and state it. Implied **average
team size = staff-months ÷ calendar months**. If the implied team is absurd (e.g.
40 people for 500 FP), the effort estimate or the productivity rate is off —
re-check. A plausible team shape validates the whole valuation.

## 12.4 Implementation & honesty

- Computed by **`scripts/sizing.rs`** together with Module 11 (same run, same
  `metrics/sizing.md`). It takes FP (from Module 11), the salary lines (`salary.<source>`,
  blended as their median), and the burden band from `sizing-inputs.txt`; it prints the effort table and a **per-band cost table
  with System / DevOps-Test / Total columns** (not a single trio), the two anchors,
  and the schedule check — as **ready-to-paste Markdown that the README embeds
  verbatim** (do not re-summarize in prose).
- **Do not price the DevOps/Test subtotal at full-lifecycle rates and also count
  it inside the System headline** — the System full-lifecycle figure already
  includes writing the system's tests. Price DevOps/Test with a coding-centric
  rate, label it the more speculative figure, and keep the System full-lifecycle
  number as the headline.
- **Cite every external number** (Jones for productivity; each salary survey with
  its value) so the estimate is auditable.
- State the caveat: replacement cost, not market value; order-of-magnitude; and
  that maintainability debt (the findings) makes *changing* the code cost more per
  FP than a clean rebuild — which is the debt the audit quantifies.

**Inspection prompt**
> Using the Module 11 function-point size, estimate cost-to-build / replacement
> value across MULTIPLE productivity bands. (1) Effort: man-days = FP ÷ FP-per-
> man-day for a band of 0.32 / 0.5 / 1 / 2 / 3 FP/day incl. the 0.32 baseline row
> (convert to man-months at ~22 days); cite Capers Jones ("Software Economics and
> Function Point Metrics", IFPUG 2017; full-lifecycle ~7 FP/staff-month) AND Steve
> McConnell ("Software Estimation") for the 10× productivity spread (not a 1 FP/day
> figure); show the LOC/man-day equivalent from the measured LOC/FP. (2) Cost:
> fully-loaded day rate = median base × burden (1.5–1.75×) ÷ 260; look up current
> medians from PayScale, Salary.com, US BLS (+ note aggregators) and cite each;
> give a cost row per band **split System / DevOps-Test / Total** (using the Module
> 11.0 FP subtotals). (3) Two anchors: full-lifecycle (Jones baseline, recommended
> headline = System figure) vs coding-centric best case; single headline range across
> all bands. (4) Schedule check: calendar months ≈ FP^0.4 and implied team size.
> Run `sizing`; state the replacement-cost (not market-
> value) caveat.
