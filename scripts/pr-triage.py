#!/usr/bin/env python3
"""Triage the PhotoCraft backlog: classify, rank and de-rot the open issues and PRs.

Why this exists
---------------
PhotoCraft gets far more pull requests than it can review. The failure mode is
not "nobody writes the fix", it is:

  * a PR sits for two days, main moves, and the PR is now *conflicted*;
  * nobody notices, because the queue is unlabelled and unsorted;
  * the next reviewer faces 282 PRs and picks arbitrarily.

`git merge` conflicts are the expensive part of review, and they are the part a
script can do for free. This tool answers the only three questions that matter,
in the order they can be acted on:

  1. What can I merge right now, with zero work?      --merge-now
  2. What is green but conflicted, so `update-branch`
     will probably fix it automatically?              --autorebase
  3. What is actually broken and needs a human?       --failing

It also classifies every open item into area / kind / platform / size labels and
can push those labels back to GitHub (--apply-labels), which is what makes the
queue filterable in the first place.

Requirements
------------
Python 3.9+ (stdlib only) and the GitHub CLI (`gh`), authenticated. No pip
install, no tokens in the environment: `gh` owns the credentials.

Usage
-----
    scripts/pr-triage.py                       # full report to stdout
    scripts/pr-triage.py -o docs/backlog.md    # write the report to a file
    scripts/pr-triage.py --merge-now           # gh merge commands, ready to run
    scripts/pr-triage.py --autorebase          # rebase the green-but-conflicted
    scripts/pr-triage.py --apply-labels        # push labels to GitHub (dry-run
                                               # first; --yes to actually write)
    scripts/pr-triage.py --json out.json       # machine-readable dump
    scripts/pr-triage.py --repo OWNER/REPO     # default storytold/photocraft

Every mutating flag is dry-run by default. Nothing here merges, rebases or
labels until you pass --yes.

Classification is keyword heuristics on titles, not an LLM and not a semantic
model: it is fast, offline, deterministic, and good enough to sort 900 items
into reviewable buckets. A wrong area label is cheap; an unsorted queue is not.
"""

from __future__ import annotations

import argparse
import collections
import json
import re
import subprocess
import sys
import textwrap
import time
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone

DEFAULT_REPO = "storytold/photocraft"

# --------------------------------------------------------------------------
# Classification rules.
#
# Ordered: the first area that matches wins, so specific patterns must come
# before general ones ("psd" before "file", "tablet" before "input"). The
# vocabulary deliberately mirrors docs/gaps.md, docs/scorecard.md and the
# scorecard/*.toml areas, so a label here is a label the docs already use.
# --------------------------------------------------------------------------

