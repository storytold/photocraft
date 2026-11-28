#!/usr/bin/env python3
"""Engine split, step 6: session slots and extension traits.

State that only crates above engine core read left `Session`'s fields for session slots, each read
through its module's extension trait:

    s.color        ColorState          color_cmds::ColorExt              color() / color_mut()
    s.presets      Tracked<PresetState> presets::PresetsExt              presets() / presets_mut()
    s.actions      ActionState         actions_cmds::ActionsExt          actions() / actions_mut()
    s.object_finder ObjectFinderCache  smartselect_cmds::SmartSelectExt  object_finder() / object_finder_mut()
    s.print_spooler  Option<Spooler>        print_cmds::PrintExt   print_spooler() -> Option<&_> / print_spooler_mut()
    s.printer_source Option<PrinterSource>  print_cmds::PrintExt   printer_source() -> Option<&_> / printer_source_mut()

`presets()` / `presets_mut()` return the `Tracked` itself, so `.revision()` and `.edit(..)` stay as
they were and a deref takes one more `*` (`&mut *s.presets` is `&mut **s.presets_mut()`).

The `impl Session` methods of modules above core are trait methods now (same names, same files):
`open_document` (ColorExt), `start_link_scan`, `missing_links`, `missing_links_revision`,
`poll_link_scan` (link_watch::LinkWatchExt), `load_presets_json` (PresetsExt),
`brush_presets_changed`, `attach_preset_store`, `sync_preset_store`, `load_brush_tips`
(preset_store::PresetStoreExt), `prefs_to_json`, `prefs_value`, `load_prefs_json`, `saved_prefs`,
`set_saved_prefs` (prefs::SavedPrefsExt). Calling them needs the trait imported. The crate-private
`s.presets_changed()` is the free function `presets::presets_changed(s)`.

    python3 scripts/engine_split/slot_access.py split
        Rewrites every Rust file of the engine and its dependents (`ROOTS`) and adds the trait imports.

    python3 scripts/engine_split/slot_access.py branch --base <merge-base> --head <branch>
        On a branch rebased onto the step (`--head` is the branch from before the rebase,
        `ORIG_HEAD` right after it): rewrites the files the branch changed, and lists the branch's
        edits to the methods that became trait methods or free functions. A branch from before
        step 5 runs `session_state.py branch` first: its rules match `s.actions.<recording field>`.

    cargo check --message-format json ... | python3 scripts/engine_split/slot_access.py fix
        Applies what the compiler points at: a field access the rewrite missed, a read accessor
        that has to be `_mut` (or the other way round), an accessor on a type that isn't a session
        (put back), a missing or unused trait import. Run check and fix again until nothing changes.

Running any mode twice gives the same result. What it cannot do is printed at the end of every run.
"""
import argparse, json, os, re, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rust_source import block_end, compiled_files, edited_definitions, git, mask  # noqa: E402

CORE = "crates/engine-core/src/"
# The engine and the crates that use it; nothing below the engine has a session.
ROOTS = ("crates/engine-core", "crates/engine/", "crates/ui-egui", "crates/automation", "apps")

