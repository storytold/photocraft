use super::*;
use craft::behavior::{EditEvent, Mixed};
use craft::keymap::Mods;
use craft::ui::contract::{MemoKey, ReorderMove};
use std::sync::Arc;

fn group(name: &str) -> PresetKey {
    PresetKey::Group(name.into())
}

fn item(key: &str) -> PresetKey {
    PresetKey::Item(key.into())
}

fn reorder(key: PresetKey, to: usize) -> PresetEvent<PresetKey> {
    let mv = ReorderMove::new(key.clone(), vec![key], to, false);
    PresetEvent::Reorder(EditEvent::set(Mixed::Same(mv.clone()), mv, Arc::from("Reorder"), None))
}

fn run(command: &str, params: Value) -> Option<Act> {
    Some(Act::Run(command.into(), params))
}

/// Every browser event of the shared mapping, for each engine prefix that takes `{prefix}.edit` and `{prefix}.new`:
/// the command and params it runs, or the folder move it makes.
#[test]
fn each_preset_event_runs_its_edit_command_for_every_prefix() {
    // Folder A holds a1 and a2, folder B holds b1.
    let rows = [group("A"), item("a1"), item("a2"), group("B"), item("b1")];
    for prefix in ["gradient.presets", "pattern.presets", "style.presets", "shape.presets", "tool.presets"] {
        let panel = PresetPanel { id: "panel", label: "Panel", prefix, new_params: || json!({"name": "New"}), rename_new: true };
        let edit = format!("{prefix}.edit");
        let cases: Vec<(PresetEvent<PresetKey>, Option<Act>)> = vec![
            (PresetEvent::rename(item("a1"), "b"), run(&edit, json!({"action": "rename", "preset": "a1", "name": "b"}))),
            (PresetEvent::rename(group("A"), "C"), run(&edit, json!({"action": "renameGroup", "group": "A", "name": "C"}))),
            (PresetEvent::Delete(item("a1")), run(&edit, json!({"action": "delete", "preset": "a1"}))),
            (PresetEvent::Delete(group("A")), run(&edit, json!({"action": "deleteGroup", "group": "A"}))),
            (PresetEvent::move_to(item("a1"), group("B")), run(&edit, json!({"action": "move", "preset": "a1", "to": "B"}))),
            (PresetEvent::Footer(PresetFooterEvent::NewGroup), run(&edit, json!({"action": "newGroup", "name": "Group"}))),
            (PresetEvent::Footer(PresetFooterEvent::New), run(&format!("{prefix}.new"), json!({"name": "New"}))),
            (PresetEvent::Footer(PresetFooterEvent::Delete), run(&edit, json!({"action": "delete", "preset": "a2"}))),
            // a1 lands after b1: the end of B.
            (reorder(item("a1"), 4), run(&edit, json!({"action": "move", "preset": "a1", "to": "B", "index": 1}))),
            // a2 lands first in A.
            (reorder(item("a2"), 1), run(&edit, json!({"action": "move", "preset": "a2", "to": "A", "index": 0}))),
            // B goes before A; A after B is the last place.
            (reorder(group("B"), 0), Some(Act::MoveFolder { from: "B".into(), before: Some("A".into()) })),
            (reorder(group("A"), 2), Some(Act::MoveFolder { from: "A".into(), before: None })),
            (PresetEvent::Action(0), None),
        ];
        for (event, expected) in cases {
            assert_eq!(act(panel, &event, &rows, Some("a2")), expected, "{prefix}: {event:?}");
        }
    }
}

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.sync_views();
    app
}

