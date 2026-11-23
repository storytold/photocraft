#!/usr/bin/env python3
"""Engine split, step 5: `Session` state.

The action recording and playback state left `ActionState` (`s.actions`) for the core type
`ActionRecording` (`s.recording`); the journal mark `recording` is called `target` there. Small
state types moved to `crates/engine-core/src/state.rs` (their old paths are re-exported), the
action-history bookkeeping to `action_history.rs`, and the stored-preferences document with the
methods that combine preferences, colour settings and presets to `prefs/saved.rs`.
`Clip::from_svg` is the free function `edit_cmds::clip_from_svg`, and the preset store is a session
slot read through `preset_store::PresetStoreExt` (`s.preset_store()`, `s.preset_store_mut()`).

    python3 scripts/engine_split/session_state.py split
        Rewrites every Rust file under `crates/` and `apps/`: the recording fields,
        `Clip::from_svg` and the preset store field. The rest of the step (type moves, slots, hooks) is hand-written, its
        own commit.

    python3 scripts/engine_split/session_state.py branch --base <merge-base> --head <branch>
        On a branch rebased onto the step. `--head` is the branch commit from before the rebase
        (`ORIG_HEAD` right after it). Rewrites the files the branch changed, and lists the
        branch's edits to definitions that moved (re-apply those in the new file).

Running either mode twice gives the same result. What it cannot do is printed at the end of
every run.
"""
import argparse, os, re, subprocess, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rust_source import git, git_show, mask  # noqa: E402

FIELDS = ["record_tools", "recording", "take", "playing", "playback_stack", "playback_steps", "budget_shared", "batch_depth", "script_depth", "script_steps"]
RENAMED = {"recording": "target"}
RECORDING = re.compile(r"\.actions\.(" + "|".join(FIELDS) + r")\b")
FROM_SVG = re.compile(r"\bedit_cmds::Clip::from_svg\(")
BARE_FROM_SVG = re.compile(r"(?<![\w:])Clip::from_svg\(")
IMPORTS_FROM_SVG = re.compile(r"^\s*use [^;]*\bclip_from_svg\b", re.M)
# The preset store is a slot (`preset_store::PresetStoreExt`); `services.preset_store` is the UI's own.
PRESET_STORE = [
    (re.compile(r"(?<!services)\.preset_store\.as_ref\(\)"), ".preset_store()"),
    (re.compile(r"(?<!services)\.preset_store\.as_mut\(\)"), ".preset_store_mut()"),
    (re.compile(r"(?<!services)\.preset_store\.(is_none|is_some)\(\)"), r".preset_store().\1()"),
]

# Definitions that moved in this step: (file it was in, item, new file).
MOVED = [
    ("cmd/file_cmds/open.rs", "struct BatchOpen", "state.rs"),
    ("cmd/edit_cmds.rs", "struct Clip", "state.rs"),
    ("cmd/smart_cmds.rs", "struct SmartLink", "state.rs"),
    ("cmd/channel_cmds/view.rs", "struct QuickMaskOptions", "state.rs"),
    ("cmd/edit_menu_cmds.rs", "struct EditState", "state.rs"),
    ("cmd/edit_menu_cmds.rs", "struct FadeSource", "state.rs"),
    ("cmd/edit_menu_cmds.rs", "enum FadeTarget", "state.rs"),
    ("cmd/edit_menu_cmds.rs", "struct CustomShape", "state.rs"),
    ("cmd/edit_menu_cmds.rs", "struct FindCursor", "state.rs"),
    ("cmd/analysis_cmds.rs", "struct AnalysisState", "state.rs"),
    ("cmd/analysis_cmds.rs", "struct LogRow", "state.rs"),
    ("cmd/analysis_cmds.rs", "struct DataPoints", "state.rs"),
    ("cmd/analysis_cmds.rs", "fn available", "state.rs"),
    ("cmd/automate_cmds.rs", "struct FileMenuState", "state.rs"),
    ("cmd/automate_cmds.rs", "struct ScriptEvents", "prefs.rs"),
    ("cmd/automate_cmds.rs", "struct ScriptBinding", "prefs.rs"),
    ("color_channels.rs", "struct Restriction", "state.rs"),
    ("color_channels.rs", "struct ColorChannels", "state.rs"),
    ("cmd/brush_cmds.rs", "struct LiveCommit", "state.rs"),
    ("cmd/pattern_cmds.rs", "struct PatternLibrary", "state.rs"),
    ("cmd/pattern_cmds.rs", "fn builtin", "state.rs"),
    ("link_watch.rs", "type Stamp", "state.rs"),
    ("link_watch.rs", "type LinkScan", "state.rs"),
    ("shortcut_sets.rs", "struct ShortcutSet", "prefs/shortcuts.rs"),
    ("cmd/actions_cmds.rs", "fn is_action_key", "prefs/shortcuts.rs"),
    ("cmd/actions_cmds.rs", "fn history_policy_in_use", "action_history.rs"),
    ("cmd/actions_cmds.rs", "struct ActionState", "actions_record.rs (the recording fields: ActionRecording)"),
    ("prefs.rs", "struct SavedPrefs", "prefs/saved.rs"),
    ("prefs.rs", "fn parse_prefs_json", "prefs/saved.rs"),
    ("prefs.rs", "fn parse_legacy_prefs", "prefs/saved.rs"),
    ("prefs.rs", "fn store_options", "prefs/saved.rs"),
    ("prefs.rs", "fn read_saved", "prefs/saved.rs"),
    ("prefs.rs", "fn prefs_to_json", "prefs/saved.rs"),
    ("prefs.rs", "fn prefs_value", "prefs/saved.rs"),
    ("prefs.rs", "fn load_prefs_json", "prefs/saved.rs"),
    ("prefs.rs", "fn saved_prefs", "prefs/saved.rs"),
    ("prefs.rs", "fn set_saved_prefs", "prefs/saved.rs"),
    ("prefs.rs", "fn prefs_view", "prefs/saved.rs"),
    ("prefs.rs", "fn set_pref", "prefs/saved.rs"),
]
SRC = "crates/engine-core/src/"