# field -> (module, trait, type, definition file, optional slot); `TRACKED` slots hold `Tracked<type>`.
SLOTS = {
    "color": ("color_cmds", "ColorExt", "ColorState", "cmd/color_cmds.rs", False),
    "presets": ("presets", "PresetsExt", "PresetState", "presets.rs", False),
    "actions": ("actions_cmds", "ActionsExt", "ActionState", "cmd/actions_cmds.rs", False),
    "object_finder": ("smartselect_cmds", "SmartSelectExt", "ObjectFinderCache", "cmd/smartselect_cmds.rs", False),
    "print_spooler": ("print_cmds", "PrintExt", "Spooler", "cmd/print_cmds.rs", True),
    "printer_source": ("print_cmds", "PrintExt", "PrinterSource", "cmd/print_cmds.rs", True),
}
# trait -> (module, methods besides the accessors)
TRAITS = {
    "ColorExt": ("color_cmds", ["open_document"]),
    "LinkWatchExt": ("link_watch", ["start_link_scan", "missing_links", "missing_links_revision", "poll_link_scan"]),
    "PresetsExt": ("presets", ["load_presets_json"]),
    "PresetStoreExt": ("preset_store", ["brush_presets_changed", "attach_preset_store", "sync_preset_store", "load_brush_tips"]),
    "SavedPrefsExt": ("prefs", ["prefs_to_json", "prefs_value", "load_prefs_json", "saved_prefs", "set_saved_prefs"]),
    "ActionsExt": ("actions_cmds", []),
    "SmartSelectExt": ("smartselect_cmds", []),
    "PrintExt": ("print_cmds", []),
}
TRACKED = {"presets"}
# `craft_tracked::Tracked`'s own methods: (reads, writes).
TRACKED_METHODS = ({"revision", "source"}, {"edit"})
# Former inherent methods and helpers, by file under `CORE`: what each is now.
MOVED = [
    ("cmd/color_cmds.rs", "open_document", "a `ColorExt` method"),
    ("link_watch.rs", "start_link_scan", "a `LinkWatchExt` method"),
    ("link_watch.rs", "missing_links", "a `LinkWatchExt` method"),
    ("link_watch.rs", "missing_links_revision", "a `LinkWatchExt` method"),
    ("link_watch.rs", "poll_link_scan", "a `LinkWatchExt` method"),
    ("link_watch.rs", "note_missing_links", "a free function `note_missing_links(s, ..)`"),
    ("link_watch.rs", "apply_link_stamps", "a free function `apply_link_stamps(s, ..)`"),
    ("presets.rs", "load_presets_json", "a `PresetsExt` method"),
    ("presets.rs", "presets_changed", "a free function `presets_changed(s)`"),
    ("preset_store.rs", "brush_presets_changed", "a `PresetStoreExt` method"),
    ("preset_store.rs", "attach_preset_store", "a `PresetStoreExt` method"),
    ("preset_store.rs", "sync_preset_store", "a `PresetStoreExt` method"),
    ("preset_store.rs", "load_brush_tips", "a `PresetStoreExt` method"),
    ("prefs/saved.rs", "prefs_to_json", "a `SavedPrefsExt` method"),
    ("prefs/saved.rs", "prefs_value", "a `SavedPrefsExt` method"),
    ("prefs/saved.rs", "load_prefs_json", "a `SavedPrefsExt` method"),
    ("prefs/saved.rs", "saved_prefs", "a `SavedPrefsExt` method"),
    ("prefs/saved.rs", "set_saved_prefs", "a `SavedPrefsExt` method"),
    ("prefs/saved.rs", "prefs_view", "a free function `prefs_view(s)`"),
    ("prefs/saved.rs", "set_pref", "a free function `set_pref(s, ..)`"),
]
# Methods whose names other types also have: they never trigger an import on their own.
COMMON = {"open_document", "missing_links"}
ACCESSOR_TRAIT = {}
for f, (_, tr, _, _, _) in SLOTS.items():
    ACCESSOR_TRAIT[f] = tr
    ACCESSOR_TRAIT[f + "_mut"] = tr
METHOD_TRAIT = {m: t for t, (_, ms) in TRAITS.items() for m in ms}
# Receivers that hold a field of the same name and are never a session.
NOT_SESSION = {"tools", "ui", "list", "imported", "file", "saved", "imp", "parsed", "panel", "brush", "gradient", "tool", "style", "shape", "edit", "pattern", "export", "document"}
# A session expression right before the field: `s`, `session`, `self` (in `impl ... Session` files),
# `x.session`, `session()`.
SESSION = r"(?:(?<![\w.])(?:s|session|sess|self)|(?:[\w\)]+\.)+session|(?<![\w.])session\(\))"
STD_MUT = {"push", "push_str", "insert", "remove", "retain", "retain_mut", "clear", "extend", "truncate", "sort", "sort_by", "sort_by_key", "sort_unstable", "dedup", "drain", "get_mut", "iter_mut", "values_mut", "last_mut", "first_mut", "entry", "take", "replace", "pop", "append", "swap", "as_mut", "get_or_insert", "get_or_insert_with", "split_off", "resize", "splice", "or_insert", "touch"}
ASSIGN = re.compile(r"\s*(?:=(?![=>])|\+=|-=|\*=|/=|%=|\|=|&=|\^=|<<=|>>=)")
PLACE = re.compile(r"(?:\.\w+(?!\s*[\(\w])|\[[^\[\]\n]*\])*")

