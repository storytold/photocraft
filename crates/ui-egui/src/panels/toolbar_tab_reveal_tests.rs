use super::*;
use crate::app::framework::BuildApp;
use egui_kittest::kittest::Queryable;

const HEIGHT: f32 = 360.0;

/// Whether the rail's button for `tool` has the focus. A tool's button is a node inside a node of the same name (the
/// slot and the button in it), so the name matches two. The name matches whole: "Move Toolbar to the Right" contains
/// "Move Tool".
fn focused(harness: &egui_kittest::Harness<'_, Box<PhotocraftApp>>, tool: &str) -> bool {
    harness.query_all_by_label(tool).any(|node| node.is_focused())
}

/// The rect of the rail's button for `tool`: the innermost of the nodes carrying its name.
fn button_rect(harness: &egui_kittest::Harness<'_, Box<PhotocraftApp>>, tool: &str) -> egui::Rect {
    harness.query_all_by_label(tool).last().unwrap_or_else(|| panic!("no {tool} button")).rect()
}

/// A harness that draws the toolbar in a window much shorter than its tools, so the rail scrolls.
fn short_toolbar_harness() -> egui_kittest::Harness<'static, Box<PhotocraftApp>> {
    let mut ready = false;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    // The supported shortcut preference clears the canvas Tab binding, so Tab is the focus key.
    app.session.prefs.edit(|prefs| prefs.shortcuts.insert("window.togglePanels".into(), String::new()));
    let mut harness = egui_kittest::Harness::builder().with_context(app.ctx().clone()).with_size(vec2(400.0, HEIGHT)).build_app(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                egui_extras::install_image_loaders(ui.ctx());
                crate::theme::install_fonts(ui.ctx());
                crate::theme::apply(ui.ctx(), &crate::theme::AppTheme::pro());
                ready = true;
                return;
            }
            crate::shortcuts::handle(app, ui.ctx());
            toolbar(app, ui);
        },
        app,
    );
    harness.run_steps(4);
    harness
}

/// From no focus, Tab enters the rail's buttons (one roving group) and the arrow keys walk them: the last tool is
/// reached in a window shorter than the rail, it is scrolled into view, and Space picks it.
#[test]
fn tab_reveals_and_activates_the_last_tool() {
    let mut harness = short_toolbar_harness();
    assert_eq!(harness.state().ui.tool, Tool::Move);
    let mut entered = false;
    for _ in 0..40 {
        harness.key_press(egui::Key::Tab);
        harness.run_steps(3);
        entered = focused(&harness, "Move Tool");
        if entered {
            break;
        }
    }
    assert!(entered, "Tab must reach the rail's buttons");
    // The node's rect is its visible part, so a button the rail clipped is shorter than a whole one.
    let full_height = button_rect(&harness, "Move Tool").height();
    // Right walks the grid in reading order and goes on to the next row at a row's end; Up and Down stop at the first
    // and last rows.
    let mut reached = false;
    for _ in 0..60 {
        harness.key_press(egui::Key::ArrowRight);
        harness.run_steps(3);
        reached = focused(&harness, "Zoom Tool");
        if reached {
            break;
        }
    }
    assert!(reached, "the arrow keys must reach the last tool");
    harness.run_steps(20);
    let rect = button_rect(&harness, "Zoom Tool");
    assert!(rect.height() >= full_height - 0.5, "the focused tool is clipped: {rect:?} against a full height of {full_height}");
    assert!(rect.top() >= 0.0 && rect.bottom() <= HEIGHT, "the focused tool is outside the window: {rect:?}");
    harness.key_press(egui::Key::Space);
    harness.run_steps(3);
    assert_eq!(harness.state().ui.tool, Tool::Zoom, "Space must select the revealed tool");
}
