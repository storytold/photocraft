//! #12555: a Remove stroke on a Smart Object layer opens no rasterize prompt. Its fill goes to a
//! new normal layer "Remove tool edits" directly above the smart object, which keeps its pixels,
//! with the options bar's Create new layer off as well as on; the first such removal explains the
//! new layer in a notice that stays until closed, and Do not show again is remembered. Both Remove
//! modes go through this: filled at each release, and collected for the options bar's ✓.

use craft::keymap::Mods;
use egui::{Modifiers, vec2};
use egui_kittest::Harness;
use serde_json::json;

use super::Kind;
use super::tests::{app_with, past, prompt};
use crate::PhotocraftApp;
use crate::canvas::testing::{ToolEvent, tool_event};
use crate::retouch_ui;
use crate::state::Tool;
use photocraft_doc::LayerId;

/// A drag across the middle of the canvas, as a Remove stroke.
fn drag(app: &mut PhotocraftApp, y: f64) {
    tool_event(app, ToolEvent::Down { x: 30.0, y, pressure: 1.0 }, Modifiers::NONE);
    for i in 1..=12 {
        tool_event(app, ToolEvent::Move { x: 30.0 + f64::from(i) * 5.0, y, pressure: 1.0 }, Modifiers::NONE);
    }
    tool_event(app, ToolEvent::Up { x: 90.0, y }, Modifiers::NONE);
}

fn layers(app: &PhotocraftApp) -> Vec<LayerId> {
    app.session.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l.id).collect()
}

fn name(app: &PhotocraftApp, id: LayerId) -> String {
    app.session.active().unwrap().doc.layer(id).unwrap().name.clone()
}

fn kind(app: &PhotocraftApp, id: LayerId) -> &'static str {
    app.session.active().unwrap().doc.layer(id).unwrap().content.kind_name()
}

/// The Smart Object's own pixels: its rendered cache.
fn cache(app: &PhotocraftApp, id: LayerId) -> photocraft_raster::Surface {
    app.session.active().unwrap().doc.layer(id).unwrap().surface().expect("the smart object holds rendered pixels").clone()
}

#[test]
fn a_remove_stroke_on_a_smart_object_asks_nothing_and_lands_on_the_edits_layer() {
    for after_each in [true, false] {
        let mut app = app_with(Kind::SmartObject);
        app.ui.tool = Tool::Remove;
        app.ui.tool_options.remove_after_each_stroke = after_each;
        let stack = layers(&app);
        let (bg, smart) = (stack[0], stack[1]);
        let before = cache(&app, smart);
        let steps = past(&app);
        drag(&mut app, 40.0);
        assert!(prompt(&app).is_none(), "{after_each}: the Remove tool asks nothing on a smart object");
        assert!(app.framework.dialogs.is_empty(), "{after_each}: {:?}", app.framework.dialogs.open_dialogs());
        if !after_each {
            assert_eq!(layers(&app), vec![bg, smart], "{after_each}: the strokes wait for the options bar");
            assert!(retouch_ui::remove_pending_commit(&mut app), "{after_each}: the ✓ applies the waiting strokes");
        }
        let after = layers(&app);
        assert_eq!(after, vec![bg, smart, *after.last().unwrap()], "{after_each}: one new layer, directly above the smart object");
        let edits = after[2];
        assert_eq!(name(&app, edits), "Remove tool edits", "{after_each}");
        assert_eq!(kind(&app, edits), "Pixel", "{after_each}: a normal layer");
        assert_eq!(kind(&app, smart), "Smart Object", "{after_each}: the smart object stays editable");
        assert_eq!(cache(&app, smart), before, "{after_each}: its pixels are untouched");
        let filled = app.session.active().unwrap().doc.layer(edits).unwrap().surface().unwrap().rgba(60, 40);
        assert!(filled[1] > 0.5 && filled[0] < 0.5, "{after_each}: the fill, sampled from the smart object, is on the new layer: {filled:?}");
        assert_eq!(past(&app), steps + 1, "{after_each}: one history state");
        assert!(!app.ui.status_error, "{after_each}: {}", app.ui.status);
    }
}

/// The whole app, so the shell's pass hands the outbox to the window's toast queue: a document
/// whose active layer is a Smart Object over a filled pixel layer.
fn app_harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(32).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 120, "height": 80})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), s, Default::default());
        app.background_jobs = false;
        app
    });
    h.run_steps(2);
    h
}

