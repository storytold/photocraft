use egui_kittest::{Harness, kittest::Queryable};

use super::*;
use crate::smart_filter_dialog::tests::{cancel, confirm, newest, pass, set};

/// An app with a 64×48 document.
fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app
}

/// The real app with a 64×48 document and the strip open.
fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        let theme = crate::theme::AppTheme::pro();
        PhotocraftApp::setup_context(&cc.egui_ctx, theme.clone());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default());
        app.ui.theme = theme;
        app
    });
    h.state_mut().run("window.panel.designTimeline", json!({})).unwrap();
    h.run_steps(6);
    h
}

/// The active document's timeline.
fn timeline(app: &PhotocraftApp) -> Option<Arc<DesignTimeline>> {
    app.session.active().and_then(|d| d.doc.design_timeline.clone())
}

/// Edit Parameters on step `i`, returning its dialog.
fn open_edit(app: &mut PhotocraftApp, i: usize) -> DialogId {
    edit(app, i).unwrap();
    pass(app);
    newest(app)
}

#[test]
fn start_record_roll_back_and_forward() {
    let mut h = harness();
    h.get_all_by_label("Start Design Timeline").next().expect("the strip's Start button").click();
    h.run_steps(2);
    assert!(timeline(h.state()).is_some());
    h.state_mut().run("layer.new.layer", json!({"name": "A"})).unwrap();
    h.state_mut().run("layer.new.layer", json!({"name": "B"})).unwrap();
    h.run_steps(2);
    let labels: Vec<String> = timeline(h.state()).unwrap().steps.iter().map(|s| s.label.clone()).collect();
    assert_eq!(labels.len(), 2);
    // Clicking the first step rolls the marker to just after it.
    h.get_all_by_label(&labels[0]).next().unwrap().click();
    h.run_steps(2);
    assert_eq!(timeline(h.state()).unwrap().in_effect(), 1);
    h.get_all_by_label("Roll to End").next().unwrap().click();
    h.run_steps(2);
    assert_eq!(timeline(h.state()).unwrap().in_effect(), 2);
}

#[test]
fn a_failed_step_shows_why() {
    let mut a = app();
    a.run("design.start", json!({})).unwrap();
    a.run("layer.new.layer", json!({"name": "A"})).unwrap();
    a.run("layer.setProps", json!({"opacity": 0.5})).unwrap();
    let first = timeline(&a).unwrap().steps[0].step;
    a.run("design.suppress", json!({"step": first})).unwrap();
    let tl = timeline(&a).unwrap();
    assert!(tl.steps[1].state.is_failure());
    let (why, failure) = describe(&tl.steps[1].state);
    assert_eq!(failure, Some(true));
    assert!(spoken(&tl.steps[1]).ends_with(&why.unwrap()), "the accessible name says why, not only the colour");
}