REPORT = []


def members():
    """Per slot field: (fields, methods, methods taking `&mut self`), read from the engine source."""
    out = {}
    srcs = {}
    for d, _, files in os.walk(CORE):
        for name in files:
            if name.endswith(".rs"):
                p = os.path.join(d, name)
                with open(p, encoding="utf-8") as fh:
                    srcs[p] = fh.read()
    for field, (_, _, ty, defn, optional) in SLOTS.items():
        fields, methods, muts = set(), set(), set()
        if not optional:
            m = re.search(r"pub struct " + ty + r"\b[^{;]*\{(.*?)\n\}", srcs.get(CORE + defn, ""), re.S)
            if m:
                fields |= set(re.findall(r"^\s*(?:pub(?:\([a-z]+\))? )?(\w+):", m.group(1), re.M))
            for src in srcs.values():
                for im in re.finditer(r"\nimpl (?:crate::(?:\w+::)*)?" + ty + r" \{(.*?)\n\}", src, re.S):
                    for fm in re.finditer(r"^    (?:pub(?:\([a-z]+\))? )?fn (\w+)(?:<[^>]*>)?\((&mut self)?", im.group(1), re.M):
                        methods.add(fm.group(1))
                        if fm.group(2):
                            muts.add(fm.group(1))
        if field in TRACKED:
            methods |= TRACKED_METHODS[0] | TRACKED_METHODS[1]
            muts |= TRACKED_METHODS[1]
        out[field] = (fields, methods, muts)
    return out


MEMBERS = None


def all_muts():
    s = set(STD_MUT)
    for _, _, muts in MEMBERS.values():
        s |= muts
    return s


def writes(masked, start, end, field):
    """Whether the access `masked[start:end]` (`X.field`) is written through, from the text around it."""
    head = masked[max(0, start - 200) : start]
    if re.search(r"&\s*mut\s*$", head):
        return True
    rest = masked[end : end + 300]
    place = PLACE.match(rest)
    after = rest[place.end() :]
    if ASSIGN.match(after):
        return True
    call = re.match(r"\s*\.(\w+)\s*(?:::<[^>]*>)?\s*\(", after)
    if call and call.group(1) in all_muts():
        return True
    return False


# A receiver chain right before `.field`: `s`, `app.session`, `h.state_mut().session`.
RECEIVER = re.compile(r"(?:\w+(?:\(\))?\.)*\w+(?:\(\))?$")


def receiver_start(masked, start):
    m = RECEIVER.search(masked[max(0, start - 200) : start])
    return start - len(m.group(0)) if m else start


