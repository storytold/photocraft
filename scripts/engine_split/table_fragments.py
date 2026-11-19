#!/usr/bin/env python3
"""Move a branch's edits of the old inline command table into the table fragments.

Before the table was split, `crates/engine/src/cmd.rs` held every core command in one `vec![...]`
inside `fn build()`. The entries now live in `crates/engine/src/cmd/table/<owner>/<menu>.rs`, and
`cmd/command_ids.txt` pins the registry order. A branch that edited the old table carries its
edits in `cmd.rs`. This script applies them to the fragments:

    python3 scripts/engine_split/table_fragments.py --base <merge-base> --head <branch>

Run it in a checkout of the new main (or of the branch rebased onto it, with its `cmd.rs` hunks
for the old table dropped). Per command id, compared between BASE and HEAD:
  changed  the entry in its fragment is replaced by HEAD's text (if the fragment still has
           BASE's text; otherwise it is reported as a CONFLICT and left alone);
  added    the entry goes after its predecessor's entry, in the predecessor's fragment, and its id
           goes after the predecessor's id in command_ids.txt;
  removed  the entry and its id are deleted.
Running it twice gives the same result. Anything it cannot place is listed at the end.
Edits outside the table (imports, helper fns, test modules) are not touched.
"""
import argparse, os, re, subprocess, sys

CMD = "crates/engine/src/cmd.rs"
TABLE = "crates/engine/src/cmd/table"
IDS = "crates/engine/src/cmd/command_ids.txt"
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


def build_entries(source):
    """Entries of the `vec![` that starts `fn build()` in the old cmd.rs."""
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
    return split_entries(source[start : i - 1])


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


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base", required=True, help="the branch's merge-base with the old main")
    ap.add_argument("--head", required=True, help="the branch")
    a = ap.parse_args()
    base = {entry_id(e): e for e in build_entries(git_show(a.base, CMD)) if entry_id(e)}
    head_list = [e for e in build_entries(git_show(a.head, CMD)) if entry_id(e)]
    head = {entry_id(e): e for e in head_list}
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
            if cid in base and tidy(base[cid]) == text:
                continue  # the branch did not touch it
            if tidy(cur) == text:
                continue
            if cid in base and tidy(cur) != tidy(base[cid]):
                report.append(f"CONFLICT {cid} in {path}: the fragment differs from BASE too; merge by hand")
                continue
            fr.replace(path, cur, ("\n" if cur.startswith("\n") else "") + text)
            report.append(f"changed  {cid} in {path}")
            continue
        if cid in base:
            report.append(f"MISSING  {cid}: edited by the branch, but no fragment holds it")
            continue
        pred = next((entry_id(head_list[k]) for k in range(pos - 1, -1, -1) if fr.find(entry_id(head_list[k]))), None)
        if pred is None:
            report.append(f"UNPLACED {cid}: no predecessor in the fragments")
            continue
        path, pe = fr.find(pred)
        fr.insert_after(path, pe, "\n" + text)
        ids.insert(ids.index(pred) + 1, cid) if cid not in ids else None
        report.append(f"added    {cid} after {pred} in {path}")
    for cid, e in base.items():
        if cid not in head and (here := fr.find(cid)):
            fr.delete(*here)
            ids = [i for i in ids if i != cid]
            report.append(f"removed  {cid} from {here[0]}")
    fr.save()
    open(IDS, "w").write("\n".join(ids) + "\n")
    print("\n".join(report) or "nothing to do")
    return 1 if any(r.startswith(("MISSING", "UNPLACED", "CONFLICT")) for r in report) else 0


if __name__ == "__main__":
    sys.exit(main())
