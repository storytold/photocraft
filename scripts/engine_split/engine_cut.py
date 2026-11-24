#!/usr/bin/env python3
"""Engine split, steps 7-12: cut one engine crate out of `photocraft-engine-core`.

`scripts/engine_split/crates.tsv` names the crate of each engine module. A cut moves the modules of
one crate, with every file their `mod` declarations reach, from `crates/engine-core/src` to the same
paths under `crates/engine-<crate>/src`, so git's rename detection carries edits to them.

    python3 scripts/engine_split/engine_cut.py split <crate>
        On the day's main: `git mv` the crate's files, drop their declarations from core's `lib.rs`,
        `cmd.rs` and `cmd/table.rs`, and in the moved files point `crate::cmd::<core item>` at
        `crate::commands::<core item>` (the crate's own `cmd` module holds only its modules). Calls of
        `Session` methods that became the crate's traits get the trait's import (TRAITS). The
        hand-written part of the step (the crate's Cargo.toml, lib.rs, cmd.rs, install.rs, the
        install lists, tooling) is its own commit, cherry-picked after this one.

    python3 scripts/engine_split/engine_cut.py imports <crate>
        Prints the import block the crate's root needs: every name its code reaches as `crate::x`
        that the crate doesn't define, so moved code compiles without path edits.

    python3 scripts/engine_split/engine_cut.py branch <crate> --base <merge-base> --head <branch>
        On a branch rebased onto the cut (`--head` is the branch from before the rebase, `ORIG_HEAD`
        right after it):
          - moves files the branch added under core that belong to the crate's modules (new files in
            a directory that still exists in core, such as `cmd/`, stay where git put them);
          - moves `mod` lines the branch added to core's `cmd.rs`/`lib.rs` for the crate's modules,
            and `TABLES` entries it added to core's `install.rs` naming them, into the crate's files;
          - adds the import-block names the branch's moved code needs;
          - adds trait imports for the trait methods it calls;
          - lists, as NOT DONE, tests the branch added or edited in core that run one of the
            crate's command ids: core installs only its own commands, so they fail at run time
            with `UnknownCommand` and belong in the top crate's `tests/it`.

    cargo check --message-format json ... | python3 scripts/engine_split/engine_cut.py fix
        Widens to `pub` each core item the compiler reports as private where another engine crate
        uses it (E0603, E0624, and E0616 when the note names the field).

Running split or branch twice gives the same result. What it cannot do is printed at the end.
"""
import argparse, json, os, re, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rust_source import MOD_DECL, compiled_files, git, git_show, inline_mods, mask, module_file  # noqa: E402

CORE = "crates/engine-core/src"
MAP = os.path.join(os.path.dirname(os.path.abspath(__file__)), "crates.tsv")
ROOTS = ["crates", "apps"]

# Session methods that became a crate's extension traits at its cut: method -> (module, trait).
TRAITS = {
    "paint": {
        "offer_live_stroke": ("brush_cmds", "LiveStrokeExt"),
        "offer_live_retouch": ("retouch_cmds", "LiveRetouchExt"),
        "offer_live_dab": ("retouch_cmds", "LiveRetouchExt"),
    },
}

# Names a crate's root defines itself, which its import block must not bring from core.
OWN = {"cmd", "install", "session", "command_tables", "register"}


def crate_src(crate):
    return f"crates/engine-{crate}/src"


def units(crate):
    out = []
    with open(MAP) as f:
        for line in f:
            if line.startswith("#") or line.startswith("unit\t") or not line.strip():
                continue
            u, c = line.rstrip("\n").split("\t")
            if c == crate:
                out.append(u)
    return out


def unit_root(u, src=CORE):
    for f in (f"{src}/{u}.rs", f"{src}/{u}/mod.rs"):
        if os.path.isfile(f):
            return f
    return None


def reach(root):
    """`root` and every file its `mod` declarations reach."""
    seen, todo = [], [root]
    while todo:
        f = todo.pop()
        if f in seen:
            continue
        seen.append(f)
        src = open(f).read()
        m = mask(src)
        for d in MOD_DECL.finditer(m):
            child = module_file(f, d.group(2), src[d.start(1) : d.end(1)], [dir_ for dir_, _ in inline_mods(src, m, d.start())])
            if child:
                todo.append(os.path.normpath(child))
    return seen