def rewrite_text(text, path):
    masked = mask(text)
    edits = []  # (start, end, replacement)
    impl_session = re.search(r"\bimpl\b[^{\n]*\bSession\s*\{", masked) is not None
    used_traits = set()
    # A deref of a `Tracked` slot on a session receiver: one more `*` through the accessor.
    for field in TRACKED:
        rx = r"(&\s*mut\s*|&\s*)?\*\s*(" + SESSION + r")\.(" + field + r")\b(?!\s*[\(\w])"
        for m in re.finditer(rx, masked):
            if m.group(2) == "self" and not impl_session:
                continue
            borrow = m.group(1) or ""
            mut = "mut" in borrow or (not borrow and ASSIGN.match(masked[m.end() : m.end() + 80]) is not None)
            edits.append((m.start(2), m.start(2), "*"))
            edits.append((m.start(3), m.end(3), field + ("_mut()" if mut else "()")))
            used_traits.add(SLOTS[field][1])
    for field, (fields, methods, _) in MEMBERS.items():
        optional = SLOTS[field][4]
        # `X.field.member`: only members of the slot type, so same-named fields elsewhere stay.
        if not optional:
            for m in re.finditer(r"(?<![\w])(\w+)(\(\))?\.(" + field + r")\.(\w+)(\s*(?:::<[^>]*>)?\s*\()?", masked):
                recv, member, is_call = m.group(1), m.group(4), m.group(5) is not None
                if recv in NOT_SESSION or (recv == "self" and not impl_session):
                    continue
                if (is_call and member not in methods) or (not is_call and member not in fields):
                    continue
                fs, fe = m.start(3), m.end(3)
                mut = writes(masked, receiver_start(masked, fs - 1), fe, field) or (is_call and member in MEMBERS[field][2])
                edits.append((fs, fe, field + ("_mut()" if mut else "()")))
                used_traits.add(SLOTS[field][1])
        # Whole-field uses on a session receiver.
        for m in re.finditer(r"(&\s*(?:mut\s+)?)?(" + SESSION + r")\.(" + field + r")\b(?!\s*[\(\w])", masked):
            fs, fe = m.start(3), m.end(3)
            if any(a <= fs < b for a, b, _ in edits):
                continue
            recv_text = m.group(2)
            if recv_text in ("s", "sess", "self"):
                continue  # other types name their fields the same: `fix` rewrites the session ones

            if re.match(r"\s*\.", masked[fe:]) and not optional:
                continue  # `X.field.member` with a member the slot type doesn't have
            borrow = m.group(1)
            rest = masked[fe : fe + 80]
            if optional:
                opt = re.match(r"\.(as_ref|as_mut|is_some|is_none|clone|take|as_deref)\(\)", rest)
                if opt:
                    kind = opt.group(1)
                    to = {"as_ref": "()", "as_deref": "()", "is_some": "().is_some()", "is_none": "().is_none()", "clone": "().cloned()", "as_mut": "_mut().as_mut()", "take": "_mut().take()"}[kind]
                    edits.append((fs, fe + opt.end(), field + to))
                    used_traits.add(SLOTS[field][1])
                    continue
            if ASSIGN.match(rest) and not borrow:
                rs = m.start(2)
                edits.append((rs, rs, "*"))
                edits.append((fs, fe, field + "_mut()"))
            elif borrow:
                mut = "mut" in borrow
                edits.append((m.start(1), m.start(2), ""))
                edits.append((fs, fe, field + ("_mut()" if mut else "()")))
            elif optional:
                REPORT.append(f"{path}: `{field}` used as a whole; write `{field}()` (Option<&_>) or `{field}_mut()`")
                continue
            else:
                mut = writes(masked, m.start(2), fe, field)
                edits.append((fs, fe, field + ("_mut()" if mut else "()")))
            used_traits.add(SLOTS[field][1])
    # `s.presets_changed()` is the crate-private free function.
    if path.startswith(CORE) or ("/" + CORE) in path:
        calls = list(re.finditer(r"(?<![\w.])(" + SESSION + r")\.presets_changed\(\)", masked))
        for m in calls:
            edits.append((m.start(), m.end(), f"presets_changed({m.group(1)})"))
        if calls and not re.search(r"\bfn presets_changed\b", masked):
            used_traits.add("presets_changed")
    # Former inherent methods.
    for meth, tr in METHOD_TRAIT.items():
        if meth in COMMON:
            if re.search(SESSION + r"(?:\.\w+|\(\))*\." + meth + r"\(", masked):
                used_traits.add(tr)
        elif re.search(r"\." + meth + r"\(", masked):
            used_traits.add(tr)
    new = doc_links(apply(text, edits), path)
    for tr in sorted(used_traits):
        new = add_import(new, path, tr)
    return new


