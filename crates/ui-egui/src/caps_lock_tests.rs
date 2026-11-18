//! Caps Lock swaps the cursor to the precise crosshair, as in Photoshop (#4639). The tools under
//! Cursors › Other Cursors choose that swap themselves through `ToolContext::precise`. This file
//! covers the one cursor that wins over the selected tool's: an armed dialog eyedropper (#9836).
//! The test drives the real canvas (`document_area`) and compares the cursor that Caps Lock
//! produces with the one the Precise preference draws.

use std::sync::Arc;

use craft::imaging::ui::adjust::TonePicker;
use craft::ui::runtime::windows::WindowHost;
use egui::CursorIcon;
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn capslock_harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 40, "hardness": 1.0}})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = Tool::Brush;
    let mut h = Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(900.0, 600.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    // 100 %: one document pixel per point; the pointer hovers the canvas centre.
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [200.0, 150.0];
    v.clear_pending();
    h.run_steps(2);
    let p = h.state().last_canvas_rect.center();
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(3);
    h
}

#[derive(Debug, PartialEq)]
struct CursorImageFp {
    size: [u16; 2],
    hotspot: [u16; 2],
    rgba: Arc<[u8]>,
}

/// Everything the cursor path can change in one frame: the cursor icon, the OS cursor bitmap
/// (Windows/macOS/Linux) and the shapes painted on the canvas. Comparison across two
/// preference/caps-lock states of the same harness is platform-independent.
#[derive(Debug, PartialEq)]
struct CursorFp {
    icon: CursorIcon,
    image: Option<CursorImageFp>,
    shapes: Vec<egui::epaint::ClippedShape>,
}

fn fp(h: &Harness<'_, PhotocraftApp>) -> CursorFp {
    let out = h.output();
    CursorFp {
        icon: out.platform_output.cursor_icon,
        image: out.platform_output.cursor_image.as_ref().map(|i| CursorImageFp { size: i.size, hotspot: i.hotspot, rgba: Arc::clone(&i.rgba) }),
        shapes: out.shapes.clone(),
    }
}

/// Caps Lock reaches the tool host from the runtime's injected Caps Lock (`WindowHost`), which the
/// frame reads in place of the platform's.
fn caps(h: &mut Harness<'_, PhotocraftApp>, on: bool) {
    WindowHost::of(&h.ctx).set_caps_lock(Some(on));
    h.run_steps(2);
}

fn other(h: &mut Harness<'_, PhotocraftApp>, value: &str) {
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": value})).unwrap();
    h.run_steps(2);
}

/// #9836: an armed Levels or Curves eyedropper shows its own cursor alone, not with the tool's
/// drawn over it, and takes caps lock's precise crosshair as well, as in Photoshop, instead of only
/// following the preference.
#[test]
fn caps_lock_gives_the_curves_eyedroppers_the_precise_crosshair() {
    let mut h = capslock_harness();
    // A tool whose own cursor ignores caps lock, so only the eyedropper can answer it.
    h.state_mut().ui.tool = Tool::Move;
    let id = crate::adjust_dialog::open(h.state_mut(), "image.adjustments.curves").expect("the Curves dialog opens");
    // PLAN: dialogs-w32 owns the accessor from a `DialogId` to its `HostedAdjustDialog`; `adjust_dialog::hosted_mut` is the name assumed here.
    crate::adjust_dialog::hosted_mut(h.state_mut(), id).set_picker(Some(TonePicker::Black));
    assert!(crate::dialogs::canvas_input::armed(h.state()).is_some(), "the black-point eyedropper is armed");
    h.run_steps(3);
    other(&mut h, "standard");
    let standard = fp(&h);
    other(&mut h, "precise");
    let precise = fp(&h);
    assert_eq!(precise.icon, CursorIcon::Crosshair, "Precise: the crosshair");
    assert_ne!(standard, precise, "Standard shows a cursor of its own where Precise shows the crosshair");
    other(&mut h, "standard");
    caps(&mut h, true);
    assert_eq!(fp(&h), precise, "caps lock turns the pipette into the precise crosshair");
    other(&mut h, "precise");
    assert_eq!(fp(&h), standard, "and the precise crosshair back into the pipette");
    caps(&mut h, false);
    assert_eq!(fp(&h), precise, "releasing caps lock restores the preference");
}