AREA_RULES: list[tuple[str, str]] = [
    ("file-format", r"\bpsd\b|\bpsb\b|psd|photoshop (cannot|can't|can not|won't)|save as|corrupt|"
                    r"round.?trip|tiff|\bpng\b|jpe?g|\bjpg\b|heic|heif|avif|\braw\b|camera raw|"
                    r"\braf\b|\bcr2\b|\bcr3\b|\bnef\b|\bdng\b|\borf\b|\barw\b|openraster|\bora\b|"
                    r"\bxcf\b|\bpdn\b|jxl|jpeg ?2000|\bjp2\b|\bpdf\b|\btga\b|\bexr\b|\bwebp\b|"
                    r"file format|file.?compat|\.pcraft|codec|import|export|open .*file|cannot open|"
                    r"won't open|file info|xmp|\bexif\b"),
    ("type-font", r"\bfont|\btype\b|type tool|text tool|\btext\b|glyph|\bime\b|complex script|"
                  r"character|paragraph|typograph|kerning|leading|opentype|variable font|tofu|"
                  r"\brtl\b|bidi|shaping|harfbuzz|fallback font"),
    ("tablet-pen", r"tablet|wacom|stylus|\bpen\b|pressure|tilt|xppen|xp-pen|huion|windows ink|"
                   r"barrel|eraser button|palm rejection"),
    ("brush-paint", r"brush|paint|pencil|eraser|smudge|clone stamp|heal|dodge|burn|sponge|"
                    r"blur tool|sharpen tool|stroke|dab|smoot|mixer brush|history brush|"
                    r"gradient tool|paint bucket|fill tool"),
    ("selection-mask", r"select|mask|lasso|marquee|quick mask|refine|feather|magic wand|"
                       r"quick selection|object selection|subject|pen tool|\bpath\b|anchor point|"
                       r"channels?\b"),
    ("transform", r"transform|warp|puppet|liquify|scale|rotate|skew|distort|perspective|flip|"
                  r"crop|artboard|canvas size|image size|nudge|align"),
    ("performance", r"\blag|slow|perf|jank|stutter|\bfps\b|frame time|memory|\bram\b|\boom\b|"
                    r"out of memory|speed|responsive|budget|optimi[sz]|cache|rayon|parallel|"
                    r"throughput|scratch disk|undo cache"),
    ("crash-stability", r"crash|panic|blank window|black window|freeze|hang|segfault|\babort\b|"
                        r"unresponsive|not responding|exits|closes without|won't start|fails to start|"
                        r"wgpu|vulkan|graphics adapter|gpu crash"),
    ("gpu-render", r"\bgpu\b|wgpu|vulkan|metal|directx|shader|composit|renderer|adapter|"
                   r"antialias|high.?throughput"),
    ("localization", r"locali[sz]|translat|\bi18n\b|\bl10n\b|language|chinese|japanese|korean|"
                     r"\brtl\b|arabic|hebrew|farsi|persian|vietnamese|german|spanish|french|"
                     r"portugu|hindi|russian|ukrainian|polish|czech|greek|dutch|italian|indonesian|"
                     r"kurdish|fluent|\bftl\b"),
    ("automation-mcp", r"\bmcp\b|script|automation|\baction|droplet|command line|\bcli\b|"
                       r"control (channel|protocol|server)|\bapi\b|plug-?in|\.8bf\b|uxp|batch"),
    ("ai-generative", r"\bai\b|generative|neural|machine learning|\bml\b|onnx|model|"
                      r"background removal|select subject|upscale|super zoom|sky replacement|"
                      r"remove tool|content.?aware"),
    # Kept separate from layers-panels-ui: drag/drop and clipboard were ~8% of the
    # open issues on their own, and they are one subsystem (platform services),
    # not a pile of unrelated UI nits.
    ("input-clipboard", r"\bdrag\b|drag.?and.?drop|\bdrop\b|clipboard|\bcopy\b|\bpaste\b|"
                        r"\bmouse\b|wheel|scroll|scrolling|cursor|pointer|trackpad|pinch|"
                        r"gesture|touch|\bnudge\b"),
    ("layers-panels-ui", r"layer|panel|toolbar|menu|dock|workspace|context menu|options bar|"
                         r"properties|window|dialog|preferences|\bui\b|\bux\b|theme|appearance|"
                         r"ruler|guide|grid|status bar|tab|icon|cursor|scroll|tooltip|slider|"
                         r"shortcut|keyboard|hotkey"),
    ("install-packaging", r"install|brew|homebrew|flatpak|appimage|\bmsi\b|\bdeb\b|\brpm\b|\baur\b|"
                          r"\bnix\b|snap|\bapk\b|package|download|updat|release|windows|macos|"
                          r"\bmac\b|linux|wayland|\bx11\b|android|freebsd|\brisc-v\b|winget|"
                          r"chocolatey|\bport\b|omastore|appimage|updater|sparkle"),
    ("print-export", r"print|export|\bsave for web\b|color profil|\bicc\b|\bcms\b|color manage|"
                     r"soft proof|gamut"),
    ("video", r"video|timeline|animation|frame animation|\bgif\b|data.?driven graphic"),
    ("docs", r"\bdocs?\b|readme|documentation|comment|typo|attribution|license|contribut"),
    ("ci-build", r"\bci\b|workflow|action|build fail|compile|\bcargo\b|clippy|rustfmt|"
                 r"dependency|lockfile|toolchain|\bxtask\b|test"),
]

