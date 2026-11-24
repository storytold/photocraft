#!/usr/bin/env python3
"""Engine split (photocraft#15710): move the core lib tests that run a cut crate's commands to the top
crate's `crates/engine/tests/it`, where every engine crate is installed.

After a cut, core's test binary installs only core's commands, so a core test that runs one of the
cut crate's ids fails at run time with `UnknownCommand`, and one that names the crate's items no
longer compiles. Both move, against the top crate's public API.

    python3 scripts/engine_split/move_tests.py list <crate> [--previous <tsv>] > list.tsv
        The tests to move: every `#[test]` in core whose body, or a test helper it calls, holds one
        of the crate's command ids as a string or names one of the crate's modules by path. With
        `--previous` (a run's list of failing tests: name<TAB>file), its rows that still exist and
        still hold such a reference are kept too, and the rows the scan alone did not find are
        printed to stderr (an id reached through a fixture file or a built string).

    python3 scripts/engine_split/move_tests.py move <crate> <list.tsv> [--dry]
        Moves each listed test, with the test helpers it needs, to one `<crate>_in_<area>.rs` module
        per core source area (nested `mod`s when an area has several test files), declares them in
        `tests/it/main.rs`, cuts the tests and the helpers nothing else uses from core, drops test
        modules left without a test, and prunes imports. Paths become `photocraft_engine::...` and
        `crate::session()` becomes `Session::new()`.

What it cannot do: a moved test that uses a private core item (not a test helper) stays a compile
error in `tests/it`; it needs the item's public equivalent, or stays in core with a setup that
doesn't run the crate's commands.
"""
import os, re, sys, collections

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rust_source import mask, block_end, test_regions, CFG_TEST, MOD_DECL, module_file, git_show

ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
SRC = ROOT + "/crates/engine-core/src"
IT = ROOT + "/crates/engine/tests/it"
CRATE = ["paint"]
DRY = "--dry" in sys.argv  # move mode

KW = re.compile(r"(?:pub(?:\([^)]*\))?\s+)?(?:(?:async|unsafe|const|extern|default)\s+)*(fn|struct|enum|union|const|static|mod|use|impl|type|trait|macro_rules!)(?![A-Za-z0-9_])")
IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
TEST_ATTR = re.compile(r"#\[test\]")


class Item:
    is_test = False
    cfg_test = False
    body = None


def skip_ws(m, i, b):
    while i < b and m[i].isspace():
        i += 1
    return i


def attr_end(m, i):
    """Index after the `#[...]` or `#![...]` at i."""
    j = i + 1
    if m[j] == "!":
        j += 1
    depth = 0
    while j < len(m):
        if m[j] == "[":
            depth += 1
        elif m[j] == "]":
            depth -= 1
            if depth == 0:
                return j + 1
        j += 1
    return j


def parse(src, m, a, b):
    items, pos = [], a
    while True:
        i = skip_ws(m, pos, b)
        if i >= b:
            break
        it = Item()
        it.start = pos
        h = i
        if m.startswith("#!", h):
            it.kind, it.name, it.end = "inner", None, attr_end(m, h)
            it.hdr = h
            it.attrs = ""
            it.body = None
            items.append(it)
            pos = it.end
            continue
        while m.startswith("#[", h):
            h = skip_ws(m, attr_end(m, h), b)
        it.hdr = h
        it.attrs = src[i:h]
        km = KW.match(m, h)
        if km:
            kind = km.group(1)
            nm = IDENT.match(m, skip_ws(m, km.end(), b)) if kind not in ("impl", "use") else None
            it.kind, it.name = kind, (nm.group(0) if nm else None)
            if kind == "macro_rules!":
                it.kind = "macro"
        else:
            it.kind, it.name = "other", None
        e = block_end(m, h)
        if it.kind in ("use", "const", "static", "type", "other", "macro"):
            k = skip_ws(m, e, b)
            if k < b and m[k] == ";" and (m[e - 1] != ";"):
                e = k + 1
        it.end = min(e, b)
        it.body = None
        if it.kind == "mod":
            ob = m.find("{", h, it.end)
            if ob >= 0 and m[it.end - 1] == "}":
                it.body = (ob + 1, it.end - 1)
        it.is_test = bool(TEST_ATTR.search(it.attrs)) and it.kind == "fn"
        it.cfg_test = bool(CFG_TEST.search(it.attrs))
        items.append(it)
        pos = it.end
    return items


class Node:
    def __init__(self, f, src, m, a, b, name, parent, modpath):
        self.f, self.src, self.m, self.name, self.parent, self.modpath = f, src, m, name, parent, modpath
        self.a, self.b = a, b
        self.items = parse(src, m, a, b)
        self.by_name = collections.defaultdict(list)
        for it in self.items:
            if it.name and not it.is_test and it.kind in ("fn", "const", "static", "struct", "enum", "union", "type", "trait", "macro"):
                self.by_name[it.name].append(it)
        self.sel = set()  # ids of selected items
        self.children = {}
        self.moved = set()


