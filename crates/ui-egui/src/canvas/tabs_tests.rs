//! The document tab strip in the whole app: tabs follow their documents through Save As, Ctrl+Tab cycles them in
//! document order, and the tab text names the target (real tree).

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_engine::actions_cmds::ActionStep;
use serde_json::json;

use crate::PhotocraftApp;
use crate::theme::AppTheme;

use super::tabs::tab_title_at;

/// A real app window with `docs` documents named "ch1-pNN.png".
fn harness(docs: usize) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_max_steps(64).build_eframe(move |cc| {
        let theme = AppTheme::pro();
        PhotocraftApp::setup_context(&cc.egui_ctx, theme.clone());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        // The theme is an interface preference, applied on the first frame: set the preference.
        app.save_theme(&theme).unwrap();
        for n in 1..=docs {
            app.run("file.new", json!({"width": 400, "height": 300, "name": format!("ch1-p{n:02}.png")})).unwrap();
        }
        app
    });
    h.run_steps(4);
    h
}

/// How many tabs of the strip carry `name`. A tab's accessible name is its name, then its detail and modified mark.
fn tabs_named(h: &Harness<'_, Box<PhotocraftApp>>, name: &str) -> usize {
    h.query_all_by_role(Role::Tab).filter(|n| n.accesskit_node().label().is_some_and(|label| label.contains(name))).count()
}

// `AppShellOutput::panels_hidden` carries only the Shift+Tab state (plain Tab sets the shell's `hidden`,
// `ui-shell/src/shell/reveal.rs:95-99`) and the app harness does not hand the shell output to a trial; the panels'
// visibility is read from the tree, by the Layers panel the default workspace docks.
/// Whether the docked panels are drawn.
fn panels_shown(h: &Harness<'_, Box<PhotocraftApp>>) -> bool {
    h.query_all_by_label("Layers").next().is_some()
}

#[test]
fn saved_document_tabs_track_file_identity() {
    let mut h = harness(2);
    h.state_mut().services.export = Some(Box::new(|_, _, _| Ok((vec![1], Vec::new()))));
    h.state_mut().services.write = Some(Box::new(|_, _| Ok(())));
    h.state_mut().save_as(Some("folder/My saved image.psd".into())).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().doc.name, "My saved image.psd");
    assert_eq!(tabs_named(&h, "My saved image.psd"), 1, "the saved document's tab carries its new name");
    assert_eq!(tabs_named(&h, "ch1-p02.png"), 0, "no tab keeps the name the document had before Save As");

    h.state_mut().session.set_active(0);
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().doc.name, "ch1-p01.png");
    assert_eq!(tabs_named(&h, "My saved image.psd"), 1, "switching away keeps the saved tab");

    h.state_mut().session.set_active(1);
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.run_steps(2);
    let active = h.state().session.active().unwrap();
    assert_eq!(active.doc.name, "My saved image.psd");
    assert!(active.is_dirty());
    assert_eq!(tabs_named(&h, "My saved image.psd"), 1, "the edited saved document keeps one tab");
    assert_eq!(tabs_named(&h, "ch1-p01.png"), 1);
}

/// What a Ctrl press reports: on Windows and Linux Ctrl is also the command key, on macOS it isn't.
fn ctrl() -> egui::Modifiers {
    egui::Modifiers { ctrl: true, command: !cfg!(target_os = "macos"), ..egui::Modifiers::NONE }
}

/// #2340: Ctrl+Tab shows the next document's tab and Ctrl+Shift+Tab the previous one, wrapping
/// around at either end, as in Photoshop. Plain Tab still hides the panels instead.
#[test]
fn ctrl_tab_cycles_through_the_document_tabs() {
    let mut h = harness(3);
    let active = |h: &Harness<'_, Box<PhotocraftApp>>| h.state().session.active_index();
    assert_eq!(active(&h), Some(2), "the last new document is active");
    let press = |h: &mut Harness<'_, Box<PhotocraftApp>>, shift: bool| {
        h.key_press_modifiers(if shift { ctrl() | egui::Modifiers::SHIFT } else { ctrl() }, egui::Key::Tab);
        h.run_steps(2);
    };
    press(&mut h, false);
    assert_eq!(active(&h), Some(0), "Ctrl+Tab on the last tab wraps to the first");
    press(&mut h, false);
    assert_eq!(active(&h), Some(1));
    press(&mut h, true);
    assert_eq!(active(&h), Some(0), "Ctrl+Shift+Tab goes back");
    press(&mut h, true);
    assert_eq!(active(&h), Some(2), "and wraps to the last tab");
    assert!(panels_shown(&h), "Ctrl+Tab doesn't hide the panels");
    // Tab order is document order, so a dragged tab is cycled through at its new place.
    h.state_mut().session.move_document(0, 2);
    h.state_mut().session.set_active(1);
    press(&mut h, false);
    assert_eq!(h.state().session.active().unwrap().doc.name, "ch1-p01.png");
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
    assert_eq!(active(&h), Some(2), "plain Tab doesn't switch documents");
    assert!(!panels_shown(&h), "plain Tab hides the panels");
}

#[test]
fn ctrl_tab_records_a_relative_document_switch() {
    let mut h = harness(2);
    h.state_mut().run("actions.record", json!({"name": "Switch"})).unwrap();
    h.key_press_modifiers(ctrl(), egui::Key::Tab);
    h.run_steps(2);
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let steps = &h.state().session.actions.list.action(0).expect("the recorded action").steps;
    assert_eq!(steps, &[ActionStep::new("document.activate", json!({"offset": -1}))]);
}

#[test]
fn quick_mask_mode_names_the_quick_mask_in_the_tab() {
    let mut h = harness(1);
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    let title = |h: &Harness<'_, Box<PhotocraftApp>>| tab_title_at(h.state(), 0, 100.0);

    let detail = title(&h);
    assert!(detail.starts_with("ch1-p01.png @ 100%"), "{detail}");
    assert!(detail.contains("Layer 1") && detail.contains("RGB"), "the active layer and the mode: {detail}");
    assert!(!detail.contains('*'), "the tab's modified mark shows unsaved changes, not the text");

    h.state_mut().run("select.editInQuickMaskMode", json!({"on": true})).unwrap();
    let detail = title(&h);
    assert!(detail.contains("Quick Mask"), "Quick Mask mode is named: {detail}");
    assert!(!detail.contains("Layer 1") && !detail.contains("RGB"), "the layer is not the target: {detail}");

    h.state_mut().run("select.editInQuickMaskMode", json!({"on": false})).unwrap();
    let detail = title(&h);
    assert!(detail.contains("Layer 1") && detail.contains("RGB") && !detail.contains("Quick Mask"), "{detail}");
}
