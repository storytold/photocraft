use egui_kittest::kittest::Queryable;
use photocraft_doc::{Effect, Pattern, PixelFormat};
use photocraft_raster::Surface;

use super::*;

fn make_quadrant_pattern() -> Pattern {
    let mut surface = Surface::new(PixelFormat::RGBA8);
    {
        let tile = surface.tile_mut(photocraft_geom::TileCoord { tx: 0, ty: 0 });
        let mut bytes = tile.bytes_mut();
        for y in 0..20usize {
            for x in 0..20usize {
                let rgba = if x == 0 && y == 0 {
                    [255, 255, 255, 255]
                } else if x < 10 && y < 10 {
                    [255, 0, 0, 255]
                } else if x >= 10 && y < 10 {
                    [0, 255, 0, 255]
                } else if x < 10 && y >= 10 {
                    [0, 0, 255, 255]
                } else {
                    [0, 0, 0, 255]
                };
                let idx = (y * photocraft_geom::TILE_SIZE as usize + x) * 4;
                bytes[idx..idx + 4].copy_from_slice(&rgba);
            }
        }
    }
    Pattern::new("Quadrant Marker", surface, 20, 20)
}

fn test_app() -> (PhotocraftApp, Pattern) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let pat = make_quadrant_pattern();
    app.session.patterns.items.push(pat.clone());
    app.run("file.new", json!({"width": 400, "height": 300, "resolution": 72.0, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Shape"})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 75, "width": 200, "height": 150})).unwrap();
    app.run("edit.fill", json!({"color": "#ff8000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    (app, pat)
}

fn active_effect(app: &PhotocraftApp) -> &Effect {
    let state = app.session.active().unwrap();
    &state.doc.layer(state.active_layer.unwrap()).unwrap().effects.items[0]
}

fn open_pattern_overlay_fields(app: &PhotocraftApp) -> Map<String, Value> {
    let state = app.session.active().unwrap();
    let mut f = initial_fields(state.doc.layer(state.active_layer.unwrap()).unwrap(), Some("patternOverlay"), 120.0);
    f.insert("patternList".into(), pattern_list(app));
    f
}

fn run_and_click_snap(app: PhotocraftApp, fields: Map<String, Value>) -> (PhotocraftApp, Map<String, Value>) {
    let mut harness = egui_kittest::Harness::builder().with_size(vec2(1000.0, 1000.0)).build_ui_state(
        |ui, state: &mut (PhotocraftApp, Map<String, Value>)| {
            if ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                body(&mut state.0, ui, &mut state.1);
            }
        },
        (app, fields),
    );
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
    harness.run_steps(3);
    let pattern_combo = harness.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Pattern");
    let snap = harness.get_by_role_and_label(egui::accesskit::Role::Button, "Snap to Origin");
    assert!(snap.rect().min.x >= pattern_combo.rect().max.x, "Snap to Origin should sit right of the Pattern control");
    snap.click();
    harness.run_steps(2);
    let (app, fields) = harness.state_mut();
    (std::mem::replace(app, PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())), fields.clone())
}

fn pixel_letter(px: [f32; 4]) -> char {
    let r = (px[0].clamp(0.0, 1.0) * 255.0).round() as u8;
    let g = (px[1].clamp(0.0, 1.0) * 255.0).round() as u8;
    let b = (px[2].clamp(0.0, 1.0) * 255.0).round() as u8;
    match (r > 200, g > 200, b > 200) {
        (true, true, true) => 'W',
        (true, false, false) => 'R',
        (false, true, false) => 'G',
        (false, false, true) => 'B',
        (false, false, false) => 'K',
        _ => '?',
    }
}

#[test]
fn snap_to_origin_sits_beside_pattern_changes_only_phase_and_cancel_restores_old_phase() {
    let (mut app, pat) = test_app();
    app.run(
        "layer.layerStyle.patternOverlay",
        json!({
            "pattern": pat.id,
            "blend": "Multiply",
            "opacity": 80,
            "angle": 15,
            "scale": 120,
            "link": true,
            "phaseX": 7.0,
            "phaseY": 11.0
        }),
    )
    .unwrap();
    let original = active_effect(&app).clone();
    let fields = open_pattern_overlay_fields(&app);
    let (app, pending) = run_and_click_snap(app, fields);

    let params = &pending["effects"][0]["params"];
    assert_eq!(params["phaseX"], json!(0.0));
    assert_eq!(params["phaseY"], json!(0.0));
    assert_eq!(params["blend"], json!("Multiply"));
    assert_eq!(params["opacity"], json!(80.0));
    assert_eq!(params["pattern"], json!(pat.id));
    assert_eq!(params["angle"], json!(15.0));
    assert_eq!(params["scale"], json!(120.0));
    assert_eq!(params["link"], json!(true));

    // Cancel discards pending edits, restoring/keeping the original phase on the document.
    assert_eq!(active_effect(&app), &original);
}

#[test]
fn snap_to_origin_linked_renders_expected_pixels_before_and_after_move() {
    let (mut app, pat) = test_app();
    app.run(
        "layer.layerStyle.patternOverlay",
        json!({
            "pattern": pat.id,
            "blend": "Normal",
            "opacity": 100,
            "angle": 0,
            "scale": 100,
            "link": true,
            "phaseX": 5.0,
            "phaseY": 9.0
        }),
    )
    .unwrap();

    let fields = open_pattern_overlay_fields(&app);
    let (mut app, pending) = run_and_click_snap(app, fields);
    confirm(&mut app, &pending).unwrap();

    let flat = photocraft_compose::flatten(&app.session.active().unwrap().doc);
    let row: String = (100..126).map(|x| pixel_letter(flat.get(x, 150))).collect();
    assert_eq!(row, "BBBBBBBBBBKKKKKKKKKKBBBBBB", "row y=150 from x=100 should be B*10 K*10 B*6");

    app.run("layer.translate", json!({"dx": 7, "dy": 4})).unwrap();
    app.run(
        "layer.layerStyle.patternOverlay",
        json!({
            "pattern": pat.id,
            "blend": "Normal",
            "opacity": 100,
            "angle": 0,
            "scale": 100,
            "link": true,
            "phaseX": 3.0,
            "phaseY": 6.0
        }),
    )
    .unwrap();

    let fields = open_pattern_overlay_fields(&app);
    let (mut app, pending) = run_and_click_snap(app, fields);
    confirm(&mut app, &pending).unwrap();

    let flat = photocraft_compose::flatten(&app.session.active().unwrap().doc);
    let col: String = (79..101).map(|y| pixel_letter(flat.get(120, y))).collect();
    assert_eq!(col, "GGGGGGGGGGKKKKKKKKKKGG", "col x=120 from y=79 should be G*10 K*10 GG");
}

#[test]
fn snap_to_origin_unlinked_sets_phase_to_zero() {
    let (mut app, pat) = test_app();
    app.run(
        "layer.layerStyle.patternOverlay",
        json!({
            "pattern": pat.id,
            "blend": "Normal",
            "opacity": 100,
            "angle": 0,
            "scale": 100,
            "link": false,
            "phaseX": 13.0,
            "phaseY": 17.0
        }),
    )
    .unwrap();

    let fields = open_pattern_overlay_fields(&app);
    let (mut app, pending) = run_and_click_snap(app, fields);
    confirm(&mut app, &pending).unwrap();

    let Effect::PatternOverlay { link, phase, .. } = active_effect(&app) else {
        panic!("expected PatternOverlay");
    };
    assert!(!*link);
    assert_eq!(*phase, (0.0, 0.0));
}
