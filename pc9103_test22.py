import pathlib
F = pathlib.Path("crates/ui-egui/src/pattern_fill_ui_tests.rs")
s = F.read_text()

old = """    h.get_by_role_and_label(egui::accesskit::Role::Button, "Patterns").click();
    h.run_steps(3);
    // The swatch far past the cap is present in the popup's tree — reachable by scrolling
    // the capped area, which is what the fix added. (kittest clicks laid-out rects without
    // honoring scroll clipping, so the end-to-end apply is exercised on an in-cap swatch.)
    h.get_by_label("Bulk 299");"""
assert s.count(old) == 1

new = """    // Opening the picker must add a scroll area (the capped grid, #8886): egui persists a
    // scroll_area::State per ScrollArea, so the count grows when the grid becomes scrollable.
    let before = h.ctx.data(|d| d.count::<egui::containers::scroll_area::State>());
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Patterns").click();
    h.run_steps(3);
    let after = h.ctx.data(|d| d.count::<egui::containers::scroll_area::State>());
    assert!(after > before, "the picker grid is a scroll area: state count {before} -> {after}");
    // The swatch far past the cap is present in the popup's tree. (kittest clicks laid-out
    // rects without honoring scroll clipping, so the end-to-end apply is exercised on an
    // in-cap swatch.)
    h.get_by_label("Bulk 299");"""
s = s.replace(old, new)
F.write_text(s)
print("scroll-area state delta discriminator")
