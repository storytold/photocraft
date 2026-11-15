//! #14963: Toggling `Link with Layer` in the Layer Style dialog's Pattern Overlay page must keep
//! the pattern in place on the canvas instead of jumping between the layer-relative grid and the
//! document grid.
use egui::vec2;
use egui_kittest::kittest::Queryable;
use photocraft_color::PixelFormat;
use photocraft_doc::pattern::Pattern;
use photocraft_doc::{Effect, FxCommon, Layer, LayerContent};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

use super::*;

fn setup_test_doc(phase: (f32, f32)) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "resolution": 72, "background": "white"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Shape"})).unwrap();

    // 20x20 test pattern: TL red, TR green, BL blue, BR black, white origin pixel at (0, 0).
    let mut tile = Surface::new(PixelFormat::RGBA8);
    tile.fill_rect(Rect::new(0, 0, 10, 10), &[1.0, 0.0, 0.0, 1.0]);
    tile.fill_rect(Rect::new(10, 0, 20, 10), &[0.0, 1.0, 0.0, 1.0]);
    tile.fill_rect(Rect::new(0, 10, 10, 20), &[0.0, 0.0, 1.0, 1.0]);
    tile.fill_rect(Rect::new(10, 10, 20, 20), &[0.0, 0.0, 0.0, 1.0]);
    tile.fill_rect(Rect::new(0, 0, 1, 1), &[1.0, 1.0, 1.0, 1.0]);
    let pat = Pattern::new("Test Quadrant", tile, 20, 20);
    let pat_id = pat.id.clone();

    let mut shape = Surface::new(PixelFormat::RGBA8);
    shape.fill_rect(Rect::new(100, 75, 300, 225), &[1.0, 128.0 / 255.0, 0.0, 1.0]);

    app.session
        .edit("Setup Shape and Pattern Overlay", |doc, active| {
            doc.patterns.push(pat.clone());
            let id = active.unwrap();
            let layer: &mut Layer = doc.layer_mut(id).unwrap();
            layer.content = LayerContent::Raster(shape);
            layer.effects.items = vec![Effect::PatternOverlay {
                common: FxCommon::new(photocraft_color::BlendMode::Normal, 1.0),
                name: pat.name.clone(),
                id: pat_id,
                scale: 1.0,
                angle: 0.0,
                link: true,
                phase,
            }];
            Ok(())
        })
        .unwrap();
    app.session.patterns.items.push(pat);
    app
}

fn render_doc(app: &PhotocraftApp) -> Vec<f32> {
    let st = app.session.active().unwrap();
    let surf = photocraft_compose::composite_to_surface(&st.doc, st.doc.bounds(), PixelFormat::RGBA8, None);
    surf.try_read_region(st.doc.bounds()).unwrap()
}

fn open_pattern_overlay_harness(app: PhotocraftApp) -> egui_kittest::Harness<'static, (PhotocraftApp, Map<String, Value>)> {
    let mut fields = {
        let state = app.session.active().unwrap();
        initial_fields(state.doc.layer(state.active_layer.unwrap()).unwrap(), Some("patternOverlay"), state.doc.global_light.angle)
    };
    fields.insert("patternList".into(), pattern_list(&app));
    let mut harness = egui_kittest::Harness::builder().with_size(vec2(1000.0, 1200.0)).build_ui_state(
        |ui, state: &mut (PhotocraftApp, Map<String, Value>)| {
            if ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                body(&mut state.0, ui, &mut state.1);
            }
        },
        (app, fields),
    );
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
    harness.run_steps(3);
    harness
}

#[test]
fn unticking_link_with_layer_after_move_keeps_pixels_unchanged() {
    let mut app = setup_test_doc((0.0, 0.0));
    app.run("layer.translate", json!({"dx": 7, "dy": 4})).unwrap();
    let before = render_doc(&app);

    let mut harness = open_pattern_overlay_harness(app);
    harness.get_by_label("Link with Layer").click();
    harness.run_steps(3);

    let (mut app, fields) = harness.into_state();
    assert_eq!(fields["effects"][0]["params"]["link"], false);
    confirm(&mut app, &fields).unwrap();

    let after = render_doc(&app);
    assert_eq!(after, before, "unticking Link with Layer must leave every pixel unchanged");
}

#[test]
fn unticking_and_ticking_link_on_unmoved_layer_keeps_phase_and_pixels() {
    let app = setup_test_doc((5.0, 3.0));
    let before = render_doc(&app);

    let mut harness = open_pattern_overlay_harness(app);
    harness.get_by_label("Link with Layer").click();
    harness.run_steps(3);
    assert_eq!(harness.state().1["effects"][0]["params"]["link"], false);

    harness.get_by_label("Link with Layer").click();
    harness.run_steps(3);

    let (mut app, fields) = harness.into_state();
    assert_eq!(fields["effects"][0]["params"]["link"], true);
    assert_eq!(fields["effects"][0]["params"]["phaseX"], 5.0);
    assert_eq!(fields["effects"][0]["params"]["phaseY"], 3.0);

    confirm(&mut app, &fields).unwrap();
    let after = render_doc(&app);
    assert_eq!(after, before, "unticking and ticking Link with Layer on an unmoved layer must keep pixels unchanged");
}

#[test]
fn command_link_false_without_phase_keeps_existing_behavior() {
    let mut app = setup_test_doc((0.0, 0.0));
    app.run("layer.translate", json!({"dx": 7, "dy": 4})).unwrap();
    app.run("layer.layerStyle.patternOverlay", json!({"link": false})).unwrap();

    let st = app.session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let Some(Effect::PatternOverlay { link, phase, .. }) = layer.effects.items.first() else {
        panic!("expected PatternOverlay effect");
    };
    assert!(!link);
    assert_eq!(*phase, (0.0, 0.0));
}
