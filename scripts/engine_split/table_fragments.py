#!/usr/bin/env python3
"""Move a branch's edits of the old inline command table into the table fragments.

Before the table was split, `crates/engine/src/cmd.rs` held every core command in one `vec![...]`
inside `fn build()`. The entries now live in `crates/engine-core/src/cmd/table/<owner>/<menu>.rs`, and
`install/command_ids.txt` pins the registry order. A branch that edited the old table carries its
edits in `cmd.rs`. This script applies them to the fragments:

    python3 scripts/engine_split/table_fragments.py --base <merge-base> --head <branch>

Run it in a checkout of the new main (or of the branch rebased onto it, with its `cmd.rs` hunks
for the old table dropped). `--head` is the branch commit from before the rebase (`<branch>@{1}` or
`ORIG_HEAD` right after it). Per command id, compared between BASE and HEAD:
  changed  the entry in its fragment is replaced by HEAD's text (if the fragment still has
           BASE's text; otherwise it is reported as a CONFLICT and left alone);
  added    the entry goes after its predecessor's entry, in the predecessor's fragment, and its id
           goes after the predecessor's id in command_ids.txt;
  removed  the entry and its id are deleted (if the fragment still has BASE's text).
The adjustment table and its loop (`ADJ`, between the `vec![...]` and the `v.extend(...)` list) are
three-way merged into table/image/adjustments.rs. Any other change inside `build()` (the module
table list, the test-only command) is reported as UNHANDLED: `install_tables.py` moves module
tables into `TABLES` in install.rs; edit anything else there by hand.
Entries are compared without comments and whitespace, so rustfmt does not matter. Names a new
entry uses from cmd.rs are added to its fragment's `use crate::cmd::{..}` list. Run
`cargo fmt --all` afterwards. Running the script twice gives the same result. CONFLICT, UNHANDLED,
UNPLACED, MISSING and DUPLICATE lines mean exit status 1; resolve those by hand.
Edits outside `build()` (imports in cmd.rs, helper fns, test modules) are not touched.
"""
import argparse, os, re, subprocess, sys, tempfile

CMD = "crates/engine/src/cmd.rs"
TABLE = "crates/engine-core/src/cmd/table"
IDS = "crates/engine-core/src/install/command_ids.txt"
ID_RE = re.compile(r'"([a-zA-Z]+\.[A-Za-z0-9.]+)"')


def skip(text, i):
    """Index after the comment, string or char literal starting at i, or None."""
    if text.startswith("//", i):
        j = text.find("\n", i)
        return len(text) if j < 0 else j
    m = re.match(r'r(#*)"', text[i:])
    if m and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
        end = text.index('"' + m.group(1), i + len(m.group(0)))
        return end + 1 + len(m.group(1))
    if text[i] == '"':
        i += 1
        while text[i] != '"':
            i += 2 if text[i] == "\\" else 1
        return i + 1
    if text[i] == "'":
        m = re.match(r"'(\\.[^']*|[^\\'])'", text[i:])
        if m:
            return i + len(m.group(0))
    return None


def split_entries(body):
    """The top-level comma-separated entries of a `vec![...]` body."""
    entries, depth, start, i = [], 0, 0, 0
    while i < len(body):
        j = skip(body, i)
        if j is not None:
            i = j
            continue
        c = body[i]
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        elif c == "," and depth == 0:
            entries.append(body[start : i + 1])
            start = i + 1
        i += 1
    if body[start:].strip():
        entries.append(body[start:])
    return entries


def entry_id(entry):
    code = "\n".join(l for l in entry.splitlines() if not l.strip().startswith("//"))
    m = ID_RE.search(code)
    return m.group(1) if m else None


def git_show(ref, path):
    return subprocess.run(["git", "show", f"{ref}:{path}"], capture_output=True, text=True, check=True).stdout