def crate_files(crate, src=CORE):
    files = []
    for u in units(crate):
        r = unit_root(u, src)
        if r:
            files += [f for f in reach(r) if f not in files]
    return files


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def write(path, text):
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)


def decl_re(name):
    """A declaration line of module `name` with its attributes, or a `pub use cmd::name;` line."""
    return re.compile(r"^(?:[ \t]*#\[[^\n]*\]\n)*[ \t]*(?:pub(?:\([a-z]+\))?\s+)?(?:mod\s+" + name + r"\s*;|use\s+cmd::" + name + r"\s*;)[ \t]*\n", re.M)


def drop_decls(crate):
    """Removes the crate's module declarations from core's lib.rs, cmd.rs and cmd/table.rs."""
    changed = []
    for u in units(crate):
        parts = u.split("/")
        parent = {1: [f"{CORE}/lib.rs"], 2: [f"{CORE}/cmd.rs", f"{CORE}/lib.rs"], 3: [f"{CORE}/cmd/table.rs"]}[len(parts)]
        for p in parent:
            text = read(p)
            new = decl_re(parts[-1]).sub("", text)
            if new != text:
                write(p, new)
                changed.append(p)
    return changed


def own_cmd_names(crate):
    return {u.split("/")[1] for u in units(crate) if u.startswith("cmd/")} | {"table"}


CMD_PATH = re.compile(r"\bcrate::cmd::(\w+)")
CMD_USE = re.compile(r"\buse crate::cmd::\{")


SUPER_PATH = re.compile(r"(?<![A-Za-z0-9_:])super::(\w+)")


def point_cmd_at_core(text, crate, path=""):
    """`crate::cmd::x` for an x the crate doesn't hold becomes `crate::commands::x`, and so does
    `super::x` at the top level of a module declared in `cmd.rs` (`cmd/<module>.rs`), whose parent
    was core's `cmd`."""
    own = own_cmd_names(crate)
    masked = mask(text)
    edits = [(m.start(), m.end(), f"crate::commands::{m.group(1)}") for m in CMD_PATH.finditer(masked) if m.group(1) not in own]
    if path in {f"{crate_src(crate)}/{u}.rs" for u in units(crate) if u.startswith("cmd/") and u.count("/") == 1}:
        # Only at the file's own depth: inside an inline `mod x { .. }`, `super` is the module itself.
        edits += [
            (m.start(), m.end(), f"crate::commands::{m.group(1)}")
            for m in SUPER_PATH.finditer(masked)
            if m.group(1) not in own and m.group(1) != "super" and enclosing_mod(masked, m.start()) is None
        ]
    edits += [(m.start(), m.end(), "use crate::commands::{") for m in CMD_USE.finditer(masked)]
    for s, e, r in sorted(edits, reverse=True):
        text = text[:s] + r + text[e:]
    return text


def rust_files(roots):
    """The .rs files under `roots` that some crate target compiles: a file no module reaches never
    builds, and CI's unreached-rs guard flags any edit to it."""
    return sorted(compiled_files(roots))


def tree_files(src):
    """Every .rs file under a crate's `src` (a crate cut by this script may not have its
    Cargo.toml yet, so `compiled_files` can't reach it)."""
    return sorted(os.path.join(d, n) for d, _, fs in os.walk(src) for n in fs if n.endswith(".rs"))


def trait_root(path, crate):
    if path.startswith(crate_src(crate) + "/"):
        return "crate"
    if path.startswith(CORE + "/"):
        return None  # core can't name an upper crate's trait
    return f"photocraft_engine_{crate}" if path.startswith(f"crates/engine-{crate}/") else "photocraft_engine"


def add_trait_imports(path, crate):
    """Adds `use <root>::<module>::<Trait>;` to each module scope that calls a trait method
    without the trait in scope (its own import, or its parent's through `use super::*`)."""
    methods = TRAITS.get(crate, {})
    root = trait_root(path, crate)
    if not methods or root is None:
        return False
    text = read(path)
    masked = mask(text)
    needed = {}
    for m in re.finditer(r"\.(" + "|".join(methods) + r")\s*\(", masked):
        module, trait = methods[m.group(1)]
        scope = enclosing_mod(masked, m.start())
        if not trait_in_scope(masked, scope, trait):
            needed[(scope, trait)] = module
    if not needed:
        return False
    inserts = []
    for (scope, trait), module in needed.items():
        line = f"use {root}::{module}::{trait};\n"
        if scope is not None:
            nl = text.index("\n", scope) + 1
            indent = re.match(r"[ \t]*", text[nl:]).group(0)
            inserts.append((nl, indent + line))
        else:
            um = re.search(r"^use ", masked, re.M)
            inserts.append((um.start() if um else 0, line))
    for at, line in sorted(inserts, reverse=True):
        text = text[:at] + line + text[at:]
    write(path, text)
    return True


