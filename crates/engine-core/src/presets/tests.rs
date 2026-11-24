use crate::prefs::SavedPrefsExt;
use crate::presets::PresetsExt;
use serde_json::{Value, json};

use crate::Session;
use photocraft_doc::{Fill, LayerContent, LayerId};

fn session(depth: u32) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": depth})).unwrap();
    s
}

fn active(s: &Session) -> LayerId {
    s.active().unwrap().active_layer.unwrap()
}

// ------------------------------------------------------------------ styles

#[test]
fn style_preset_new_from_an_explicit_effect_list() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    // The Layer Style dialog's "New Style…" saves its pending state, not the layer.
    let r = s
        .execute(
            "style.presets.new",
            json!({"name": "Pending", "effects": [["stroke", {"size": 6, "color": "#123456"}]], "blend": "Multiply", "fillOpacity": 40}),
        )
        .unwrap();
    assert_eq!(r["name"], json!("Pending"));
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("style.presets.apply", json!({"preset": "Pending"})).unwrap();
    let d = s.active().unwrap();
    let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
    assert_eq!(l.effects.items.len(), 1);
    assert!(matches!(&l.effects.items[0], photocraft_doc::Effect::Stroke(st) if st.size == 6.0));
    assert_eq!(l.blend, photocraft_color::BlendMode::Multiply);
    assert!((l.fill_opacity - 0.4).abs() < 1e-6);
    // Malformed lists are graceful errors.
    assert!(s.execute("style.presets.new", json!({"effects": [["nope", {}]]})).is_err());
    assert!(s.execute("style.presets.new", json!({"effects": ["stroke"]})).is_err());
}

// ------------------------------------------------------------------ gradients

#[test]
fn gradient_builtins_and_listing() {
    let mut s = crate::session();
    let v = s.execute("gradient.presets.list", json!({})).unwrap();
    let groups = v["groups"].as_array().unwrap();
    let names: Vec<&str> = groups.iter().map(|g| g["name"].as_str().unwrap()).collect();
    assert_eq!(names[0], "Basics");
    assert!(names.contains(&"Blues") && names.contains(&"Grays"));
    assert_eq!(v["current"]["name"], "Foreground to Background");
    assert_eq!(groups[0]["presets"][1]["transparency"][1], json!([1.0, 0.0]));
}

/// `gradient.presets.stop {action: "set"}` is the Gradient Editor's commit: it replaces every stop
/// of the tool gradient at once, keeps the live foreground/background names and the preset's name,
/// refuses a one-stop and a 1025-stop list, and leaves the document alone.
#[test]
fn the_tool_gradient_takes_a_whole_set_of_stops() {
    let mut s = session(8);
    let revision = s.active().unwrap().revision;
    let name = s.presets().gradient.name.clone();
    let v = s
        .execute(
            "gradient.presets.stop",
            json!({"action": "set", "stops": [[0.0, "foreground"], [0.25, "#ff0000"], [1.0, "background"]], "transparency": [[0.0, 100.0], [1.0, 20.0]]}),
        )
        .unwrap();
    assert_eq!(v["current"]["name"], json!(name));
    assert_eq!(v["current"]["stops"], json!([[0.0, "foreground"], [0.25, "#ff0000"], [1.0, "background"]]));
    assert_eq!(v["current"]["transparency"], json!([[0.0, 100.0], [1.0, 20.0]]));
    assert_eq!(s.active().unwrap().revision, revision, "a tool gradient edit is no document step");
    // A shorter list replaces; it does not merge with the old stops.
    let v = s.execute("gradient.presets.stop", json!({"action": "set", "stops": [[0.0, "#000000"], [1.0, "#ffffff"]], "transparency": []})).unwrap();
    assert_eq!(v["current"]["stops"].as_array().unwrap().len(), 2, "{v}");
    assert_eq!(v["current"]["transparency"], json!([]));
    // Refusals leave the gradient as it was.
    assert!(s.execute("gradient.presets.stop", json!({"action": "set", "stops": [[0.0, "#000000"]]})).is_err());
    let many: Vec<Value> = (0..1025).map(|i| json!([i as f64 / 1024.0, "#808080"])).collect();
    assert!(s.execute("gradient.presets.stop", json!({"action": "set", "stops": many})).is_err());
    let v = s.execute("gradient.presets.list", json!({})).unwrap();
    assert_eq!(v["current"]["stops"], json!([[0.0, "#000000"], [1.0, "#ffffff"]]));
}

#[test]
fn gradient_apply_creates_fill_layer_and_select_recolours_it() {
    let mut s = session(8);
    let r = s.execute("gradient.presets.apply", json!({"preset": "Blue 01", "angle": 0})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    let stops = |s: &Session| match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Fill(Fill::Gradient { stops, angle, .. }) => (stops.clone(), *angle),
        _ => panic!("not a gradient fill"),
    };
    let (st, angle) = stops(&s);
    assert_eq!(angle, 0.0);
    assert!((st[0].1.c[2] - 0x91 as f32 / 255.0).abs() < 1e-3);
    // Clicking another preset with the fill layer selected changes it (one undo step).
    let r = s.execute("gradient.presets.select", json!({"preset": "Green 01"})).unwrap();
    assert_eq!(r["layer"], json!(id.0));
    assert!(stops(&s).0[0].1.c[1] > 0.25);
    s.execute("edit.undo", json!({})).unwrap();
    assert!((stops(&s).0[0].1.c[2] - 0x91 as f32 / 255.0).abs() < 1e-3);
}