def doc_links(text, path):
    """`Session::method` in docs names the trait method: an intra-doc link to its path, else `Trait::method`."""

    def rep(m):
        link, name = m.group(1), m.group(2)
        trait = METHOD_TRAIT.get(name) or ACCESSOR_TRAIT.get(name)
        if not trait:
            return m.group(0)
        if link:
            return f"[`{trait}::{name}`]({trait_path(path, trait)}::{name})"
        return f"`{trait}::{name}`"

    return re.sub(r"(\[)?`(?:photocraft_engine::)?Session::(\w+)`(?(1)\])", rep, text)


def apply(text, edits):
    for s, e, r in sorted(set(edits), key=lambda x: (x[0], x[1]), reverse=True):
        text = text[:s] + r + text[e:]
    return text


def trait_path(path, trait):
    module = TRAITS[trait][0] if trait in TRAITS else "presets"
    root = "crate" if path.startswith(CORE) or ("/" + CORE) in path else "photocraft_engine"
    return f"{root}::{module}::{trait}"


def add_import(text, path, trait):
    """Adds `use <path>::<trait>;` unless the file already names the trait in a `use` or defines it."""
    masked = mask(text)
    # A `use` inside an inline module (`mod tests { .. }`) doesn't reach the file's other code.
    if any(enclosing_mod(masked, m.start()) is None for m in re.finditer(r"\b(?:use|trait)\b[^;{]*\b" + trait + r"\b", masked)):
        return text
    line = f"use {trait_path(path, trait)};\n"
    m = re.search(r"^use ", masked, re.M)
    if m:
        # Before the outer attributes (`#[cfg(test)]`) of that first import, so they stay on it.
        pos = m.start()
        while pos > 0:
            prev = text.rfind("\n", 0, pos - 1) + 1
            if not masked[prev:pos].lstrip().startswith("#["):
                break
            pos = prev
        return text[:pos] + line + text[pos:]
    # After the file's header: inner doc comments, blank lines and inner attributes (`#![..]`,
    # over several lines too).
    pos = 0
    while pos < len(text):
        if masked.startswith("#![", pos):
            end = attr_end(masked, pos + 2)
            pos = text.find("\n", end)
            pos = len(text) if pos < 0 else pos + 1
            continue
        lm = re.match(r"(?://!.*|[ \t]*)\n", text[pos:])
        if not lm:
            break
        pos += lm.end()
    return text[:pos] + line + text[pos:]


def attr_end(masked, open_bracket):
    """The index after the `]` that closes the `[` at `open_bracket`."""
    depth = 0
    for i in range(open_bracket, len(masked)):
        if masked[i] == "[":
            depth += 1
        elif masked[i] == "]":
            depth -= 1
            if depth == 0:
                return i + 1
    return len(masked)


def enclosing_mod(masked, pos):
    """The body start of the innermost inline `mod x { ... }` holding `pos`, or None at file level."""
    best = None
    for m in re.finditer(r"\bmod\s+\w+\s*\{", masked[:pos]):
        if block_end(masked, m.start()) > pos:
            best = m.end()
    return best


def add_import_in(text, path, trait, body):
    """Adds the trait's `use` at the top of the inline module whose body starts at `body`."""
    masked = mask(text)
    end = block_end(masked, body - 1)
    if re.search(r"\buse\b[^;{]*\b" + trait + r"\b", masked[body:end]):
        return text
    nl = text.index("\n", body) + 1
    indent = re.match(r"[ \t]*", text[nl:]).group(0)
    return text[:nl] + f"{indent}use {trait_path(path, trait)};\n" + text[nl:]


def rewrite(path):
    with open(path, encoding="utf-8") as f:
        old = f.read()
    new = rewrite_text(old, path)
    if new != old:
        with open(path, "w", encoding="utf-8") as f:
            f.write(new)
        return True
    return False


# ---------------------------------------------------------------- fix mode

ACCESSOR_READ = re.compile(r"\.(" + "|".join(SLOTS) + r")\(\)")


