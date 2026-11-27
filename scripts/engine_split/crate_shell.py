#!/usr/bin/env python3
"""Engine split, step 3: `crates/engine/src` becomes the `photocraft-engine-core` crate.

The engine's sources live in `crates/engine-core/src`. `crates/engine` keeps the package name
`photocraft-engine`: a thin top crate (`src/facade.rs`) that re-exports core, installs every
engine command and holds `tests/` and `examples/`. Core's `Session::new()` installs nothing, so
core's own tests create sessions with `crate::session()`, and a test that reads the registry
without a session installs first with `crate::install()`.

    python3 scripts/engine_split/crate_shell.py split
        On a tree whose `crates/engine/src/lib.rs` is still the whole engine: `git mv` the sources
        to `crates/engine-core/src` and the manifest to `crates/engine-core/Cargo.toml`, then
        rewrite core's tests (below). The hand-written part of the
        step (the top crate, the Cargo files, tooling) is its own commit.

    python3 scripts/engine_split/crate_shell.py branch --base <merge-base> --head <branch>
        On a branch rebased onto the split. `--head` is the branch commit from before the rebase
        (`ORIG_HEAD` right after it). It
          - moves every file the branch left under `crates/engine/src` (git follows renames of
            files and of directories that disappeared, not new files at the old `src/` root)
            into `crates/engine-core/src`, except the top crate's `facade.rs`;
          - rewrites core's tests, which covers the branch's new tests;
          - applies the branch's added and removed entries of `crates/engine/Cargo.toml`, per
            section, to the manifests: dependencies live in `crates/engine-core/Cargo.toml`, and
            dev-dependencies go to the manifest whose tests or examples the branch touches (both
            when it touches both or neither). Resolve a conflict in `crates/engine/Cargo.toml` by
            taking main's version first. Entries are copied whole (multi-line values, comments
            above them, `[dependencies.x]` tables).

Core's tests, in test code only (`#[cfg(test)]` modules and items, bare `#[test]` functions):
  `Session::new()` -> `crate::session()`;
  a `#[test]` function that calls a registry reader (`commands::find`, `command_specs`, and the
  readers in READERS) and creates no session gets `crate::install().unwrap();` as its first line.
  `Session` leaves a `use crate::...`/`use super::...` line that the rewrite left it unused in.
Running either mode twice gives the same result. Run `cargo fmt --all` after it: dropping an unused
`Session` import can leave the others out of rustfmt's order. What it cannot do is printed at the end of every run.
"""
import argparse, os, re, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rust_source import block_end, git, git_show, in_test, item_ranges, mask, test_fns, test_regions  # noqa: E402

OLD_SRC = "crates/engine/src"
CORE_SRC = "crates/engine-core/src"
TOP_OWN = {"facade.rs"}
OLD_CARGO = "crates/engine/Cargo.toml"
CORE_CARGO = "crates/engine-core/Cargo.toml"

# `Session::new()` and `Session::default()` behind any lowercase module path (`crate::`, `super::`,
# `session::`): the whole match becomes `crate::session()`.
SESSION_NEW = re.compile(r"(?<![\w:])(?:[a-z_][a-z0-9_]*::)*Session::(?:new|default)\(\)")
# Functions that read the command registry with no session to show an install happened.
READERS = re.compile(
    r"\b(?:commands|cmd|registry)::(?:find|command_specs)\s*\(|\bcommand_specs\s*\(|"
    r"\b(?:bindable_items|bindable|replayable|is_step_command|check_known|menu_item_needs_settings|menu_item_commands|"
    r"menu_item_command|unknown_command_hint|unknown_method|command_defs|prefs_overrides|overrides_to_prefs|web_document|"
    r"print_image)\s*\(|\b(?:shortcut_sets::)?bound\s*\(|"
    # An unknown-command error's message lists the registered ids nearest to it.
    r"\bUnknownCommand\s*\("
)
CREATES_SESSION = re.compile(r"\bsession\s*\(|\binstall\s*\(\s*\)")


