//! The kept height of the toolbar's rail (#15493): `measure` answers as a measuring run does, and measures again
//! when the width or the toolbar layout changes.

use super::*;
use crate::app::framework::BuildApp;
use egui::vec2;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// What one pass measured: the width, the kept height and a fresh measuring run's.
type Seen = Rc<RefCell<Vec<(f32, f32, f32)>>>;

/// A harness that measures the rail `width` wide on every pass, through `measure` and through a fresh run.
fn harness(width: Rc<Cell<f32>>, seen: Seen) -> egui_kittest::Harness<'static, Box<PhotocraftApp>> {
    let mut ready = false;
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let mut harness = egui_kittest::Harness::builder().with_context(app.ctx().clone()).with_size(vec2(400.0, 900.0)).build_app(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                egui_extras::install_image_loaders(ui.ctx());
                crate::theme::install_fonts(ui.ctx());
                crate::theme::apply(ui.ctx(), &crate::theme::AppTheme::pro());
                ready = true;
                return;
            }
            let w = width.get();
            let kept = measure(app, ui, w);
            let fresh = measure_now(app, ui, w);
            seen.borrow_mut().push((w, kept, fresh));
        },
        app,
    );
    harness.run_steps(4);
    harness
}

fn last(seen: &Seen) -> (f32, f32, f32) {
    *seen.borrow().last().expect("a pass measured the rail")
}

#[test]
fn the_kept_height_follows_the_width_and_the_toolbar_layout() {
    let width = Rc::new(Cell::new(40.0));
    let seen: Seen = Rc::default();
    let mut harness = harness(Rc::clone(&width), Rc::clone(&seen));

    let (_, one_column, fresh) = last(&seen);
    assert_eq!(one_column, fresh, "one column: the kept height is the measured one");
    harness.run_steps(2);
    assert_eq!(last(&seen).1, one_column, "the same inputs keep the height");

    width.set(120.0);
    harness.run_steps(2);
    let (_, wide, fresh) = last(&seen);
    assert_eq!(wide, fresh, "a wider rail is measured again");
    assert!(wide < one_column, "more columns make a shorter rail: {wide} against {one_column}");

    // Hide most tools: the layout changes, and with it the slots.
    harness.state_mut().session.prefs.edit(|prefs| {
        prefs.toolbar.hidden = Tool::ALL.iter().skip(4).map(|tool| format!("{tool:?}")).collect();
    });
    harness.run_steps(2);
    let (_, fewer, fresh) = last(&seen);
    assert_eq!(fewer, fresh, "a changed layout is measured again");
    assert!(fewer < wide, "fewer tools make a shorter rail: {fewer} against {wide}");
}
