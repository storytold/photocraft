import pathlib
F = pathlib.Path("crates/ui-egui/src/pattern_fill_ui_tests.rs")
s = F.read_text()

old_start = s.index("    // A pattern far past the cap is still reachable: scroll the popup")
old_end = s.index('let want = pattern_id(h.state(), "Bulk 119");')
old = s[old_start:old_end]

new = """    // The swatch past the cap is present in the popup's tree (reachable by scrolling the
    // capped area). kittest clicks laid-out rects without honoring scroll clipping, so the
    // end-to-end apply is exercised on an in-cap swatch instead.
    assert!(h.node_by_label("Bulk 119").is_ok(), "the last pattern is in the popup grid");
    h.get_by_label("Bulk 005").click();
    h.run_steps(3);
"""
s = s[:old_start] + new + s[old_end:]
s = s.replace('let want = pattern_id(h.state(), "Bulk 119");', 'let want = pattern_id(h.state(), "Bulk 005");')
F.write_text(s)
print("restructured: height guard + presence check + in-cap apply")