KIND_RULES: list[tuple[str, str]] = [
    # Note: no bare "escape" here. In an image editor Escape is a key people press
    # ("Ctrl+T shows no Escape button"), and matching it labelled ordinary UI bugs
    # as security. Only sandbox escapes count.
    ("security", r"security|\bcve\b|symlink|path traversal|sandbox|sandbox escape|"
                 r"escape the sandbox|malicious|untrusted input|harden|sanitiz|\bxss\b"),
    ("perf", r"speed|\bperf\b|slow|\blag|faster|optimiz|budget|within budget|memory|\bcache\b"),
    ("crash", r"crash|panic|blank window|black window|freeze|hang|segfault|not responding|"
              r"closes without"),
    ("bug", r"^\s*fix|fix\(|\bbug\b|broken|not working|doesn'?t|does not|can'?t|cannot|"
            r"won'?t|fails?|incorrect|wrong|regress|blank|missing icon|no effect|"
            r"inconsistent|stops responding|destroy|loses?|resets?|ignores?"),
    ("feature", r"^\s*feat|feat\(|\badd\b|\badds\b|feature|implement|support for|wire|"
                r"new tool|introduce|allow|enable|request|there is no|there's no|"
                r"\bno \w+ (support|option|sensitivity|button|preview)|"
                r"not (even )?support|does not support|lack"),
    ("docs", r"\bdocs?\b|readme|documentation|comment|typo|attribution|license|guide"),
    ("test", r"\btests?\b|coverage|fuzz|regression test|oracle|corpus"),
    ("refactor", r"refactor|cleanup|clean up|simplify|rename|move |split|extract|dedup"),
    ("chore", r"chore|bump|update .*dependency|dependency|ci|workflow|release|version|packaging"),
]

PLATFORM_RULES: list[tuple[str, str]] = [
    ("android", r"android|\bapk\b|nativeactivity"),
    ("web-wasm", r"\bwasm\b|webassembly|browser|trunk|web build|photopea"),
    ("windows", r"windows|\bmsi\b|windows ink|\bgdi\b|\bmsvc\b|win10|win11"),
    ("macos", r"macos|\bmac\b|imac|coretext|appkit|\bnotariz|\bdmg\b|sparkle|\bicns\b|"
              r"retina|\bmetal\b"),
    ("linux", r"linux|wayland|\bx11\b|xrandr|gtk|kde|gnome|flatpak|appimage|\baur\b|arch|"
              r"debian|ubuntu|fedora|gentoo|\bnix\b|cups|ipp"),
    ("freebsd", r"freebsd"),
]

# Size buckets by changed files. Review cost grows super-linearly with the
# number of files, and small PRs are both faster to review and less likely to
# conflict, so sorting by size is the single cheapest throughput win.
SIZE_BUCKETS = [(2, "XS"), (6, "S"), (15, "M"), (40, "L")]


def size_of(changed_files: int | None) -> str:
    if not changed_files:
        return "?"
    for limit, name in SIZE_BUCKETS:
        if changed_files <= limit:
            return name
    return "XL"


def classify(title: str) -> dict[str, str]:
    """Return area / kind / platform for one title. Best-effort, never raises."""
    t = (title or "").lower()
    area = next((name for name, pat in AREA_RULES if re.search(pat, t)), "other")
    kind = next((name for name, pat in KIND_RULES if re.search(pat, t)), "other")
    platform = next((name for name, pat in PLATFORM_RULES if re.search(pat, t)), "all")
    return {"area": area, "kind": kind, "platform": platform}


def labels_for(item: dict) -> list[str]:
    """The label set to apply to one item (issues and PRs share the vocabulary)."""
    out = []
    if item.get("is_pr"):
        out.append("size/" + size_of(item.get("changed_files")))
        if item.get("draft"):
            out.append("draft")
        state = item.get("mergeable_state") or ""
        if state == "dirty":
            out.append("needs-rebase")
        elif state == "clean":
            out.append("ready-to-merge")
        elif state == "unstable":
            out.append("ci-failing" if item.get("checks") == "FAILING"
                       else "ci-pending" if item.get("checks") == "PENDING"
                       else "needs-ci")
    cls = item.get("class") or {}
    for key in ("area", "kind", "platform"):
        if cls.get(key) and cls[key] != "all":
            out.append(f"{key}/{cls[key]}")
    if item.get("unlabelled_age_days", 0) > 7:
        out.append("needs-triage")
    return sorted(set(out))