def words(text):
    return set(IDENT.findall(text))


def item_text(node, it):
    return node.src[it.start:it.end]


def item_masked(node, it):
    return node.m[it.hdr:it.end]


def collect_tests(node, out):
    for it in node.items:
        if it.is_test:
            out[tuple(node.modpath + [it.name])] = (node, it)
        elif it.kind == "mod" and it.body:
            child = Node(node.f, node.src, node.m, it.body[0], it.body[1], it.name, node, node.modpath + [it.name])
            child.item = it
            node.children[id(it)] = child
            collect_tests(child, out)


def fs_modpath(f):
    rel = os.path.relpath(f, SRC)[:-3].split(os.sep)
    if rel[-1] == "mod":
        rel = rel[:-1]
    return rel


def header_end(src):
    """Offset after the file's leading `//!` lines."""
    pos = 0
    for line in src.splitlines(keepends=True):
        if line.startswith("//!") or not line.strip():
            pos += len(line)
        else:
            break
    return pos


def build_units():
    regions = test_regions(SRC + "/lib.rs")
    units, tests = [], {}
    for f, rg in sorted(regions.items()):
        src = open(f).read()
        m = mask(src)
        if rg is None:
            node = Node(f, src, m, header_end(src), len(src), None, None, fs_modpath(f))
            node.whole = True
            units.append(node)
            collect_tests(node, tests)
        else:
            top = parse(src, m, 0, len(src))
            for it in top:
                if it.kind == "mod" and it.body and it.cfg_test:
                    node = Node(f, src, m, it.body[0], it.body[1], it.name, None, fs_modpath(f) + [it.name])
                    node.whole = False
                    node.item = it
                    units.append(node)
                    collect_tests(node, tests)
    return units, tests


def unit_of(node):
    while node.parent:
        node = node.parent
    return node


def select(node, it):
    if id(it) in node.sel:
        return
    node.sel.add(id(it))
    if it.kind in ("struct", "enum", "union", "trait"):
        for o in node.items:
            if o.kind == "impl":
                hdr = node.m[o.hdr:node.m.find("{", o.hdr)]
                if it.name in words(hdr):
                    select(node, o)
    need(node, words(item_masked(node, it)) - {it.name} if it.kind != "impl" else words(item_masked(node, it)))


def need(node, names):
    for name in names:
        n = node
        while n:
            lst = n.by_name.get(name)
            if lst:
                for it in lst:
                    select(n, it)
                break
            n = n.parent


def mark_chain(node):
    """Nodes from the unit root to node get emitted."""
    while node.parent:
        node.parent.children_emit = getattr(node.parent, "children_emit", set())
        node.parent.children_emit.add(id(node))
        node = node.parent


def emission(node, ctx, indent="", helpers_only=False):
    """Text of the node's selected items in order, nested nodes as mod blocks. ctx: rewrite settings.
    helpers_only: the node is a test-module parent; its tests are not emitted."""
    parts, used = [], set()
    uses = []
    for it in node.items:
        if helpers_only and it.is_test:
            continue
        t = item_text(node, it)
        if it.kind == "inner" and node.parent is None:
            parts.append(t)
        elif it.kind == "use":
            uses.append(it)
        elif id(it) in node.sel or (it.is_test and it.name in node.moved):
            parts.append(rewrite(t, ctx, node))
            used |= words(rewrite(item_masked(node, it), ctx, node))
        elif it.kind == "mod" and id(it) in node.children and id(node.children[id(it)]) in getattr(node, "children_emit", ()):
            child = node.children[id(it)]
            ctext, cused = emission(child, ctx)
            attrs = CFG_TEST.sub("", it.attrs).strip()
            parts.append("\n" + attrs + "\nmod " + it.name + " {\n" + ctext + "\n}\n")
            used |= cused
    use_text = []
    STRIP[0] = True
    for it in uses:
        orig = item_text(node, it)
        t = rewrite(orig, ctx, node)
        if node.parent is None and re.match(r"\s*(?:#\[[^\]]*\]\s*)*use\s+super::", orig):
            t = resolve_super_use(t, node, ctx, used | ctx.get("extra_used", set()))
        t = prune_use(t, used)
        if t:
            use_text.append(t)
    STRIP[0] = False
    return "\n".join(use_text) + "\n" + "\n".join(parts), used


