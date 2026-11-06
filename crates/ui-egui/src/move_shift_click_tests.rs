//! #11102: Move Auto-Select, Layer, measured selected-ID sets (author comment 6616635118).
//! Photoshop's scripting activeLayer can stay nonempty when its target-layer list is empty.

use egui::Modifiers;
use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::{DialogKind, Tool};

struct Fixture {
    app: PhotocraftApp,
    background: LayerId,
    layers: [LayerId; 3],
}

impl Fixture {
    fn new() -> Self {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 80, "height": 60})).unwrap();
        app.sync_views();
        let background = app.session.active().unwrap().doc.background().unwrap().id;
        let layers = [("A", 8), ("B", 32), ("C", 56)].map(|(name, x)| {
            let id = LayerId(app.run("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap());
            app.run("select.rect", json!({"x": x, "y": 8, "width": 12, "height": 12})).unwrap();
            app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
            id
        });
        app.run("select.deselect", json!({})).unwrap();
        app.ui.tool = Tool::Move;
        app.ui.tool_options.move_auto_select = true;
        app.ui.tool_options.move_target = "layer".into();
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        Self { app, background, layers }
    }

    fn select(&mut self, ids: &[LayerId]) {
        self.app.run("select.deselectLayers", json!({})).unwrap();
        for id in ids {
            self.app.run("layer.select", json!({"layer": id.0, "mode": "add"})).unwrap();
        }
    }

    fn assert_selected(&self, expected: &[LayerId]) {
        let mut actual = self.app.session.active().unwrap().selected_layers();
        let mut expected = expected.to_vec();
        actual.sort_unstable_by_key(|id| id.0);
        expected.sort_unstable_by_key(|id| id.0);
        assert_eq!(actual, expected);
    }

    fn bounds(&self) -> Vec<Rect> {
        let doc = &self.app.session.active().unwrap().doc;
        std::iter::once(self.background).chain(self.layers).map(|id| doc.layer(id).unwrap().surface().unwrap().content_bounds()).collect()
    }

    fn gesture(&mut self, from: [f64; 2], to: [f64; 2], mods: Modifiers) {
        tool_event(&mut self.app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, mods);
        if from != to {
            tool_event(&mut self.app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, mods);
        }
        tool_event(&mut self.app, ToolEvent::Up { x: to[0], y: to[1] }, mods);
        assert!(self.app.layer_box.is_none(), "release ends the marquee");
        assert!(self.app.ui.dialogs.iter().all(|d| d.kind != DialogKind::Error), "no move-lock dialog");
        assert!(!self.app.ui.status_error, "{}", self.app.ui.status);
    }

    fn convert_background(&mut self) {
        self.select(&[self.background]);
        self.app.run("layer.new.layerFromBackground", json!({})).unwrap();
        assert!(!self.app.session.active().unwrap().doc.is_background(self.background));
    }
}

#[test]
fn point_clicks_toggle_members_but_keep_the_singleton_and_plain_multi_selection() {
    let mut f = Fixture::new();
    let [a, b, c] = f.layers;
    let before = f.bounds();
    f.select(&[b]);
    f.gesture([14.0, 14.0], [14.0, 14.0], Modifiers::NONE);
    f.assert_selected(&[a]);
    f.gesture([38.0, 14.0], [38.0, 14.0], Modifiers::SHIFT);
    f.assert_selected(&[a, b]);
    f.gesture([62.0, 14.0], [62.0, 14.0], Modifiers::SHIFT);
    f.assert_selected(&[a, b, c]);
    f.gesture([38.0, 14.0], [38.0, 14.0], Modifiers::SHIFT);
    f.assert_selected(&[a, c]);

    f.select(&[a]);
    f.gesture([14.0, 14.0], [14.0, 14.0], Modifiers::SHIFT);
    f.assert_selected(&[a]);
    f.select(&[a, b, c]);
    f.gesture([14.0, 14.0], [14.0, 14.0], Modifiers::NONE);
    f.assert_selected(&[a, b, c]);

    f.select(&[b]);
    f.gesture([14.0, 14.0], [14.0, 14.0], Modifiers::COMMAND);
    f.assert_selected(&[b]);
    assert_eq!(f.bounds(), before, "selection-only gestures move no pixels");
}

