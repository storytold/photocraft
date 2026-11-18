use photocraft_doc::{DocId, Fill, LayerContent, LayerId};
use serde_json::json;

use super::*;
use crate::color_picker_ui::testing;

fn fill(app: &PhotocraftApp, id: LayerId) -> Option<Fill> {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => sh.fill.clone(),
        _ => None,
    }
}

/// OK on picker `dialog`, and the error its commit reported: a refused commit leaves the error in the status line.
fn ok(app: &mut PhotocraftApp, dialog: DialogId) -> Result<(), String> {
    app.ui.status = Default::default();
    app.ui.status_error = false;
    crate::dialogs::testing::confirm(app, dialog).unwrap();
    if app.ui.status_error { Err(app.ui.status.to_string()) } else { Ok(()) }
}

/// The foreground colour as `"#rrggbb"`.
fn foreground_hex(app: &PhotocraftApp) -> String {
    let c = app.session.tools.foreground;
    color_picker_ui::hex([c[0], c[1], c[2]])
}

/// The popovers' Color Picker edits the stroke too, previews it on the canvas, and fails gracefully (never panics)
/// when its target goes stale.
#[test]
fn stroke_picker_previews_commits_and_rejects_stale_or_bad_targets() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    let id = LayerId(
        app.run("shape.create", json!({"rect": [8, 8, 40, 30], "fill": "#336699", "stroke": {"width": 4, "color": [1.0, 0.0, 0.0, 0.5]}})).unwrap()["layer"]
            .as_u64()
            .unwrap(),
    );
    let stroke = |app: &PhotocraftApp| match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Shape(sh) => sh.stroke.clone(),
        _ => None,
    };
    let dialog = open_paint(&mut app, Paint::Stroke, Some(id), false, [0.0, 0.0, 0.0, 1.0]).unwrap();
    assert_eq!(rgb_hex(testing::current(&app, dialog).unwrap().color), "#ff0000", "opens on the stroke colour");
    testing::set_color(&mut app, dialog, "#00ff00");
    let (shown, _) = display_doc(&mut app, 0).unwrap();
    let LayerContent::Shape(sh) = &shown.layer(id).unwrap().content else { panic!() };
    assert_eq!(sh.stroke.as_ref().unwrap().paint, Fill::Solid(Color::rgba(0.0, 1.0, 0.0, 0.5)), "previewed with the stroke's alpha");
    assert_eq!(sh.fill, fill(&app, id), "the fill is untouched");
    ok(&mut app, dialog).unwrap();
    let after = stroke(&app).unwrap();
    assert_eq!((after.paint, after.width), (Fill::Solid(Color::rgba(0.0, 1.0, 0.0, 0.5)), 4.0));

    // Neither a layer nor the tool options: nothing to open.
    assert!(open_paint(&mut app, Paint::Fill, None, false, [0.0; 4]).is_none());

    // Another document became active: the edit is refused, and the first document keeps its fill.
    let first_fill = fill(&app, id);
    let dialog = open_paint(&mut app, Paint::Fill, Some(id), false, [0.0; 4]).unwrap();
    testing::set_color(&mut app, dialog, "#0000ff");
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    let other = app.session.active_index().unwrap();
    assert!(display_doc(&mut app, other).is_none(), "another document shows no preview");
    assert!(ok(&mut app, dialog).is_err(), "another document is active");
    assert!(app.session.set_active(0));
    assert_eq!(fill(&app, id), first_fill);

    // A deleted layer: an error, not a panic, and no preview.
    let dialog = open_paint(&mut app, Paint::Fill, Some(id), false, [0.0; 4]).unwrap();
    testing::set_color(&mut app, dialog, "#0000ff");
    app.run("layer.delete", json!({})).unwrap();
    assert!(display_doc(&mut app, 0).is_none());
    assert!(ok(&mut app, dialog).is_err(), "the layer is gone");
}

/// A shape without a stroke, or with a 0 px one, previews the stroke the picker's OK turns on: the picked colour at
/// the enabled width, even for the colour it opened on.
#[test]
fn stroke_picker_previews_a_stroke_that_is_off_as_ok_turns_it_on() {
    for stroke in [json!(null), json!({"width": 0, "color": "#ff0000"})] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        let id = LayerId(app.run("shape.create", json!({"rect": [8, 8, 40, 30], "fill": "#336699", "stroke": stroke})).unwrap()["layer"].as_u64().unwrap());
        let dialog = open_paint(&mut app, Paint::Stroke, Some(id), false, [0.0, 0.0, 0.0, 1.0]).unwrap();
        let (shown, _) = display_doc(&mut app, 0).expect("the unchanged colour still previews the stroke OK adds");
        let LayerContent::Shape(sh) = &shown.layer(id).unwrap().content else { panic!() };
        let previewed = sh.stroke.clone().expect("a previewed stroke");
        ok(&mut app, dialog).unwrap();
        let LayerContent::Shape(sh) = &app.session.active().unwrap().doc.layer(id).unwrap().content else { panic!() };
        let committed = sh.stroke.clone().expect("OK turned the stroke on");
        assert_eq!((previewed.paint, previewed.width), (committed.paint, committed.width), "{stroke}: the preview is what OK commits");
        assert_eq!(committed.width, 3.0);
    }
}