def rewrite(text, ctx, node):
    t = text.replace("crate::session()", "Session::new()")
    t = re.sub(r"(?<![A-Za-z0-9_])crate::", "photocraft_engine::", t)
    if not re.match(r"\s*(?:#\[[^\]]*\]\s*)*(?:pub\s+)?use\b", t.lstrip()):
        t = re.sub(r"(?<![A-Za-z0-9_:])(?:photocraft_engine::file_cmds::)?flattened\(((?:[^()]|\([^()]*\))*)\)", r"photocraft_compose::flatten_to_surface(\1, None)", t)
    if node.parent is None:
        t = re.sub(r"(?<![A-Za-z0-9_:])super::", ctx["parent"], t)
    return t


# ---- use-tree pruning -------------------------------------------------------------------------

def parse_use_tree(s, i=0):
    """Returns (tree, next index). tree = (path_str, alias or None, children or None, glob bool)."""
    start = i
    while i < len(s) and s[i] not in ",{};}":
        i += 1
    seg = s[start:i].strip()
    if i < len(s) and s[i] == "{":
        # path::{...}
        j, kids = i + 1, []
        while True:
            while j < len(s) and s[j].isspace():
                j += 1
            if s[j] == "}":
                j += 1
                break
            k, j = parse_use_tree(s, j)
            kids.append(k)
            while j < len(s) and s[j].isspace():
                j += 1
            if s[j] == ",":
                j += 1
        return (seg, None, kids), j
    alias = None
    if " as " in seg:
        seg, alias = [x.strip() for x in seg.split(" as ")]
    return (seg, alias, None), i


def print_use_tree(t):
    seg, alias, kids = t
    if kids is not None:
        return seg + "{" + ", ".join(print_use_tree(k) for k in kids) + "}"
    return seg + (" as " + alias if alias else "")


STRIP = [False]
EXCL = [set()]


def _traits():
    out = set()
    for dp, _, fs in os.walk(ROOT + "/crates"):
        for f in fs:
            if f.endswith(".rs"):
                try:
                    out |= set(re.findall(r"\btrait\s+([A-Za-z0-9_]+)", open(os.path.join(dp, f)).read()))
                except Exception:
                    pass
    return out


TRAITS = _traits()


def prune_tree(t, used, parent=""):
    seg, alias, kids = t
    if kids is None:
        if seg.endswith("*"):
            return t
        name = alias or seg.split("::")[-1]
        if STRIP[0] and name in ("Session", "json") and "::" in seg and seg.count("::") >= 1 and not seg.startswith("serde_json") and seg != "photocraft_engine::Session":
            return None
        if name == "self":
            name = alias or parent.rstrip(":").split("::")[-1]
        if name in EXCL[0]:
            return None
        if name == "_" or name in used or name in TRAITS:
            return t
        return None
    nk = [x for x in (prune_tree(k, used, seg) for k in kids) if x]
    if not nk:
        return None
    return (seg, None, nk)


def prune_use(text, used):
    m = re.search(r"((?:#\[[^\]]*\]\s*)*)((?:pub(?:\([^)]*\))?\s+)?use\s+)([^;]*);", text, re.S)
    if not m:
        return text
    body = m.group(3).strip()
    if "cfg(" in m.group(1):
        return text
    tree, _ = parse_use_tree(body + ";")
    p = prune_tree(tree, used)
    if not p:
        return ""
    # keep leading comments of the original text
    lead = text[: m.start()]
    return lead + m.group(1) + m.group(2) + print_use_tree(p) + ";"


# ---- main -------------------------------------------------------------------------------------

UNITS = []
_PARENT_INFO = {}


def parent_unit(u):
    """The test unit (a whole test file) that declares u, when u's parent module is a test module."""
    pf = parent_file(u)
    return next((x for x in UNITS if x.whole and x.f == pf and x is not u), None)


def parent_info(pf):
    """(visible, private_imports): names the parent file defines or re-exports, and names it imports privately."""
    if pf in _PARENT_INFO:
        return _PARENT_INFO[pf]
    gs = open(pf).read()
    items = parse(gs, mask(gs), 0, len(gs))
    visible, private = set(), set()
    for it in items:
        if it.cfg_test and it.kind != "use":
            continue
        if it.kind == "use":
            t = gs[it.hdr:it.end]
            tree, _ = parse_use_tree(re.match(r"(?:pub(?:\([^)]*\))?\s+)?use\s+([^;]*);", t, re.S).group(1).strip() + ";")
            names = []

            def leaves(tr):
                seg, alias, kids = tr
                if kids is None:
                    names.append(alias or seg.split("::")[-1])
                else:
                    for k in kids:
                        leaves(k)
            leaves(tree)
            (visible if gs[it.hdr:it.hdr + 3] == "pub" else private).update(names)
        elif it.name:
            visible.add(it.name)
    _PARENT_INFO[pf] = (visible, private)
    return _PARENT_INFO[pf]