# --------------------------------------------------------------------------
# GitHub access. Everything goes through `gh`, so this script never sees a
# token and cannot leak one.
# --------------------------------------------------------------------------


def gh(*args: str, retries: int = 2) -> str:
    """Run `gh api ...` and return stdout, or '' if it could not be fetched.

    Retries on empty output as well as on failure, because that is what throttling
    looks like from here: GitHub computes `mergeable` asynchronously and returns a
    null, rather than an error, when it has not finished or when the burst of
    per-PR requests is too fast. An unretried null silently becomes "this PR is not
    mergeable", which would make the whole report read as if nothing is ready.
    """
    import time
    for attempt in range(retries + 1):
        try:
            proc = subprocess.run(
                ["gh", "api", *args],
                capture_output=True, text=True, timeout=120,
            )
        except (subprocess.SubprocessError, OSError):
            proc = None
        if proc is not None and proc.returncode == 0 and proc.stdout.strip():
            return proc.stdout
        if attempt < retries:
            # Longer waits for the cases that are definitely throttling rather
            # than merely slow: a 403/429 body or a completely empty response.
            throttled = (proc is not None
                         and any(code in (proc.stderr or "")
                                 for code in ("403", "429", "rate limit")))
            time.sleep(6.0 if throttled else 1.5 * (attempt + 1))
    return ""


def gh_json(*args: str):
    raw = gh(*args)
    if not raw.strip():
        return None
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return None


def gh_post(path: str, payload: dict) -> bool:
    """POST JSON to the API. `gh` still owns the credentials; the body goes in on
    stdin so nested values are encoded correctly (a `-f labels[]=a,b` form field
    would send one label literally named "a,b")."""
    try:
        proc = subprocess.run(
            ["gh", "api", path, "--method", "POST", "--input", "-"],
            input=json.dumps(payload), capture_output=True, text=True, timeout=60,
        )
        return proc.returncode == 0
    except (subprocess.SubprocessError, OSError):
        return False


def fetch_items(repo: str, workers: int = 8) -> list[dict]:
    """Fetch every open issue and PR (the issues endpoint covers both)."""
    # `has("pull_request")` must be evaluated against the raw issue object: if
    # `pull_request` is named in the constructed object it always exists (null
    # for issues), which would mark every issue as a PR.
    jq = ('[.[] | {number, title, state, created_at, updated_at, labels, comments, '
          'draft, user: .user.login} + {is_pr: has("pull_request")}]')
    out: list[dict] = []
    page = 1
    while True:
        batch = gh_json(
            f"repos/{repo}/issues",
            "--method", "GET",
            "-f", "state=open", "-f", "per_page=100", "-f", f"page={page}",
            "--jq", jq,
        )
        if not batch:
            break
        out.extend(batch)
        if len(batch) < 100:
            break
        page += 1

    prs = [i for i in out if i["is_pr"]]

    # PR mergeability and CI state are per-PR endpoints; GitHub does not compute
    # `mergeable` for the list endpoint, which is exactly why a plain
    # `gh pr list` looks healthier than the queue really is.
    def detail(pr: dict) -> None:
        d = gh_json(
            f"repos/{repo}/pulls/{pr['number']}",
            "--jq", "{mergeable, mergeable_state, changed_files, additions, deletions, "
                    "head_ref: .head.ref, head_sha: .head.sha, rebaseable, commits}",
        )
        if d:
            pr.update(d)

    with ThreadPoolExecutor(max_workers=workers) as pool:
        list(pool.map(detail, prs))

    # Second pass for PRs whose mergeability GitHub had not computed yet. It is
    # computed lazily and a burst of requests exhausts the background queue, so
    # the stragglers are re-asked serially (and slowly) after the fan-out has
    # settled. Without this a throttled run reports most PRs as unmergeable.
    unresolved = [p for p in prs if not p.get("mergeable_state")]
    for round_no in (1, 2):
        if not unresolved:
            break
        time.sleep(5 * round_no)
        for p in unresolved:
            d = gh_json(f"repos/{repo}/pulls/{p['number']}", "--jq",
                        "{mergeable, mergeable_state}")
            if d:
                p.update(d)
        unresolved = [p for p in unresolved if not p.get("mergeable_state")]

    def checks(pr: dict) -> None:
        sha = pr.get("head_sha")
        if not sha:
            return
        # `per_page` goes in the URL, not in `-f`: gh switches the verb to POST as
        # soon as any field is given, and POST /check-runs is a 404.
        c = gh_json(
            f"repos/{repo}/commits/{sha}/check-runs?per_page=100",
            "--jq", '{concl: [.check_runs[].conclusion], status: [.check_runs[].status]}',
        )
        pr["checks"] = summarise_checks(c or {})

    with ThreadPoolExecutor(max_workers=workers) as pool:
        list(pool.map(checks, prs))

    # Re-ask for CI results, but only where a missing result is surprising. A PR
    # opened in the last day often has no check runs yet for the perfectly good
    # reason that CI has not finished; an older one with no results is more
    # likely a throttled read, so only those are worth a second, slower pass.
    for round_no in (1, 2):
        missing = [p for p in prs
                   if p.get("checks") == "no-checks" and age_days(p["created_at"]) >= 1]
        if not missing:
            break
        time.sleep(4 * round_no)
        for p in missing:
            checks(p)
    return out


