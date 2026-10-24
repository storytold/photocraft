import pathlib
F = pathlib.Path("crates/ui-egui/src/pattern_fill_ui_tests.rs")
s = F.read_text()

old = """    let tallest = h.ctx.memory(|m| {
        m.areas()
            .layer_ids()
            .filter_map(|lid| m.areas().area_rect(lid.id))
            .map(|r| r.height())
            .fold(0.0f32, f32::max)
    });"""
assert s.count(old) == 1

new = """    let tallest = h.ctx.memory(|m| {
        m.layer_ids()
            .filter_map(|lid| m.area_rect(lid.id))
            .map(|r| r.height())
            .fold(0.0f32, f32::max)
    });"""
s = s.replace(old, new)
F.write_text(s)
print("Memory::layer_ids + Memory::area_rect(lid.id)")
