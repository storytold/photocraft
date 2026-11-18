use super::*;
use crate::canvas::ToolEvent;

fn app_with_gradient(style: &str) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
    app.ui.tool = Tool::Gradient;
    app.ui.tool_options.gradient_style = style.into();
    app
}

fn mods() -> egui::Modifiers {
    egui::Modifiers::NONE
}

fn drag(app: &mut PhotocraftApp, a: [f64; 2], b: [f64; 2]) {
    crate::canvas::tool_event(app, ToolEvent::Down { x: a[0], y: a[1], pressure: 1.0 }, mods());
    crate::canvas::tool_event(app, ToolEvent::Move { x: (a[0] + b[0]) / 2.0, y: (a[1] + b[1]) / 2.0, pressure: 1.0 }, mods());
    crate::canvas::tool_event(app, ToolEvent::Move { x: b[0], y: b[1], pressure: 1.0 }, mods());
    crate::canvas::tool_event(app, ToolEvent::Up { x: b[0], y: b[1] }, mods());
}

fn get(app: &mut PhotocraftApp) -> Value {
    app.run(cmds::GET, json!({})).unwrap()
}

/// A live drag makes one Gradient Fill layer ending at the release; a later drag that starts away
/// from the live widget redraws the selected gradient instead of adding a layer.
#[test]
fn live_drag_creates_a_fill_layer_and_a_drag_away_from_the_widget_redraws_it() {
    let mut app = app_with_gradient("linear");
    drag(&mut app, [20.0, 60.0], [180.0, 60.0]);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 2, "a Gradient Fill layer");
    let g = get(&mut app);
    assert!((g["to"][0].as_f64().unwrap() - 180.0).abs() < 1e-3);
    // The press at [100, 10] is off the line, so the live overlay does not take it.
    drag(&mut app, [100.0, 10.0], [100.0, 110.0]);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 2, "no new layer");
    assert!((get(&mut app)["from"][1].as_f64().unwrap() - 10.0).abs() < 1e-3);
}

#[test]
fn shift_snaps_live_drags_to_45_degrees() {
    let mut app = app_with_gradient("linear");
    let shift = egui::Modifiers::SHIFT;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, shift);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 70.0, y: 62.0, pressure: 1.0 }, shift);
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 90.0, y: 84.0 }, shift);
    let g = get(&mut app);
    let (fx, fy, tx, ty) = (g["from"][0].as_f64().unwrap(), g["from"][1].as_f64().unwrap(), g["to"][0].as_f64().unwrap(), g["to"][1].as_f64().unwrap());
    assert!(((tx - fx) - (ty - fy)).abs() < 1e-2 && tx > 80.0, "45°: {g}");
}

#[test]
fn classic_drag_preserves_preset_transparency_at_all_depths() {
    for depth in [8, 16, 32] {
        for reverse in [false, true] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 200, "height": 120, "depth": depth, "background": "transparent"})).unwrap();
            app.ui.tool = Tool::Gradient;
            app.ui.tool_options.gradient_classic = true;
            app.ui.tool_options.gradient_reverse = reverse;
            app.ui.tool_options.gradient_dither = false;
            app.ui.tool_options.fill_opacity = 50.0;
            app.session.tools.foreground = [1.0, 0.0, 0.0, 1.0];
            app.run("gradient.presets.select", json!({"preset": "Foreground to Transparent"})).unwrap();
            let before = app.session.active().unwrap().history.past_len();
            drag(&mut app, [20.0, 60.0], [180.0, 60.0]);
            let st = app.session.active().unwrap();
            let surface = st.doc.layers[0].surface().unwrap();
            for x in [0, 100, 199] {
                let u = ((x as f32 - 20.0) / 160.0).clamp(0.0, 1.0);
                let expected = 0.5 * if reverse { u } else { 1.0 - u };
                let px = surface.pixel(x, 60);
                assert!((px[3] - expected).abs() < 0.01, "depth {depth}, reverse {reverse}, x {x}: {px:?}");
                if expected > 0.01 {
                    assert!(px[0] > 0.99 && px[1] < 0.01 && px[2] < 0.01, "preset colour: {px:?}");
                }
            }
            assert_eq!(st.history.past_len(), before + 1);
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(app.session.active().unwrap().doc.layers[0].surface().unwrap().pixel(100, 60)[3], 0.0);
            app.run("gradient.presets.select", json!({"stops": [[0, "#00ff00"], [1, "#00ff00"]], "transparency": [[0, 60], [1, 20]]})).unwrap();
            drag(&mut app, [20.0, 60.0], [180.0, 60.0]);
            let px = app.session.active().unwrap().doc.layers[0].surface().unwrap().pixel(100, 60);
            assert!((px[3] - 0.2).abs() < 0.01 && px[1] > 0.99 && px[0] < 0.01, "custom semi-transparent colour: {px:?}");
        }
    }
}

