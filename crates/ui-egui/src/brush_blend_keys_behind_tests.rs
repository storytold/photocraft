//! #11896: Behind is one of the painting modes, so after Behind a Shift+Alt letter or a
//! Shift+minus/plus step that sets another mode leaves Behind, in the same one `tools.setBrush`.

use craft::ui::runtime::commands::CommandRef;
use serde_json::{Value, json};

use super::*;

/// A 64 × 64 document with a layer filled 220,30,30, black foreground, the Brush.
fn app(tool: Tool) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#dc1e1e"})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#000000"})).unwrap();
    app.ui.tool = tool;
    app
}

/// One blend key command through the module's entry point; every blend key is taken.
fn fire(app: &mut PhotocraftApp, command: impl Into<CommandRef>) {
    assert!(fired(app, &test_fired(command.into())));
}

fn brush(app: &mut PhotocraftApp) -> Value {
    app.run("brush.get", json!({})).unwrap()
}

fn behind_on(app: &mut PhotocraftApp) {
    fire(app, BlendShortcut::Behind);
    assert_eq!(brush(app)["behind"], true, "Shift+Alt+Q turns Behind on");
}

fn journal(app: &PhotocraftApp) -> usize {
    app.session.journal.len()
}

/// The layer's pixel at (x, y) as 8-bit RGB.
fn pixel(app: &PhotocraftApp, x: i32, y: i32) -> [u8; 3] {
    let st = app.session.active().unwrap();
    let c = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y);
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as u8)
}

#[test]
fn shift_alt_m_after_behind_multiplies_over_the_pixels() {
    let mut app = app(Tool::Brush);
    behind_on(&mut app);
    let n = journal(&app);
    fire(&mut app, BlendShortcut::Multiply);
    let b = brush(&mut app);
    assert_eq!((b["behind"].clone(), b["mode"].clone()), (json!(false), json!("Multiply")), "{b}");
    assert_eq!(journal(&app), n + 1, "one tools.setBrush");
    // A black stroke across the red layer multiplies it to black, not only under transparency.
    app.run("paint.stroke", json!({"points": [[4, 32], [60, 32]], "size": 9, "hardness": 100})).unwrap();
    assert_eq!(pixel(&app, 32, 32), [0, 0, 0]);
}

#[test]
fn shift_alt_n_l_and_r_after_behind_leave_it() {
    for (letter, mode) in [(BlendShortcut::Normal, "Normal"), (BlendShortcut::HardMix, "HardMix")] {
        let mut app = app(Tool::Brush);
        behind_on(&mut app);
        let n = journal(&app);
        fire(&mut app, letter);
        let b = brush(&mut app);
        assert_eq!(b["behind"], false, "{letter:?}: {b}");
        assert_eq!(b["mode"], json!(mode), "{letter:?}: {b}");
        assert_eq!(journal(&app), n + 1, "{letter:?}: one tools.setBrush");
    }
    // Clear: Behind off and Clear on.
    let mut app = app(Tool::Brush);
    behind_on(&mut app);
    let n = journal(&app);
    fire(&mut app, BlendShortcut::Clear);
    assert_eq!(brush(&mut app)["behind"], false);
    assert!(app.ui.tool_options.brush_clear, "Clear on");
    assert_eq!(journal(&app), n + 1, "one tools.setBrush");
    // The Pencil too.
    let mut app = self::app(Tool::Pencil);
    behind_on(&mut app);
    fire(&mut app, BlendShortcut::Multiply);
    assert_eq!(brush(&mut app)["behind"], false);
}

#[test]
fn a_shift_step_after_behind_leaves_it() {
    for step in [BlendStep::Previous, BlendStep::Next] {
        let mut app = app(Tool::Brush);
        behind_on(&mut app);
        let n = journal(&app);
        fire(&mut app, step);
        assert_eq!(brush(&mut app)["behind"], false, "{step:?}");
        assert_eq!(journal(&app), n + 1, "{step:?}: one tools.setBrush");
    }
}