_PUB_SRC = []


def module_public(parent):
    """Whether the top crate names the module `parent` (photocraft_engine::a::b::) publicly."""
    if not _PUB_SRC:
        for d in (SRC, ROOT + "/crates/engine/src"):
            for dp, _, fs in os.walk(d):
                for f in fs:
                    if f.endswith(".rs"):
                        _PUB_SRC.append(open(os.path.join(dp, f)).read())
    name = parent.rstrip(":").split("::")[-1]
    pat = re.compile(r"\bpub\s+mod\s+" + name + r"\b|\bpub\s+use\s+[^;]*\b" + name + r"\b(?!\s*::)")
    return any(pat.search(t) for t in _PUB_SRC)


def resolve_super_use(t, node, ctx, used):
    """`use super::..` re-rooted to the parent module's path: keep the names the parent defines or re-exports
    (a glob becomes the names the moved text uses); a name the parent only imports privately is dropped (the
    parent's own imports come in by parent_uses). When the parent is a test module, its helpers are copied
    instead, so the line goes."""
    if parent_unit(node):
        return ""
    visible, private = parent_info(parent_file(node))
    m = re.search(r"((?:pub(?:\([^)]*\))?\s+)?use\s+)([^;]*);", t, re.S)
    tree, _ = parse_use_tree(m.group(2).strip() + ";")
    parent = ctx["parent"]
    leaves = []

    def walk(tr, base):
        seg, alias, kids = tr
        if kids is None:
            leaves.append((base + seg, alias))
        else:
            for k in kids:
                walk(k, base + seg)
    walk(tree, "")
    names = []
    for path, alias in leaves:
        if not path.startswith(parent):
            return t
        rest = path[len(parent):]
        if rest == "*":
            if module_public(parent):
                return t  # a glob of a public module keeps what the top crate can reach
            continue
        elif "::" in rest or rest not in private:
            names.append(rest + (" as " + alias if alias else ""))
    names = sorted(set(names))
    if not names:
        return ""
    return t[: m.start()] + m.group(1) + parent + "{" + ", ".join(names) + "};"


def area_of(modpath):
    p = modpath[1:] if modpath[0] == "cmd" and len(modpath) > 1 else modpath
    a = p[0]
    if a == "tests":
        return "lib"
    return re.sub(r"_tests$", "", a)


def parent_path(unit):
    p = unit.modpath[:-1]
    if p and p[0] == "cmd" and len(p) > 1:
        p = p[1:]
    if not p:
        return "photocraft_engine::"
    return "photocraft_engine::" + "::".join(p) + "::"


def move(tsv):
    names = [l.split("\t")[0] for l in open(tsv).read().splitlines() if l.strip() and not l.startswith("#")]
    units, tests = build_units()
    UNITS[:] = units
    found, missing = {}, []
    for n in names:
        key = tuple(n.split("::"))
        if key in tests:
            found[n] = tests[key]
        else:
            # core module path may have an extra first segment; try suffix match
            cands = [k for k in tests if k[-len(key):] == key]
            if not cands:
                cands = [k for k in tests if k[-1] == key[-1] and key[-2] in k]
            if not cands:
                cands = [k for k in tests if k[-1] == key[-1]]
            if len(cands) == 1:
                found[n] = tests[cands[0]]
            else:
                missing.append((n, len(cands)))
    print(f"{len(found)} found, {len(missing)} missing")
    for n, c in missing:
        print("  MISSING", n, c)
    if missing:
        sys.exit(1)

    # per node: moved test names; select helpers
    by_unit = collections.OrderedDict()
    for n, (node, it) in found.items():
        node.moved.add(it.name)
        node.sel.add(id(it))
        u = unit_of(node)
        by_unit.setdefault(id(u), (u, []))[1].append((node, it))
    for _, (u, lst) in by_unit.items():
        for node, it in lst:
            mark_chain(node)
            need(node, words(item_masked(node, it)))

    # emit: group by area
    areas = collections.OrderedDict()
    for _, (u, lst) in by_unit.items():
        areas.setdefault(area_of(u.modpath), []).append(u)
    outputs = {}
    for area, us in areas.items():
        ctexts = []
        for u in us:
            ctx = {"parent": parent_path(u)}
            text, used = emission(u, ctx)
            pu = parent_unit(u)
            if pu:
                text = parent_helpers(pu, text, used) + text
            ctexts.append((u, text))
        ctexts = [(u, inject(parent_uses(u, t) + t)) for u, t in ctexts]
        if len(ctexts) == 1:
            body = ctexts[0][1]
        else:
            body = ""
            for u, t in ctexts:
                body += f"\nmod {u.modpath[-1]} {{\n{t}\n}}\n"
        n = sum(len(lst) for u, lst in [by_unit[id(x)] for x in us])
        head = f"//! Tests that run {CRATE[0]} commands, moved from the engine-core `{area}` tests: {n} test(s).\n\n"
        outputs[area] = head + body

    # removal from core
    edits = collections.defaultdict(list)  # file -> [(a, b)]
    removed_files = []
    for _, (u, lst) in by_unit.items():
        plan_removal(u, edits)
    stats = {a: sum(len(by_unit[id(x)][1]) for x in us) for a, us in areas.items()}
    print("areas:", len(areas), "tests:", sum(stats.values()))
    for a in sorted(stats):
        print(f"  {a}: {stats[a]}")
    if DRY:
        return
    for area, text in outputs.items():
        open(f"{IT}/{CRATE[0]}_in_{area}.rs", "w").write(text)
    # main.rs: sorted block
    mp = IT + "/main.rs"
    lines = open(mp).read().split("\n")
    idx = [i for i, l in enumerate(lines) if re.fullmatch(r"mod [a-z0-9_]+;", l)]
    first = idx[0]
    last = first
    while last + 1 < len(lines) and re.fullmatch(r"mod [a-z0-9_]+;", lines[last + 1]):
        last += 1
    block = lines[first : last + 1] + [f"mod {CRATE[0]}_in_{a};" for a in outputs]
    block = sorted(set(block))
    lines[first : last + 1] = block
    open(mp, "w").write("\n".join(lines))
    apply_edits(edits, by_unit)
    renamed = {}
    for area, us in areas.items():
        for u in us:
            for node, it in by_unit[id(u)][1]:
                path = [f"{CRATE[0]}_in_{area}"] + ([u.modpath[-1]] if len(us) > 1 else []) + node.modpath[len(u.modpath) :] + [it.name]
                renamed["::".join(node.modpath + [it.name])] = "::".join(path)
    remap_times(CRATE[0], renamed)