/// One Remove stroke across the smart object, through the tool's own finish.
fn stroke(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    retouch_ui::finish_stroke(h.state_mut(), Tool::Remove, &[[30.0, 40.0, 1.0], [90.0, 40.0, 1.0]], Mods::default(), false);
    h.run_steps(4);
}

/// The notice still waiting in the app's outbox, before a pass hands it to the window.
fn posted<'a>(h: &'a Harness<'static, Box<PhotocraftApp>>) -> Vec<&'a crate::notices::Notice> {
    h.state().ui.notices.iter().filter(|notice| notice.title == retouch_ui::EDITS_NOTICE).collect()
}

/// The titles up: what the app has posted and the window's toast cards.
fn shown(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<String> {
    crate::notices::shown(h.state(), &h.ctx).into_iter().map(|(title, _, _)| title).collect()
}

#[test]
fn the_first_removal_explains_the_new_layer_and_the_notice_stays_until_closed() {
    let mut h = app_harness();
    let smart = layers(h.state())[1];
    retouch_ui::finish_stroke(h.state_mut(), Tool::Remove, &[[30.0, 40.0, 1.0], [90.0, 40.0, 1.0]], Mods::default(), false);
    let posted = posted(&h);
    assert_eq!(posted.len(), 1, "the removal posts one notice about the new layer");
    assert!(posted[0].sticky, "it stays until closed");
    assert_eq!(posted[0].dismiss_pref.as_deref(), Some(retouch_ui::EDITS_NOTICE_PREF), "its card offers Do not show again");
    h.run_steps(4);
    assert_eq!(shown(&h).iter().filter(|title| *title == retouch_ui::EDITS_NOTICE).count(), 1, "the card is on screen: {:?}", shown(&h));
    // Sticky: the later passes keep it, and a second removal adds no second card.
    h.run_steps(4);
    assert_eq!(shown(&h).iter().filter(|title| *title == retouch_ui::EDITS_NOTICE).count(), 1, "the card stays until closed");
    h.state_mut().run("layer.select", json!({"layer": smart.0})).unwrap();
    h.run_steps(2);
    stroke(&mut h);
    assert_eq!(shown(&h).iter().filter(|title| *title == retouch_ui::EDITS_NOTICE).count(), 1, "the card is not stacked: {:?}", shown(&h));
    assert_eq!(kind(h.state(), smart), "Smart Object", "both removals left the smart object intact");
}

/// The answer the card's Do not show again writes is the notice's dismiss key, stored in the dialog
/// preferences; with it stored, later removals bring no card back.
#[test]
fn do_not_show_again_keeps_the_notice_away_from_later_removals() {
    let mut h = app_harness();
    let smart = layers(h.state())[1];
    h.state_mut().session.prefs.edit(|prefs| {
        prefs.dialogs.insert(retouch_ui::EDITS_NOTICE_PREF.into(), serde_json::Value::Bool(true));
    });
    stroke(&mut h);
    assert!(!shown(&h).iter().any(|title| title == retouch_ui::EDITS_NOTICE), "no card: {:?}", shown(&h));
    // The removal itself is unaffected by the silenced notice.
    let after = layers(h.state());
    assert_eq!(after.len(), 3, "{after:?}: the edits layer still lands above the smart object");
    assert_eq!(name(h.state(), after[2]), "Remove tool edits");
    assert_eq!(kind(h.state(), smart), "Smart Object");
}

/// A pixel layer with Create new layer on keeps today's behaviour: the fill goes to a new layer and
/// nothing is noticed about it.
#[test]
fn a_removal_on_a_pixel_layer_with_create_new_layer_notices_nothing() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
    app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
    app.ui.tool = Tool::Remove;
    app.ui.tool_options.remove_new_layer = true;
    let before = layers(&app);
    drag(&mut app, 40.0);
    assert_eq!(layers(&app).len(), before.len() + 1, "Create new layer adds the edits layer");
    assert_eq!(name(&app, layers(&app)[1]), "Remove tool edits");
    assert!(app.ui.notices.iter().all(|notice| notice.title != retouch_ui::EDITS_NOTICE), "no notice for a pixel layer: {:?}", app.ui.notices);
}