def fix(stream):
    edits = {}  # path -> [(start, end, replacement)]
    imports = {}  # path -> {trait}
    removals = {}  # path -> {1-based line of an unused trait import}
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
        spans = [s for s in d.get("spans", []) if s.get("is_primary")]
        if not spans:
            continue
        sp = spans[0]
        path = sp["file_name"]
        if not os.path.exists(path) or "/vendor/" in path or path.startswith("vendor/"):
            continue
        text = read(path)
        b0, b1 = byte_to_char(text, sp["byte_start"]), byte_to_char(text, sp["byte_end"])
        snippet = text[b0:b1]
        text_msg = d["message"]
        if code in ("E0609", "E0615"):
            m = re.match(r"(?:no field|attempted to take value of method) `(\w+)` on type `[^`]*Session[^`]*`", text_msg)
            if m and m.group(1) in SLOTS:
                field = m.group(1)
                module = enclosing_mod(mask(text), b0)
                masked = mask(text)
                mut = writes(masked, receiver_start(masked, b0 - 1), b1, field)
                rest = masked[b1 : b1 + 80]
                if ASSIGN.match(rest):
                    edits.setdefault(path, []).append((receiver_start(masked, b0 - 1), receiver_start(masked, b0 - 1), "*"))
                    mut = True
                borrow = re.search(r"&\s*(mut\s+)?$", masked[: receiver_start(masked, b0 - 1)])
                if borrow:
                    edits.setdefault(path, []).append((borrow.start(), borrow.end(), ""))
                    mut = bool(borrow.group(1))
                opt = re.match(r"\.(as_ref|as_deref|clone|as_mut|take)\(\)", rest) if SLOTS[field][4] else None
                if opt:
                    # An Option slot: the read accessor is already `Option<&_>`.
                    to = {"as_ref": "()", "as_deref": "()", "clone": "().cloned()", "as_mut": "_mut().as_mut()", "take": "_mut().take()"}[opt.group(1)]
                    edits.setdefault(path, []).append((b0, b1 + opt.end(), field + to))
                else:
                    edits.setdefault(path, []).append((b0, b1, field + ("_mut()" if mut else "()")))
                imports.setdefault(path, set()).add((SLOTS[field][1], module))
                continue
        if code in ("E0596", "E0594"):
            ms = list(ACCESSOR_READ.finditer(snippet))
            if ms:
                m = ms[-1]
                edits.setdefault(path, []).append((b0 + m.start(1), b0 + m.end(1) + 2, m.group(1) + "_mut()"))
                continue
            after = re.match(r"\.(" + "|".join(SLOTS) + r")_mut\(\)", text[b1 : b1 + 40])
            if after:
                edits.setdefault(path, []).append((b1 + 1, b1 + after.end(), after.group(1) + "()"))
                continue
        if code == "E0599":
            m = re.match(r"no method named `(\w+)` found for (.*)", text_msg)
            if m:
                name, on = m.group(1), m.group(2)
                trait = ACCESSOR_TRAIT.get(name) or METHOD_TRAIT.get(name)
                in_scope_hint = any("not in scope" in (c.get("message") or "") or "trait" in (c.get("message") or "") for c in d.get("children", []))
                if trait and "Session" in on and in_scope_hint:
                    imports.setdefault(path, set()).add((trait, enclosing_mod(mask(text), b0)))
                    continue
                if name in ACCESSOR_TRAIT and "Session" not in on:
                    field = name.removesuffix("_mut")
                    end = b1 + (2 if text[b1 : b1 + 2] == "()" else 0)
                    edits.setdefault(path, []).append((b0, end, field))
                    if not re.match(r"\s*[\.\[]", text[end : end + 2]):
                        # A whole-field rewrite: `*x.field_mut() = v` was `x.field = v`; otherwise
                        # it was `&x.field` / `&mut x.field`.
                        rs = receiver_start(mask(text), b0 - 1)
                        if rs > 0 and text[rs - 1] == "*":
                            edits.setdefault(path, []).append((rs - 1, rs, ""))
                        else:
                            edits.setdefault(path, []).append((rs, rs, "&mut " if name.endswith("_mut") else "&"))
                    continue
        if code == "E0425" and re.match(r"cannot find function `presets_changed`", text_msg):
            imports.setdefault(path, set()).add(("presets_changed", enclosing_mod(mask(text), b0)))
            continue
        if d.get("level") == "warning" and code == "unused_imports":
            # The import the warning points at, wherever it is (file level or an inline module).
            if any(tr in snippet for tr in [*TRAITS, "presets_changed"]):
                removals.setdefault(path, set()).add(sp["line_start"])
            continue
        if d.get("level") == "error":
            left.append(f"{path}:{sp['line_start']}: {code or ''} {text_msg}")
    if left or edits or imports:
        # An unused-import warning next to errors may only mean the code using it didn't type-check.
        removals = {}
    for path in set(edits) | set(imports) | set(removals):
        text = apply(read(path), edits.get(path, []))
        # Module offsets were taken in the text before `edits`; move them past the edits above them.
        shift = lambda off: off + sum(len(r) - (e - b) for b, e, r in set(edits.get(path, [])) if e <= off)
        moved = {(tr, None if module is None else shift(module)) for tr, module in imports.get(path, ())}
        # Innermost module first, so earlier insertion offsets stay valid.
        for tr, module in sorted(moved, key=lambda x: -(x[1] or 0)):
            text = add_import(text, path, tr) if module is None else add_import_in(text, path, tr, module)
        lines = text.splitlines(keepends=True)
        for n in sorted(removals.get(path, ()), reverse=True):
            if re.match(r"\s*use [\w:]*::\w+;\n", lines[n - 1]):
                del lines[n - 1]
        text = "".join(lines)
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)
    n = sum(len(v) for v in edits.values()) + sum(len(v) for v in imports.values()) + sum(len(v) for v in removals.values())
    print(f"slot_access fix: {n} edits in {len(set(edits) | set(imports) | set(removals))} files; {len(left)} errors left for hand fixes")
    for line in left:
        print("  " + line)


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def byte_to_char(text, b):
    return len(text.encode("utf-8")[:b].decode("utf-8", errors="ignore"))