def parent_helpers(pu, text, used):
    """The test-module parent's helpers (and imports) that the moved unit's text uses, not defined in it."""
    defined = set(re.findall(r"\b(?:fn|struct|enum|const|static|type|trait)\s+([A-Za-z_]\w*)", text))
    need(pu, (used | words(text)) - defined)
    ctx = {"parent": parent_path(pu), "extra_used": used | words(text)}
    htext, hused = emission(pu, ctx, helpers_only=True)
    gtext = parent_uses(pu, htext + text)
    return gtext + htext + "\n"


ALL_FILES = None


def parent_file(u):
    global ALL_FILES
    if not u.whole:
        return u.f
    if ALL_FILES is None:
        ALL_FILES = []
        for dp, _, fs in os.walk(SRC):
            ALL_FILES += [os.path.join(dp, f) for f in fs if f.endswith(".rs")]
    for g in ALL_FILES:
        gs = open(g).read()
        gm = mask(gs)
        for d in MOD_DECL.finditer(gm):
            if module_file(g, d.group(2), gs[d.start(1):d.end(1)], root=os.path.dirname(g) == SRC and os.path.basename(g) == "lib.rs") == u.f:
                return g
    return None


def parent_uses(u, body):
    """The parent file's own `use` items that the unit's text needs (what `use super::*` gave it), rewritten."""
    g = parent_file(u)
    if not g:
        return ""
    gs = open(g).read()
    gm = mask(gs)
    items = parse(gs, gm, 0, len(gs))
    local = {it.name for it in items if it.kind == "mod" and it.name}
    have = set()
    for mm in re.finditer(r"\buse\s+([^;]*);", body):
        for w in re.findall(r"[A-Za-z_]\w*(?!\w|\s*::)", re.sub(r"[A-Za-z_]\w*\s+as\s+", "", mm.group(1))):
            have.add(w)
    for mm in re.finditer(r"\b(?:fn|struct|enum|const|static|type|trait)\s+([A-Za-z_]\w*)", body):
        have.add(mm.group(1))
    used = words(re.sub(r"(?<![.])\.\s*[A-Za-z_]\w*", ".", body)) - have  # method names are not imports
    EXCL[0] = have
    out = []
    for it in items:
        if it.kind != "use" or ("cfg(" in it.attrs and not it.cfg_test) or "pub" in gs[it.hdr:it.hdr + 4]:
            continue
        t = gs[it.hdr:it.end]
        mm = re.match(r"use\s+([^;]*);", t, re.S)
        first = re.match(r"[A-Za-z_]\w*", mm.group(1).strip())
        if not first or first.group(0) in local or first.group(0) in ("self", "super"):
            continue
        t = re.sub(r"(?<![A-Za-z0-9_])crate::", "photocraft_engine::", t)
        STRIP[0] = True
        t = prune_use(t, used)
        STRIP[0] = False
        if t:
            out.append(t.strip())
    EXCL[0] = set()
    return "\n".join(out) + "\n"