#[test]
fn locked_only_click_clears_each_measured_selection_and_shift_keeps_its_base() {
    let mut f = Fixture::new();
    let [a, b, c] = f.layers;
    let before = f.bounds();
    for selected in [vec![a], vec![a, b, c], vec![f.background]] {
        f.select(&selected);
        f.gesture([74.0, 40.0], [74.0, 40.0], Modifiers::NONE);
        f.assert_selected(&[]);
    }
    f.select(&[a, b]);
    f.gesture([74.0, 40.0], [74.0, 40.0], Modifiers::SHIFT);
    f.assert_selected(&[a, b]);
    assert_eq!(f.bounds(), before, "the Background never moves");

    // B's pixels are above the still-locked Background, with no other movable layer there.
    f.app.run("layer.setProps", json!({"layer": b.0, "locks": {"all": true}})).unwrap();
    f.select(&[a]);
    f.gesture([38.0, 14.0], [38.0, 14.0], Modifiers::NONE);
    f.assert_selected(&[]);
    assert_eq!(f.bounds(), before, "the all-locked layer and Background stay put");
}

#[test]
fn converted_background_point_pick_obeys_position_and_all_but_not_image_pixels_lock() {
    for (locks, selectable) in [(json!({}), true), (json!({"all": true}), false), (json!({"position": true}), false), (json!({"pixels": true}), true)] {
        let mut f = Fixture::new();
        f.convert_background();
        f.app.run("layer.setProps", json!({"layer": f.background.0, "locks": locks})).unwrap();
        f.select(&[f.layers[0]]);
        let before = f.bounds();
        f.gesture([74.0, 40.0], [74.0, 40.0], Modifiers::NONE);
        f.assert_selected(if selectable { std::slice::from_ref(&f.background) } else { &[] });
        assert_eq!(f.bounds(), before);
    }

    let mut f = Fixture::new();
    f.convert_background();
    f.select(&[f.layers[0]]);
    f.gesture([74.0, 40.0], [77.0, 42.0], Modifiers::NONE);
    f.assert_selected(&[f.background]);
    assert_eq!(f.bounds()[0], Rect::new(3, 2, 83, 62), "an unlocked converted Background can move");
}

#[test]
fn locked_top_layer_is_clicked_through_while_the_context_menu_still_lists_it() {
    for locks in [json!({"position": true}), json!({"all": true})] {
        let mut f = Fixture::new();
        f.convert_background();
        let a = f.layers[0];
        f.app.run("layer.setProps", json!({"layer": a.0, "locks": locks})).unwrap();
        f.select(&[f.layers[1]]);
        let before = f.bounds();
        let list = f.app.run("layer.pickAt", json!({"x": 14, "y": 14, "list": true})).unwrap();
        assert_eq!(list["layers"][0]["layer"], a.0);
        f.gesture([14.0, 14.0], [14.0, 14.0], Modifiers::NONE);
        f.assert_selected(&[f.background]);
        assert_eq!(f.bounds(), before);
    }
}

#[test]
fn background_marquee_adds_with_shift_and_replaces_without_it_including_empty_hits() {
    let mut f = Fixture::new();
    let [a, b, c] = f.layers;
    let before = f.bounds();
    for (initial, from, to, mods, expected) in [
        (vec![a], [28.0, 4.0], [70.0, 24.0], Modifiers::SHIFT, vec![a, b, c]),
        (vec![a, b], [28.0, 4.0], [70.0, 24.0], Modifiers::SHIFT, vec![a, b, c]),
        (vec![a], [4.0, 4.0], [22.0, 24.0], Modifiers::SHIFT, vec![a]),
        (vec![a], [4.0, 32.0], [70.0, 48.0], Modifiers::SHIFT, vec![a]),
        (vec![a], [28.0, 4.0], [70.0, 24.0], Modifiers::NONE, vec![b, c]),
        (vec![a], [4.0, 32.0], [70.0, 48.0], Modifiers::NONE, vec![]),
        (vec![], [-4.0, -4.0], [70.0, 24.0], Modifiers::NONE, vec![a, b, c]),
    ] {
        f.select(&initial);
        f.gesture(from, to, mods);
        f.assert_selected(&expected);
        assert_eq!(f.bounds(), before, "marquees select without moving pixels");
    }
}