#[test]
fn gradient_new_rename_move_delete_groups() {
    let mut s = crate::session();
    let rev = s.prefs.rev();
    s.execute("gradient.presets.select", json!({"stops": [[0, "#112233"], [0.5, "foreground"], [1, "#ffffff"]], "transparency": [[0, 100], [1, 50]]})).unwrap();
    let r = s.execute("gradient.presets.new", json!({"name": "Mine", "group": "My Gradients"})).unwrap();
    assert_eq!(r["group"], "My Gradients");
    assert!(s.prefs.rev() > rev, "preset edits mark preferences dirty");
    let dup = s.execute("gradient.presets.new", json!({"name": "Mine"})).unwrap();
    assert_eq!(dup["name"], "Mine 2");
    s.execute("gradient.presets.edit", json!({"action": "rename", "preset": "Mine", "name": "Ours"})).unwrap();
    assert!(s.execute("gradient.presets.edit", json!({"action": "rename", "preset": "Ours", "name": "Mine 2"})).is_err());
    s.execute("gradient.presets.edit", json!({"action": "move", "preset": "Mine 2", "to": "My Gradients", "index": 0})).unwrap();
    let v = s.execute("gradient.presets.list", json!({})).unwrap();
    let mine = v["groups"].as_array().unwrap().iter().find(|g| g["name"] == "My Gradients").unwrap().clone();
    assert_eq!(mine["presets"][0]["name"], "Mine 2");
    assert_eq!(mine["presets"][1]["stops"][1], json!([0.5, "foreground"]));
    assert_eq!(mine["presets"][1]["transparency"][1], json!([1.0, 50.0]));
    s.execute("gradient.presets.edit", json!({"action": "delete", "preset": ["Ours", "Mine 2"]})).unwrap();
    s.execute("gradient.presets.edit", json!({"action": "deleteGroup", "group": "My Gradients"})).unwrap();
    s.execute("gradient.presets.edit", json!({"action": "deleteGroup", "group": "Blues"})).unwrap();
    s.execute("gradient.presets.reset", json!({})).unwrap();
    let v = s.execute("gradient.presets.list", json!({})).unwrap();
    assert!(v["groups"].as_array().unwrap().iter().any(|g| g["name"] == "Blues"));
    // Bad params.
    assert!(s.execute("gradient.presets.select", json!({"preset": "nope"})).is_err());
    assert!(s.execute("gradient.presets.select", json!({"stops": [[0, "#zzzzzz"], [1, "#000000"]]})).is_err());
    assert!(s.execute("gradient.presets.edit", json!({"action": "explode"})).is_err());
}

#[test]
fn gradient_rename_group_refuses_a_name_another_group_has() {
    let mut s = crate::session();
    s.execute("gradient.presets.edit", json!({"action": "newGroup", "name": "Pastels"})).unwrap();
    let names = |s: &mut Session| -> Vec<Value> {
        s.execute("gradient.presets.list", json!({})).unwrap()["groups"].as_array().unwrap().iter().map(|g| g["name"].clone()).collect()
    };
    let before = names(&mut s);
    assert!(before.contains(&json!("Blues")));
    let err = s.execute("gradient.presets.edit", json!({"action": "renameGroup", "group": "Pastels", "name": "Blues"})).unwrap_err();
    assert!(err.to_string().contains("already exists"), "{err}");
    assert_eq!(names(&mut s), before, "a refused rename leaves the groups unchanged");
    // Renaming a group to its own name is fine.
    s.execute("gradient.presets.edit", json!({"action": "renameGroup", "group": "Pastels", "name": "Pastels"})).unwrap();
    assert_eq!(names(&mut s), before);
}

#[test]
fn presets_persist_through_preferences_json() {
    let mut s = crate::session();
    s.execute("gradient.presets.new", json!({"name": "Saved", "stops": [[0, "#ff0000"], [1, "#0000ff"]]})).unwrap();
    s.execute("tool.presets.new", json!({"name": "Big Brush", "tool": "brush", "options": {"brush": {"size": 300.0}}})).unwrap();
    s.execute("shape.presets.new", json!({"name": "Tri", "path": "M 0 0 L 10 0 L 5 10 Z"})).unwrap();
    s.execute("style.presets.edit", json!({"action": "deleteGroup", "group": "Buttons"})).unwrap();
    let text = s.prefs_to_json();
    let mut t = crate::session();
    t.load_prefs_json(&text).unwrap();
    let names = |t: &mut Session, cmd: &str, key: &str| -> Vec<String> {
        let v = t.execute(cmd, json!({})).unwrap();
        v["groups"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g[key].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap().to_string()).collect::<Vec<_>>())
            .collect()
    };
    assert!(names(&mut t, "gradient.presets.list", "presets").contains(&"Saved".to_string()));
    assert!(names(&mut t, "shape.presets.list", "shapes").contains(&"Tri".to_string()));
    let styles = t.execute("style.presets.list", json!({})).unwrap();
    assert!(!styles["groups"].as_array().unwrap().iter().any(|g| g["name"] == "Buttons"));
    let tp = t.execute("tool.presets.list", json!({"tool": "Brush Tool"})).unwrap();
    assert!(tp["presets"].as_array().unwrap().iter().any(|p| p["name"] == "Big Brush"));
    // Older preference files without presets keep the defaults.
    let mut u = crate::session();
    u.load_prefs_json("{}").unwrap();
    assert_eq!(names(&mut u, "gradient.presets.list", "presets").len(), crate::session().presets().gradients.iter().map(|g| g.items.len()).sum::<usize>());
}