def inject(body):
    if re.search(r"(?<![A-Za-z0-9_:])Result<[^,<>]*>", body) and not re.search(r"\buse [^;]*\bResult\b", body):
        body = "use photocraft_engine::Result;\n" + body
    if re.search(r"\bjson!", body) and not re.search(r"\buse serde_json::[^;]*\bjson\b", body):
        body = "use serde_json::json;\n" + body
    if re.search(r"\bSession\b", body) and not re.search(r"\buse photocraft_engine::(?:\{[^}]*)?\bSession\b", body):
        body = "use photocraft_engine::Session;\n" + body
    return body


def refs_remaining(unit_file_text_without, name):
    return re.search(r"(?<![A-Za-z0-9_])" + re.escape(name) + r"(?![A-Za-z0-9_])", unit_file_text_without) is not None


def plan_removal(u, edits):
    """Fill edits[file] with spans to cut: moved tests, then helpers nothing else references; empty mods."""
    src, m = u.src, u.m
    cut = []  # (a,b,node,item)
    def walk(node):
        for it in node.items:
            if it.is_test and it.name in node.moved and id(it) in node.sel:
                cut.append((it.start, it.end, node, it))
        for ch in node.children.values():
            walk(ch)
    walk(u)
    # helpers: candidates selected non-test items
    cands = []
    def walk2(node):
        for it in node.items:
            if id(it) in node.sel and not it.is_test and it.kind != "use":
                cands.append((it.start, it.end, node, it))
        for ch in node.children.values():
            walk2(ch)
    walk2(u)
    spans = [(a, b) for a, b, _, _ in cut]
    def remaining_text(skip):
        out, last = [], 0
        for a, b in sorted(spans + skip):
            if a >= last:
                out.append(m[last:a])
                last = max(last, b)
            else:
                last = max(last, b)
        out.append(m[last:])
        return "".join(out)
    removed_helpers = []
    changed = True
    while changed:
        changed = False
        for a, b, node, it in cands:
            if (a, b) in spans or not it.name:
                continue
            others = remaining_text([(a, b)])
            # an impl block goes when its type is gone: handled below
            if not refs_remaining(others, it.name):
                spans.append((a, b))
                removed_helpers.append(it.name)
                changed = True
    # impls of removed structs
    for a, b, node, it in cands:
        if it.kind == "impl" and (a, b) not in spans:
            hdr = node.m[it.hdr:node.m.find("{", it.hdr)]
            names = [x for x in words(hdr)]
            for oa, ob, on, oi in cands:
                if oi.name and oi.kind in ("struct", "enum", "union", "trait") and (oa, ob) in spans and oi.name in names:
                    spans.append((a, b))
                    break
    # nested mods left without tests
    def walk3(node):
        for ch in node.children.values():
            walk3(ch)
            it = ch.item
            left = [t for t in ch.items if t.is_test and (t.start, t.end) not in spans] + [
                c for c in ch.children.values() if (c.item.start, c.item.end) not in spans]
            if not left and (it.start, it.end) not in spans:
                spans.append((it.start, it.end))
    walk3(u)
    u.spans = sorted(set(spans))
    edits[u.f].append(u)


def apply_edits(edits, by_unit):
    for f, us in edits.items():
        src = open(f).read()
        spans = []
        for u in us:
            spans += u.spans
        # merge overlapping: drop spans contained in bigger ones
        spans = sorted(set(spans))
        merged = []
        for a, b in spans:
            if merged and a < merged[-1][1]:
                merged[-1] = (merged[-1][0], max(b, merged[-1][1]))
            else:
                merged.append((a, b))
        out, last = [], 0
        for a, b in merged:
            out.append(src[last:a])
            last = b
        out.append(src[last:])
        new = "".join(out)
        # prune imports of the touched test modules
        for u in us:
            pass
        open(f, "w").write(new)
        prune_file(f, us)
        # whole file left without tests -> delete
    return


def prune_file(f, us):
    src = open(f).read()
    m = mask(src)
    # test modules in this file again
    nodes = []
    for u in us:
        if getattr(u, "whole", False):
            nodes.append((header_end(src), len(src)))
        else:
            for it in parse(src, m, 0, len(src)):
                if it.kind == "mod" and it.body and it.cfg_test and it.name == u.name:
                    nodes.append(it.body)
    if not nodes:
        return
    # prune use items inside each region (top-level and nested) by the region's remaining words
    edits = []
    def region(a, b):
        items = parse(src, m, a, b)
        rest = " ".join(m[it.hdr:it.end] for it in items if it.kind != "use")
        uw = words(rest)
        for it in items:
            if it.kind == "use":
                t = src[it.start:it.end]
                nt = prune_use(t, uw)
                if nt != t:
                    edits.append((it.start, it.end, nt))
            elif it.kind == "mod" and it.body:
                region(*it.body)
    for a, b in nodes:
        region(a, b)
    for a, b, t in sorted(edits, reverse=True):
        src = src[:a] + t + src[b:]
    open(f, "w").write(src)