def scope_items(masked, scope):
    """The text of a module scope (the file for `None`, else the inline module whose body starts
    at `scope`) with its nested inline modules blanked."""
    from rust_source import block_end

    if scope is None:
        start, end = 0, len(masked)
    else:
        start = scope
        end = block_end(masked, masked.rfind("{", 0, scope)) - 1
    out = list(masked[start:end])
    for m in re.finditer(r"\bmod\s+\w+\s*\{", masked[start:end]):
        a = start + m.start()
        if enclosing_mod(masked, a) != scope:
            continue
        b = block_end(masked, a)
        for k in range(m.end() + start, min(b, end) - 1):
            out[k - start] = " "
    return "".join(out)


def trait_in_scope(masked, scope, trait):
    items = scope_items(masked, scope)
    # A whole use statement, groups included (`use a::{b, Trait};`), or the trait's own definition.
    if re.search(r"\buse\b[^;]*\b" + re.escape(trait) + r"\b", items) or re.search(r"\btrait\s+" + re.escape(trait) + r"\b", items):
        return True
    if scope is not None and re.search(r"\buse\s+super::\*\s*;", items):
        head = next(m.start() for m in re.finditer(r"\bmod\s+\w+\s*\{", masked[:scope]) if m.end() == scope)
        return trait_in_scope(masked, enclosing_mod(masked, head), trait)
    return False


def enclosing_mod(masked, pos):
    from rust_source import block_end

    best = None
    for m in re.finditer(r"\bmod\s+\w+\s*\{", masked[:pos]):
        if block_end(masked, m.start()) > pos:
            best = m.end()
    return best


def split(crate):
    files = crate_files(crate)
    if not files:
        print(f"nothing of `{crate}` left in {CORE}")
    for f in files:
        dst = crate_src(crate) + f[len(CORE) :]
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        git("mv", f, dst)
    print(f"moved {len(files)} files to {crate_src(crate)}")
    print("declarations dropped in:", ", ".join(sorted(set(drop_decls(crate)))) or "none")
    finish(crate, tree_files(crate_src(crate)))


def finish(crate, moved):
    n = 0
    for f in moved:
        text = read(f)
        new = point_cmd_at_core(text, crate, f)
        if new != text:
            write(f, new)
            n += 1
    print(f"crate::cmd paths pointed at core's commands in {n} files")
    t = sum(add_trait_imports(f, crate) for f in sorted(set(rust_files(ROOTS)) | set(moved)))
    print(f"trait imports added in {t} files")


def import_names(crate, files=None):
    """(names the crate's code reaches as `crate::x` that it doesn't define, the subset only its
    tests reach)."""
    from rust_source import in_test, test_regions

    lib = f"{crate_src(crate)}/lib.rs"
    regions = test_regions(lib) if os.path.exists(lib) else {}
    files = files if files is not None else tree_files(crate_src(crate))
    own = OWN | {u.split("/")[-1] for u in units(crate)}
    if os.path.exists(lib):
        # Modules the crate's root declares itself (beside the moved ones).
        own |= {d.group(2) for d in MOD_DECL.finditer(mask(read(lib)))}
    used, in_code = set(), set()
    for f in files:
        masked = mask(read(f))
        ranges = regions.get(f, [])
        hits = [(m.start(), m.group(1)) for m in re.finditer(r"\bcrate::([A-Za-z_][A-Za-z0-9_]*)", masked)]
        for m in re.finditer(r"\buse crate::\{", masked):
            hits += [(m.start(), n) for n in group_roots(masked, m.end() - 1)]
        for pos, n in hits:
            if n in own or n == "self":
                continue
            used.add(n)
            if not in_test(ranges, pos):
                in_code.add(n)
    return sorted(in_code), sorted(used - in_code)