// ------------------------------------------------------------------ patterns

#[test]
fn pattern_groups_select_apply_and_sync() {
    let mut s = session(8);
    let v = s.execute("pattern.presets.list", json!({})).unwrap();
    let groups = v["groups"].as_array().unwrap();
    assert_eq!(groups[0]["name"], "Geometric");
    assert_eq!(groups[0]["patterns"].as_array().unwrap().len(), 4);
    s.execute("pattern.presets.select", json!({"pattern": "Bricks"})).unwrap();
    // Pattern fill layers and Fill default to the selected pattern.
    let r = s.execute("pattern.presets.apply", json!({})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    let name_of = |s: &Session| match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Fill(Fill::Pattern { name, .. }) => name.clone(),
        _ => panic!(),
    };
    assert_eq!(name_of(&s), "Bricks");
    let r = s.execute("pattern.presets.select", json!({"pattern": "Dots"})).unwrap();
    assert_eq!(r["layer"], json!(id.0));
    assert_eq!(name_of(&s), "Dots");
    // A newly defined pattern lands in the trailing group, and can be moved / grouped.
    let r = s.execute("pattern.presets.new", json!({"name": "Mine", "rect": [0, 0, 8, 8], "group": "Custom"})).unwrap();
    let pid = r["pattern"].as_str().unwrap().to_string();
    let v = s.execute("pattern.presets.list", json!({})).unwrap();
    let custom = v["groups"].as_array().unwrap().iter().find(|g| g["name"] == "Custom").unwrap().clone();
    assert_eq!(custom["patterns"][0]["id"], json!(pid));
    s.execute("pattern.presets.edit", json!({"action": "move", "preset": "Mine", "to": "Geometric"})).unwrap();
    s.execute("pattern.presets.edit", json!({"action": "rename", "preset": "Mine", "name": "Ours"})).unwrap();
    s.execute("pattern.presets.edit", json!({"action": "delete", "preset": "Ours"})).unwrap();
    let v = s.execute("pattern.presets.list", json!({})).unwrap();
    assert_eq!(v["groups"][0]["patterns"].as_array().unwrap().len(), 4);
    // Deleting a group deletes its patterns from the library.
    s.execute("pattern.presets.edit", json!({"action": "deleteGroup", "group": "Textures"})).unwrap();
    assert!(crate::pattern_cmds::resolve(&s, "Weave").is_none());
}

/// #16221: syncing groups already in line with the library leaves the presets' revision alone (the Patterns panel
/// syncs in every pass, and the menus rebuild on that revision); a library change still moves it.
#[test]
fn pattern_sync_writes_only_when_the_groups_change() {
    let mut s = session(8);
    super::patterns::sync(&mut s);
    let settled = s.presets().revision();
    super::patterns::sync(&mut s);
    assert_eq!(s.presets().revision(), settled, "groups in line with the library are not written");
    let mut extra = s.patterns.items[0].clone();
    extra.id = "sync-test-extra".into();
    s.patterns.items.push(extra);
    super::patterns::sync(&mut s);
    assert_ne!(s.presets().revision(), settled, "a new library pattern joins a group");
    assert!(s.presets().pattern_groups.iter().any(|g| g.items.iter().any(|id| id == "sync-test-extra")));
    let joined = s.presets().revision();
    s.patterns.items.retain(|p| p.id != "sync-test-extra");
    super::patterns::sync(&mut s);
    assert_ne!(s.presets().revision(), joined, "a deleted pattern leaves its group");
    assert!(!s.presets().pattern_groups.iter().any(|g| g.items.iter().any(|id| id == "sync-test-extra")));
}

#[test]
fn pattern_batch_delete_keeps_the_library_when_one_name_is_missing() {
    let mut s = session(8);
    let before = s.execute("pattern.presets.list", json!({})).unwrap();
    let err = s.execute("pattern.presets.edit", json!({"action": "delete", "preset": ["Bricks", "missing"]})).unwrap_err();
    assert!(err.to_string().contains("missing"), "{err}");
    assert_eq!(s.execute("pattern.presets.list", json!({})).unwrap(), before);
    assert!(crate::pattern_cmds::resolve(&s, "Bricks").is_some());
}

#[test]
fn pattern_batch_delete_rejects_non_string_members_without_deleting() {
    for invalid in [json!(null), json!(7), json!({})] {
        let mut s = session(8);
        let before = s.execute("pattern.presets.list", json!({})).unwrap();
        assert!(s.execute("pattern.presets.edit", json!({"action": "delete", "preset": ["Bricks", invalid]})).is_err());
        assert_eq!(s.execute("pattern.presets.list", json!({})).unwrap(), before);
    }
}

#[test]
fn pattern_batch_delete_refuses_a_document_only_pattern_up_front() {
    let mut s = session(8);
    let added = s.execute("edit.definePattern", json!({"name": "Document Only"})).unwrap();
    let id = added["pattern"].as_str().unwrap().to_string();
    let pattern = s.patterns.items.iter().find(|p| p.id == id).unwrap().clone();
    s.edit("Embed pattern fixture", |doc, _| {
        doc.patterns.push(pattern.clone());
        Ok(())
    })
    .unwrap();
    s.execute("pattern.delete", json!({"pattern": id})).unwrap();
    let before = s.patterns.items.clone();
    assert!(s.execute("pattern.presets.edit", json!({"action": "delete", "preset": ["Bricks", id]})).is_err());
    assert_eq!(s.patterns.items, before, "nothing is deleted when one entry is not in the library");
}