BAD_CONCLUSIONS = ("failure", "timed_out", "cancelled", "startup_failure", "action_required")
UNSETTLED_STATUS = ("in_progress", "queued", "waiting", "pending")


def summarise_checks(runs: dict) -> str:
    """Map a check-run listing to GREEN / FAILING / PENDING / no-checks.

    `conclusion` is null while a run is in flight, so an unfinished run is
    `PENDING` even though nothing has failed yet: do not let a still-running CI
    look like a pass.
    """
    concl = [c for c in (runs.get("concl") or []) if c]
    status = runs.get("status") or []
    if not concl and not status:
        return "no-checks"
    if any(c in BAD_CONCLUSIONS for c in concl):
        return "FAILING"
    if any(s in UNSETTLED_STATUS for s in status) or any(c is None for c in concl):
        return "PENDING"
    return "GREEN"


def fetch_one(repo: str, number: int) -> dict:
    """Fetch a single item. Used by the auto-label workflow, which runs per event
    and must not pay for a scan of the whole backlog."""
    jq = ('{number, title, state, created_at, updated_at, labels, comments, draft, '
          'is_pr: has("pull_request")}')
    item = gh_json(f"repos/{repo}/issues/{number}", "--jq", jq)
    if not item:
        return {}
    if item.get("is_pr"):
        d = gh_json(f"repos/{repo}/pulls/{number}", "--jq",
                    "{mergeable_state, changed_files, additions, deletions, "
                    "head_ref: .head.ref, head_sha: .head.sha, draft}")
        if d:
            item.update(d)
        sha = item.get("head_sha")
        if sha:
            c = gh_json(f"repos/{repo}/commits/{sha}/check-runs?per_page=100", "--jq",
                        '{concl: [.check_runs[].conclusion], status: [.check_runs[].status]}')
            item["checks"] = summarise_checks(c or {})
    return item


def age_days(iso: str) -> int:
    try:
        dt = datetime.fromisoformat((iso or "").replace("Z", "+00:00"))
    except ValueError:
        return 0
    return (datetime.now(timezone.utc) - dt).days


# --------------------------------------------------------------------------
# Ranking. The whole point of the tool: turn 900 items into three short lists.
# --------------------------------------------------------------------------


def rank(prs: list[dict]) -> dict[str, list[dict]]:
    merge_now, rebase, failing, no_ci, rest = [], [], [], [], []
    # `no-checks` is tested before the dirty branch on purpose: a conflicted PR
    # with no CI result belongs with the other "no CI" PRs, because the thing you
    # have to do to it (make CI run) is the same, and because leaving it in the
    # leftovers made the headline count disagree with the section listing it.
    for p in prs:
        state = p.get("mergeable_state")
        chk = p.get("checks")
        if state == "clean" and chk == "GREEN":
            merge_now.append(p)
        elif chk == "no-checks":
            no_ci.append(p)
        elif state == "dirty":
            (rebase if chk == "GREEN" else rest).append(p)
        elif chk == "FAILING":
            failing.append(p)
        else:
            rest.append(p)
    # Smallest first: a 2-file PR is minutes to review and, once merged, often
    # removes the conflict from a larger PR stacked behind it.
    by_size = lambda ps: sorted(ps, key=lambda p: (p.get("changed_files") or 0, p["number"]))
    return {
        "merge_now": by_size(merge_now),
        "rebase": sorted(rebase, key=lambda p: (p.get("changed_files") or 0, p["number"])),
        "failing": by_size(failing),
        "no_ci": by_size(no_ci),
        "rest": by_size(rest),
    }


