//! #12555: a Remove stroke on a Smart Object layer opens no rasterize prompt. Its fill goes to a
//! new normal layer "Remove tool edits" directly above the smart object, which keeps its pixels,
//! with the options bar's Create new layer off as well as on; the first such removal explains the
//! new layer in a notice card that stays until closed, and its Don't show again answer is kept.
//! Both Remove modes go through this: filled at each release, and collected for the options bar's ✓.
//! The removal itself runs as a background job on the desktop, so the card is checked both there and
//! inline (the web build and the shell's own tests).

use craft::keymap::Mods;
use egui::accesskit::Role;
use egui::{Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
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

/// A hidden Smart Object never gets that far: the canvas refuses a painting press on a hidden layer
/// before the rasterize question is asked, so no prompt, no edits layer and no history state (#12554).
#[test]
fn a_press_on_a_hidden_smart_object_is_refused_before_any_question() {
    for after_each in [true, false] {
        let mut app = app_with(Kind::SmartObject);
        let stack = layers(&app);
        let (bg, smart) = (stack[0], stack[1]);
        app.run("layer.setProps", json!({"layer": smart.0, "visible": false})).unwrap();
        app.ui.tool = Tool::Remove;
        app.ui.tool_options.remove_after_each_stroke = after_each;
        let steps = past(&app);
        drag(&mut app, 40.0);
        assert!(prompt(&app).is_none(), "{after_each}: no rasterize question for a hidden layer");
        assert!(app.framework.dialogs.is_empty(), "{after_each}: {:?}", app.framework.dialogs.open_dialogs());
        assert!(app.remove_pending.is_empty(), "{after_each}: no stroke is collected");
        assert_eq!(layers(&app), vec![bg, smart], "{after_each}: no edits layer");
        assert_eq!(past(&app), steps, "{after_each}: no history state");
        assert_eq!(kind(&app, smart), "Smart Object");
    }
}

/// The whole app, so the shell's passes run: the outbox goes to the window's toast queue and a
/// background job lands in `jobs_ui`. `background` is whether job-capable commands run on a worker
/// thread, as they do on the desktop.
fn app_harness(background: bool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 120, "height": 80})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default());
        app.background_jobs = background;
        app
    });
    h.run_steps(2);
    h
}

/// One Remove stroke across the smart object, through the tool's own finish.
fn stroke(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    retouch_ui::finish_stroke(h.state_mut(), Tool::Remove, &[[30.0, 40.0, 1.0], [90.0, 40.0, 1.0]], Mods::default(), false);
}

/// Until the removal has landed: with background jobs on, the test waits for the job's worker
/// (`Session::wait_job`, which applies its edit), and the next frames' `jobs_ui::tick` takes the
/// job's end, which posts the card.
fn wait_until_landed(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    for job in h.state().session.jobs() {
        let _ = h.state_mut().session.wait_job(job.id);
    }
    h.run_steps(3);
}

/// The notice still waiting in the app's outbox, before a pass hands it to the window.
fn posted<'a>(h: &'a Harness<'static, Box<PhotocraftApp>>) -> Vec<&'a crate::notices::Notice> {
    h.state().ui.notices.iter().filter(|notice| notice.title == retouch_ui::EDITS_NOTICE).collect()
}

/// The titles up: what the app has posted and the window's toast cards.
fn shown(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<String> {
    crate::notices::shown(h.state(), &h.ctx).into_iter().map(|(title, _, _)| title).collect()
}

/// How many cards carry the notice's text.
fn cards(h: &Harness<'static, Box<PhotocraftApp>>) -> usize {
    shown(h).iter().filter(|title| *title == retouch_ui::EDITS_NOTICE).count()
}

/// The card's Don't show again button: the framework sets its words in the viewport's language, with
/// a typesetter's apostrophe, so the label is compared with that folded back.
fn dismiss_button<'a>(h: &'a Harness<'static, Box<PhotocraftApp>>) -> egui_kittest::Node<'a> {
    let want = "Don't show again";
    let find = |role: Role| h.query_all_by_role(role).find(|n| n.accesskit_node().label().is_some_and(|l| l.replace('\u{2019}', "'") == want));
    find(Role::Button).or_else(|| find(Role::Link)).unwrap_or_else(|| panic!("the notice card offers {want}"))
}

/// A document whose active layer is a filled pixel layer, for what a plain Create new layer removal
/// does and does not say.
fn pixel_harness(background: bool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 120, "height": 80})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#00ff00"})).unwrap();
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default());
        app.background_jobs = background;
        app
    });
    h.run_steps(2);
    h
}

