"""Shared helpers for the engine split scripts: Rust source scanning and git.

`mask` blanks comments, strings and char literals so a regex over the result sees only code, at
the same offsets. `test_regions` finds a crate's test code: whole files reached through a
`#[cfg(test)]` (or `#[cfg(all(test, ...))]`) `mod x;`, and the `#[cfg(test)]` items and bare `#[test]`
functions of other files (the test harness compiles `#[test]` functions only in test builds).
`#[cfg(any(test, ...))]` items are not test code: other builds compile them too.
"""
import os, re, subprocess

CFG_TEST = re.compile(r"#\[cfg\((?:test|all\(test\b[^\]]*\))\)\]")
MOD_DECL = re.compile(r"((?:#\[[^\]]*\]\s*)*)(?:pub(?:\([a-z]+\))?\s+)?mod\s+([a-z0-9_]+)\s*;")
PATH_ATTR = re.compile(r'#\[path\s*=\s*"([^"]+)"\]')
TEST_ATTR = re.compile(r"#\[test\]")
TEST_FN = re.compile(r"#\[test\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([a-z]+\))?\s+)?(?:async\s+)?fn\s+([a-z0-9_]+)")
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


def module_file(parent, name, attrs):
    """The file of `mod name;` declared in `parent` with attributes `attrs`, or None."""
    d = os.path.dirname(parent)
    p = PATH_ATTR.search(attrs)
    if p:
        f = os.path.normpath(os.path.join(d, p.group(1)))
        return f if os.path.isfile(f) else None
    base = d if os.path.basename(parent) in ("lib.rs", "mod.rs", "main.rs") else os.path.join(d, os.path.basename(parent)[:-3])
    for f in (os.path.join(base, name + ".rs"), os.path.join(base, name, "mod.rs")):
        if os.path.isfile(f):
            return f
    return None


def test_regions(root_file):
    """{file: None} for a test file, {file: [(start, end)]} with its test items' ranges otherwise."""
    regions, todo = {}, [(root_file, False)]
    while todo:
        f, is_test = todo.pop()
        if f in regions:
            continue
        src = open(f).read()
        m = mask(src)
        regions[f] = None if is_test else []
        for d in MOD_DECL.finditer(m):
            attrs = src[d.start(1) : d.end(1)]
            child = module_file(f, d.group(2), attrs)
            if child:
                todo.append((child, is_test or bool(CFG_TEST.search(attrs))))
        if not is_test:
            regions[f] = item_ranges(m)
    return regions


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