def cleanup():
    """Drops test units left with no `#[test]`: whole-file test modules (the file and its `mod`
    line) and inline `#[cfg(test)] mod x { .. }` blocks."""
    regions = test_regions(SRC + "/lib.rs")
    for f, rg in sorted(regions.items()):
        if not f.startswith(SRC) or not os.path.exists(f):
            continue
        src = open(f).read()
        if rg is None:
            if "#[test]" in src:
                continue
            for g in list(regions):
                if g == f or not os.path.exists(g):
                    continue
                gs = open(g).read()
                gm = mask(gs)
                root = os.path.dirname(g) == SRC and os.path.basename(g) == "lib.rs"
                for d in MOD_DECL.finditer(gm):
                    if module_file(g, d.group(2), gs[d.start(1) : d.end(1)], root=root) == f:
                        gs = gs[: d.start()] + gs[d.end() :]
                        open(g, "w").write(gs)
                        break
            os.remove(f)
            print("removed", os.path.relpath(f, ROOT))
        else:
            m = mask(src)
            # Only `tests` modules: another test-only module can hold helpers its siblings use.
            cuts = [(it.start, it.end) for it in parse(src, m, 0, len(src)) if it.kind == "mod" and it.body and it.cfg_test and re.search(r"tests?$", it.name) and "#[test]" not in src[it.body[0] : it.body[1]]]
            for a, b in reversed(cuts):
                src = src[:a] + src[b:]
            if cuts:
                open(f, "w").write(src)
                print("emptied test modules dropped in", os.path.relpath(f, ROOT))


def crate_ids(crate):
    """The command ids the crate's tables define."""
    ids = set()
    for dp, _, fs in os.walk(f"{ROOT}/crates/engine-{crate}/src"):
        for n in fs:
            if n.endswith(".rs"):
                ids |= set(re.findall(r'(?:\b\w+!\(\s*|\bid:\s*)"([a-z][A-Za-z0-9_]*\.[A-Za-z0-9_.]+)"', open(os.path.join(dp, n)).read()))
    return ids


def crate_modules(crate):
    out = set()
    for line in open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "crates.tsv")):
        parts = line.rstrip("\n").split("\t")
        if len(parts) == 2 and parts[1] == crate and not parts[0].endswith("_tests"):
            out.add(parts[0].split("/")[-1])
    return out - {"paint", "table"}


def closure(node, it):
    """(node, item) pairs: the test and every helper it reaches by name, transitively."""
    seen, out, todo = set(), [], [(node, it)]
    while todo:
        n, x = todo.pop()
        if (id(n), id(x)) in seen:
            continue
        seen.add((id(n), id(x)))
        out.append((n, x))
        for name in words(n.m[x.hdr : x.end]):
            k = n
            while k:
                if k.by_name.get(name):
                    todo += [(k, o) for o in k.by_name[name]]
                    break
                k = k.parent
    return out


def list_tests(crate, previous, old_ref):
    ids = crate_ids(crate)
    id_re = re.compile(r'"(' + "|".join(re.escape(i) for i in sorted(ids)) + r')"')
    mod_re = re.compile(r"(?<![A-Za-z0-9_])(?:crate::|super::)*(?:cmd::)?(" + "|".join(sorted(crate_modules(crate))) + r")::")
    prev = set()
    if previous:
        prev = {l.split("\t")[0].split("::")[-1] + "\t" + l.split("\t")[1] for l in open(previous).read().splitlines() if l.strip() and not l.startswith("#")}
    old = {}
    _, tests = build_units()
    rows, kept, scan_only, prev_only = [], 0, 0, []
    for key, (node, it) in sorted(tests.items()):
        rel = os.path.relpath(node.f, ROOT)
        items = closure(node, it)
        texts = [n.src[x.start : x.end] for n, x in items]
        hit_mod = any(mod_re.search(n.m[x.hdr : x.end]) for n, x in items)
        hit_id = any(id_re.search(tx) for tx in texts)
        if old_ref and rel not in old:
            old[rel] = git_show(old_ref, rel) or ""
        unchanged = bool(old_ref) and all(tx in old[rel] for tx in texts)
        in_prev = (it.name + "\t" + rel) in prev
        if hit_mod:
            why = "names a moved module"
        elif unchanged and previous:
            why = "previous run" if in_prev else None
            if in_prev and not hit_id:
                prev_only.append("::".join(key))
        else:
            why = "runs " + ", ".join(sorted(set(m.group(1) for tx in texts for m in id_re.finditer(tx)))) if hit_id else None
        if why:
            rows.append(f"{'::'.join(key)}\t{rel}\t{why}")
    print(f"# Core tests that run {crate} commands or name its modules (scripts/engine_split/move_tests.py list {crate}): test<TAB>file<TAB>why")
    print("\n".join(rows))
    print(f"{len(rows)} tests listed", file=sys.stderr)
    for n in prev_only:
        print("PREVIOUS RUN ONLY (no id in the source):", n, file=sys.stderr)