#[test]
fn a_step_preview_renders_from_its_checkpoint() {
    let mut a = app();
    a.run("design.start", json!({})).unwrap();
    a.run("layer.new.layer", json!({"name": "A"})).unwrap();
    let tl = timeline(&a).unwrap();
    let ctx = egui::Context::default();
    assert!(thumb(&ctx, &tl, &tl.steps[0]).is_none(), "the first request starts the render");
    for _ in 0..500 {
        if let Some(tex) = thumb(&ctx, &tl, &tl.steps[0]) {
            assert_eq!(tex.size(), [64, 48], "small documents preview at their own size");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the preview never rendered");
}

/// A cached preview is reused only for the checkpoint it shows, never for another at its address.
#[test]
fn a_step_preview_is_not_reused_for_another_checkpoint() {
    let mut a = app();
    a.run("design.start", json!({})).unwrap();
    a.run("layer.new.layer", json!({"name": "A"})).unwrap();
    let tl = timeline(&a).unwrap();
    let st = &tl.steps[0];
    let doc = st.checkpoint.0.clone().unwrap();
    let ctx = egui::Context::default();
    let stale = ctx.load_texture("stale", egui::ColorImage::filled([1, 1], Color32::RED), egui::TextureOptions::LINEAR);
    let thumbs = ctx.data_mut(|d| d.get_temp_mut_or_default::<Thumbs>(egui::Id::unique("design_timeline_thumbs")).clone());
    thumbs.lock().unwrap().insert((st.step, Arc::as_ptr(&doc) as usize), (std::sync::Weak::new(), Some(stale)));
    assert!(thumb(&ctx, &tl, st).is_none(), "a new render starts");
}

#[test]
fn edit_parameters_replays_the_step_and_restores_the_marker() {
    let mut a = app();
    a.run("design.start", json!({})).unwrap();
    a.run("layer.new.layer", json!({"name": "A"})).unwrap();
    a.run("filter.noise.addNoise", json!({"amount": 10})).unwrap();
    a.run("layer.new.layer", json!({"name": "B"})).unwrap();
    assert!(has_dialog("filter.noise.addNoise"));
    let d = open_edit(&mut a, 1);
    assert_eq!(timeline(&a).unwrap().in_effect(), 1, "the dialog previews on what the step saw");
    set(&mut a, d, "amount", json!(40));
    confirm(&mut a, d);
    let tl = timeline(&a).unwrap();
    assert_eq!(tl.steps[1].params["amount"].as_f64(), Some(40.0));
    assert_eq!(tl.in_effect(), 3, "the marker goes back to the end");
    // Cancel puts the marker back too.
    let d = open_edit(&mut a, 1);
    cancel(&mut a, d);
    poll(&mut a);
    assert_eq!(timeline(&a).unwrap().in_effect(), 3);
}

/// Another document activated while the dialog is open (a background open) is left alone:
/// OK and Cancel act on the edited step's document and keep the other one active.
#[test]
fn edit_parameters_applies_to_the_document_it_opened_on() {
    let mut a = app();
    a.run("design.start", json!({})).unwrap();
    a.run("layer.new.layer", json!({"name": "A"})).unwrap();
    a.run("filter.noise.addNoise", json!({"amount": 10})).unwrap();
    a.run("layer.new.layer", json!({"name": "B"})).unwrap();
    let edited = a.session.active().unwrap().doc.id;
    let activate_other = |a: &mut PhotocraftApp, other: Option<usize>| match other {
        Some(i) => {
            a.session.set_active(i);
            a.session.active().unwrap().doc.id
        }
        None => {
            a.session.add_document(
                photocraft_doc::Document::new("Other", photocraft_geom::Size::new(8, 8), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8),
                None,
            );
            let last = a.session.documents().len() - 1;
            a.session.set_active(last);
            a.session.active().unwrap().doc.id
        }
    };
    let timeline_of = |a: &PhotocraftApp, id| a.session.documents().iter().find(|d| d.doc.id == id).and_then(|d| d.doc.design_timeline.clone()).unwrap();
    let d = open_edit(&mut a, 1);
    set(&mut a, d, "amount", json!(40));
    let other = activate_other(&mut a, None);
    let other_index = a.session.active_index();
    confirm(&mut a, d);
    assert_eq!(timeline_of(&a, edited).steps[1].params["amount"].as_f64(), Some(40.0));
    assert_eq!(timeline_of(&a, edited).in_effect(), 3);
    assert!(a.session.documents().iter().find(|d| d.doc.id == other).unwrap().doc.design_timeline.is_none(), "the other document is untouched");
    assert_eq!(a.session.active().unwrap().doc.id, other, "and stays active");
    // Cancel restores the edited document's marker, from wherever.
    let i = a.session.documents().iter().position(|d| d.doc.id == edited).unwrap();
    a.session.set_active(i);
    let d = open_edit(&mut a, 1);
    activate_other(&mut a, other_index);
    cancel(&mut a, d);
    poll(&mut a);
    assert_eq!(timeline_of(&a, edited).in_effect(), 3);
    assert_eq!(a.session.active().unwrap().doc.id, other);
}

/// Edit Parameters previews on the layer the step acted on, not the top one.
#[test]
fn edit_parameters_targets_the_steps_layer() {
    let mut a = app();
    a.run("design.start", json!({})).unwrap();
    a.run("layer.new.layer", json!({"name": "A"})).unwrap();
    let layer_a = a.session.active().unwrap().active_layer;
    a.run("layer.new.layer", json!({"name": "B"})).unwrap();
    a.run("layer.select", json!({"layer": "A"})).unwrap();
    a.run("filter.noise.addNoise", json!({"amount": 10})).unwrap();
    a.run("layer.select", json!({"layer": "B"})).unwrap();
    open_edit(&mut a, 2);
    assert_eq!(a.session.active().unwrap().active_layer, layer_a);
}

/// With another document made active (a background open), the dialog previews on neither.
#[test]
fn edit_parameters_previews_only_on_the_document_it_edits() {
    let mut a = app();
    a.run("design.start", json!({})).unwrap();
    a.run("layer.new.layer", json!({"name": "A"})).unwrap();
    a.run("edit.fillForeground", json!({})).unwrap();
    a.run("filter.noise.addNoise", json!({"amount": 10})).unwrap();
    let d = open_edit(&mut a, 2);
    assert!(!edits_other(&a, d));
    a.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let other = a.session.active_index().unwrap();
    assert!(edits_other(&a, d));
    assert_eq!(crate::canvas::ensure_filter_preview(&mut a, other, &egui::Context::default()), None);
    assert!(a.filter_preview.is_none(), "nothing computed for the other document");
}

/// An adjustment's Edit Parameters previews only on the document it edits too.
#[test]
fn an_adjustment_edit_previews_only_on_its_document() {
    let mut a = app();
    a.run("design.start", json!({})).unwrap();
    a.run("layer.new.layer", json!({"name": "A"})).unwrap();
    a.run("edit.fillForeground", json!({})).unwrap();
    a.run("image.adjustments.brightnessContrast", json!({"brightness": 40, "contrast": -10})).unwrap();
    assert!(has_dialog("image.adjustments.brightnessContrast"));
    open_edit(&mut a, 2);
    assert!(crate::adjust_preview::display_doc(&mut a, 0).is_some(), "previews on the edited document");
    a.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    a.run("layer.new.layer", json!({})).unwrap();
    let other = a.session.active_index().unwrap();
    assert!(crate::adjust_preview::display_doc(&mut a, other).is_none());
}

/// Cancelling the replay Edit Parameters started puts the marker back, as Cancel would.
#[test]
fn a_cancelled_edit_replay_puts_the_marker_back() {
    let mut a = app();
    a.run("design.start", json!({})).unwrap();
    a.run("layer.new.layer", json!({"name": "A"})).unwrap();
    a.run("edit.fillForeground", json!({})).unwrap();
    a.run("filter.noise.addNoise", json!({"amount": 10})).unwrap();
    a.run("layer.new.layer", json!({"name": "B"})).unwrap();
    let d = open_edit(&mut a, 2);
    a.background_jobs = true;
    confirm(&mut a, d);
    let doc = a.session.active().unwrap().doc.id;
    let job = a.session.job_on(doc).expect("the replay runs as a job").id;
    assert!(a.session.cancel_job(job));
    let ctx = egui::Context::default();
    for _ in 0..500 {
        crate::jobs_ui::tick(&mut a, &ctx);
        if a.jobs.edit_steps.is_empty() && a.session.job_on(doc).is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(timeline(&a).unwrap().in_effect(), 4, "back at the end");
}
