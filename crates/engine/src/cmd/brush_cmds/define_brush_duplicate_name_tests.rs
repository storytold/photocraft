use super::*;

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8, "depth": depth, "background": "transparent"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    set_source(&mut s, Rect::new(1, 1, 3, 4), 0.0);
    s
}

fn set_source(s: &mut Session, rect: Rect, gray: f32) {
    s.edit("setup brush source", |doc, active| {
        let bounds = doc.bounds();
        let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surface.fill_rect(bounds, &[0.0; 4]);
        surface.fill_rect(rect, &[gray, gray, gray, 1.0]);
        Ok(())
    })
    .unwrap();
}

fn check_duplicate_names(command: &str) {
    for depth in [8, 16, 32] {
        for name in ["Duplicate", "dUPLICATE"] {
            let mut s = session(depth);
            s.execute(command, json!({"name": "Duplicate"})).unwrap();
            s.execute("brush.presets.update", json!({"name": "Duplicate", "brush": {"spacing": 0.6, "angle": 31, "roundness": 0.625}})).unwrap();
            s.execute("brush.presets.save", json!({"name": "Tail"})).unwrap();
            let before = s.tools.presets.clone();
            let old_tip = before.iter().find(|p| p.name == "Duplicate").unwrap().brush.tip.clone();
            let revision = s.tools.presets_rev;

            set_source(&mut s, Rect::new(2, 1, 6, 3), 0.5);
            s.execute(command, json!({"name": name})).unwrap();

            assert_eq!(s.tools.presets.len(), before.len() + 1, "{command}, depth {depth}, name {name}");
            assert_eq!(&s.tools.presets[..before.len()], before.as_slice(), "existing tips, settings and order survive");
            let added = s.tools.presets.last().unwrap();
            assert_eq!(added.name, name);
            assert!(!added.builtin && added.group.is_empty() && added.folder.is_empty(), "append at the root");
            assert_ne!(added.brush.tip, old_tip);
            assert_eq!(s.tools.brush, added.brush, "the new tip becomes current immediately");
            assert_eq!(s.tools.current_preset.as_deref(), Some(name));
            assert!(s.tools.presets_rev > revision, "the store is notified of the added preset");
            assert!(matches!(&added.brush.tip, TipShape::Sampled(tip) if tip.width == 4 && tip.height == 2));
            let listed = s.execute("brush.presets.list", json!({})).unwrap();
            assert_eq!(listed["presets"].as_array().unwrap().len(), before.len() + 1);
            assert_eq!(listed["presets"].as_array().unwrap().last().unwrap()["name"], name);

            // Which duplicate to pick by name is unmeasured: retain today's first match.
            s.execute("tools.setBrush", json!({"preset": name})).unwrap();
            assert_eq!(s.tools.brush.tip, old_tip);
        }
    }
}

#[test]
fn edit_define_brush_preset_appends_duplicate_names() {
    check_duplicate_names("edit.defineBrushPreset");
}

#[test]
fn define_from_selection_appends_duplicate_names() {
    check_duplicate_names("brush.defineFromSelection");
}

#[test]
fn defining_identical_pixels_twice_keeps_both_presets() {
    for command in ["edit.defineBrushPreset", "brush.defineFromSelection"] {
        let mut s = session(8);
        s.execute(command, json!({"name": "Identical"})).unwrap();
        let first = s.tools.presets.last().unwrap().clone();
        let count = s.tools.presets.len();
        s.execute(command, json!({"name": "Identical"})).unwrap();
        assert_eq!(s.tools.presets.len(), count + 1);
        assert_eq!(s.tools.presets[count - 1], first);
        assert_eq!(s.tools.presets[count], first, "identical names and pixels are not deduplicated");
    }
}

#[test]
fn save_still_replaces_the_first_case_insensitive_name_match() {
    let mut s = session(8);
    s.execute("edit.defineBrushPreset", json!({"name": "Duplicate"})).unwrap();
    let first_index = s.tools.presets.len() - 1;
    set_source(&mut s, Rect::new(2, 1, 6, 3), 0.5);
    s.execute("edit.defineBrushPreset", json!({"name": "DUPLICATE"})).unwrap();
    let second = s.tools.presets.last().unwrap().clone();
    let count = s.tools.presets.len();
    s.execute("tools.setBrush", json!({"size": 17, "angle": 45})).unwrap();
    let saved = s.tools.brush.clone();

    s.execute("brush.presets.save", json!({"name": "duplicate"})).unwrap();

    assert_eq!(s.tools.presets.len(), count, "save replaces instead of appending");
    assert_eq!(s.tools.presets[first_index].name, "duplicate");
    assert_eq!(s.tools.presets[first_index].brush, saved);
    assert_eq!(s.tools.presets.last().unwrap(), &second, "later duplicates are untouched");
}
