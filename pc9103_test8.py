import pathlib
F = pathlib.Path("crates/ui-egui/src/pattern_fill_ui_tests.rs")
s = F.read_text()

old = """    // Fill the library well past what fits in the capped popup (cap = 60% of the viewport
    // height; at 44 px per swatch cell that is a dozen-ish rows of six).
    for i in 0..40 {
        h.state_mut().session.patterns.items.push(bulk_pattern(&format!("Bulk {i:02}"), i as u8));
    }"""
assert s.count(old) == 1

new = """    // Fill the library well past what fits in the capped popup (cap = 60% of the 900 px
    // viewport = 540 px; at 44 px per swatch cell in six columns, 120 patterns is 20 rows
    // ~ 880 px uncapped).
    for i in 0..120 {
        h.state_mut().session.patterns.items.push(bulk_pattern(&format!("Bulk {i:03}"), i as u8));
    }"""
s = s.replace(old, new)
F.write_text(s)
print("120 patterns")