class Fragments:
    """Every `vec![...]` of the fragment files, with its entries."""

    def __init__(self):
        self.files = {}
        for d, _, names in os.walk(TABLE):
            for n in names:
                if n.endswith(".rs") and os.path.dirname(os.path.join(d, n)) != TABLE:
                    p = os.path.join(d, n)
                    if "vec![" in open(p).read():
                        self.files[p] = open(p).read()

    def find(self, cid):
        """(path, entry text) of the entry for a command id, or None."""
        for p, t in self.files.items():
            for e in self.entries(t):
                if entry_id(e) == cid:
                    return p, e
        return None

    @staticmethod
    def spans(t):
        """(start, end) of each vec! body in a fragment file."""
        out, pos = [], 0
        while True:
            k = t.find("vec![", pos)
            if k < 0:
                return out
            s, depth, i = k + 5, 1, k + 5
            while depth:
                j = skip(t, i)
                if j is not None:
                    i = j
                    continue
                depth += t[i] == "["
                depth -= t[i] == "]"
                i += 1
            out.append((s, i - 1))
            pos = i

    def entries(self, t):
        return [e for s, f in self.spans(t) for e in split_entries(t[s:f])]

    def replace(self, path, old, new):
        t = self.files[path]
        assert t.count(old) == 1, (path, "entry not unique")
        self.files[path] = t.replace(old, new)

    def insert_after(self, path, old, new):
        self.replace(path, old, old + new)

    def delete(self, path, old):
        self.replace(path, old, "")

    def save(self):
        for p, t in self.files.items():
            open(p, "w").write(t)


def tidy(e):
    """Entry text as a fragment stores it: starts at its own line, ends with a comma."""
    e = e.strip("\n")
    return e if e.rstrip().endswith(",") else e.rstrip() + ","


def norm(e):
    """Entry text without comments and whitespace, and without trailing commas, for comparing."""
    out, i = [], 0
    while i < len(e):
        j = skip(e, i)
        if j is None:
            out.append(e[i])
            i += 1
        elif e.startswith("//", i):
            i = j
        else:
            out.append(e[i:j])
            i = j
    s = re.sub(r"\s+", "", "".join(out))
    return re.sub(r",(?=[)\]}])", "", s).rstrip(",")


def words(e):
    """Identifiers an entry uses as bare names (not paths, fields or inside literals)."""
    out, i = [], 0
    while i < len(e):
        j = skip(e, i)
        if j is None:
            out.append(e[i])
            i += 1
        else:
            out.append(" ")
            i = j
    return set(re.findall(r"(?<![:\w.])([A-Za-z_]\w*)(?![\w:])", "".join(out)))


def cmd_imports(source):
    """Names the top-level `use` items of cmd.rs bring in (visible to its descendants)."""
    names = set()
    for m in re.finditer(r"^(?:pub(?:\([a-z]+\))? )?use ([^;]*);", source, re.M | re.S):
        item = m.group(1)
        inner = re.search(r"\{(.*)\}", item, re.S)
        for n in (inner.group(1).split(",") if inner else [item]):
            n = n.strip().split("::")[-1].split(" as ")[-1].strip()
            if n and n != "self":
                names.add(n)
    return names


def build_parts(source):
    """(entries, adj_region, tail) of the old `fn build()`: the table vec, what the adjustment
    table adds, and the module table list with the rest of the function."""
    start = source.index("vec![", source.index("fn build()")) + len("vec![")
    depth, i = 1, start
    while depth:
        j = skip(source, i)
        if j is not None:
            i = j
            continue
        depth += source[i] == "["
        depth -= source[i] == "]"
        i += 1
    rest = source[i:]
    rest = rest[rest.index("\n") + 1 :]  # after `];`
    k = rest.index("    v.extend(")
    return split_entries(source[start : i - 1]), rest[:k].strip("\n") + "\n", rest[k:]


def merge_region(cur, base, head):
    """Three-way merge of text; (merged, ok)."""
    with tempfile.TemporaryDirectory() as d:
        paths = []
        for name, text in (("cur", cur), ("base", base), ("head", head)):
            p = os.path.join(d, name)
            open(p, "w").write(text)
            paths.append(p)
        r = subprocess.run(["git", "merge-file", "-p", *paths], capture_output=True, text=True)
        return r.stdout, r.returncode == 0


ADJ_FILE = os.path.join(TABLE, "image", "adjustments.rs")
ADJ_START = "    let mut v = Vec::new();\n"
ADJ_END = "\n    v\n}"