def rewrite_tests(root):
    regions = test_regions(os.path.join(root, "lib.rs"))
    counts = {"session": 0, "install": 0, "files": 0}
    for f, ranges in sorted(regions.items()):
        if not f.startswith(root + os.sep):
            continue
        src = open(f).read()
        m = mask(src)
        edits = []  # (start, end, replacement)
        for x in SESSION_NEW.finditer(m):
            if in_test(ranges, x.start()):
                edits.append((x.start(), x.end(), "crate::session()"))
                counts["session"] += 1
        readers = file_readers(m, ranges)
        for name, body_start, body_end in test_fns(m):
            if not in_test(ranges, body_start):
                continue
            body = m[body_start:body_end]
            read = readers.search(body)
            first_session = min((x.start() for x in (CREATES_SESSION.search(body), SESSION_NEW.search(body)) if x), default=len(body))
            if read and read.start() < first_session:
                line_start = m.rfind("\n", 0, body_start) + 1
                indent = re.match(r"[ \t]*", src[line_start:]).group(0) + "    "
                edits.append((body_start, body_start, f"\n{indent}crate::install().unwrap();"))
                counts["install"] += 1
        if edits:
            for a, b, text in sorted(edits, reverse=True):
                src = src[:a] + text + src[b:]
            src = drop_unused_session_imports(src, ranges)
            open(f, "w").write(src)
            counts["files"] += 1
    print(f"core tests: {counts['session']} Session::new() -> crate::session(), {counts['install']} install lines, {counts['files']} files")


USE_SESSION = re.compile(r"^[ \t]*use (?:crate|super)::(?:Session|\{([^}]*)\});[ \t]*\n", re.M)


HELPER_FN = re.compile(r"\bfn\s+([a-z_][a-z0-9_]*)\s*(?:<[^>{]*>)?\s*\(")


def file_readers(m, ranges):
    """READERS plus the file's test helpers that call one (`fn label(id) { find(id)... }`)."""
    names = []
    for h in HELPER_FN.finditer(m):
        if not in_test(ranges, h.start()):
            continue
        open_brace = m.find("{", h.end())
        if open_brace < 0:
            continue
        if READERS.search(m[open_brace : block_end(m, open_brace)]):
            names.append(h.group(1))
    if not names:
        return READERS
    return re.compile(READERS.pattern + r"|\b(?:" + "|".join(sorted(set(names))) + r")\s*\(")


def drop_unused_session_imports(src, ranges):
    """Removes `Session` from test code's `use crate::...`/`use super::...` lines where the
    `Session::new()` rewrite left it unused (in the test file, or in the test item holding the line)."""
    regions = test_regions_of(src, ranges)
    for start, end in sorted(regions, reverse=True):
        m = mask(src[start:end])
        for u in reversed(list(USE_SESSION.finditer(m))):
            names = [n.strip() for n in u.group(1).split(",")] if u.group(1) is not None else ["Session"]
            if "Session" not in names:
                continue
            others = len(re.findall(r"\bSession\b", m)) - 1
            if others > 0:
                continue
            rest = [n for n in names if n and n != "Session"]
            a, b = start + u.start(), start + u.end()
            line = src[a:b]
            indent = re.match(r"[ \t]*", line).group(0)
            root = "crate" if "use crate::" in line else "super"
            text = "" if not rest else f"{indent}use {root}::{rest[0]};\n" if len(rest) == 1 else f"{indent}use {root}::{{{', '.join(rest)}}};\n"
            src = src[:a] + text + src[b:]
            m = mask(src[start:end])
    return src


def test_regions_of(src, ranges):
    """The test code of the edited file: all of it for a test file, else its test items."""
    if ranges is None:
        return [(0, len(src))]
    return item_ranges(mask(src))


def split():
    if os.path.isdir(CORE_SRC):
        print(f"{CORE_SRC} exists; moving nothing")
    else:
        if not os.path.isfile(os.path.join(OLD_SRC, "lib.rs")):
            sys.exit(f"{OLD_SRC}/lib.rs is missing: not a tree from before the split")
        os.makedirs(os.path.dirname(CORE_SRC), exist_ok=True)
        git("mv", OLD_SRC, CORE_SRC)
        print(f"moved {OLD_SRC} -> {CORE_SRC}")
    if not os.path.isfile(CORE_CARGO):
        # Core keeps the engine's manifest (its history, its dependencies); the top crate's is new.
        git("mv", OLD_CARGO, CORE_CARGO)
        print(f"moved {OLD_CARGO} -> {CORE_CARGO}")
    rewrite_tests(CORE_SRC)


