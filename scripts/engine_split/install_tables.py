#!/usr/bin/env python3
"""Move a branch's added or removed command tables into the engine's install list.

The engine's command tables are listed, in registry order, in `TABLES` in
`crates/engine-core/src/install.rs` (`crates/engine/src/install.rs` before the crate split). Before that, `crates/engine/src/cmd.rs` listed them: first as
`v.extend(<module>::specs());` lines in `fn build()`, later as `("<name>", <path>),` entries in
`SPECS`. A branch that added or removed a table carries that edit in `cmd.rs`. This script applies
it to `TABLES`:

    python3 scripts/engine_split/install_tables.py --base <merge-base> --head <branch>

Run it in a checkout of the new main (or of the branch rebased onto it, with its `cmd.rs` hunks
for the old list dropped). `--head` is the branch commit from before the rebase (`<branch>@{1}` or
`ORIG_HEAD` right after it): the rebased branch has no old list left to read, and its added tables
would be lost. Per table, compared between BASE and HEAD:
  added    the entry goes after its predecessor's entry in TABLES (the nearest earlier table of
           HEAD's list that TABLES has; after the last table fragment when there is none);
  removed  the entry is deleted.
Running it twice gives the same result. Anything it cannot place is listed at the end. Edits of
inline commands are `table_fragments.py`'s; run both.
"""
import argparse, re, subprocess, sys

CMD = "crates/engine/src/cmd.rs"
INSTALL = "crates/engine-core/src/install.rs"
INSTALL_BEFORE_SPLIT = "crates/engine/src/install.rs"
EXTEND_RE = re.compile(r"^\s*v\.extend\(((?:crate::)?(?:cmd::)?)([a-z0-9_:]+?)::([a-z0-9_]+)\(\)\);", re.M)
ENTRY_RE = re.compile(r'^\s*\("([^"]+)",\s*([A-Za-z0-9_:]+)\),', re.M)


def git_show(ref, path):
    r = subprocess.run(["git", "show", f"{ref}:{path}"], capture_output=True, text=True)
    return r.stdout if r.returncode == 0 else None


def block(source, start):
    """The text from `start` to the `];` that closes the list it opens."""
    i = source.index(start)
    return source[i : source.index("\n];", i)]


def tables(ref):
    """[(name, path as install.rs writes it)] of the table list in REF's cmd.rs, in order."""
    source = git_show(ref, CMD)
    if source is None:
        sys.exit(f"{ref} has no {CMD}")
    if "const SPECS:" in source:
        out = []
        for name, path in ENTRY_RE.findall(block(source, "const SPECS:")):
            if not path.startswith("crate::"):
                path = "crate::cmd::" + path
            out.append((name, path))
        return out
    out = []
    for prefix, module, fn in EXTEND_RE.findall(source[source.index("fn build()") :]):
        # A path written relative to cmd.rs names a module of `cmd`.
        path = ("crate::" + module if prefix == "crate::" else "crate::cmd::" + module) + "::" + fn
        name = module.split("::")[-1] + ("" if fn == "specs" else "::" + fn)
        out.append((name, path))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base", required=True, help="the branch's merge-base with the old main")
    ap.add_argument("--head", required=True, help="the branch")
    a = ap.parse_args()
    if git_show(a.head, INSTALL) is not None or git_show(a.head, INSTALL_BEFORE_SPLIT) is not None:
        print(
            f"warning: {a.head} is already on the install.rs layout; if you rebased, pass the "
            "pre-rebase commit as --head (a rebased branch has lost its added tables)",
            file=sys.stderr,
        )
        return 2
    base = dict(tables(a.base))
    head = tables(a.head)
    head_names = {n for n, _ in head}
    text = open(INSTALL).read()
    start = text.index("const TABLES:")
    end = text.index("\n];", start)
    lines = text[start:end].split("\n")
    header, rows = lines[0], lines[1:]
    names = lambda: [m.group(1) if (m := ENTRY_RE.match(r)) else None for r in rows]
    report = []
    for pos, (name, path) in enumerate(head):
        if name in base or name in names():
            continue
        pred = next((head[k][0] for k in range(pos - 1, -1, -1) if head[k][0] in names()), None)
        current = names()
        if pred is not None:
            at = len(current) - current[::-1].index(pred)
        else:
            fragments = [i for i, r in enumerate(rows) if "crate::cmd::table::" in r]
            if not fragments:
                report.append(f"UNPLACED {name}: no predecessor in TABLES")
                continue
            at = fragments[-1] + 1
            pred = "the last table fragment"
        rows.insert(at, f'    ("{name}", {path}),')
        report.append(f"added    {name} ({path}) after {pred}")
    for name in base:
        if name not in head_names and name in names():
            rows = [r for r, n in zip(rows, names()) if n != name]
            report.append(f"removed  {name}")
    text = text[:start] + "\n".join([header] + rows) + text[end:]
    open(INSTALL, "w").write(text)
    print("\n".join(report) or "nothing to do")
    if any(r.startswith("added") for r in report):
        print(
            "next: run the test `install::order_tests` with PHOTOCRAFT_WRITE_COMMAND_IDS=1 to regenerate "
            "install/command_ids.txt, and review its diff"
        )
    return 1 if any(r.startswith("UNPLACED") for r in report) else 0


if __name__ == "__main__":
    sys.exit(main())