def add_imports(fr, path, entry, known):
    """Add names the entry uses from cmd.rs to the fragment's `use crate::cmd::{..}` list."""
    t = fr.files[path]
    m = re.match(r"use crate::cmd::\{(.*?)\};\n", t, re.S)
    if not m:
        return []
    have = {x.strip() for x in m.group(1).split(",") if x.strip()}
    need = sorted(n for n in words(entry) if n in known and n not in have)
    if not need:
        return []
    allu = sorted(have | set(need), key=lambda s: (s[0].islower(), s))
    fr.files[path] = "use crate::cmd::{%s};\n" % ", ".join(allu) + t[m.end() :]
    return need


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base", required=True, help="the branch's merge-base with the old main")
    ap.add_argument("--head", required=True, help="the branch")
    a = ap.parse_args()
    base_src, head_src = git_show(a.base, CMD), git_show(a.head, CMD)
    b_entries, b_adj, b_tail = build_parts(base_src)
    h_entries, h_adj, h_tail = build_parts(head_src)
    base = {entry_id(e): e for e in b_entries if entry_id(e)}
    head_list = [e for e in h_entries if entry_id(e)]
    head_ids = {entry_id(e) for e in head_list}
    known = cmd_imports(head_src)
    fr = Fragments()
    ids = open(IDS).read().split("\n")
    if ids and ids[-1] == "":
        ids.pop()
    report = []
    for pos, e in enumerate(head_list):
        cid = entry_id(e)
        text = tidy(e)
        here = fr.find(cid)
        if here:
            path, cur = here
            if norm(cur) == norm(text) or (cid in base and norm(base[cid]) == norm(text)):
                continue  # already applied, or the branch did not touch it
            if cid not in base:
                report.append(f"DUPLICATE {cid}: the branch adds it, but {path} already has a different one")
            elif norm(cur) != norm(base[cid]):
                report.append(f"CONFLICT {cid} in {path}: the fragment differs from BASE too; merge by hand")
            else:
                fr.replace(path, cur, ("\n" if cur.startswith("\n") else "") + text)
                report.append(f"changed  {cid} in {path}")
                added = add_imports(fr, path, text, known)
                if added:
                    report.append(f"imports  added to {path}: {', '.join(added)}")
            continue
        if cid in base:
            report.append(f"MISSING  {cid}: edited by the branch, but no fragment holds it")
            continue
        if cid in ids:
            report.append(f"DUPLICATE {cid}: the branch adds it, but the registry already has that id (a module table?)")
            continue
        pred = next((entry_id(head_list[k]) for k in range(pos - 1, -1, -1) if fr.find(entry_id(head_list[k]))), None)
        if pred is None:
            report.append(f"UNPLACED {cid}: no predecessor in the fragments")
            continue
        path, pe = fr.find(pred)
        comma = "" if pe.rstrip().endswith(",") else ","
        fr.replace(path, pe, pe.rstrip() + comma + pe[len(pe.rstrip()) :] + "\n" + text)
        ids.insert(ids.index(pred) + 1, cid)
        report.append(f"added    {cid} after {pred} in {path}")
        added = add_imports(fr, path, text, known)
        if added:
            report.append(f"imports  added to {path}: {', '.join(added)}")
    for cid, e in base.items():
        if cid in head_ids or not (here := fr.find(cid)):
            continue
        if norm(here[1]) == norm(e):
            fr.delete(*here)
            ids = [i for i in ids if i != cid]
            report.append(f"removed  {cid} from {here[0]}")
        else:
            report.append(f"CONFLICT {cid} in {here[0]}: the branch removes it, but the fragment differs from BASE")
    # the adjustment table and its loop
    if norm(h_adj) != norm(b_adj):
        cur = open(ADJ_FILE).read()
        i, j = cur.index(ADJ_START) + len(ADJ_START), cur.rindex(ADJ_END)
        merged, ok = merge_region(cur[i:j] + "\n", b_adj, h_adj)
        if not ok:
            report.append(f"CONFLICT ADJ table: {ADJ_FILE} differs from BASE where the branch edits it; merge by hand")
        elif merged != cur[i:j] + "\n":
            open(ADJ_FILE, "w").write(cur[:i] + merged.rstrip("\n") + cur[j:])
            report.append(f"changed  ADJ table in {ADJ_FILE}")
    if norm(h_tail) != norm(b_tail):
        report.append("UNHANDLED the branch edits the end of build() (module table list or test-only command): run install_tables.py, or edit TABLES in install.rs by hand")
    fr.save()
    open(IDS, "w").write("\n".join(ids) + "\n")
    print("\n".join(report) or "nothing to do")
    if any(r.startswith(("changed", "added", "removed")) for r in report):
        print("run `cargo fmt --all` next")
    return 1 if any(r.split()[0] in ("MISSING", "UNPLACED", "CONFLICT", "UNHANDLED", "DUPLICATE") for r in report) else 0


if __name__ == "__main__":
    sys.exit(main())
