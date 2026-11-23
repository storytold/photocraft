"""Shared helpers for the engine split scripts: Rust source scanning and git.

`mask` blanks comments, strings and char literals so a regex over the result sees only code, at
the same offsets. `test_regions` finds a crate's test code: whole files reached through a
`#[cfg(test)]` (or `#[cfg(all(test, ...))]`) `mod x;`, and the `#[cfg(test)]` items and bare `#[test]`
functions of other files (the test harness compiles `#[test]` functions only in test builds).
`#[cfg(any(test, ...))]` items are not test code: other builds compile them too.
`compiled_files` lists the files some crate target's module tree reaches; the rewrites edit only
those, since a file no module reaches never compiles and CI's unreached-rs guard flags any edit to it.
`edited_definitions` finds a branch's edits to definitions a step moved or changed, for the steps'
`branch` modes.
"""
import glob, os, re, subprocess

CFG_TEST = re.compile(r"#\[cfg\((?:test|all\(test\b[^\]]*\))\)\]")
MOD_DECL = re.compile(r"((?:#\[[^\]]*\]\s*)*)(?:pub(?:\([a-z]+\))?\s+)?mod\s+([a-z0-9_]+)\s*;")
PATH_ATTR = re.compile(r'#\[path\s*=\s*"([^"]+)"\]')
TEST_ATTR = re.compile(r"#\[test\]")
TEST_FN = re.compile(r"#\[test\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([a-z]+\))?\s+)?(?:async\s+)?fn\s+([a-z0-9_]+)")
INLINE_MOD = re.compile(r"((?:#\[[^\]]*\]\s*)*)(?:pub(?:\([a-z]+\))?\s+)?\bmod\s+([a-z0-9_]+)\s*\{")
TARGET_PATH = re.compile(r'^\s*path\s*=\s*"([^"]+\.rs)"', re.M)
IDENT_BEFORE = re.compile(r"[A-Za-z0-9_]")


def mask(src):
    """`src` with comments, strings and char literals blanked; same length, newlines kept."""
    out = list(src)
    i, n = 0, len(src)

    def blank(a, b):
        for k in range(a, min(b, n)):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = src[i]
        ident_before = i > 0 and IDENT_BEFORE.match(src[i - 1])
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
        elif src.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif src.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            blank(i, j)
            i = j
        elif c in "rb" and not ident_before and re.match(r'(?:br|r)#*"', src[i : i + 12]):
            m = re.match(r'(?:br|r)(#*)"', src[i:])
            end = '"' + m.group(1)
            j = src.find(end, i + len(m.group(0)))
            j = n if j < 0 else j + len(end)
            blank(i, j)
            i = j
        elif c == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            blank(i, j + 1)
            i = j + 1
        elif c == "'":
            m = re.match(r"'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]+\}|.)|[^\\'\n])'", src[i:])
            if m:
                blank(i, i + len(m.group(0)))
                i += len(m.group(0))
            else:
                i += 1  # a lifetime or label
        else:
            i += 1
    return "".join(out)


def block_end(masked, start):
    """The index after the item that starts at `start`: its brace block, or its `;`."""
    i, nest = start, 0
    while i < len(masked) and (nest or masked[i] not in "{;"):
        # A `;` inside brackets (`[f32; 4]` in a signature) doesn't end the item.
        nest += {"(": 1, "[": 1, ")": -1, "]": -1}.get(masked[i], 0)
        i += 1
    if i >= len(masked) or masked[i] == ";":
        return i + 1
    depth = 0
    while i < len(masked):
        if masked[i] == "{":
            depth += 1
        elif masked[i] == "}":
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    return i


def module_file(parent, name, attrs, inline=(), root=False):
    """The file of `mod name;` declared in `parent` with attributes `attrs`, inside the inline
    modules whose directories are `inline` (outermost first), or None. A crate root (`root`, any
    file name) and a `mod.rs` hold their children beside them; any other file (a `lib.rs` reached
    through `#[path]` too) in a directory of its own name."""
    d = os.path.dirname(parent)
    if not root and os.path.basename(parent) != "mod.rs":
        d = os.path.join(d, os.path.basename(parent)[:-3])
    d = os.path.join(d, *inline)
    p = PATH_ATTR.search(attrs)
    if p:
        if not inline:
            d = os.path.dirname(parent)
        f = os.path.normpath(os.path.join(d, p.group(1)))
        return f if os.path.isfile(f) else None
    for f in (os.path.join(d, name + ".rs"), os.path.join(d, name, "mod.rs")):
        if os.path.isfile(f):
            return f
    return None


def inline_mods(src, masked, pos):
    """The inline `mod x { .. }` blocks around `pos`, outermost first, as (directory, test): the
    directory is the module's `#[path]`, else its name; test, whether a `#[cfg(test)]` holds it."""
    mods = []
    for m in INLINE_MOD.finditer(masked, 0, pos):
        if block_end(masked, m.start(2)) > pos:
            attrs = src[m.start(1) : m.end(1)]
            p = PATH_ATTR.search(attrs)
            mods.append((p.group(1) if p else m.group(2), bool(CFG_TEST.search(attrs))))
    return mods


