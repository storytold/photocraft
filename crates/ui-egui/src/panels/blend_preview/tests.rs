//! #970: hovering a blend mode in the Layers panel previews it on the canvas; only a choice
//! records a history step.

use photocraft_color::BlendMode;
use photocraft_doc::LayerId;
use serde_json::json;

use super::{damage, display_doc, hover};
use crate::PhotocraftApp;

/// Background and a filled layer `top` (active).
fn session() -> (photocraft_engine::Session, LayerId) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let top = s.execute("layer.new.layer", json!({"name": "top"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    s.execute("edit.fill", json!({"color": "#cc3366"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    (s, LayerId(top))
}

fn blend(app: &PhotocraftApp, id: LayerId) -> BlendMode {
    app.session.active().unwrap().doc.layer(id).unwrap().blend
}

fn steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

#[test]
fn a_hovered_mode_shows_on_the_canvas_until_the_pointer_leaves() {
    let (s, top) = session();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    let (undo, revision) = (steps(&app), app.session.active().unwrap().revision);
    hover(&mut app, &[top], Some(BlendMode::Multiply));
    let (shown, key) = display_doc(&mut app, 0).expect("previewed");
    assert_eq!(shown.layer(top).unwrap().blend, BlendMode::Multiply);
    let (again, same) = display_doc(&mut app, 0).unwrap();
    assert!(std::sync::Arc::ptr_eq(&shown, &again) && key == same, "one document per hovered mode, not one per frame");
    assert_eq!((blend(&app, top), steps(&app)), (BlendMode::Normal, undo), "hovering records nothing");
    // From the document to the preview and between previews: only the layer's area redraws.
    let doc = app.session.active().unwrap().doc.id;
    assert_eq!(damage(&app, doc, revision, 0, key).map(|r| r.width()), Some(16));
    hover(&mut app, &[top], Some(BlendMode::Screen));
    let (_, screen) = display_doc(&mut app, 0).unwrap();
    assert_ne!(screen, key);
    assert!(damage(&app, doc, revision, key, screen).is_some());
    // The current mode, or no mode, shows the document; and so does a list no longer drawn.
    hover(&mut app, &[top], Some(BlendMode::Normal));
    assert!(display_doc(&mut app, 0).is_none());
    hover(&mut app, &[top], None);
    assert!(display_doc(&mut app, 0).is_none());
    assert!(damage(&app, doc, revision, screen, 0).is_some(), "back to the document: the layer's area");
    hover(&mut app, &[top], Some(BlendMode::Multiply));
    app.frame += 2;
    assert!(display_doc(&mut app, 0).is_none(), "a stale hover previews nothing");
}

#[test]
fn a_hovered_mode_previews_on_every_selected_layer() {
    let (mut s, top) = session();
    let other = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
    s.execute("edit.fill", json!({"color": "#3366cc"})).unwrap();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    hover(&mut app, &[top, other], Some(BlendMode::Multiply));
    let (shown, key) = display_doc(&mut app, 0).expect("previewed");
    assert_eq!((shown.layer(top).unwrap().blend, shown.layer(other).unwrap().blend), (BlendMode::Multiply, BlendMode::Multiply));
    // A different set of layers is a different preview.
    hover(&mut app, &[top], Some(BlendMode::Multiply));
    assert_ne!(display_doc(&mut app, 0).unwrap().1, key);
}

#[test]
fn blend_hover_disjoint_coverage_retains_views_without_hiding_gaps() {
    let (s, top) = session();
    let mut app = PhotocraftApp::new(s, crate::Services::default());
    hover(&mut app, &[top], Some(BlendMode::Multiply));
    let (_, key) = display_doc(&mut app, 0).unwrap();
    let st = app.session.active().unwrap();
    let (id, revision, region) = (st.doc.id, st.revision, st.doc.bounds());
    let a = photocraft_geom::Rect::new(0, 0, 2, 2);
    let b = photocraft_geom::Rect::new(4, 0, 6, 2);
    assert_eq!(super::clip_damage(&mut app, id, revision, key, region, a), a);
    assert_eq!(super::uncovered(&mut app, id, revision, key, b), Some(b));
    assert_eq!(super::uncovered(&mut app, id, revision, key, a), None);
    let gap = photocraft_geom::Rect::new(2, 0, 4, 2);
    assert_eq!(super::uncovered(&mut app, id, revision, key, gap), Some(gap));
    // Repeated distinct views have bounded bookkeeping, then the whole region is current.
    let mut fell_back = false;
    for x in (8..40).step_by(4) {
        let visible = photocraft_geom::Rect::new(x, 0, x + 2, 2);
        if super::uncovered(&mut app, id, revision, key, visible) == Some(region) {
            fell_back = true;
            break;
        }
    }
    assert!(fell_back);
    assert_eq!(super::uncovered(&mut app, id, revision, key, region), None);
    hover(&mut app, &[top], Some(BlendMode::Screen));
    let (_, next) = display_doc(&mut app, 0).unwrap();
    assert_eq!(super::clip_damage(&mut app, id, revision, next, region, a), a);
    assert_eq!(super::uncovered(&mut app, id, revision, next, b), Some(b));
    hover(&mut app, &[top], None);
    assert_eq!(super::clip_damage(&mut app, id, revision, 0, region, a), region);
}
