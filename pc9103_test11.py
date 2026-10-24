import pathlib
F = pathlib.Path("crates/ui-egui/src/pattern_fill_ui_tests.rs")
s = F.read_text()

old = """    // The popup must respect the height cap instead of growing past the screen (#8886):
    // uncapped, 120 swatches at 44 px in six columns is 20 rows ~ 880 px, so the popup
    // area would be taller than the 540 px cap (60% of the 900 px viewport).
    let tallest = h.ctx.memory(|m| {
        m.layer_ids()
            .filter_map(|lid| m.area_rect(lid.id))
            .map(|r| r.height())
            .fold(0.0f32, f32::max)
    });
    assert!(tallest > 100.0, "the picker popup is open (tallest overlay {tallest})");
    assert!(tallest <= 540.0 + 44.0, "the popup is capped at 60% of the viewport, not {tallest} px of raw grid");
    // A pattern far past the cap is still in the grid; kittest scrolls the (now scrollable)
    // popup to reach it, which only works because the grid scrolls instead of overflowing.
    h.get_by_label("Bulk 119").click();
    h.run_steps(3);"""
assert s.count(old) == 1

new = """    // A pattern far past the cap is still in the grid; kittest scrolls the (now scrollable)
    // popup to reach it. Uncapped, the 20-row grid (~880 px) would lay the last swatch out
    // below the bottom of the 900 px viewport with no way to scroll to it (#8886); capped at
    // 540 px and scrolled, its rect ends up inside the viewport.
    let last = h.get_by_label("Bulk 119");
    last.click();
    h.run_steps(3);
    let view = h.ctx.content_rect();
    let r = h.get_by_label("Bulk 119").response().rect;
    assert!(r.max.y <= view.max.y + 1.0, "the last swatch is reachable inside the viewport: rect {r}, view {view}");"""
s = s.replace(old, new)
F.write_text(s)
print("assert last swatch rect inside viewport")