def stray_files():
    left = []
    for d, _, names in os.walk(OLD_SRC):
        for n in names:
            rel = os.path.relpath(os.path.join(d, n), OLD_SRC)
            if rel not in TOP_OWN:
                left.append(rel)
    problems = []
    for rel in sorted(left):
        dest = os.path.join(CORE_SRC, rel)
        if os.path.exists(dest):
            problems.append(f"{OLD_SRC}/{rel}: {dest} exists; merge the two by hand")
            continue
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        if git("ls-files", "--error-unmatch", os.path.join(OLD_SRC, rel), check=False):
            git("mv", os.path.join(OLD_SRC, rel), dest)
        else:
            os.rename(os.path.join(OLD_SRC, rel), dest)
        print(f"moved {OLD_SRC}/{rel} -> {dest}")
    return problems


def sections(text):
    """[(header, [lines])]; the lines before the first header have header ''."""
    out = [("", [])]
    for line in text.split("\n"):
        if re.match(r"^\[.*\]\s*$", line):
            out.append((line.strip(), []))
        else:
            out[-1][1].append(line)
    return out


DEP_TABLE = re.compile(r"^\[(?:target\..*\.)?(?:dev-|build-)?dependencies\.[^\]]+\]$")
KEY = re.compile(r"^\s*([A-Za-z0-9_.\"-]+)\s*=")


def depth(line):
    """Net opened brackets of a TOML line, ignoring strings and comments."""
    line = re.sub(r'"(?:[^"\\]|\\.)*"|\'[^\']*\'', "", line)
    line = line.split("#")[0]
    return sum(line.count(c) for c in "{[") - sum(line.count(c) for c in "}]")


def entries(lines):
    """[(name, first, end, body)] of a section's `key = value` entries. `first` includes the comment
    lines right above the key; a value that spans lines runs to its closing bracket."""
    out, i, comment_from = [], 0, None
    while i < len(lines):
        line = lines[i]
        if line.lstrip().startswith("#"):
            comment_from = i if comment_from is None else comment_from
            i += 1
            continue
        m = KEY.match(line)
        if not m:
            comment_from = None
            i += 1
            continue
        end, d = i + 1, depth(line)
        while d > 0 and end < len(lines):
            d += depth(lines[end])
            end += 1
        out.append((m.group(1), i if comment_from is None else comment_from, end, lines[i:end]))
        comment_from = None
        i = end
    return out


def cargo_edits(base, head):
    """The branch's Cargo.toml edits: [(op, header, name, text_lines, body)] with op add/remove/table_add/table_remove/table_change."""
    b, h = git_show(base, OLD_CARGO), git_show(head, OLD_CARGO)
    if b is None or h is None:
        return []
    bs, hs = dict(sections(b)), dict(sections(h))
    ops = []
    for sec in sorted(set(bs) | set(hs)):
        if DEP_TABLE.match(sec):
            if sec not in bs:
                ops.append(("table_add", sec, "", hs[sec], hs[sec]))
            elif sec not in hs:
                ops.append(("table_remove", sec, "", bs[sec], bs[sec]))
            elif bs[sec] != hs[sec]:
                ops.append(("table_change", sec, "", hs[sec], hs[sec]))
            continue
        be = {n: (f, e, body) for n, f, e, body in entries(bs.get(sec, []))}
        he = {n: (f, e, body) for n, f, e, body in entries(hs.get(sec, []))}
        for n, (f, e, body) in he.items():
            if n not in be or be[n][2] != body:
                ops.append(("add", sec, n, hs[sec][f:e], body))
        for n, (f, e, body) in be.items():
            if n not in he:
                ops.append(("remove", sec, n, bs[sec][f:e], body))
    return ops


def trim_end(lines):
    end = len(lines)
    while end and not lines[end - 1].strip():
        end -= 1
    return end


