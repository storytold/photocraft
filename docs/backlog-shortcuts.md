# Working the backlog faster

> **Last reviewed:** 2026-10-11 · Companion to the generated [backlog triage](backlog-triage.md)
> (`scripts/pr-triage.py`), [gaps](gaps.md) and the [scorecard](scorecard.md).

This is about *throughput*, not about which feature to build. `docs/gaps.md` ranks the
feature work; this file is about clearing the queue in front of it.

## The diagnosis

On 2026-10-11 the repository had **942 open items: 647 issues and 295 pull requests.**
Only **26 of those 295 PRs (9%)** were mergeable with green CI. **107 (36%)** had merge
conflicts. **929 items (98%)** carried no labels at all.

Read that again, because it inverts the usual assumption. The bottleneck is not that
fixes aren't written — 291 PRs are sitting there — it is that almost none of them can
be merged without someone first doing mechanical rebase work, and nothing in the
queue is sorted well enough to show you where to start.

Three consequences, in order of leverage:

1. **Merge conflicts are the expensive part of review, and they are the part a script
   can do for free.** A conflicted PR asks the reviewer to reason about someone else's
   rebase *and* the change. A clean one asks about the change.
2. **Unlabelled means unsortable.** You cannot work "all the tablet bugs" when nothing
   is tagged tablet.
3. **Small PRs first is a real strategy, not a preference.** Merging a 2-file PR often
   removes the conflict from a 40-file PR stacked behind it.

## 1. The fifteen-minute win

26 PRs are clean and green right now. `gh pr merge` on each is all they need — no code,
no rebase, no judgement:

```sh
scripts/pr-triage.py --merge-now           # print the commands
scripts/pr-triage.py --merge-now --yes     # do it
```

Then the 32 PRs whose CI already passed but which drifted out of date. These are one
command each, and most come back clean with no human input:

```sh
scripts/pr-triage.py --autorebase --yes
```

Doing those two things clears ~58 items from a 942-item backlog in minutes. Nothing
else in this file has that ratio.

**Why it matters more than it looks:** several of the merge-now PRs are performance
fixes for the scorecard's worst rows. #2596 takes P28 (Content-Aware Scale, 24 MP) from
**954,192 ms to 2,641 ms — 361×, and inside its 3,000 ms budget** — and it is sitting
unmerged. #2882 does the same for P26/P27 (Feather/Smooth), #2870 for P29 (Select
Subject). The single largest measured performance win in the project is blocked on
someone pressing merge.

## 2. Automate it once, then stop paying

Two workflows in this change remove the recurring cost:

| Workflow | What it does | Why |
|---|---|---|
| `.github/workflows/pr-derot.yml` | Every 6 hours, calls `gh pr update-branch` on every open non-draft PR that isn't already clean | Stops conflicts from forming. PRs that still conflict afterwards are reported — that list *is* the real work queue |
| `.github/workflows/auto-label.yml` | Labels each issue/PR as it is opened | Fixes the 98%-unlabelled problem at intake, so it never comes back |

Neither merges, approves or closes anything.

The de-rot job deliberately attempts *every* non-clean PR rather than only the ones
GitHub reports as `BEHIND`: measured against this repo, `mergeStateStatus` almost never
returns `BEHIND` (out-of-date PRs come back `UNSTABLE` because their checks are stale
too), so waiting for `BEHIND` would skip nearly everything. `update-branch` is a no-op
when there is nothing to merge, and reports a conflict when there is a real one. Add
the `no-derot` label to a branch that must not be touched.

## 3. Labelling the existing backlog — read this first

```sh
scripts/pr-triage.py --apply-labels --yes
```

**This notifies roughly 900 authors.** It is the right thing to do eventually and a
terrible thing to do accidentally. Run it once, on purpose, when you want the
notification, and say so in advance. The workflow above only labels what arrives
afterwards, which is why it is safe to leave running.

Existing labels are never removed, so correcting the classifier by hand sticks.

## 4. Faster dev loop

The gates in `AGENTS.md` §5 are correct and slow. While iterating, run the cheap subset:

```sh
cargo test -p <crate> --lib --tests     # skip examples + doctests (ui-egui has 11 examples)
cargo test -p <crate> --test <file>     # one integration-test file
cargo nextest run --workspace           # each test in its own process, in parallel
cargo test --doc --workspace            # nextest does not run doctests
```

