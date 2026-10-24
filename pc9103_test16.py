import pathlib
F = pathlib.Path("crates/ui-egui/src/pattern_fill_ui_tests.rs")
s = F.read_text()

old = """    // A pattern far past the cap is still in the grid and selectable: kittest scrolls the
    // (now scrollable) popup to reach it.
    h.get_by_label("Bulk 119").click();
    h.run_steps(3);"""
assert s.count(old) == 1, "anchor not found"

new = """    // A pattern far past the cap is still reachable: scrolling it into view moves its rect
    // into the viewport, and the click applies it. Uncapped there was no scroll area, so
    // the bottom rows sat permanently below the screen (#8886).
    let last = h.get_by_label("Bulk 119");
    last.scroll_to_me();
    h.run_steps(2);
    let r = h.get_by_label("Bulk 119").rect();
    assert!(r.max.y <= h.ctx.content_rect().max.y + 1.0, "after scroll_to_me the last swatch is on screen: {r}");
    h.get_by_label("Bulk 119").click();
    h.run_steps(3);"""
s = s.replace(old, new)
F.write_text(s)
print("scroll_to_me then click applied")