def apply_cargo(path, ops):
    """Applies `ops` to the manifest at `path`; returns what it could not do."""
    problems = []
    secs = sections(open(path).read())
    for op, sec, name, text, body in ops:
        where = f"{path} {sec}"
        found = next((lines for header, lines in secs if header == sec), None)
        if op.startswith("table_"):
            if op == "table_remove":
                secs[:] = [(h, l) for h, l in secs if h != sec]
            elif found is None:
                secs.append((sec, text))
            elif trim_end(found) and [x for x in found if x.strip()] != [x for x in text if x.strip()]:
                problems.append(f"{where}: the table is there already in another form; the branch has {len(text)} lines for it")
            continue
        if found is None:
            if op == "remove":
                continue
            secs.append((sec, []))
            found = secs[-1][1]
        have = next((e for e in entries(found) if e[0] == name), None)
        if op == "remove":
            if have is None:
                continue
            if have[3] != body:
                problems.append(f"{where}: `{name}` changed on main since the branch's base; the branch removes it, remove it by hand")
                continue
            del found[have[1] : have[2]]
        elif have is not None:
            if have[3] != body:
                problems.append(f"{where}: `{name}` is there already in another form; the branch has `{body[0].strip()}`")
        else:
            at = trim_end(found)
            found[at:at] = text
    text = "\n".join(([] if not secs[0][1] else ["\n".join(secs[0][1])]) + [h + "\n" + "\n".join(lines) for h, lines in secs[1:]])
    open(path, "w").write(text if text.endswith("\n") else text + "\n")
    return problems


def route_cargo(base, head, ops):
    """{manifest: ops}. Dev-dependencies go to the manifest whose tests or examples the branch touches,
    both when it touches both or neither; everything else to core. A normal dependency the branch adds
    while it touches the top's tests or examples is also a dev-dependency of the top. An added feature
    is not forwarded by the top: reported."""
    files = git("diff", "--name-only", base, head).split()
    core = any(f.startswith(OLD_SRC + "/") for f in files)
    top = any(f.startswith(("crates/engine/tests/", "crates/engine/examples/")) for f in files)
    both = core == top
    out = {CORE_CARGO: [], OLD_CARGO: []}
    notes = []
    for op in ops:
        kind, sec, name, text, body = op
        if "dev-dependencies" in sec:
            if both or core:
                out[CORE_CARGO].append(op)
            if both or top:
                out[OLD_CARGO].append(op)
            continue
        out[CORE_CARGO].append(op)
        if sec == "[dependencies]" and kind == "add" and top:
            out[OLD_CARGO].append((kind, "[dev-dependencies]", name, text, body))
        if sec == "[features]" and kind == "add":
            notes.append(f"{OLD_CARGO} [features]: the branch adds `{name}`; the top crate forwards features, so add `{name} = [\"photocraft-engine-core/{name}\"]` there")
    return out, notes


def branch(base, head):
    if not os.path.isdir(CORE_SRC):
        sys.exit(f"{CORE_SRC} is missing: rebase onto the split first")
    problems = stray_files()
    rewrite_tests(CORE_SRC)
    routed, notes = route_cargo(base, head, cargo_edits(base, head))
    problems += notes
    for path, ops in routed.items():
        if ops:
            problems += apply_cargo(path, ops)
            print(f"{path}: {len(ops)} edits")
    return problems


CANNOT_DO = """
What this script cannot do:
  - Registry readers reached through another function. A test that calls `bindable_items`, a `menu_item_*`
    function, `command_hints` or the like before it creates a session gets an install line only when the
    reader or a helper in its own file is in READERS; CI's per-process runs find the rest ("NotInstalled").
  - Cargo edits the top crate needs beyond the routing above: features it must forward (listed as NOT DONE),
    and a dependency used by both `tests/` and core's tests when the branch's changes don't show it.
  - Conflicted files: an unmerged path stops `git mv` part-way; resolve conflicts, then run it again.
  - Non-test code: a new production `Session::new()` is left alone on purpose.
"""


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="mode", required=True)
    sub.add_parser("split")
    b = sub.add_parser("branch")
    b.add_argument("--base", required=True)
    b.add_argument("--head", required=True)
    a = p.parse_args()
    top = git("rev-parse", "--show-toplevel").strip()
    os.chdir(top)
    problems = split() if a.mode == "split" else branch(a.base, a.head)
    for line in problems or []:
        print("NOT DONE  " + line)
    print(CANNOT_DO)


if __name__ == "__main__":
    main()