# --------------------------------------------------------------------------
# Reporting
# --------------------------------------------------------------------------


def pr_line(p: dict) -> str:
    files = p.get("changed_files") or "?"
    add = p.get("additions") or 0
    return (f"- **#{p['number']}** `{size_of(p.get('changed_files'))}` "
            f"{files} files, +{add} — {p['title'].strip()}"
            f"  \n  `{p.get('head_ref', '?')}` · {p.get('checks', '?')} · "
            f"{p.get('mergeable_state', '?')} · opened {p['created_at'][:10]}")


def build_report(items: list[dict], repo: str) -> str:
    prs = [i for i in items if i["is_pr"]]
    issues = [i for i in items if not i["is_pr"]]
    buckets = rank(prs)
    now = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC")

    area_i = collections.Counter((i.get("class") or {}).get("area", "other") for i in issues)
    area_p = collections.Counter((p.get("class") or {}).get("area", "other") for p in prs)
    unlabelled = sum(1 for i in items if not i.get("labels"))
    draft = sum(1 for p in prs if p.get("draft"))

    L: list[str] = []
    w = L.append
    w(f"# Backlog triage — {repo}")
    w("")
    w(f"> Generated {now} by `scripts/pr-triage.py`. Do not edit by hand: regenerate with")
    w("> `scripts/pr-triage.py -o docs/backlog-triage.md`.")
    w(">")
    w("> A **point-in-time snapshot**: the backlog moves by dozens of items a day, and")
    w("> unlike `docs/scorecard.md` this is deliberately *not* enforced by CI — a")
    w("> freshness check here would fail constantly for no benefit. Regenerate it when")
    w("> you want a current view. `docs/backlog-shortcuts.md` is the hand-written")
    w("> companion and does not go stale.")
    w("")
    w("## Headline")
    w("")
    w("| | Count |")
    w("|---|---:|")
    w(f"| Open items | {len(items)} |")
    w(f"| Open issues | {len(issues)} |")
    w(f"| Open PRs | {len(prs)} |")
    w(f"| PRs mergeable **and** green (merge now) | **{len(buckets['merge_now'])}** "
      f"({len(buckets['merge_now']) * 100 // max(1, len(prs))}%) |")
    w(f"| PRs green but conflicted (auto-rebase) | {len(buckets['rebase'])} |")
    w(f"| PRs with failing CI | {len(buckets['failing'])} |")
    w(f"| PRs with no CI results at all | {len(buckets['no_ci'])} |")
    dirty = sum(1 for p in prs if p.get("mergeable_state") == "dirty")
    w(f"| PRs with merge conflicts (`dirty`) | {dirty} "
      f"({dirty * 100 // max(1, len(prs))}%) |")
    w(f"| PRs still dirty with failing/absent CI | {len(buckets['rest'])} |")
    w(f"| Draft PRs | {draft} |")
    w(f"| Items with **no labels** | {unlabelled} "
      f"({unlabelled * 100 // max(1, len(items))}%) |")
    w("")

    # Honesty about the data. GitHub computes mergeability lazily and throttles
    # bursts, so a bad run can come back mostly null. Reporting "0 to merge"
    # then would be a lie that costs a day of work, so say so instead.
    # Two different kinds of missing data, and they must not be conflated: an
    # unresolved mergeability verdict means *this run was throttled*, while a PR
    # with no CI results usually just means the PR is new. Only the first one
    # undermines the numbers above.
    unresolved = [p for p in prs if not p.get("mergeable_state")]
    no_ci = [p for p in prs if p.get("checks") == "no-checks"]
    if unresolved:
        w(f"> **Incomplete run.** {len(unresolved)} of {len(prs)} PRs returned no "
          "mergeability verdict, which means GitHub throttled this run rather than "
          "answered. Every count below is a *lower bound* — in particular "
          "'merge now' may be missing entries. Re-run before acting on it.")
        w("")
    if no_ci:
        stale = [p for p in no_ci if age_days(p["created_at"]) >= 2]
        w(f"> **{len(no_ci)} PRs have no CI results**"
          + (f", {len(stale)} of them open for two days or more" if stale else "")
          + ". Most are brand-new PRs whose checks have not reported yet; the older "
            "ones are usually forks with Actions disabled and need a push or a "
            "rebase to trigger a run.")
        w("")
    w("The bottleneck is not writing fixes, it is *de-rotting and reviewing* the")
    w("queue. Work the lists in the order below: merge-now first (zero cost), then")
    w("auto-rebase (one command each), and only then look at failing CI.")
    w("")
    w("## 1. Merge now — clean and green")
    w("")
    w("These need no code and no rebase: `gh pr merge N` is a no-brainer for each.")
    w("Sorted smallest first, because merging a small PR often clears the conflict")
    w("on a bigger one behind it.")
    w("")
    for p in buckets["merge_now"]:
        w(pr_line(p))
    w("")
    w("```sh")
    for p in buckets["merge_now"]:
        w(f"gh pr merge {p['number']} --squash")
    w("```")
    w("")
    w("## 2. Auto-rebase — green but conflicted")
    w("")
    w("CI already passed; the only blocker is that main moved. `update-branch`")
    w("replays them on top of main; most come back clean with no human input.")
    w("Anything that still conflicts after that is a real conflict and needs a person.")
    w("")
    for p in buckets["rebase"]:
        w(pr_line(p))
    w("")
    w("```sh")
    for p in buckets["rebase"]:
        w(f"gh pr update-branch {p['number']}")
    w("```")
    w("")
    w("## 3. Failing CI — needs a human")
    w("")
    if buckets["failing"]:
        for p in buckets["failing"]:
            w(pr_line(p))
    else:
        w("None.")
    w("")
    w("## 4. No CI results")
    w("")
    w("CI never reported for these. Most are brand-new PRs whose checks have not")
    w("finished; the older ones are usually forks with Actions disabled. Push an")
    w("empty commit or rebase to trigger a run before reviewing — a PR with no CI")
    w("is not a PR you can merge, whatever its diff looks like. Some of these are")
    w("also conflicted, in which case the rebase triggers CI and clears both.")
    w("")
    for p in buckets["no_ci"]:
        w(pr_line(p))
    w("")
    w("## 5. Everything else (dirty *and* not green)")
    w("")
    w("Cheapest first. Many of these are worth closing rather than resurrecting:")
    w("if the author has not answered in a week and CI is red, ask once, then close")
    w("with a pointer to re-open when it is rebased.")
    w("")
    for p in buckets["rest"]:
        w(pr_line(p))
    w("")
    w("## Open issues by area")
    w("")
    w("| Area | Issues | PRs |")
    w("|---|---:|---:|")
    for area, n in area_i.most_common():
        w(f"| {area} | {n} | {area_p.get(area, 0)} |")
    w("")
    w("A big PR count next to a big issue count is a queue problem, not a code")
    w("problem: the fixes exist and are waiting on review.")
    w("")
    w("## Most-discussed open issues")
    w("")
    w("Comment count is a rough proxy for how many people hit the same thing.")
    w("An issue with eight comments is usually eight users, not one argument.")
    w("")
    w("| Issue | Comments | Age (d) | Area | Title |")
    w("|---|---:|---:|---|---|")
    for i in sorted(issues, key=lambda x: -x.get("comments", 0))[:25]:
        t = i["title"].replace("|", "\\|").strip()
        w(f"| #{i['number']} | {i.get('comments', 0)} | {age_days(i['created_at'])} "
          f"| {(i.get('class') or {}).get('area', 'other')} | {t} |")
    w("")
    return "\n".join(L) + "\n"