`cargo nextest` is the biggest single one: measured on this repo, the full workspace
went from **297 s to about 120 s** on a 32-thread Windows machine ([#2204](https://github.com/storytold/photocraft/issues/2204)).

Parallel agents: give each its own `CARGO_TARGET_DIR=target/agent-<name>` to dodge the
Cargo build lock (§6 of `AGENTS.md`). Each target dir is ~10 GB; delete them when done.

One command for the whole gate, when you actually need it:

```sh
cargo xtask ci        # fmt --check, clippy -D warnings, test, layers, wasm — stops at first failure
```

Available xtask commands: `layers`, `wasm`, `ci`, `corpus`, `test-corpus`, `stats`,
`parity`, `i18n-coverage`, `perf`, `scorecard`, `version`, `ico`. Run `cargo xtask`
with no arguments for the usage text.

## 5. Where the cheap work is

Three sources of small, independent, high-yield tasks. All are measured, none require
design work:

- **43 of 150 preferences do nothing** (scorecard "Settings that do nothing",
  [#204](https://github.com/storytold/photocraft/issues/204)). Each is a small wiring
  job with an existing dialog and an existing field. **Beware the pile-up: 11 open PRs
  are already competing for parts of #204** (#2543, #2577, #2587, #2640, #2658, #2668,
  #2971, #3026 among them). Check for an existing PR before starting; these are
  independent, so merging them as a batch works.
- **`cargo xtask parity`** rewrites `docs/parity-checklist.md` with every Photoshop menu
  item that is still missing. Low-hanging fruit is usually a missing command whose
  algorithm already exists in `algo`, `paint`, `vector` or `text`.
- **`docs/scorecard.md`**: each area's `missing` and `partial` rows, plus the perf
  scenarios that are over budget.

## 6. Performance, ranked by ratio not by vibes

Of the perf rows carrying numbers, **21 of 24 are over budget** (the scorecard counts
22 of 25 including failures). Ranked by how far over, the top of the list is:

| Ratio | Id | Scenario | p95 | Budget |
|---:|---|---|---:|---:|
| 318× | P28 | Content-Aware Scale 24 MP | 954,192 ms | ≤ 3,000 ms |
| 59× | P11 | Paste a 12 MP layer (repeat) | 1,174 ms | ≤ 20 ms |
| 54× | P14 | Move a layer, 15000×10000 16-bit | 4,318 ms | ≤ 80 ms |
| 43× | P5 | Toggle visibility | 425 ms | ≤ 10 ms |
| 39× | P33 | Brush dab, A4 300 ppi CMYK | 235 ms | ≤ 6 ms |
| 31× | P10 | Paste a 12 MP layer (first) | 3,136 ms | ≤ 100 ms |

P28 is 15.9 minutes for one operation, and its fix is already written, reviewed by CI,
and unmerged (#2596). Start there.

Note the P1–P7 cluster (move layer, opacity, fill, blend mode, visibility, undo, redo):
all 21–43× over, all on the same 10–25 ms interactive budget. They smell like one shared
cause — a full recomposite per property edit — rather than seven independent problems.
Two open PRs attack exactly that (#1164, #1169, both GPU stack-prefix reuse). Worth
profiling once before writing seven separate fixes.

## 7. What not to do

- **Don't start new features while ~295 PRs are open.** The queue is the bug.
- **Don't rebase by hand what `gh pr update-branch` can do.**
- **Don't review the biggest PR first.** Review smallest-first; it unblocks more.
- **Don't bulk-label without warning** (see §3).
- **Don't trust `gh pr list`'s `mergeStateStatus == "CLEAN"` alone** — it does not tell
  you CI passed. `pr-triage.py` cross-references mergeability *and* check runs, which is
  how it found that only 26 of 291 PRs are genuinely ready.

## 8. Command reference

```sh
scripts/pr-triage.py                       # full triage report to stdout
scripts/pr-triage.py -o docs/backlog-triage.md    # regenerate the committed report
scripts/pr-triage.py --merge-now [--yes]   # merge the clean+green PRs
scripts/pr-triage.py --autorebase [--yes]  # rebase green-but-conflicted PRs
scripts/pr-triage.py --apply-labels [--yes]# label everything (NOTIFIES ~900 PEOPLE)
scripts/pr-triage.py --only 1234 --apply-labels --yes   # label one item (used by CI)
scripts/pr-triage.py --json items.json     # machine-readable dump
scripts/pr-triage.py --repo owner/repo     # point at a fork
```

Every mutating flag is dry-run until you pass `--yes`.

Requires Python 3.9+ (stdlib only) and an authenticated `gh`. It never sees a token:
`gh` owns the credentials and the script shells out to it.
