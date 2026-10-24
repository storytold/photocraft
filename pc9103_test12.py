import pathlib
F = pathlib.Path("crates/ui-egui/src/pattern_fill_ui_tests.rs")
s = F.read_text()

old = """    // A pattern far past the height cap is still in the grid and selectable: kittest scrolls
    // the (now scrollable) popup to reach it. Uncapped, the 20-row grid overflowed the screen
    // with no way to reach the bottom rows (#8886).
    h.get_by_label("Bulk 119").click();
    h.run_steps(3);"""
assert s.count(old) == 1

new = """    // The popup itself must respect the height cap: uncapped, 120 swatches at 44 px in six
    // columns is 20 rows ~ 880 px, taller than the 540 px cap (60% of the 900 px viewport)
    // and unreachable at the bottom (#8886). The popup is the overlay area that appeared when
    // the picker opened — the tallest area that is not the dialog window itself.
    let heights = h.ctx.memory(|m| {
        m.layer_ids()
            .filter_map(|lid| m.area_rect(lid.id).map(|r| (format!("{:?}", lid.id), r.height())))
            .collect::<Vec<_>>()
    });
    let popup = heights
        .iter()
        .filter(|(_, hgt)| *hgt > 100.0 && *hgt < 899.0)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .copied();
    let Some((id, tallest)) = popup else {
        panic!("no popup area among {heights:?}")
    };
    assert!(tallest <= 540.0 + 44.0, "the picker popup ({id}) is capped at 60% of the viewport, not {tallest} px of raw grid");
    // A pattern far past the cap is still in the grid and selectable: kittest scrolls the
    // (now scrollable) popup to reach it.
    h.get_by_label("Bulk 119").click();
    h.run_steps(3);"""
s = s.replace(old, new)
F.write_text(s)
print("popup-area height assertion")