// ------------------------------------------------------------------ styles

#[test]
fn styles_apply_to_selected_layers_and_new_from_layer() {
    let mut s = session(16);
    let a = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().map(LayerId).unwrap_or_else(|| active(&s));
    let b = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().map(LayerId).unwrap_or_else(|| active(&s));
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    let r = s.execute("style.presets.apply", json!({"preset": "Gold"})).unwrap();
    assert_eq!(r["layers"].as_array().unwrap().len(), 2);
    let fx = |s: &Session, id: LayerId| s.active().unwrap().doc.layer(id).unwrap().effects.items.len();
    assert_eq!((fx(&s, a), fx(&s, b)), (3, 3));
    // ⇧-click adds; plain apply replaces; Default Style clears.
    s.execute("style.presets.apply", json!({"preset": "Black Stroke", "add": true, "layer": a.0})).unwrap();
    assert_eq!(fx(&s, a), 4);
    s.execute("style.presets.apply", json!({"preset": "Hollow", "layer": b.0})).unwrap();
    let lb = s.active().unwrap().doc.layer(b).unwrap().clone();
    assert_eq!((lb.effects.items.len(), lb.fill_opacity), (1, 0.0));
    s.execute("style.presets.apply", json!({"preset": "Default Style (None)", "layer": b.0})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer(b).unwrap().fill_opacity, 1.0);
    // One undo step per apply.
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(fx(&s, b), 1);
    // New Style from the layer, applied elsewhere.
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("style.presets.new", json!({"name": "From A", "group": "Mine"})).unwrap();
    s.execute("style.presets.apply", json!({"preset": "From A", "layer": b.0})).unwrap();
    assert_eq!(fx(&s, b), 4);
    assert!(s.execute("style.presets.apply", json!({"preset": "nope"})).is_err());
    // Thumbnails render the effect around the swatch.
    let st = s.presets().styles[0].items.iter().find(|x| x.name == "Outer Glow").unwrap().clone();
    let px = super::styles::thumbnail(&s, &st, 48);
    assert_eq!(px.len(), 48 * 48 * 4);
    assert!(px[(4 * 48 + 24) * 4 + 3] == 0 || px[(7 * 48 + 24) * 4 + 3] > 0, "glow reaches outside the square");
    assert!(px[(24 * 48 + 24) * 4 + 3] == 255);
}

/// A preset saved without blending options but with an explicit Opacity carries only that
/// opacity: ⇧-click (`add`) sets it and leaves the layer's blend mode and Fill alone.
#[test]
fn an_opacity_only_style_added_sets_just_the_opacity() {
    let mut s = session(8);
    let a = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().map(LayerId).unwrap_or_else(|| active(&s));
    s.execute("style.presets.new", json!({"name": "Faded", "includeBlending": false, "effects": [["stroke", {"size": 2}]], "opacity": 40})).unwrap();
    s.edit("Blending fixture", |doc, _| {
        let l = doc.layer_mut(a).unwrap();
        (l.blend, l.fill_opacity, l.opacity) = (photocraft_doc::BlendMode::Multiply, 0.7, 0.9);
        Ok(())
    })
    .unwrap();
    s.execute("style.presets.apply", json!({"preset": "Faded", "add": true, "layer": a.0})).unwrap();
    let l = s.active().unwrap().doc.layer(a).unwrap().clone();
    assert_eq!((l.blend, l.fill_opacity, l.opacity), (photocraft_doc::BlendMode::Multiply, 0.7, 0.4));
    assert_eq!(l.effects.items.len(), 1);
}

#[test]
fn styles_with_pattern_overlay_copy_the_pattern() {
    let mut s = session(8);
    s.execute("layer.new.layer", json!({})).unwrap();
    let id = active(&s);
    s.presets_mut().styles[0].items.push(super::styles::StylePreset {
        name: "Bricky".into(),
        effects: vec![crate::layer_style::effect_from_params("patternOverlay", &json!({"pattern": "Bricks"})).unwrap()],
        blend: None,
        fill_opacity: None,
        opacity: None,
    });
    s.execute("style.presets.apply", json!({"preset": "Bricky", "layer": id.0})).unwrap();
    assert!(s.active().unwrap().doc.patterns.iter().any(|p| p.name == "Bricks"));
}

// ------------------------------------------------------------------ shapes

#[test]
fn shape_path_language_parses() {
    let p = super::shapes::parse("M 0 0 L 10 0 L 10 10 Z ! O 5 5 2").unwrap();
    assert_eq!(p.subpaths.len(), 2);
    assert_eq!(p.subpaths[0].knots.len(), 3);
    assert_eq!(p.subpaths[1].op, photocraft_doc::vector::PathOp::Subtract);
    assert!(super::shapes::parse("M 0 0 L x").is_err());
    assert!(super::shapes::parse("").is_err());
    assert!(super::shapes::parse("L 1 1").is_err());
    // Every built-in parsed and has an area.
    for g in super::shapes::builtin() {
        assert!(!g.items.is_empty(), "{}", g.name);
        for sh in g.items {
            let cov = super::shapes::thumbnail(&sh.path, 32);
            assert!(cov.iter().sum::<f32>() > 20.0, "{} is empty", sh.name);
        }
    }
    let n: usize = super::shapes::builtin().iter().map(|g| g.items.len()).sum();
    assert!(n >= 30, "{n}");
}

