use super::*;
use egui_kittest::kittest::Queryable;

// PLAN: the ctx-based `frame` helper is replaced by a kittest harness: only a harness exposes accessible names, and
// `frame` would have no caller once the other tests are gone.

/// A harness that draws the toolbar in a window tall enough for every slot. The first pass installs the theme and fonts.
fn toolbar_harness(app: Box<PhotocraftApp>) -> egui_kittest::Harness<'static, Box<PhotocraftApp>> {
    let mut ready = false;
    let mut harness = egui_kittest::Harness::builder().with_context(app.ctx().clone()).with_size(vec2(400.0, 1800.0)).build_ui_state(
        move |ui, app: &mut Box<PhotocraftApp>| {
            if !ready {
                egui_extras::install_image_loaders(ui.ctx());
                theme::install_fonts(ui.ctx());
                theme::apply(ui.ctx(), &theme::AppTheme::pro());
                ready = true;
                return;
            }
            toolbar(app, ui);
        },
        app,
    );
    harness.run_steps(4);
    harness
}

/// #3569: after a restart a slot is still on the tool it was left on, whichever tool is selected:
/// a click picks that tool, not the slot's first.
#[test]
fn slots_come_back_on_their_last_tool_after_a_restart() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.prefs.edit(|p| p.group_tools = json!(["PolygonLasso", "PaintBucket", "Pencil", "Sharpen", "SingleRowMarquee"]));
    crate::tool_memory::restore(&mut app);
    let mut harness = toolbar_harness(app);
    // Each slot's button is named by the tool it shows.
    let slots = [
        ("Pencil Tool", Tool::Pencil),
        ("Paint Bucket Tool", Tool::PaintBucket),
        ("Polygonal Lasso Tool", Tool::PolygonLasso),
        ("Sharpen Tool", Tool::Sharpen),
        ("Single Row Marquee Tool", Tool::SingleRowMarquee),
    ];
    for (name, tool) in slots {
        // Another slot's tool is current, so the slot shows its remembered tool rather than the current one.
        harness.state_mut().ui.tool = Tool::Move;
        harness.run_steps(2);
        // The slot button holds the flyout's own button of the same name: the outer one is first.
        harness.get_all_by_label_contains(name).next().expect(name).click();
        harness.run_steps(3);
        assert_eq!(harness.state().ui.tool, tool, "the slot of {name}");
    }
}