#[test]
fn the_first_removal_explains_the_new_layer_and_the_notice_stays_until_closed() {
    let mut h = app_harness(false);
    let smart = layers(h.state())[1];
    stroke(&mut h);
    let posted = posted(&h);
    assert_eq!(posted.len(), 1, "the inline removal posts one notice about the new layer");
    assert!(posted[0].sticky, "it stays until closed");
    assert_eq!(posted[0].dismiss_pref.as_deref(), Some(retouch_ui::EDITS_NOTICE_PREF), "its card offers Don't show again");
    h.run_steps(4);
    assert_eq!(cards(&h), 1, "the card is on screen: {:?}", shown(&h));
    // Sticky: later passes keep it, and a second removal adds no second card.
    h.run_steps(4);
    assert_eq!(cards(&h), 1, "the card stays until closed");
    h.state_mut().run("layer.select", json!({"layer": smart.0})).unwrap();
    h.run_steps(2);
    stroke(&mut h);
    h.run_steps(4);
    assert_eq!(cards(&h), 1, "the card is not stacked: {:?}", shown(&h));
    assert_eq!(kind(h.state(), smart), "Smart Object", "both removals left the smart object intact");
}

/// The shipping desktop path: `paint.remove` runs as a background job, so the stroke itself has no
/// result to read — the card comes when the job's result lands.
#[test]
fn the_notice_card_arrives_when_the_removal_runs_as_a_background_job() {
    for after_each in [true, false] {
        let mut h = app_harness(true);
        h.state_mut().ui.tool = Tool::Remove;
        h.state_mut().ui.tool_options.remove_after_each_stroke = after_each;
        let before = layers(h.state());
        let smart = before[1];
        if after_each {
            stroke(&mut h);
        } else {
            retouch_ui::remove_pending_add(h.state_mut(), &[[30.0, 40.0, 1.0], [90.0, 40.0, 1.0]]);
            assert!(retouch_ui::remove_pending_commit(h.state_mut()), "{after_each}: the ✓ applies the waiting strokes");
        }
        assert_eq!(layers(h.state()), before, "{after_each}: the removal is still running as a job");
        wait_until_landed(&mut h);
        let after = layers(h.state());
        assert_eq!(after.len(), before.len() + 1, "{after_each}: the job added the edits layer");
        assert_eq!(name(h.state(), *after.last().unwrap()), "Remove tool edits", "{after_each}");
        assert_eq!(kind(h.state(), smart), "Smart Object", "{after_each}: the smart object kept its pixels");
        assert_eq!(cards(&h), 1, "{after_each}: the job's end brings the card: {:?}", shown(&h));
        assert!(!h.state().ui.status_error, "{after_each}: {}", h.state().ui.status);
    }
}

/// The card's Don't show again answer is the notice's dismiss key in the dialog preferences, and
/// with it stored a later removal brings no card back — the removal itself goes on working.
#[test]
fn dont_show_again_on_the_card_keeps_later_removals_quiet() {
    let mut h = app_harness(false);
    let smart = layers(h.state())[1];
    stroke(&mut h);
    h.run_steps(4);
    assert_eq!(cards(&h), 1, "the first removal brings the card");
    dismiss_button(&h).click();
    h.run_steps(4);
    assert_eq!(cards(&h), 0, "the card is gone once it was answered");
    assert_eq!(h.state().session.prefs().dialogs.get(retouch_ui::EDITS_NOTICE_PREF), Some(&json!(true)), "the answer stored the dismiss key");
    h.state_mut().run("layer.select", json!({"layer": smart.0})).unwrap();
    h.run_steps(2);
    stroke(&mut h);
    h.run_steps(4);
    assert_eq!(cards(&h), 0, "a later removal stays quiet: {:?}", shown(&h));
    // The removal itself goes on working: the smart object was picked again, so its second stroke
    // adds a second numbered edits layer above it.
    let after = layers(h.state());
    assert_eq!(after.len(), 4, "two removals, two edits layers: {after:?}");
    assert_eq!(after[1], smart, "the smart object is where it was");
    assert_eq!(name(h.state(), after[2]), "Remove tool edits 1", "the newer layer sits directly above the smart object");
    assert_eq!(name(h.state(), after[3]), "Remove tool edits");
    assert_eq!(kind(h.state(), smart), "Smart Object");
}

/// A pixel layer with Create new layer on keeps today's behaviour: the fill goes to a new layer and
/// nothing is noticed about it, inline or as a job.
#[test]
fn a_removal_on_a_pixel_layer_with_create_new_layer_notices_nothing() {
    for background in [false, true] {
        let mut h = pixel_harness(background);
        let smart_free = kind(h.state(), layers(h.state())[1]);
        assert_eq!(smart_free, "Pixel", "the harness document has no Smart Object layer");
        h.state_mut().ui.tool = Tool::Remove;
        h.state_mut().ui.tool_options.remove_new_layer = true;
        let before = layers(h.state());
        stroke(&mut h);
        wait_until_landed(&mut h);
        let after = layers(h.state());
        assert_eq!(after.len(), before.len() + 1, "background {background}: Create new layer adds the edits layer");
        assert_eq!(name(h.state(), *after.last().unwrap()), "Remove tool edits", "background {background}");
        assert_eq!(cards(&h), 0, "background {background}: no notice for a pixel layer: {:?}", shown(&h));
    }
}