def test_regions(root_file):
    """{file: None} for a test file, {file: [(start, end)]} with its test items' ranges otherwise."""
    regions, todo = {}, [(root_file, False, True)]
    while todo:
        f, is_test, root = todo.pop()
        if f in regions:
            continue
        src = open(f).read()
        m = mask(src)
        regions[f] = None if is_test else []
        for d in MOD_DECL.finditer(m):
            attrs = src[d.start(1) : d.end(1)]
            mods = inline_mods(src, m, d.start())
            child = module_file(f, d.group(2), attrs, [dir_ for dir_, _ in mods], root)
            if child:
                in_test = any(test for _, test in mods)
                todo.append((child, is_test or in_test or bool(CFG_TEST.search(attrs)), False))
        if not is_test:
            regions[f] = item_ranges(m)
    return regions


def compiled_files(roots):
    """Every .rs file under `roots` that a target root of a crate there reaches through `mod` lines."""
    targets = []
    for root in roots:
        for d, dirs, files in os.walk(root):
            dirs[:] = [x for x in dirs if x not in ("target", "vendor")]
            if "Cargo.toml" not in files:
                continue
            targets += [os.path.join(d, f) for f in ("build.rs", "src/lib.rs", "src/main.rs")]
            targets += [os.path.join(d, p) for p in TARGET_PATH.findall(open(os.path.join(d, "Cargo.toml")).read())]
            for sub in ("src/bin", "tests", "benches", "examples"):
                targets += glob.glob(os.path.join(d, sub, "*.rs")) + glob.glob(os.path.join(d, sub, "*", "main.rs"))
    roots_ = {os.path.normpath(t) for t in targets if os.path.isfile(t)}
    seen, todo = set(), sorted(roots_)
    while todo:
        f = todo.pop()
        if f in seen:
            continue
        seen.add(f)
        src = open(f).read()
        m = mask(src)
        for d in MOD_DECL.finditer(m):
            child = module_file(f, d.group(2), src[d.start(1) : d.end(1)], [dir_ for dir_, _ in inline_mods(src, m, d.start())], f in roots_)
            if child:
                todo.append(os.path.normpath(child))
    return seen


def item_ranges(masked):
    """The ranges of a non-test file's test items: `#[cfg(test)]` items and bare `#[test]` functions."""
    return [(a.start(), block_end(masked, a.end())) for a in (*CFG_TEST.finditer(masked), *TEST_ATTR.finditer(masked))]


def in_test(ranges, pos):
    return ranges is None or any(a <= pos < b for a, b in ranges)


def test_fns(masked):
    """[(name, body_start, body_end)] of the `#[test]` functions in `masked`; body_start is after `{`."""
    out = []
    for t in TEST_FN.finditer(masked):
        open_brace = masked.find("{", t.end())
        if open_brace < 0:
            continue
        out.append((t.group(1), open_brace + 1, block_end(masked, open_brace) - 1))
    return out


def git(*args, check=True):
    r = subprocess.run(["git", *args], capture_output=True, text=True)
    if check and r.returncode != 0:
        raise SystemExit(f"git {' '.join(args)}: {r.stderr.strip()}")
    return r.stdout


def git_show(ref, path):
    r = subprocess.run(["git", "show", f"{ref}:{path}"], capture_output=True, text=True)
    return r.stdout if r.returncode == 0 else None


HUNK = re.compile(r"^@@ -(\d+)(?:,(\d+))? ")


def definition_lines(src, item):
    """1-based (first, last) lines of `item` (`"fn name"`, `"struct Name"`, ...) in `src`, its doc
    comments and attributes included."""
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


def edited_definitions(base, head, items, roots):
    """The `(file, item)` pairs of `items` that the branch `base..head` edits: a changed line names the
    item, or a hunk's old-side lines overlap the item's definition in `base`. `file` is relative to
    whichever of `roots` (directory prefixes ending in `/`) holds it."""
    diff = subprocess.run(["git", "diff", "-U0", base, head, "--", *roots], capture_output=True, text=True, check=True).stdout
    current = None
    hunks, ranges, paths = {}, {}, {}
    for line in diff.splitlines():
        if line.startswith("--- "):
            path = line[4:].removeprefix("a/")
            current = None
            if path != "/dev/null":
                current = path
                for root in roots:
                    current = current.removeprefix(root)
                paths[current] = path
        elif line.startswith("+++ "):
            continue
        elif current and (h := HUNK.match(line)):
            start, count = int(h[1]), int(h[2] or 1)
            # A pure insertion (count 0) sits after `start`.
            ranges.setdefault(current, []).append((start, start + max(count, 1) - 1) if count else (start, start + 1))
        elif line.startswith(("+", "-")) and current:
            hunks.setdefault(current, []).append(line[1:])
    out = []
    for file, item in items:
        name = item.split()[1]
        named = any(re.search(rf"\b{re.escape(name)}\b", line) for line in hunks.get(file, []))
        inside = False
        if file in ranges and (src := git_show(base, paths[file])) is not None:
            span = definition_lines(src, item)
            inside = span is not None and any(a <= span[1] and b >= span[0] for a, b in ranges[file])
        if named or inside:
            out.append((file, item))
    return out