def group_roots(masked, brace):
    """The first path segment of each top-level branch of the use group whose `{` is at `brace`:
    `{pixels::{self, PixelTarget}, allocation::DenseBudget}` gives `pixels`, `allocation`."""
    depth, start, roots = 0, brace + 1, []
    for i in range(brace, len(masked)):
        c = masked[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
        if (c == "," and depth == 1) or depth == 0:
            m = re.match(r"\s*([A-Za-z_][A-Za-z0-9_]*)", masked[start:i])
            if m:
                roots.append(m.group(1))
            start = i + 1
            if depth == 0:
                break
    return roots


def import_block(crate):
    code, tests = import_names(crate)
    out = "use photocraft_engine_core::{" + ", ".join(code) + "};"
    if tests:
        out += "\n#[cfg(test)]\nuse photocraft_engine_core::{" + ", ".join(tests) + "};"
    return out


def imports(crate):
    print("// The core items this crate's code names as `crate::x` (photocraft#15710).")
    print(import_block(crate))


def crate_ids(crate):
    ids = set()
    for f in tree_files(crate_src(crate)):
        ids |= set(re.findall(r'(?:\b\w+!\(\s*|\bid:\s*)"([a-z][A-Za-z0-9_]*\.[A-Za-z0-9_.]+)"', read(f)))
    return ids


def branch(crate, base, head):
    not_done = []
    changed = git("diff", "--name-status", "-M", base, head).splitlines()
    added = [l.split("\t")[-1] for l in changed if l.startswith("A")]
    # Files git left in core that the crate's modules declare.
    roots = [unit_root(u) for u in units(crate)]
    moved = []
    for f in [f for r in roots if r for f in reach(r)]:
        dst = crate_src(crate) + f[len(CORE) :]
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        git("mv", f, dst)
        moved.append(dst)
    # Declarations and table entries the branch added in core for the crate's modules.
    own = {u.split("/")[-1] for u in units(crate)}
    for core_file, crate_file in ((f"{CORE}/cmd.rs", f"{crate_src(crate)}/cmd.rs"), (f"{CORE}/lib.rs", f"{crate_src(crate)}/lib.rs")):
        text = read(core_file)
        for name in own:
            m = decl_re(name).search(text)
            if m:
                text = text[: m.start()] + text[m.end() :]
                ct = read(crate_file)
                if not decl_re(name).search(ct):
                    write(crate_file, ct.rstrip("\n") + "\n" + m.group(0))
        write(core_file, text)
    move_table_entries(crate, own, not_done)
    finish(crate, moved + [f for f in added if f.startswith(crate_src(crate) + "/") and os.path.exists(f)])
    # Import block names the moved code now needs.
    lib = f"{crate_src(crate)}/lib.rs"
    lt = read(lib)
    m = re.search(r"use photocraft_engine_core::\{[^}]*\};(?:\n#\[cfg\(test\)\]\nuse photocraft_engine_core::\{[^}]*\};)?", lt)
    if m:
        write(lib, lt[: m.start()] + import_block(crate) + lt[m.end() :])
    # Tests in core that run the crate's commands.
    ids = crate_ids(crate)
    pat = re.compile(r'"(' + "|".join(re.escape(i) for i in sorted(ids)) + r')"') if ids else None
    for l in changed:
        f = l.split("\t")[-1]
        if pat and f.startswith(CORE + "/") and f.endswith(".rs") and os.path.exists(f):
            diff = git("diff", base, head, "--", f)
            hits = sorted({m.group(1) for m in pat.finditer("\n".join(x for x in diff.splitlines() if x.startswith("+")))})
            if hits:
                not_done.append(f"{f}: the branch's lines run {', '.join(hits)}; a test doing that moves to crates/engine/tests/it")
    report(not_done)


def move_table_entries(crate, own, not_done):
    """Moves `TABLES` entries the branch added to core's install.rs for the crate's modules into the
    crate's install.rs: written there first, then removed from core, so an entry is never in neither."""
    inst, crate_inst = f"{CORE}/install.rs", f"{crate_src(crate)}/install.rs"
    text = read(inst)
    entry = re.compile(r'^[ \t]*\("([^"]+)",\s*crate::(?:cmd::)?(' + "|".join(sorted(own)) + r')::specs\),[ \t]*\n', re.M)
    found = list(entry.finditer(text))
    if not found:
        return
    ct = read(crate_inst) if os.path.exists(crate_inst) else ""
    tables = re.search(r"const TABLES: &\[Table\] = &\[\n(.*?)^\];", ct, re.S | re.M)
    if not tables:
        not_done += [f"{crate_inst}: add the table entry `{m.group(0).strip()}` to TABLES (left in core's install.rs)" for m in found]
        return
    add = "".join(f'    ("{m.group(1)}", crate::{m.group(2)}::specs),\n' for m in found if f"crate::{m.group(2)}::specs" not in tables.group(1))
    write(crate_inst, ct[: tables.end(1)] + add + ct[tables.end(1) :])
    for m in found[::-1]:
        text = text[: m.start()] + text[m.end() :]
    write(inst, text)
    for m in found:
        print(f"table entry {m.group(1)} moved to {crate_inst}'s TABLES (end of the list; check its place in the registry order)")


USE_STMT = re.compile(r"^([ \t]*)(?:pub(?:\([^)]*\))?\s+)?use\s+", re.M)


def widen(text, pos, end):
    """`text` with the item named at `pos`..`end` made `pub`: its definition line, or for a name in
    a `use` list, its own `pub use` line beside the list."""
    masked = mask(text)
    stmt = None
    for m in USE_STMT.finditer(masked, 0, pos):
        stmt = m
    if stmt is not None:
        semi = masked.find(";", stmt.start())
        if stmt.start() < pos < semi and ";" not in masked[stmt.end() : pos]:
            body = text[stmt.end() : semi]
            name = text[pos:end]
            head = text[stmt.start() : stmt.end()]
            # A plain import, or a group of this one name: the whole statement becomes `pub use`.
            if "{" not in body or re.fullmatch(r"\{\s*" + re.escape(name) + r"\s*,?\s*\}", body[body.index("{") :].strip()):
                return text[: stmt.start()] + stmt.group(1) + "pub use " + text[stmt.end() :] if not head.lstrip().startswith("pub use") else text
            if "{" in body[body.index("{") + 1 : body.rindex("}")]:
                return None
            prefix = body[: body.index("{")]
            # The statement's own attributes (a `#[cfg(..)]` above the group) go with the name.
            attrs = re.search(r"(?:^[ \t]*#\[[^\n]*\][ \t]*\n)*\Z", text[: stmt.start()], re.M).group(0)
            # Edit only at and after `pos`, so the offsets of earlier names stay valid.
            after = re.match(r"\s*,\s*", text[end:semi])
            if after:
                text = text[:pos] + text[end + after.end() :]
            else:
                before = re.search(r",\s*$", text[stmt.end() : pos])
                if not before:
                    return None
                text = text[: stmt.end() + before.start()] + text[end:]
            masked = mask(text)
            semi = masked.find(";", stmt.start())
            line_end = text.index("\n", semi) + 1
            if re.fullmatch(r"\{\s*\}", text[stmt.end() + len(prefix) : semi].strip() or "{}"):
                return text[: stmt.start()] + f"{stmt.group(1)}pub use {prefix}{name};\n" + text[line_end:]
            return text[:line_end] + attrs + f"{stmt.group(1)}pub use {prefix}{name};\n" + text[line_end:]
    ls = text.rfind("\n", 0, pos) + 1
    le = text.find("\n", pos)
    line = text[ls:le]
    new = re.sub(r"^(\s*)(?:pub\((?:crate|super|in [^)]*)\)\s+)?(?!pub\b)", r"\1pub ", line, count=1)
    return text[:ls] + new + text[le:]


def child_module(f, name):
    src = read(f)
    for d in MOD_DECL.finditer(mask(src)):
        if d.group(2) == name:
            return module_file(f, name, src[d.start(1) : d.end(1)], root=f == f"{CORE}/lib.rs")
    return None


def reexported(d):
    """(file, ("chars", start, end)) of the definition a refused `pub use` names."""
    m = re.search(r"`([A-Za-z_][A-Za-z0-9_]*)`", d["message"])
    spans = [s for c in d.get("children", []) for s in c.get("spans", [])] + [s for s in d.get("spans", []) if s.get("is_primary")]
    spans = [s for s in spans if s["file_name"].startswith(CORE + "/")]
    if not m or not spans:
        return None
    name, f = m.group(1), spans[0]["file_name"]
    text = read(f)
    line = text.split("\n")[spans[0]["line_start"] - 1]
    um = re.search(r"use\s+((?:[a-z_][a-z0-9_]*::)+)(?:\{[^}]*\b" + name + r"\b|" + name + r"\b)", line)
    if not um:
        return None
    segs = [x for x in um.group(1).split("::") if x]
    cur = f
    for seg in segs:
        if seg == "crate":
            cur = f"{CORE}/lib.rs"
        elif seg == "super":
            return None
        else:
            cur = child_module(cur, seg)
            if cur is None:
                return None
    src = read(cur)
    dm = re.search(r"^[ \t]*(?:pub\([a-z]+\)\s+)?(?:const\s+|async\s+)*(?:fn|struct|enum|const|static|type|trait|mod)\s+(" + name + r")\b", src, re.M)
    if not dm:
        um2 = re.search(r"^[ \t]*(?:pub\([a-z]+\)\s+)?use\s[^;]*\b(" + name + r")\b[^;]*;", mask(src), re.M)
        if not um2:
            return None
        return cur, ("chars", um2.start(1), um2.end(1))
    return cur, ("chars", dm.start(1), dm.end(1))


def fix(stream):
    defs_by_file = {}
    left = []
    for line in stream:
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        if msg.get("reason") != "compiler-message":
            continue
        d = msg["message"]
        code = (d.get("code") or {}).get("code")
        if code in ("E0364", "E0365"):
            # A `pub use` of an item that is only `pub(crate)` where it is defined.
            target = reexported(d)
            if target:
                defs_by_file.setdefault(target[0], set()).add(target[1])
            else:
                left.append(d.get("rendered", d["message"]).splitlines()[0])
            continue
        if code not in ("E0603", "E0624", "E0616"):
            continue
        defs = [s for c in d.get("children", []) for s in c.get("spans", []) if "defined here" in (c.get("message") or "") or "private" in (c.get("message") or "")]
        defs += [s for s in d.get("spans", []) if not s.get("is_primary")]
        defs = [s for s in defs if s["file_name"].startswith(CORE + "/")]
        if not defs:
            left.append(d.get("rendered", d["message"]).splitlines()[0])
            continue
        sp = defs[0]
        defs_by_file.setdefault(sp["file_name"], set()).add(("bytes", sp["byte_start"], sp["byte_end"]))
    n = 0
    for f, spans in defs_by_file.items():
        text = read(f)
        raw = text.encode("utf-8")
        for kind, b0, b1 in sorted(spans, key=lambda x: x[1] if x[0] == "bytes" else len(text[: x[1]].encode("utf-8")), reverse=True):
            if kind == "bytes":
                c0, c1 = len(raw[:b0].decode("utf-8", "ignore")), len(raw[:b1].decode("utf-8", "ignore"))
            else:
                c0, c1 = b0, b1
            new = widen(text, c0, c1)
            if new is not None and new != text:
                print(f"WIDENED: {f}: {text[c0:c1]}")
            if new is None:
                left.append(f"{f}: `{text[c0:c1]}` in a nested use list")
            elif new != text:
                text = new
                n += 1
        write(f, text)
    print(f"widened {n} items in {len(defs_by_file)} files")
    for l in sorted(set(left)):
        print("BY HAND:", l)
    report([])


CANNOT_DO = """
What this script cannot do:
  - items that core-crate code uses from the cut crate: they move down by hand (each cut's PR
    lists where they went), or become a hook or a trait of the crate;
  - core tests that run the cut crate's commands: they fail at run time with `UnknownCommand`
    and move to crates/engine/tests/it, against the public API;
  - the crate's own install list and `command_ids.txt` (crates/engine/src/install/) for a new
    command or table: add them by hand;
  - core items that only a test of the crate uses under `#[cfg(test)]`: core is then an ordinary
    dependency, so the item doesn't exist there.
"""


def report(not_done):
    for n in not_done:
        print("NOT DONE:", n)
    print(CANNOT_DO)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="mode", required=True)
    for name in ("split", "imports"):
        p = sub.add_parser(name)
        p.add_argument("crate")
    p = sub.add_parser("branch")
    p.add_argument("crate")
    p.add_argument("--base", required=True)
    p.add_argument("--head", required=True)
    sub.add_parser("fix")
    a = ap.parse_args()
    os.chdir(git("rev-parse", "--show-toplevel").strip())
    if a.mode == "split":
        split(a.crate)
        report([])
    elif a.mode == "imports":
        imports(a.crate)
    elif a.mode == "branch":
        branch(a.crate, a.base, a.head)
    else:
        fix(sys.stdin)


if __name__ == "__main__":
    main()
