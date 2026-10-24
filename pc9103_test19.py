import pathlib
F = pathlib.Path("crates/ui-egui/src/pattern_fill_ui_tests.rs")
s = F.read_text()

old = """    // The swatch far past the cap is present in the popup's tree — reachable by scrolling
    // the capped area, which is what the fix added. (kittest clicks laid-out rects without
    // honoring scroll clipping, so the end-to-end apply is exercised on an in-cap swatch.)
    h.get_by_label("Bulk 299");
    h.get_by_label("Bulk 005").click();
    h.run_steps(3);"""
assert s.count(old) == 1

new = """    // The swatch far past the cap is only reachable because the grid scrolls: scrolling it
    // into view moves its laid-out rect up (uncapped there is no scroll area, so the rect
    // sits permanently below the screen and nothing moves it).
    let before = h.get_by_label("Bulk 299").rect();
    h.get_by_label("Bulk 299").scroll_to_me();
    h.run_steps(8);
    let after = h.get_by_label("Bulk 299").rect();
    assert!(after.max.y < before.max.y, "the capped grid scrolls the last swatch into view: {before} -> {after}");
    h.get_by_label("Bulk 005").click();
    h.run_steps(3);"""
s = s.replace(old, new)
F.write_text(s)
print("scroll-moves-rect assertion")