CANNOT_DO = """
What this script cannot do (the compiler points at each):
  - borrow conflicts: an accessor borrows the whole session, so `s.presets_mut().x.push(s.tools.y)`
    or a slot reference held while another session field is read needs the other read in a local
    first (`let y = s.tools.y.clone(); s.presets_mut().x.push(y);`);
  - a reference to a slot taken through another binding and written later (`let c = s.color(); ...`):
    make it `color_mut()`;
  - a trait import in an inline module without `use super::*`;
  - a `Tracked` slot borrowed through another binding and dereferenced later
    (`let p = &mut s.presets; ... &mut **p`): add the `*` by hand;
  - a new `Session` field read only above core: make it a slot of its crate (`Install::slot` in
    `register`, an accessor on the module's extension trait).
"""


def main():
    global MEMBERS
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="mode", required=True)
    sub.add_parser("split")
    b = sub.add_parser("branch")
    b.add_argument("--base", required=True)
    b.add_argument("--head", required=True)
    sub.add_parser("fix")
    args = ap.parse_args()
    os.chdir(git("rev-parse", "--show-toplevel").strip())
    MEMBERS = members()
    if args.mode == "split":
        n = sum(rewrite(p) for p in sorted(compiled_files(ROOTS)))
        print(f"slot_access split: {n} files rewritten")
    elif args.mode == "branch":
        changed = git("diff", "--name-only", f"{args.base}...{args.head}").split()
        n = sum(rewrite(p) for p in changed if p.endswith(".rs") and p.startswith(ROOTS) and os.path.exists(p))
        now = {(f, "fn " + name): what for f, name, what in MOVED}
        for f, item in edited_definitions(args.base, args.head, list(now), ["crates/engine/src/", CORE]):
            REPORT.append(f"NOT DONE: the branch edits `{item}` in {CORE}{f}; it is {now[(f, item)]} now (same file): re-apply the edit there")
        print(f"slot_access branch: {n} files rewritten")
    else:
        fix(sys.stdin)
    for line in REPORT:
        print(line)
    print(CANNOT_DO)


if __name__ == "__main__":
    main()
