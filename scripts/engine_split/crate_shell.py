#!/usr/bin/env python3
"""Engine split, step 3: `crates/engine/src` becomes the `photocraft-engine-core` crate.

The engine's sources live in `crates/engine-core/src`. `crates/engine` keeps the package name
`photocraft-engine`: a thin top crate (`src/facade.rs`) that re-exports core, installs every
engine command and holds `tests/` and `examples/`. Core's `Session::new()` installs nothing, so
core's own tests create sessions with `crate::session()`, and a test that reads the registry
without a session installs first with `crate::install()`.

    python3 scripts/engine_split/crate_shell.py split
        On a tree whose `crates/engine/src/lib.rs` is still the whole engine: `git mv` the sources
        to `crates/engine-core/src`, then rewrite core's tests (below). The hand-written part of the
        step (the top crate, the Cargo files, tooling) is its own commit.

    python3 scripts/engine_split/crate_shell.py branch --base <merge-base> --head <branch>
        On a branch rebased onto the split. `--head` is the branch commit from before the rebase
        (`ORIG_HEAD` right after it). It
          - moves every file the branch left under `crates/engine/src` (git follows renames of
            files and of directories that disappeared, not new files at the old `src/` root)
            into `crates/engine-core/src`, except the top crate's `facade.rs`;
          - rewrites core's tests, which covers the branch's new tests;
          - applies the branch's added and removed lines of `crates/engine/Cargo.toml`, per
            section, to `crates/engine-core/Cargo.toml`: the dependencies live there. Resolve a
            conflict in `crates/engine/Cargo.toml` by taking main's version first.

Core's tests, in test code only (`#[cfg(test)]` modules and items):
  `Session::new()` -> `crate::session()`;
  a `#[test]` function that calls a registry reader (`commands::find`, `command_specs`, and the
  readers in READERS) and creates no session gets `crate::install().unwrap();` as its first line.
  `Session` leaves a `use crate::...`/`use super::...` line that the rewrite left it unused in.
Running either mode twice gives the same result. Run `cargo fmt --all` after it: dropping an unused
`Session` import can leave the others out of rustfmt's order. What it cannot do is listed at the end.
"""
import argparse, os, re, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rust_source import CFG_TEST, block_end, git, git_show, in_test, mask, test_fns, test_regions  # noqa: E402

OLD_SRC = "crates/engine/src"
CORE_SRC = "crates/engine-core/src"
TOP_OWN = {"facade.rs"}
OLD_CARGO = "crates/engine/Cargo.toml"
CORE_CARGO = "crates/engine-core/Cargo.toml"

SESSION_NEW = re.compile(r"\b(?:crate::)?Session::new\(\)")
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
    """The test code of the edited file: all of it for a test file, else its `#[cfg(test)]` items."""
    if ranges is None:
        return [(0, len(src))]
    m = mask(src)
    return [(a.start(), block_end(m, a.end())) for a in CFG_TEST.finditer(m)]


def split():
    if os.path.isdir(CORE_SRC):
        print(f"{CORE_SRC} exists; moving nothing")
    else:
        if not os.path.isfile(os.path.join(OLD_SRC, "lib.rs")):
            sys.exit(f"{OLD_SRC}/lib.rs is missing: not a tree from before the split")
        os.makedirs(os.path.dirname(CORE_SRC), exist_ok=True)
        git("mv", OLD_SRC, CORE_SRC)
        print(f"moved {OLD_SRC} -> {CORE_SRC}")
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


def cargo_lines(base, head):
    """The branch's Cargo.toml edits: {section: (added, removed)}, dependency lines only."""
    b, h = git_show(base, OLD_CARGO), git_show(head, OLD_CARGO)
    if b is None or h is None:
        return {}
    bs, hs = dict(sections(b)), dict(sections(h))
    key = re.compile(r"^\s*[A-Za-z0-9_-]+\s*=")
    out = {}
    for sec in set(bs) | set(hs):
        before, after = [x for x in bs.get(sec, []) if key.match(x)], [x for x in hs.get(sec, []) if key.match(x)]
        added, removed = [x for x in after if x not in before], [x for x in before if x not in after]
        if added or removed:
            out[sec] = (added, removed)
    return out


def apply_cargo(edits):
    if not edits:
        return []
    secs = sections(open(CORE_CARGO).read())
    problems = []
    for sec, (added, removed) in sorted(edits.items()):
        target = next((lines for header, lines in secs if header == sec), None)
        if target is None:
            secs.append((sec, []))
            target = secs[-1][1]
        for line in removed:
            if line in target:
                target.remove(line)
        for line in added:
            name = line.split("=")[0].strip()
            if any(l.split("=")[0].strip() == name for l in target if "=" in l):
                if line not in target:
                    problems.append(f"{CORE_CARGO} {sec}: `{name}` is there already in another form; the branch has `{line.strip()}`")
                continue
            end = len(target)
            while end and not target[end - 1].strip():
                end -= 1
            target.insert(end, line)
        print(f"{CORE_CARGO} {sec}: +{len(added)} -{len(removed)}")
    text = "\n".join(([] if not secs[0][1] else ["\n".join(secs[0][1])]) + [h + "\n" + "\n".join(lines) for h, lines in secs[1:]])
    open(CORE_CARGO, "w").write(text if text.endswith("\n") else text + "\n")
    return problems


def branch(base, head):
    if not os.path.isdir(CORE_SRC):
        sys.exit(f"{CORE_SRC} is missing: rebase onto the split first")
    problems = stray_files()
    rewrite_tests(CORE_SRC)
    problems += apply_cargo(cargo_lines(base, head))
    return problems


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


if __name__ == "__main__":
    main()