/// The Classic blend mode and opacity apply to classic drags only: a live drag makes a Normal,
/// full-opacity fill layer whatever Classic holds, and a classic drag adds no layer and blends
/// its pixels per mode.
#[test]
fn gradient_blend_mode_reaches_classic_drags_not_the_live_fill_layer() {
    use photocraft_color::BlendMode;
    for mode in [BlendMode::Difference, BlendMode::Multiply, BlendMode::Screen, BlendMode::Exclusion] {
        let mut live = app_with_gradient("linear");
        live.ui.tool_options.gradient_blend_mode = mode;
        live.ui.tool_options.fill_opacity = 40.0;
        drag(&mut live, [20.0, 60.0], [180.0, 60.0]);
        let layer = live.session.active().unwrap().doc.layers.last().unwrap();
        assert!(matches!(layer.content, photocraft_doc::LayerContent::Fill(_)), "a fill layer");
        assert_eq!(
            (layer.blend, layer.opacity),
            (BlendMode::Normal, 1.0),
            "the live fill layer is Normal at 100 % whatever the Classic mode and opacity hold ({mode:?})"
        );

        let mut classic = app_with_gradient("linear");
        classic.ui.tool_options.gradient_classic = true;
        classic.ui.tool_options.gradient_blend_mode = mode;
        classic.session.tools.foreground = [1.0, 0.0, 0.0, 1.0];
        classic.session.tools.background = [1.0, 0.0, 0.0, 1.0];
        drag(&mut classic, [20.0, 60.0], [180.0, 60.0]);
        let st = classic.session.active().unwrap();
        assert_eq!(st.doc.layers.len(), 1, "no fill layer in classic mode");
        let px = st.doc.layers[0].surface().unwrap().pixel(60, 60);
        let expected = match mode {
            BlendMode::Difference | BlendMode::Exclusion => [0.0, 1.0, 1.0],
            BlendMode::Multiply => [1.0, 0.0, 0.0],
            BlendMode::Screen => [1.0, 1.0, 1.0],
            _ => [1.0, 1.0, 1.0],
        };
        for (actual, want) in px.iter().zip(expected) {
            assert!((actual - want).abs() < 0.02, "classic {mode:?}: {px:?}");
        }
    }
}

#[test]
fn preview_shows_the_drag_before_commit() {
    let mut app = app_with_gradient("radial");
    let layers = app.session.active().unwrap().doc.layers.len();
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 60.0, pressure: 1.0 }, mods());
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 140.0, y: 60.0, pressure: 1.0 }, mods());
    let (doc, _) = display_doc(&mut app, 0, None).expect("a preview while drawing");
    assert_eq!(doc.layers.len(), layers + 1, "the shown document has the new layer");
    assert!(doc.layer(PREVIEW_LAYER).is_some(), "the new layer previews");
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "nothing committed yet");
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 140.0, y: 60.0 }, mods());
    assert!(display_doc(&mut app, 0, None).is_none(), "no preview after release");
}

/// With no gradient fill layer selected the editor edits the Gradient tool's current gradient in
/// place: a stop added through the editor changes the tool gradient and leaves the document and
/// its history alone.
#[test]
fn the_editor_edits_the_current_gradient_without_a_layer() {
    let mut h = super::editor_tests::editor(false);
    let before = h.state().session.presets.gradient.stops.len();
    let doc = h.state().session.active().unwrap().doc.clone();
    let history = h.state().session.active().unwrap().history.past_len();
    super::editor_tests::add_stop(&mut h);
    assert_eq!(h.state().session.presets.gradient.stops.len(), before + 1, "the stop went into the current gradient");
    assert_eq!(h.state().session.active().unwrap().doc, doc, "the document is untouched");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history, "no history step");
}

/// A hidden gradient fill layer is still editable from the editor (the Properties panel edits
/// it whether or not it is drawn), so the editor must not require visibility.
#[test]
fn the_editor_finds_a_hidden_gradient_layer() {
    let mut app = app_with_gradient("linear");
    drag(&mut app, [10.0, 10.0], [150.0, 90.0]);
    let id = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.setProps", json!({"layer": id.0, "visible": false})).unwrap();
    assert!(active_gradient(&app).is_none(), "the canvas widget skips a hidden layer");
    assert!(selected_gradient(&app).is_some(), "the editor still edits it");
}

/// #9311 review: with an artboard selected, the live preview puts the new gradient layer where
/// the commit will, on top of that board's contents, so it doesn't jump on release (#9147).
#[test]
fn the_preview_of_a_new_gradient_goes_inside_the_selected_artboard() {
    let mut app = app_with_gradient("linear");
    app.run("layer.new.artboard", json!({"x": 0, "y": 0, "width": 90, "height": 120, "name": "Board A"})).unwrap();
    let b = app.run("layer.new.artboard", json!({"x": 100, "y": 0, "width": 100, "height": 120, "name": "Board B"})).unwrap();
    let board = LayerId(b["layer"].as_u64().unwrap());
    // Existing content on the board, so a preview at its bottom wouldn't pass.
    app.run("layer.select", json!({"name": "Board B"})).unwrap();
    app.run("layer.new.layer", json!({"name": "content"})).unwrap();
    app.run("layer.select", json!({"name": "Board B"})).unwrap();
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 110.0, y: 60.0, pressure: 1.0 }, mods());
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 190.0, y: 60.0, pressure: 1.0 }, mods());
    let (doc, _) = display_doc(&mut app, 0, None).expect("a preview while drawing");
    let top = |d: &Document| d.layer(board).and_then(photocraft_doc::Layer::children).and_then(<[photocraft_doc::Layer]>::last).map(|l| l.id);
    assert_eq!(top(&doc), Some(PREVIEW_LAYER), "the preview sits on top of Board B's contents");
    let kids = doc.layer(board).and_then(photocraft_doc::Layer::children).map(<[photocraft_doc::Layer]>::len);
    assert_eq!(kids, Some(2), "beside the board's existing layer");
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 190.0, y: 60.0 }, mods());
    let committed = app.session.active().unwrap().active_layer;
    assert_eq!(top(&app.session.active().unwrap().doc), committed, "and so does the committed layer");
}