def remap_times(crate, moved):
    """`.config/ci-test-times.tsv`: rows of tests now in the crate's lib name its binary, rows of
    moved tests name `photocraft-engine::it` and their new path, and the `bin` rows move the tests'
    count and seconds with them (timed rows exactly, the rest at core's average untimed time)."""
    path = f"{ROOT}/.config/ci-test-times.tsv"
    src = f"{ROOT}/crates/engine-{crate}/src"
    names = set()
    for dp, _, fs in os.walk(src):
        for n in fs:
            if n.endswith(".rs"):
                names |= set(re.findall(r"#\[test\]\s*(?:#\[[^\]]*\]\s*)*fn\s+([a-z0-9_]+)", open(os.path.join(dp, n)).read()))
    counts = {f"photocraft-engine-{crate}": len(names), "photocraft-engine::it": len(moved)}
    core, top = "photocraft-engine-core", f"photocraft-engine-{crate}"
    lines = open(path).read().split("\n")
    timed = collections.defaultdict(lambda: collections.defaultdict(lambda: [0, 0.0]))  # os -> binary -> [n, s]
    out = []
    for line in lines:
        parts = line.split("\t")
        if len(parts) == 5 and parts[0] == "test" and parts[2] == core:
            if parts[4] in moved:
                parts[2], parts[4] = "photocraft-engine::it", moved[parts[4]]
            elif parts[4].split("::")[-1] in names:
                parts[2] = top
            if parts[2] != core:
                timed[parts[1]][parts[2]][0] += 1
                timed[parts[1]][parts[2]][1] += float(parts[3])
            line = "\t".join(parts)
        out.append(line)
    core_timed = collections.defaultdict(lambda: [0, 0.0])
    for line in lines:
        parts = line.split("\t")
        if len(parts) == 5 and parts[0] == "test" and parts[2] == core:
            core_timed[parts[1]][0] += 1
            core_timed[parts[1]][1] += float(parts[3])
    bins = {(l.split("\t")[1], l.split("\t")[2]): l.split("\t") for l in out if l.startswith("bin\t")}
    for (os_, b), parts in list(bins.items()):
        if b != core:
            continue
        n, s = int(parts[3]), float(parts[4])
        avg = (s - core_timed[os_][1]) / max(1, n - core_timed[os_][0])
        for dest in (top, "photocraft-engine::it"):
            tn, ts = timed[os_][dest]
            share = counts[dest]
            secs = ts + max(0, share - tn) * avg
            n, s = n - share, s - secs
            row = bins.get((os_, dest))
            if row:
                row[3], row[4] = str(int(row[3]) + share), f"{float(row[4]) + secs:.2f}"
            else:
                bins[(os_, dest)] = ["bin", os_, dest, str(share), f"{secs:.2f}"]
        parts[3], parts[4] = str(n), f"{s:.2f}"
    res, done = [], set()
    for line in out:
        if line.startswith("bin\t"):
            key = tuple(line.split("\t")[1:3])
            res.append("\t".join(bins[key]))
            done.add(key)
            if key[1] == core and (key[0], top) not in done:
                res.append("\t".join(bins[(key[0], top)]))
                done.add((key[0], top))
            continue
        res.append(line)
    open(path, "w").write("\n".join(res))


def main():
    import argparse
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="mode", required=True)
    p = sub.add_parser("list")
    p.add_argument("crate")
    p.add_argument("--previous")
    p.add_argument("--old-ref", help="the commit the previous run's list was made on; tests and helpers unchanged since keep its verdict")
    p = sub.add_parser("move")
    p.add_argument("crate")
    p.add_argument("tsv")
    p.add_argument("--dry", action="store_true")
    a = ap.parse_args()
    CRATE[0] = a.crate
    if a.mode == "list":
        list_tests(a.crate, a.previous, a.old_ref)
    else:
        move(a.tsv)
        if not a.dry:
            cleanup()


if __name__ == "__main__":
    main()
