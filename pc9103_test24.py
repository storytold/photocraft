import pathlib
F = pathlib.Path("crates/ui-egui/src/pattern_fill_ui_tests.rs")
s = F.read_text()

old = """    let before = h.ctx.data(|d| d.count::<egui::containers::scroll_area::State>());
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Patterns").click();"""
assert s.count(old) == 1
s = s.replace(old, """    let before = h.ctx.data(|d| d.count::<egui::containers::scroll_area::State>());
    let before_ids = h.ctx.memory(|m| m.layer_ids().copied().collect::<std::collections::HashSet<_>>());
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Patterns").click();""")

old2 = """    let after = h.ctx.data(|d| d.count::<egui::containers::scroll_area::State>());
    assert!(after > before, "the picker grid is a scroll area: state count {before} -> {after}");"""
assert s.count(old2) == 1
new2 = """    let after = h.ctx.data(|d| d.count::<egui::containers::scroll_area::State>());
    assert!(after > before, "the picker grid is a scroll area: state count {before} -> {after}");
    // The popup must also respect the height cap instead of growing past the screen:
    // uncapped, 300 swatches at 44 px in six columns is 50 rows ~ 2200 px. Measure the
    // popup by its own layer (the area that appeared when the picker opened), not by a
    // height window an oversized popup could escape.
    let popup = h
        .ctx
        .memory(|m| {
            m.layer_ids()
                .filter(|lid| !before_ids.contains(lid))
                .filter_map(|lid| m.area_rect(lid.id))
                .max_by(|a, b| a.height().partial_cmp(&b.height()).unwrap())
        })
        .expect("opening the picker added a popup layer");
    let cap = 0.6 * h.ctx.input(|i| i.viewport().inner_rect.map(|r| r.height()).unwrap_or(900.0)) + 44.0;
    assert!(popup.height() <= cap, "the picker popup is capped at 60% of the viewport, not {} px of raw grid", popup.height());"""
s = s.replace(old2, new2)

# remove the old height-window block if still present
if "let heights = h.ctx.memory" in s:
    start = s.index("    let heights = h.ctx.memory")
    end = s.index("\n", s.index('assert!(tallest <= 540.0')) + 1
    s = s[:start] + s[end:]
F.write_text(s)
print("layer-based popup measurement, old window removed")