#[test]
fn shapes_place_as_shape_layers() {
    let mut s = session(8);
    let r = s.execute("shape.presets.place", json!({"preset": "Heart", "rect": [10, 10, 40, 20], "fill": "#ff0000"})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    let l = s.active().unwrap().doc.layer(id).unwrap().clone();
    assert_eq!(l.name, "Heart 1");
    let LayerContent::Shape(sh) = &l.content else { panic!("not a shape layer") };
    let (x0, y0, x1, y1) = sh.path.control_bounds().unwrap();
    // Keep aspect: 20 px tall, centred horizontally in the 40 px rect.
    assert!((y0 - 10.0).abs() < 0.01 && (y1 - 30.0).abs() < 0.01, "{y0} {y1}");
    // The heart's box is 100 × 92: 20 px tall → 21.7 px wide, centred on x = 30.
    assert!(((x1 - x0) - 20.0 * 100.0 / 92.0).abs() < 0.05 && ((x0 + x1) / 2.0 - 30.0).abs() < 0.01, "{x0} {x1}");
    // Free (dragged without ⇧) fills the rect; negative drags flip.
    let r = s.execute("shape.presets.place", json!({"preset": "Arrow Right", "rect": [50, 40, -40, -20], "keepAspect": false})).unwrap();
    let l = s.active().unwrap().doc.layer(LayerId(r["layer"].as_u64().unwrap())).unwrap().clone();
    let LayerContent::Shape(sh) = &l.content else { panic!() };
    assert_eq!(sh.path.control_bounds().unwrap(), (10.0, 20.0, 50.0, 40.0));
    // Default placement: centred, half the short side.
    s.execute("shape.presets.place", json!({"preset": "Ring"})).unwrap();
    assert!(s.execute("shape.presets.place", json!({"preset": "Unicorn"})).is_err());
    assert!(s.execute("shape.presets.place", json!({"preset": "Heart", "rect": [0, 0, 0, 5]})).is_err());
    // Undo removes the layer.
    let n = s.active().unwrap().doc.layers.len();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layers.len(), n - 1);
}

#[test]
fn custom_shapes_join_the_shapes_panel() {
    let mut s = session(8);
    s.execute("shape.create", json!({"kind": "star", "rect": [0, 0, 30, 30]})).unwrap();
    s.execute("edit.defineCustomShape", json!({"name": "My Star"})).unwrap();
    let v = s.execute("shape.presets.list", json!({})).unwrap();
    let last = v["groups"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["name"], super::shapes::CUSTOM_GROUP);
    s.execute("shape.presets.place", json!({"preset": "My Star"})).unwrap();
    s.execute("shape.presets.edit", json!({"action": "rename", "preset": "My Star", "name": "Star 2"})).unwrap();
    s.execute("shape.presets.edit", json!({"action": "delete", "preset": "Star 2"})).unwrap();
    assert!(s.edit_state.custom_shapes.is_empty());
    // New preset from the active shape layer, normalised to the 100 box.
    s.execute("shape.presets.new", json!({"name": "From Layer", "group": "Mine"})).unwrap();
    let g = s.presets().shapes.iter().find(|g| g.name == "Mine").unwrap();
    let b = g.items[0].path.control_bounds().unwrap();
    assert!(b.2.max(b.3) > 99.9);
}

#[test]
fn tool_presets_reject_invalid_colors_before_applying_the_brush() {
    let mut invalid = vec![
        json!(null),
        json!(true),
        json!(7),
        json!("red"),
        json!({}),
        json!([]),
        json!([1]),
        json!([1, 0]),
        json!([1, 0, 0, 1, 0]),
        json!([null, 0.4, 0.6]),
    ];
    for channel in 0..4 {
        for bad_channel in [json!(1e100), json!(-1e100), json!(null), json!("red"), json!(true), json!({}), json!([])] {
            let mut values = json!([0.2, 0.4, 0.6, 0.8]);
            values[channel] = bad_channel;
            invalid.push(values);
        }
    }
    for values in invalid {
        let mut s = crate::session();
        s.execute("tools.setColors", json!({"foreground": [0.2, 0.4, 0.6, 0.8]})).unwrap();
        let color = s.tools.foreground;
        let brush = (*s.tools.brush).clone();
        let brush_rev = s.tools.brush_rev;
        s.execute(
            "tool.presets.new",
            json!({
                "name": "Invalid", "tool": "brush", "options": {"brush": {"size": 77}, "foreground": values}
            }),
        )
        .unwrap();
        assert!(s.execute("tool.presets.select", json!({"preset": "Invalid"})).is_err(), "{values}");
        assert_eq!(s.tools.foreground, color, "{values}");
        assert_eq!(*s.tools.brush, brush, "{values}");
        assert_eq!(s.tools.brush_rev, brush_rev, "{values}");
    }
}

#[test]
fn tool_presets_keep_finite_hdr_colors_and_leave_absent_colors_unchanged() {
    let mut s = crate::session();
    for (options, expected) in [
        (json!({"foreground": [1.5, -0.1, 0.6, 0.8]}), [1.5, -0.1, 0.6, 0.8]),
        (json!({"foreground": [0.2, 0.4, 0.6]}), [0.2, 0.4, 0.6, 1.0]),
        (json!({}), [0.2, 0.4, 0.6, 1.0]),
    ] {
        let preset = s.execute("tool.presets.new", json!({"tool": "move", "options": options})).unwrap();
        s.execute("tool.presets.select", json!({"preset": preset["name"]})).unwrap();
        assert_eq!(s.tools.foreground, expected);
    }
}