# --------------------------------------------------------------------------
# Mutations. All dry-run unless --yes.
# --------------------------------------------------------------------------


def mutate(repo: str, items: list[dict], apply_labels: bool, autorebase: bool,
           merge_now: bool, yes: bool) -> None:
    acted = 0
    if merge_now:
        for p in rank([i for i in items if i["is_pr"]])["merge_now"]:
            cmd = ["gh", "pr", "merge", str(p["number"]), "--squash"]
            print(" ".join(cmd), flush=True)
            if yes:
                subprocess.run(cmd, cwd=".", check=False)
            acted += 1
    if autorebase:
        for p in rank([i for i in items if i["is_pr"]])["rebase"]:
            cmd = ["gh", "pr", "update-branch", str(p["number"])]
            print(" ".join(cmd), flush=True)
            if yes:
                subprocess.run(cmd, check=False)
            acted += 1
    if apply_labels:
        for it in items:
            want = labels_for(it)
            have = {l["name"] if isinstance(l, dict) else l for l in it.get("labels") or []}
            add = [l for l in want if l not in have]
            if not add:
                continue
            print(f"#{it['number']} +{' '.join(add)}", flush=True)
            if yes:
                # GitHub creates labels that do not exist yet, so no seeding step
                # is needed. One call per item: a 404 here means no write access,
                # and the run continues rather than aborting the queue.
                if not gh_post(f"repos/{repo}/issues/{it['number']}/labels",
                               {"labels": add}):
                    print(f"  ! failed to label #{it['number']}", file=sys.stderr)
            acted += 1
    if not yes and acted:
        print(f"\n[dry-run] {acted} action(s) shown. Re-run with --yes to execute.")


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Triage the PhotoCraft issue/PR backlog.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=textwrap.dedent(__doc__ or "").split("Usage")[-1],
    )
    ap.add_argument("--repo", default=DEFAULT_REPO, help=f"default {DEFAULT_REPO}")
    ap.add_argument("-o", "--output", help="write the report here instead of stdout")
    ap.add_argument("--json", dest="json_out", help="also dump the raw data as JSON")
    ap.add_argument("--apply-labels", action="store_true", help="push area/kind/size labels")
    ap.add_argument("--autorebase", action="store_true", help="update green-but-conflicted branches")
    ap.add_argument("--merge-now", action="store_true", help="merge the clean+green PRs")
    ap.add_argument("--yes", action="store_true", help="actually perform the mutation")
    ap.add_argument("--only", type=int, metavar="N",
                    help="process a single issue/PR number (used by auto-label)")
    ap.add_argument("--workers", type=int, default=8)
    args = ap.parse_args()

    if subprocess.run(["gh", "--version"], capture_output=True).returncode != 0:
        print("error: the GitHub CLI (`gh`) is required and was not found.", file=sys.stderr)
        return 2

    if args.only:
        items = [fetch_one(args.repo, args.only)]
    else:
        print(f"Fetching open items from {args.repo} …", file=sys.stderr)
        items = fetch_items(args.repo, args.workers)
    if not items or not any(items):
        print("error: no items returned — check `gh auth status` and the repo name.",
              file=sys.stderr)
        return 1

    for it in items:
        it["class"] = classify(it["title"])
        it["unlabelled_age_days"] = age_days(it["created_at"]) if not it.get("labels") else 0

    print(f"  {len(items)} items "
          f"({sum(1 for i in items if i['is_pr'])} PRs, "
          f"{sum(1 for i in items if not i['is_pr'])} issues)", file=sys.stderr)

    # A single-item run (auto-label) has no report to write; it only labels.
    if not args.only:
        report = build_report(items, args.repo)
        if args.output:
            with open(args.output, "w", encoding="utf-8") as fh:
                fh.write(report)
            print(f"wrote {args.output}", file=sys.stderr)
        else:
            print(report)

    if args.json_out:
        with open(args.json_out, "w", encoding="utf-8") as fh:
            json.dump(items, fh, indent=1, default=str)
        print(f"wrote {args.json_out}", file=sys.stderr)

    if args.apply_labels or args.autorebase or args.merge_now:
        mutate(args.repo, items, args.apply_labels, args.autorebase, args.merge_now, args.yes)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
