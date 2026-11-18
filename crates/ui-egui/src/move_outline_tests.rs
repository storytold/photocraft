//! #2413: View › Show › Layer Edges while the Move tool drags. As in Photoshop, the outline belongs
//! to the layer being moved (the one Auto-Select picks on the press, not the one selected before)
//! and follows its pixels.

use craft::behavior::kurbo::{self, Shape};
use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::layer_edges;
use crate::canvas::testing::{frame, move_to, press, release};
use crate::state::{Tool, View};

/// A 64×64 document with two painted layers: A covers (8..24)², B covers (40..56)². B is active,
/// the Move tool (Auto-Select on) is current and View › Show › Layer Edges is on.
fn app() -> (PhotocraftApp, LayerId, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    let mut ids = Vec::new();
    for at in [8, 40] {
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session
            .edit("paint", |doc, a| {
                doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(at, at, at + 16, at + 16), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        ids.push(app.session.active().unwrap().active_layer.unwrap());
    }
    app.ui.tool = Tool::Move;
    assert!(app.ui.tool_options.move_auto_select);
    // Snapping would shift the expected offsets.
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app.ui.extras.rulers = false;
    app.ui.view.extras = true;
    app.ui.view.show.layer_edges = true;
    // 4 screen px per document px, the document centred.
    app.ui.views = vec![View {
        zoom: 4.0,
        center: [32.0, 32.0],
        pending: Vec::new(),
        doc_size: [64, 64],
        rotation: 0.0,
        zoom_animating: false,
        zoom_anchor: None,
        zoom_from: None,
    }];
    (app, ids[0], ids[1])
}

/// The bounds of the Layer Edges outline of document 0.
fn edges(app: &PhotocraftApp) -> Option<kurbo::Rect> {
    layer_edges(app, 0).map(|p| p.bounding_box())
}

/// The outline of document rect `r`.
fn outline(r: Rect) -> Option<kurbo::Rect> {
    Some(kurbo::Rect::new(f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1)))
}

fn active(app: &PhotocraftApp) -> Option<LayerId> {
    app.session.active().unwrap().active_layer
}

const A: Rect = Rect::new(8, 8, 24, 24);
const B: Rect = Rect::new(40, 40, 56, 56);

#[test]
fn the_outline_belongs_to_the_layer_auto_select_picked() {
    let (mut app, a, b) = app();
    let none = egui::Modifiers::NONE;
    frame(&mut app);
    assert_eq!(active(&app), Some(b));
    assert_eq!(edges(&app), outline(B), "B is selected: its edges show");
    // Press on A and drag it 5 px to the right.
    press(&mut app, [16.0, 16.0], none);
    move_to(&mut app, [21.0, 16.0], none);
    // The frame the drag starts in picks A: it outlines A where its pixels show, 5 px along, never B, which isn't moving.
    assert_eq!(active(&app), Some(a), "Auto-Select picked A");
    assert_eq!(edges(&app), outline(A.translate(5, 0)), "the frame that picks A outlines A, not the previously selected B");
    frame(&mut app);
    assert_eq!(edges(&app), outline(A.translate(5, 0)), "A's outline follows it");
    release(&mut app, [21.0, 16.0], none);
    frame(&mut app);
    assert_eq!(edges(&app), outline(A.translate(5, 0)), "after the release the outline stays on A where it landed");
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(a).unwrap().surface().unwrap().content_bounds(), A.translate(5, 0));
}