/// Crop presets are `crop` tool presets: saved, listed, persisted and deleted like the others.
#[test]
fn crop_presets_are_tool_presets_that_persist() {
    let mut s = crate::session();
    let opts =
        json!({"toolOptions": {"crop_ratio": "whr", "crop_width": "4 in", "crop_height": "5 in", "crop_resolution": "300", "crop_resolution_unit": "px/in"}});
    s.execute("tool.presets.new", json!({"name": "4 x 5 in 300 ppi", "tool": "crop", "options": opts})).unwrap();
    let r = s.execute("tool.presets.new", json!({"name": "4 x 5 in 300 ppi", "tool": "crop", "options": opts})).unwrap();
    assert_eq!(r["name"], "4 x 5 in 300 ppi 2", "a taken name gets a number");
    let mut t = crate::session();
    t.load_prefs_json(&s.prefs_to_json()).unwrap();
    let v = t.execute("tool.presets.list", json!({"tool": "crop"})).unwrap();
    assert_eq!(v["presets"].as_array().unwrap().len(), 2);
    let r = t.execute("tool.presets.select", json!({"preset": "4 x 5 in 300 ppi"})).unwrap();
    assert_eq!(r["options"]["toolOptions"]["crop_width"], "4 in");
    // Deleting several is all or nothing.
    assert!(t.execute("tool.presets.edit", json!({"action": "delete", "preset": ["4 x 5 in 300 ppi", "missing"]})).is_err());
    assert_eq!(t.presets().tool_presets.iter().filter(|p| p.tool == "crop").count(), 2);
    let r = t.execute("tool.presets.edit", json!({"action": "delete", "preset": ["4 x 5 in 300 ppi 2", "4 x 5 in 300 ppi", "4 x 5 in 300 ppi"]})).unwrap();
    assert_eq!(r["deleted"], 2);
    assert!(t.presets().tool_presets.iter().all(|p| p.tool != "crop"));
}

#[test]
fn tool_preset_names_and_params_are_checked() {
    let mut s = crate::session();
    let n = s.presets().tool_presets.len();
    let long = "x".repeat(crate::brush_preset_cmds::MAX_NAME + 1);
    for bad in [
        json!({"name": long, "tool": "crop"}),
        json!({"name": 7, "tool": "crop"}),
        json!({"name": ["a"], "tool": "crop"}),
        json!({"tool": 3}),
        json!({"tool": "crop", "options": "big"}),
        json!({"tool": "crop", "options": [1]}),
        json!(null),
        json!([]),
    ] {
        assert!(s.execute("tool.presets.new", bad.clone()).is_err(), "{bad}");
    }
    assert_eq!(s.presets().tool_presets.len(), n);
    // An empty or blank name falls back to the tool's; the longest name fits.
    assert_eq!(s.execute("tool.presets.new", json!({"name": "   ", "tool": "crop"})).unwrap()["name"], "crop");
    let max = "é".repeat(crate::brush_preset_cmds::MAX_NAME);
    assert_eq!(s.execute("tool.presets.new", json!({"name": max, "tool": "crop"})).unwrap()["name"], json!(max));
    assert!(s.execute("tool.presets.edit", json!({"action": "rename", "preset": "crop", "name": long})).is_err());
    assert!(s.execute("tool.presets.edit", json!({"action": "rename", "preset": "crop", "name": "  "})).is_err());
    // The store is capped.
    while s.presets().tool_presets.len() < super::tools::MAX_PRESETS {
        let i = s.presets().tool_presets.len();
        s.presets_mut().tool_presets.push(super::tools::ToolPreset { name: format!("p{i}"), tool: "crop".into(), options: json!({}) });
    }
    assert!(s.execute("tool.presets.new", json!({"name": "one more", "tool": "crop"})).is_err());
}

/// A damaged entry in the saved tool presets is skipped; the other presets (and preferences) load.
#[test]
fn damaged_tool_presets_skip_only_the_bad_entries() {
    let mut s = crate::session();
    s.execute("gradient.presets.new", json!({"name": "Kept", "stops": [[0, "#ff0000"], [1, "#0000ff"]]})).unwrap();
    let mut prefs: Value = serde_json::from_str(&s.prefs_to_json()).unwrap();
    prefs["presets"]["tool_presets"] = json!([
        {"name": "Good", "tool": "crop", "options": {"toolOptions": {"crop_ratio": "2:3"}}},
        {"name": 5, "tool": "crop"},
        {"tool": "crop"},
        {"name": "   ", "tool": "crop"},
        {"name": "x".repeat(300), "tool": "crop"},
        {"name": "No tool", "tool": ""},
        "junk",
        null,
        {"name": "Good", "tool": "brush"},
        {"name": "Odd options", "tool": "crop", "options": 12},
    ]);
    let mut t = crate::session();
    t.load_prefs_json(&prefs.to_string()).unwrap();
    let names: Vec<&str> = t.presets().tool_presets.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Good", "Odd options"]);
    assert!(t.presets().tool_presets[1].options.is_object());
    assert!(t.presets().gradients.iter().any(|g| g.items.iter().any(|p| p.name == "Kept")), "other presets still load");
    // Not an array at all: no tool presets, still no crash.
    prefs["presets"]["tool_presets"] = json!({"a": 1});
    let mut u = crate::session();
    u.load_prefs_json(&prefs.to_string()).unwrap();
    assert!(u.presets().tool_presets.is_empty());
}