/// The events that use a preset: Patterns select on a click and fill on a double click or a drop; Styles apply,
/// Shift adding the effects, and a drop styles the active layer; Gradients select and apply; Shapes pick, place and
/// place at a drop; Tool Presets apply.
#[test]
fn each_panel_uses_a_preset_its_own_way() {
    let mut app = app();
    crate::canvas::testing::frame(&mut app);
    let shift = Mods { shift: true, ..Mods::default() };
    let none = Mods::default();
    let at = pos2(10.0, 20.0);
    let cases: Vec<(PresetPanel, PresetEvent<PresetKey>, Option<Act>)> = vec![
        (PATTERNS, PresetEvent::select(item("p"), none), run("pattern.presets.select", json!({"pattern": "p"}))),
        (PATTERNS, PresetEvent::activate(item("p"), none), run("pattern.presets.apply", json!({"pattern": "p"}))),
        (PATTERNS, PresetEvent::drop_at(item("p"), at, none), run("pattern.presets.apply", json!({"pattern": "p"}))),
        (STYLES, PresetEvent::select(item("s"), none), run("style.presets.apply", json!({"preset": "s", "add": false}))),
        (STYLES, PresetEvent::select(item("s"), shift), run("style.presets.apply", json!({"preset": "s", "add": true}))),
        (STYLES, PresetEvent::drop_at(item("s"), at, shift), run("style.presets.apply", json!({"preset": "s"}))),
        (STYLES, PresetEvent::select(group("G"), none), None),
        (GRADIENTS, PresetEvent::select(item("g"), none), run("gradient.presets.select", json!({"preset": "g"}))),
        (GRADIENTS, PresetEvent::activate(item("g"), none), run("gradient.presets.apply", json!({"preset": "g"}))),
        (GRADIENTS, PresetEvent::drop_at(item("g"), at, none), run("gradient.presets.apply", json!({"preset": "g"}))),
        (SHAPES, PresetEvent::select(item("Star"), none), Some(Act::CustomShapeTool)),
        (SHAPES, PresetEvent::activate(item("Star"), none), run("shape.presets.place", json!({"preset": "Star", "fill": shape_fill(&app)}))),
        (TOOL_PRESETS, PresetEvent::select(item("T"), none), Some(Act::ToolPreset("T".into()))),
        (TOOL_PRESETS, PresetEvent::activate(item("T"), none), Some(Act::ToolPreset("T".into()))),
        (TOOL_PRESETS, PresetEvent::drop_at(item("T"), at, none), None),
    ];
    for (panel, event, expected) in cases {
        assert_eq!(panel_act(&app, panel, &event), expected, "{}: {event:?}", panel.id);
    }
    // A shape dropped on the canvas lands in a square centred on the drop, of 30 % of the shorter side.
    let drop = canvas_drop_rect(&app).center();
    let c = crate::canvas::document_display::doc_point_at(&app, drop).expect("the drop is on the canvas");
    let side = (48.0_f64 * 0.3).max(8.0);
    let expected = json!({"preset": "Star", "fill": shape_fill(&app), "rect": [c.x - side / 2.0, c.y - side / 2.0, side, side]});
    assert_eq!(panel_act(&app, SHAPES, &PresetEvent::drop_at(item("Star"), drop, none)), run("shape.presets.place", expected));
}

/// Regression: preset drops mapped the pointer through the raw widget rect, ignoring the rulers' inset — a dropped
/// shape landed off the pointer, and a drop on the ruler strip counted as a canvas drop. Drops map through, and
/// target, the same content rect the canvas draws with.
#[test]
fn preset_drops_map_through_the_ruler_inset() {
    let mut app = app();
    app.ui.extras.rulers = true;
    crate::canvas::testing::frame(&mut app);
    let rect = canvas_drop_rect(&app);
    let centre = rect.center();
    assert!(rect.contains(centre));
    assert!(!rect.contains(pos2(rect.left() - 4.0, centre.y)), "the ruler strip is no drop target");
    let doc = crate::canvas::document_display::doc_point_at(&app, centre).expect("the canvas centre is on the document");
    assert!((doc.x - 32.0).abs() < 1.0 && (doc.y - 24.0).abs() < 1.0, "the canvas centre is the document centre: {doc:?}");
}

/// Runs one egui pass of `ctx` with `f`.
fn pass(ctx: &egui::Context, f: impl FnMut(&mut egui::Ui)) {
    let mut out = ctx.run_ui(egui::RawInput::default(), f);
    out.textures_delta.clear();
}

/// #16221: a swatch kept past a texture-store eviction (Fill holds its pattern swatches from when it opened) paints
/// a live texture: it asks the store again and the picture loads anew, instead of drawing the freed id it was built
/// with.
#[test]
fn a_swatch_reloads_a_texture_the_store_dropped() {
    let ctx = egui::Context::default();
    let pattern = Arc::new(photocraft_engine::pattern_cmds::builtin().remove(0));
    let swatch = pattern_swatch(&pattern);
    let mut first = None;
    pass(&ctx, |ui| first = Some(swatch.texture(ui.ctx())));
    let first = first.unwrap();
    assert!(ctx.tex_manager().read().meta(first).is_some());
    // Later passes fill the store past its capacity without the swatch, so its texture goes.
    pass(&ctx, |ui| {
        for i in 0..TextureCache::CAPACITY + 8 {
            TextureCache::texture(ui.ctx(), ("filler", i), || ColorImage::new([1, 1], vec![Color32::RED]));
        }
    });
    assert!(ctx.tex_manager().read().meta(first).is_none(), "the swatch's first texture was dropped");
    let mut again = None;
    pass(&ctx, |ui| again = Some(swatch.texture(ui.ctx())));
    assert!(ctx.tex_manager().read().meta(again.unwrap()).is_some(), "the swatch paints a live texture");
}

/// #16221: a swatch draws nothing until it paints, so building a browser's swatches loads no textures; only the rows
/// a browser shows load theirs.
#[test]
fn building_swatches_loads_no_texture() {
    let ctx = egui::Context::default();
    let patterns: Vec<_> = photocraft_engine::pattern_cmds::builtin().into_iter().map(Arc::new).collect();
    let mut swatches = Vec::new();
    pass(&ctx, |_| swatches = patterns.iter().map(pattern_swatch).collect::<Vec<_>>());
    assert_eq!(TextureCache::len(&ctx), 0);
    pass(&ctx, |ui| {
        let _ = swatches[0].texture(ui.ctx());
    });
    assert_eq!(TextureCache::len(&ctx), 1, "only the painted swatch loads");
}

