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

#[test]
fn a_tab_close_resolves_the_document_to_its_current_index() {
    let mut h = harness(2);
    let other = h.state().session.active().unwrap().doc.id;
    h.query_all_by_role(Role::Button)
        .find(|n| n.accesskit_node().label().is_some_and(|label| label.starts_with("Close ch1-p01.png")))
        .expect("the other document tab's close button")
        .click();
    h.run_steps(3);
    let docs = h.state().session.documents();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].doc.id, other);
}

fn tiled_app(width: f32) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(width, 720.0)).with_pixels_per_point(1.25).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, AppTheme::pro());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 180, "height": 120, "name": "Edited photograph with a long title.psd", "depth": 16})).unwrap();
        app.run("layer.new.layer", json!({"name": "Retouch"})).unwrap();
        app.run("file.new", json!({"width": 160, "height": 100, "name": "Reference photograph.psd"})).unwrap();
        app
    });
    h.run_steps(3);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.arrange.tile", json!({})).unwrap();
    h.run_steps(4);
    h
}

fn tiled_control<'a>(h: &'a Harness<'_, Box<PhotocraftApp>>, role: Role, label: &str) -> Option<egui_kittest::Node<'a>> {
    let area = h.state().last_canvas_rect;
    h.query_all_by_role(role).filter(|n| n.accesskit_node().label().is_some_and(|text| text.contains(label))).find(|n| area.contains(n.rect().center()))
}

#[test]
fn tiled_document_titles_follow_individual_views() {
    for width in [1120.0, 652.0] {
        let mut h = tiled_app(width);
        let original = h.state().ui.views[1].zoom;
        h.state_mut().ui.views[0].zoom = 0.5;
        h.state_mut().ui.views[0].clear_pending();
        h.run_steps(3);
        let app = h.state();
        let name = &app.session.documents()[0].doc.name;
        let title = tiled_control(&h, Role::Tab, name).expect("the edited document's title is inside its tile");
        let label = title.accesskit_node().label().unwrap();
        assert!(label.contains("@ 50% (Retouch, RGB/16)") && label.ends_with(", modified"), "{label}");
        let panes = super::document_area::arranged_panes(app, app.last_canvas_rect).unwrap();
        let cell = panes.iter().find(|(i, _)| *i == 0).unwrap().1;
        let canvases = super::arranged_cells(app, app.last_canvas_rect).unwrap();
        let canvas = canvases.iter().find(|(i, _)| *i == 0).unwrap().1;
        let close = tiled_control(&h, Role::Button, &format!("Close {name}")).expect("close is available in the tile");
        assert!(cell.contains_rect(title.rect()) && cell.contains_rect(close.rect()), "title and close stay inside the tile at {width} points");
        let zoom = h.query_all_by_label("50%").find(|n| cell.contains_rect(n.rect())).expect("the edited document's tile shows its own zoom");
        assert!(title.rect().bottom() <= canvas.top() && zoom.rect().top() >= canvas.bottom(), "chrome is outside its canvas");
        assert!(matches!(app.drop_to(&h.ctx, Some(title.rect().center())), crate::file_open::DropTo::Window));
        assert_eq!(h.state().ui.views[1].zoom, original, "the other tile keeps its zoom");
        title.click();
        h.run_steps(3);
        assert_eq!(h.state().session.active_index(), Some(0));
    }
}

#[test]
fn tiled_document_close_uses_the_clicked_document_and_save_prompt() {
    let mut h = tiled_app(1120.0);
    let edited = h.state().session.documents()[0].doc.id;
    assert!(h.state().session.documents()[0].is_dirty());
    assert!(!h.state().session.documents()[1].is_dirty());
    assert_eq!(h.state().session.active_index(), Some(1));
    h.state_mut().ui.tool = crate::state::Tool::Brush;
    let revisions: Vec<_> = h.state().session.documents().iter().map(|d| d.revision).collect();
    let at = tiled_control(&h, Role::Button, "Close Edited photograph").expect("the inactive tile has its own close button").rect().center();
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(3);
    assert!(h.state().discard.is_some(), "closing edits asks whether to save");
    assert!(h.state().session.documents().iter().map(|d| d.revision).eq(revisions), "the close press produces no brush edit");
    assert_eq!(h.state().session.documents().len(), 2);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().discard.is_none());
    assert_eq!(h.state().session.documents().len(), 2, "Cancel leaves both tiles open");
    tiled_control(&h, Role::Button, "Close Reference photograph").expect("the clean tile has its own close button").click();
    h.run_steps(3);
    assert!(h.state().discard.is_none());
    let docs = h.state().session.documents();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].doc.id, edited, "the close button removed only the clean document");
    assert!(tiled_control(&h, Role::Tab, "Edited photograph").is_none(), "one document returns to the original single canvas");
}