/// From the options bar without a selected shape, OK sets the next shape's colour and turns the paint on, and edits no
/// document.
#[test]
fn tool_only_picker_sets_the_next_shapes_colour() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.ui.tool_options.shape_fill = false;
    let history = app.session.active().unwrap().history.past_len();
    let dialog = open_paint(&mut app, Paint::Fill, None, true, [0.1, 0.2, 0.3, 1.0]).unwrap();
    // The dialog's purpose is its typed target; the picker has no title getter.
    assert!(
        matches!(color_picker_ui::target(&app, dialog), Some(PickTarget::Shape(t)) if t.which == Paint::Fill && t.tool && t.layer.is_none()),
        "the options bar's Fill picker for the next shape"
    );
    testing::set_color(&mut app, dialog, "#ff8800");
    ok(&mut app, dialog).unwrap();
    assert!(app.ui.tool_options.shape_fill);
    assert_eq!(foreground_hex(&app), "#ff8800");
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
}

/// With no document open, the options bar's Color Picker still sets the next shape's colour.
#[test]
fn tool_only_picker_works_without_a_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert!(app.session.active().is_none());
    for which in [Paint::Fill, Paint::Stroke] {
        let dialog = open_paint(&mut app, which, None, true, [0.1, 0.2, 0.3, 1.0]).unwrap();
        testing::set_color(&mut app, dialog, "#ff8800");
        ok(&mut app, dialog).unwrap();
    }
    assert!(app.ui.tool_options.shape_fill);
    assert_eq!(foreground_hex(&app), "#ff8800");
    let b = app.session.tools.background;
    assert_eq!(color_picker_ui::hex([b[0], b[1], b[2]]), "#ff8800");
    // A picker for a shape layer still needs its document.
    let target = ShapeTarget { which: Paint::Fill, layer: Some(LayerId(3)), document: Some(DocId(1)), tool: false, thumbnail: false, alpha: 1.0, solid: true };
    let dialog = color_picker_ui::open_for(&mut app, "Color Picker (Fill Color)", Srgba::from_rgb_array([1.0, 0.53, 0.0]), None, PickTarget::Shape(target));
    assert_eq!(ok(&mut app, dialog).unwrap_err(), "no document open");
}

/// A locked selected shape refuses the options bar's Color Picker before anything changes: the next shape's colour
/// stays as it was, as does the document.
#[test]
fn locked_shape_refuses_the_options_bar_picker_without_changing_the_next_shapes_colour() {
    for lock in ["all", "pixels"] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        let id = LayerId(app.run("shape.create", json!({"rect": [8, 8, 40, 30], "fill": "#336699"})).unwrap()["layer"].as_u64().unwrap());
        app.run("layer.lockLayers", json!({lock: true})).unwrap();
        let (fg, options) = (app.session.tools.foreground, app.ui.tool_options.clone());
        let history = app.session.active().unwrap().history.past_len();
        let dialog = open_paint(&mut app, Paint::Fill, Some(id), true, fg).unwrap();
        testing::set_color(&mut app, dialog, "#ff8800");
        assert_eq!(ok(&mut app, dialog).unwrap_err(), "shape layer is locked", "{lock}");
        assert_eq!(app.session.tools.foreground, fg, "{lock}: the next shape's colour is unchanged");
        assert_eq!(app.ui.tool_options, options);
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
        assert_eq!(fill(&app, id), Some(Fill::Solid(Color::rgb(0.2, 0.4, 0.6))));
    }
}

/// A shape edit refused for another reason (here a job holding the document) leaves the next shape's colour and stroke
/// as they were: the edit runs first, the tool defaults only after it.
#[test]
fn a_refused_shape_edit_leaves_the_next_shapes_colour_alone() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    for which in [Paint::Fill, Paint::Stroke] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        let id = LayerId(app.run("shape.create", json!({"rect": [8, 8, 40, 30], "fill": "#336699"})).unwrap()["layer"].as_u64().unwrap());
        let (tools, options) = (app.session.tools.clone(), app.ui.tool_options.clone());
        let dialog = open_paint(&mut app, which, Some(id), true, [0.0, 0.0, 0.0, 1.0]).unwrap();
        testing::set_color(&mut app, dialog, "#ff8800");
        let gate = Arc::new(AtomicBool::new(false));
        let open = gate.clone();
        let started = app
            .session
            .start_job(
                "test.hold",
                json!({}),
                "Hold",
                true,
                move |ctx| {
                    while !open.load(Ordering::Relaxed) {
                        ctx.check()?;
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    Ok(())
                },
                |_, ()| Ok(json!({})),
            )
            .unwrap();
        let photocraft_engine::jobs::Started::Job(job) = started else { panic!("the job ran inline") };
        assert!(ok(&mut app, dialog).unwrap_err().contains("still running"), "{which:?}");
        assert_eq!(app.session.tools.foreground, tools.foreground, "{which:?}: the next shape's colours are unchanged");
        assert_eq!(app.session.tools.background, tools.background);
        assert_eq!(app.ui.tool_options, options, "{which:?}: the stroke stays off");
        gate.store(true, Ordering::Relaxed);
        app.session.cancel_job(job);
        app.session.join_cancelled_jobs();
    }
}