/// #16221: a pattern replaced under the same id (an imported `.pat` whose id is in the library) keys a new picture.
#[test]
fn a_pattern_replaced_under_its_id_keys_a_new_picture() {
    let mut patterns = photocraft_engine::pattern_cmds::builtin();
    let first = patterns.remove(0);
    let mut replaced = patterns.remove(0);
    replaced.id = first.id.clone();
    replaced.name = first.name.clone();
    assert_ne!(pattern_key(&first), pattern_key(&replaced));
    assert_eq!(pattern_key(&first), pattern_key(&first.clone()), "a clone holds the same pixels");
}

/// A pattern swatch drawn while a pattern tile failed to read shows zeros: once the tile reads again (no new
/// generation), the swatch keys a new picture, and so do the style thumbnails drawn over the patterns.
#[test]
fn pattern_and_style_pictures_follow_a_scratch_read_error_transition() {
    let mut app = app();
    let pattern = app.session.patterns.items[0].clone();
    let (swatch, library) = (pattern_key(&pattern), StyleLibrary::held(&mut app));
    assert_eq!(pattern_key(&pattern), swatch);
    assert!(Arc::ptr_eq(&library, &StyleLibrary::held(&mut app)));
    crate::scratch_reads::tests::simulate_read_error_transition();
    assert_ne!(pattern_key(&pattern), swatch, "the swatch draws again");
    let after = StyleLibrary::held(&mut app);
    assert!(!Arc::ptr_eq(&library, &after), "the style library is taken again");
    let stamps = |library: &StyleLibrary| library.styles.iter().map(|(stamp, _)| *stamp).collect::<Vec<_>>();
    assert!(!stamps(&library).is_empty());
    assert!(stamps(&library).iter().zip(stamps(&after)).all(|(before, now)| *before != now), "every style thumbnail draws again");
}

/// The flat list of the app's style presets.
fn styles(app: &PhotocraftApp) -> Vec<StylePreset> {
    app.session.presets.styles.iter().flat_map(|group| group.items.iter().cloned()).collect()
}

/// #16221: a style's picture key covers its effect values and the patterns' pixels: a same-named style with as many
/// effects, the same blend and the same fill opacity but other effect values draws anew, as it does over other
/// pattern pixels.
#[test]
fn a_style_key_covers_its_effect_values_and_the_pattern_pixels() {
    let mut app = app();
    for (name, size) in [("Thin", 2), ("Thick", 9)] {
        let effects = json!([["stroke", {"size": size, "color": "#123456"}]]);
        app.run("style.presets.new", json!({"name": name, "effects": effects, "blend": "Multiply", "fillOpacity": 40})).unwrap();
    }
    let all = styles(&app);
    let thin = all.iter().find(|style| style.name == "Thin").unwrap().clone();
    let mut thick = all.iter().find(|style| style.name == "Thick").unwrap().clone();
    thick.name = thin.name.clone();
    assert_eq!((thin.effects.len(), thin.blend, thin.fill_opacity), (thick.effects.len(), thick.blend, thick.fill_opacity));
    assert_ne!(style_stamp(&thin, 1), style_stamp(&thick, 1), "other effect values");
    assert_ne!(style_stamp(&thin, 1), style_stamp(&thin, 2), "other pattern pixels");
    assert_eq!(style_stamp(&thin, 1), style_stamp(&thin.clone(), 1));
}

/// #16221: the Styles views read one held library while the styles and patterns are unchanged, so their memo keys
/// settle between passes; a style edit or a replaced pattern gives a new one.
#[test]
fn the_style_library_is_held_until_styles_or_patterns_change() {
    let mut app = app();
    let first = StyleLibrary::held(&mut app);
    let again = StyleLibrary::held(&mut app);
    assert!(Arc::ptr_eq(&first, &again));
    assert_eq!(MemoKey::of(&first), MemoKey::of(&again), "an unchanged library keys the same");
    app.run("style.presets.new", json!({"name": "Fresh", "effects": [["stroke", {"size": 3, "color": "#000000"}]]})).unwrap();
    let edited = StyleLibrary::held(&mut app);
    assert!(!Arc::ptr_eq(&first, &edited), "a new style");
    assert_ne!(MemoKey::of(&first), MemoKey::of(&edited));
    let mut replacement = app.session.patterns.items[1].clone();
    replacement.id = app.session.patterns.items[0].id.clone();
    app.session.patterns.upsert(replacement);
    assert!(!Arc::ptr_eq(&edited, &StyleLibrary::held(&mut app)), "a pattern replaced under its id");
}

/// #16221: the preset views share one held copy of the patterns while the library is unchanged.
#[test]
fn the_patterns_are_held_until_the_library_changes() {
    let mut app = app();
    let first = held_patterns(&mut app);
    assert!(Arc::ptr_eq(&first, &held_patterns(&mut app)));
    let mut extra = app.session.patterns.items[0].clone();
    extra.id = "held-test-extra".into();
    app.session.patterns.items.push(extra);
    let grown = held_patterns(&mut app);
    assert!(!Arc::ptr_eq(&first, &grown));
    assert!(grown.iter().any(|pattern| pattern.id == "held-test-extra"));
}
