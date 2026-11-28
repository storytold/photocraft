//! Free Transform's Rotate and Flip presets commit the layer on whole pixels (#16759): the same
//! bounds and pixels as the direct Edit > Transform command, with no extra empty row or column.

use craft::behavior::kurbo::Vec2;
use egui::Key;
use egui_kittest::Harness;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::app::framework::BuildApp as _;
use crate::state::Tool;

type AppHarness = Harness<'static, Box<PhotocraftApp>>;

/// The epic's measured document: 800 x 600 with a red 300 x 200 pixel layer at 100,100.
fn harness(depth: u32) -> AppHarness {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600, "depth": depth})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 300, "height": 200})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Move;
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(1200.0, 900.0)).build_app(
        |ui, app: &mut PhotocraftApp| {
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::testing::document_area(app, ui));
        },
        app,
    );
    h.run_steps(4);
    h
}

fn open_box(h: &mut AppHarness) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    h.run_steps(4);
    assert!(h.state().ui.transform.is_some(), "Free Transform opened its box");
}

/// Enter applies the box, as the task bar's Done does.
fn commit(h: &mut AppHarness) {
    h.key_press(Key::Enter);
    h.run_steps(6);
    assert!(h.state().ui.transform.is_none(), "Enter applied the box");
}

/// The active layer's `layer.list` bounds, `[x, y, w, h]`.
fn bounds(h: &mut AppHarness) -> Value {
    let list = h.state_mut().session.execute("layer.list", json!({})).unwrap();
    list["layers"].as_array().unwrap().iter().find(|l| l["active"] == true).unwrap()["bounds"].clone()
}

/// The active layer's pixels over the whole canvas.
fn pixels(h: &AppHarness) -> Vec<f32> {
    let st = h.state().session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().read_region(st.doc.bounds())
}

fn history(h: &AppHarness) -> usize {
    h.state().session.active().unwrap().history.past_len()
}

#[test]
fn rotate_and_flip_presets_commit_whole_pixel_bounds() {
    for depth in [8, 16] {
        for (id, expected) in [
            ("edit.transform.rotate90Ccw", [150, 50, 200, 300]),
            ("edit.transform.rotate90Cw", [150, 50, 200, 300]),
            ("edit.transform.rotate180", [100, 100, 300, 200]),
            ("edit.transform.flipHorizontal", [100, 100, 300, 200]),
            ("edit.transform.flipVertical", [100, 100, 300, 200]),
        ] {
            let mut boxed = harness(depth);
            let before = history(&boxed);
            open_box(&mut boxed);
            let ctx = boxed.ctx.clone();
            crate::menus::invoke(boxed.state_mut(), &ctx, id, json!({})).unwrap();
            boxed.run_steps(4);
            commit(&mut boxed);
            assert_eq!(history(&boxed), before + 1, "{depth} bit {id}: one Free Transform step");
            assert_eq!(bounds(&mut boxed), json!(expected), "{depth} bit {id}: bounds after Free Transform");

            let mut direct = harness(depth);
            direct.state_mut().run(id, json!({})).unwrap();
            assert_eq!(bounds(&mut direct), json!(expected), "{depth} bit {id}: bounds of the direct command");
            assert!(pixels(&boxed) == pixels(&direct), "{depth} bit {id}: the same pixels as the direct command");
        }
    }
}

/// A real sub-pixel move keeps its fraction: only float noise snaps to the grid.
#[test]
fn a_fractional_move_commits_unrounded() {
    for depth in [8, 16] {
        let mut boxed = harness(depth);
        open_box(&mut boxed);
        boxed.state_mut().ui.transform.as_mut().unwrap().edit_box(|b| *b = b.moved(Vec2::new(0.25, 0.5)));
        let quad = boxed.state().ui.transform.as_ref().unwrap().quad_array();
        assert_eq!(quad, [[100.25, 100.5], [400.25, 100.5], [400.25, 300.5], [100.25, 300.5]], "{depth} bit");
        let interpolation = boxed.state().ui.transform.as_ref().unwrap().session.interpolation().id();
        commit(&mut boxed);
        // The fraction spreads the edges over one more column and row.
        assert_eq!(bounds(&mut boxed), json!([100, 100, 301, 201]), "{depth} bit");

        let mut direct = harness(depth);
        direct.state_mut().run("edit.transform", json!({"rect": [100, 100, 400, 300], "quad": quad, "interpolation": interpolation})).unwrap();
        assert!(pixels(&boxed) == pixels(&direct), "{depth} bit: the box committed the quad it showed");
    }
}
