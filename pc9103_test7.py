import pathlib
F = pathlib.Path("crates/ui-egui/src/pattern_fill_ui_tests.rs")
s = F.read_text()

old = """    h.get_by_role_and_label(egui::accesskit::Role::Button, "Patterns").click();
    h.run_steps(3);
    // A pattern far past the cap is still in the grid; kittest scrolls the (now scrollable)
    // popup to reach it, which only works because the grid scrolls instead of overflowing.
    h.get_by_label("Bulk 39").click();
    h.run_steps(3);
    let want = pattern_id(h.state(), "Bulk 39");"""
assert s.count(old) == 1

new = """    h.get_by_role_and_label(egui::accesskit::Role::Button, "Patterns").click();
    h.run_steps(3);
    // The popup must respect the height cap instead of growing past the screen (#8886):
    // uncapped, 120 swatches at 44 px in six columns is 20 rows ~ 880 px, so the popup
    // area would be taller than the 540 px cap (60% of the 900 px viewport).
    let tallest = h.ctx.memory(|m| {
        m.areas()
            .layer_ids()
            .filter_map(|lid| m.areas().area_rect(lid.id))
            .map(|r| r.height())
            .fold(0.0f32, f32::max)
    });
    assert!(tallest > 100.0, "the picker popup is open (tallest overlay {tallest})");
    assert!(tallest <= 540.0 + 44.0, "the popup is capped at 60% of the viewport, not {tallest} px of raw grid");
    // A pattern far past the cap is still in the grid; kittest scrolls the (now scrollable)
    // popup to reach it, which only works because the grid scrolls instead of overflowing.
    h.get_by_label("Bulk 119").click();
    h.run_steps(3);
    let want = pattern_id(h.state(), "Bulk 119");"""
s = s.replace(old, new)
F.write_text(s)
print("height assertion via area_rect")