// ------------------------------------------------------------------ clone source

fn clone_doc(depth: u32) -> Session {
    let mut s = session(depth);
    // A horizontal ramp in red, green = row, so transforms are measurable.
    s.edit("ramp", |doc, _| {
        let surf = doc.layers[0].surface_mut().unwrap();
        let fmt = surf.format();
        let mut px = Vec::new();
        for y in 0..48 {
            for x in 0..64 {
                px.extend(photocraft_raster::from_rgba(&fmt, [x as f32 / 63.0, y as f32 / 47.0, 0.0, 1.0]));
            }
        }
        surf.write_region(photocraft_geom::Rect::new(0, 0, 64, 48), &px);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn clone_source_errors_and_overlay() {
    let mut s = clone_doc(8);
    assert!(s.execute("paint.cloneStamp", json!({"points": [[4, 4]]})).is_err());
    assert!(s.execute("cloneSource.select", json!({"index": 5})).is_err());
    assert!(s.execute("cloneSource.set", json!({"offset": [1, 1]})).is_err());
    let o = s.execute("cloneSource.overlay", json!({"opacity": 40, "clipped": false, "blend": "difference"})).unwrap();
    assert_eq!(o["opacity"], json!(40.0));
    assert!(s.execute("cloneSource.overlay", json!({"blend": "screen"})).is_err());
    let v: Value = s.execute("cloneSource.set", json!({"source": [1, 1], "width": 5000, "rotation": 270})).unwrap();
    assert_eq!(v["width"], json!(1000.0));
    assert_eq!(v["rotation"], json!(-90.0));
}

fn shape_fill(s: &Session, id: LayerId) -> Option<Fill> {
    match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => sh.fill.clone(),
        _ => panic!("not a shape layer"),
    }
}

/// #5568: clicking a gradient with a shape layer selected fills the shape with it in one undo
/// step; a gradient fill keeps its angle, style, scale and reverse.
#[test]
fn gradient_preset_click_fills_the_selected_shape_layer() {
    let mut s = session(8);
    let id = LayerId(s.execute("shape.create", json!({"rect": [8, 8, 40, 30], "fill": "#336699"})).unwrap()["layer"].as_u64().unwrap());
    let before = shape_fill(&s, id);
    let r = s.execute("gradient.presets.select", json!({"preset": "Red 01"})).unwrap();
    assert_eq!(r["layer"], json!(id.0));
    let Some(Fill::Gradient { stops, angle, style, .. }) = shape_fill(&s, id) else { panic!("not a gradient") };
    // Red 01 starts at #7f0000; a new gradient fill is linear at 90°.
    assert_eq!(stops[0].1.to_rgba8(), [0x7f, 0, 0, 0xff]);
    assert_eq!((angle, style), (90.0, photocraft_doc::GradientStyle::Linear));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(shape_fill(&s, id), before);

    s.execute("shape.edit", json!({"layer": id.0, "fill": {"gradient": {"angle": 30, "style": "radial", "scale": 80, "reverse": true}}})).unwrap();
    s.execute("gradient.presets.select", json!({"preset": "Green 01"})).unwrap();
    let Some(Fill::Gradient { angle, style, scale, reverse, .. }) = shape_fill(&s, id) else { panic!("not a gradient") };
    assert_eq!((angle, style, scale, reverse), (30.0, photocraft_doc::GradientStyle::Radial, 0.8, true));
}

/// A locked shape refuses the click before anything changes, the Gradient tool's gradient included.
#[test]
fn gradient_preset_click_on_a_locked_shape_changes_nothing() {
    let mut s = session(8);
    let id = LayerId(s.execute("shape.create", json!({"rect": [8, 8, 40, 30], "fill": "#336699"})).unwrap()["layer"].as_u64().unwrap());
    let (before, gradient) = (shape_fill(&s, id), s.presets().gradient.name.clone());
    s.execute("layer.lockLayers", json!({"all": true})).unwrap();
    let err = s.execute("gradient.presets.select", json!({"preset": "Red 01"})).unwrap_err().to_string();
    assert!(err.contains("locked"), "{err}");
    assert_eq!(shape_fill(&s, id), before);
    assert_eq!(s.presets().gradient.name, gradient);
}

// ------------------------------------------------------------------ persistence

/// #5602: one damaged preset set in the preferences file must not discard the saved New
/// Document presets (or any other set) at the next launch.
#[test]
fn a_damaged_preset_set_keeps_the_other_sets() {
    let mut s = crate::session();
    let settings = json!({"width": 1600, "height": 900, "resolution": 144});
    s.execute("document.presets.save", json!({"name": "Banner", "settings": settings})).unwrap();
    let export = json!({
        "format": "jpg", "quality": 70, "lossless": false, "transparency": false, "scale": 50, "width": 640, "height": 480,
        "lockAspect": true, "sizing": "pixels", "metadata": "exif", "profile": "srgb", "embedProfile": false, "tgaBits": "24"
    });
    s.execute("file.export.presets.save", json!({"name": "Web JPEG", "settings": export})).unwrap();
    let saved = s.presets().to_json(&s);
    for key in
        ["gradients", "gradient", "styles", "shapes", "pattern_groups", "recent_documents", "custom_shapes", "layer_defaults", "swatches", "export_presets"]
    {
        let mut blob = saved.clone();
        blob[key] = json!("not a preset set");
        let mut next = crate::session();
        next.load_presets_json(blob);
        assert_eq!(next.presets().documents, s.presets().documents, "damaged `{key}`");
        assert_eq!(next.presets().tool_presets.len(), crate::session().presets().tool_presets.len(), "damaged `{key}`");
        if key != "export_presets" {
            assert_eq!(next.presets().export_presets, s.presets().export_presets, "damaged `{key}`");
        }
    }
    let mut blob = saved.clone();
    blob["export_presets"] = json!("not a preset set");
    let mut next = crate::session();
    next.load_presets_json(blob);
    assert!(next.presets().export_presets.is_empty());
    assert_eq!(next.presets().documents, s.presets().documents);
    let mut blob = saved.clone();
    blob["documents"] = json!("not a preset set");
    let mut next = crate::session();
    next.load_presets_json(blob);
    assert!(next.presets().documents.is_empty());
    assert_eq!(next.presets().export_presets, s.presets().export_presets);
    // Null sets keep their defaults, as before.
    let mut next = crate::session();
    next.load_presets_json(
        json!({"tool_presets": null, "gradients": null, "documents": saved["documents"].clone(), "export_presets": saved["export_presets"].clone()}),
    );
    assert_eq!(next.presets().tool_presets.len(), crate::session().presets().tool_presets.len());
    assert_eq!(next.presets().gradients.len(), crate::session().presets().gradients.len());
    assert_eq!(next.presets().documents, s.presets().documents);
    assert_eq!(next.presets().export_presets, s.presets().export_presets);
}

#[test]
fn restoring_duplicate_gradient_names_preserves_addressable_presets() {
    use crate::presets::Group;
    use crate::presets::gradients::GradientPreset;

    let mut s = crate::session();
    let presets = vec![
        Group::new("First", vec![GradientPreset::new("Foo", &["#ff0000", "#000000"]), GradientPreset::new("Foo", &["#00ff00", "#000000"])]),
        Group::new("Second", vec![GradientPreset::new("Foo 2", &["#0000ff", "#000000"]), GradientPreset::new("FOO", &["#ffff00", "#000000"])]),
    ];
    s.presets_mut().gradient = presets[0].items[1].clone();
    s.presets_mut().gradients = presets;
    let saved = s.presets().to_json(&s);
    let mut restored = crate::session();
    restored.load_presets_json(saved.clone());
    let names: Vec<String> = restored.presets().gradients.iter().flat_map(|g| g.items.iter().map(|p| p.name.clone())).collect();
    assert_eq!(names, ["Foo", "Foo 3", "Foo 2", "FOO"]);
    assert_eq!(restored.presets().gradient.name, "Foo 3");
    assert_eq!(restored.presets().gradient.stops, GradientPreset::new("", &["#00ff00", "#000000"]).stops);
    for (name, color) in [("Foo 2", "#0000ff"), ("FOO", "#ffff00")] {
        restored.execute("gradient.presets.select", json!({"preset": name})).unwrap();
        assert_eq!(restored.presets().gradient.stops, GradientPreset::new("", &[color, "#000000"]).stops);
    }
    let current = restored.presets().gradient.clone();
    let once = restored.presets().to_json(&restored);
    restored.load_presets_json(once);
    assert_eq!(restored.presets().gradient, current);
    let names: Vec<String> = restored.presets().gradients.iter().flat_map(|g| g.items.iter().map(|p| p.name.clone())).collect();
    assert_eq!(names, ["Foo", "Foo 3", "Foo 2", "FOO"]);
}

// ------------------------------------------------------------------ export presets

#[test]
fn export_presets_survive_a_presets_json_round_trip() {
    let mut s = crate::session();
    s.execute("file.export.presets.save", json!({"name": "Web", "settings": {"format": "jpg", "quality": 70}})).unwrap();
    assert_eq!(s.presets().export_presets.len(), 1);

    let saved = s.presets().to_json(&s);
    assert_eq!(saved["export_presets"].as_array().map(Vec::len), Some(1));

    let mut restored = crate::session();
    restored.load_presets_json(saved);
    assert_eq!(restored.presets().export_presets.len(), 1);
    assert_eq!(restored.presets().export_presets[0].name, "Web");
    assert_eq!(restored.presets().export_presets[0].settings.format, "jpg");
}

/// A nested command a preset command runs answers to the session's authorization hook: a denied one
/// does not run, an ungated call still succeeds.
#[test]
fn nested_preset_commands_answer_to_the_authorization_gate() {
    fn deny_select_all(id: &str, _: &Value) -> crate::Result<()> {
        if id == "select.all" { Err(crate::EngineError::Other(format!("automation command `{id}` is disabled"))) } else { Ok(()) }
    }
    let mut s = session(8);
    s.authorize = Some(deny_select_all);
    let e = super::call(&mut s, "select.all", json!({})).unwrap_err().to_string();
    assert!(e.contains("is disabled"), "{e}");
    assert!(s.active().unwrap().doc.selection.is_none(), "the denied command did not run");
    s.authorize = None;
    super::call(&mut s, "select.all", json!({})).unwrap();
    assert!(s.active().unwrap().doc.selection.is_some());
}