REPORT = []


def rewrite_text(text):
    text = RECORDING.sub(lambda m: ".recording." + RENAMED.get(m.group(1), m.group(1)), text)
    for rx, to in PRESET_STORE:
        text = rx.sub(to, text)
    text = FROM_SVG.sub("edit_cmds::clip_from_svg(", text)
    return BARE_FROM_SVG.sub("clip_from_svg(", text)


def rewrite(path):
    with open(path, encoding="utf-8") as f:
        old = f.read()
    new = rewrite_text(old)
    if BARE_FROM_SVG.search(old) and not IMPORTS_FROM_SVG.search(new):
        REPORT.append(f"{path}: `Clip::from_svg(` became `clip_from_svg(`; import `edit_cmds::clip_from_svg` there")
    if new != old:
        with open(path, "w", encoding="utf-8") as f:
            f.write(new)
        return True
    return False


def rust_files(roots):
    for root in roots:
        for d, _, files in os.walk(root):
            if "/target" in d or "/vendor" in d:
                continue
            for name in files:
                if name.endswith(".rs"):
                    yield os.path.join(d, name)


def split():
    n = sum(rewrite(p) for p in rust_files(["crates", "apps"]))
    print(f"session_state split: {n} files rewritten")


HUNK = re.compile(r"^@@ -(\d+)(?:,(\d+))? ")


def definition_lines(src, item):
    """1-based (first, last) lines of `item` in `src`, its doc comments and attributes included."""
    masked = mask(src)
    kind, name = item.split()
    m = re.search(rf"^[ \t]*(?:pub(?:\([a-z:]+\))?\s+)?{kind}\s+{re.escape(name)}\b", masked, re.M)
    if not m:
        return None
    i, depth = m.end(), 0
    while i < len(masked):
        c = masked[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                break
        elif c == ";" and depth == 0:
            break
        i += 1
    lines = src.splitlines()
    first = src.count("\n", 0, m.start()) + 1
    while first > 1 and lines[first - 2].lstrip().startswith(("///", "#[")):
        first -= 1
    return first, src.count("\n", 0, i) + 1


def moved_edits(base, head):
    diff = subprocess.run(["git", "diff", "-U0", base, head, "--", "crates/engine/src", SRC], capture_output=True, text=True, check=True).stdout
    current = None
    hunks, ranges, paths = {}, {}, {}
    for line in diff.splitlines():
        if line.startswith("--- "):
            path = line[4:].removeprefix("a/")
            current = path.removeprefix("crates/engine/src/").removeprefix(SRC) if path != "/dev/null" else None
            if current:
                paths[current] = path
        elif line.startswith("+++ "):
            continue
        elif current and (h := HUNK.match(line)):
            start, count = int(h[1]), int(h[2] or 1)
            # A pure insertion (count 0) sits after `start`.
            ranges.setdefault(current, []).append((start, start + max(count, 1) - 1) if count else (start, start + 1))
        elif line.startswith(("+", "-")) and current:
            hunks.setdefault(current, []).append(line[1:])
    for old_file, item, new_file in MOVED:
        name = item.split()[1]
        named = any(re.search(rf"\b{re.escape(name)}\b", line) for line in hunks.get(old_file, []))
        inside = False
        if old_file in ranges and (src := git_show(base, paths[old_file])) is not None:
            span = definition_lines(src, item)
            inside = span is not None and any(a <= span[1] and b >= span[0] for a, b in ranges[old_file])
        if named or inside:
            REPORT.append(f"NOT DONE: the branch edits {old_file} near `{item}`; that definition is in {SRC}{new_file} now")


def branch(base, head):
    changed = git("diff", "--name-only", f"{base}...{head}").split()
    n = sum(rewrite(p) for p in changed if p.endswith(".rs") and os.path.exists(p))
    moved_edits(base, head)
    print(f"session_state branch: {n} files rewritten")


CANNOT_DO = """
What this script cannot do (the compiler points at each):
  - a recording field reached through another binding (`let a = &mut s.actions; a.playing`) or set
    in an `ActionState { .. }` literal: those fields are `s.recording.<field>` now;
  - a file that now calls `preset_store()` / `preset_store_mut()` needs
    `use photocraft_engine::preset_store::PresetStoreExt;` (`crate::preset_store::...` in core);
  - an assignment to the old field (`s.preset_store = Some(store)`): attach a store with
    `Session::attach_preset_store`, which keeps it in the preset store slot;
  - edits to a moved definition: re-apply them in the new file named above;
  - a new `Session` field: a plain core field when the pipeline reads it, else a slot its crate
    registers (`Install::slot`, read through the crate's extension trait).
"""


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="mode", required=True)
    sub.add_parser("split")
    b = sub.add_parser("branch")
    b.add_argument("--base", required=True)
    b.add_argument("--head", required=True)
    args = ap.parse_args()
    os.chdir(git("rev-parse", "--show-toplevel").strip())
    if args.mode == "split":
        split()
    else:
        branch(args.base, args.head)
    for line in REPORT:
        print(line)
    print(CANNOT_DO)


if __name__ == "__main__":
    main()
