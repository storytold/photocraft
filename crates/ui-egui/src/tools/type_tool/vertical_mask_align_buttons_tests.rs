//! #12088: with the Vertical Type Mask tool the options bar's alignment buttons are Left / Center /
//! Right once the mask text is toggled horizontal (the target beats the tool for mask text).

use egui_kittest::kittest::Queryable;
use photocraft_doc::LayerId;

use super::super::vertical_align_buttons_tests::{HORIZONTAL, VERTICAL, bar, new_app, shown};
use crate::PhotocraftApp;
use crate::canvas::testing::{ToolEvent, tool_event};
use crate::state::Tool;

/// The Vertical Type Mask tool typing a new mask; returns its hidden working layer.
fn typing_a_mask(app: &mut PhotocraftApp) -> LayerId {
    tool_event(app, ToolEvent::Down { x: 400.0, y: 100.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 400.0, y: 100.0 }, egui::Modifiers::NONE);
    let ed = app.ui.text_edit.as_ref().filter(|ed| ed.mask).expect("typing a mask");
    LayerId(ed.layer)
}

#[test]
fn toggling_the_mask_text_horizontal_shows_left_center_and_right() {
    let mut app = new_app(Tool::VerticalTypeMask);
    typing_a_mask(&mut app);
    let mut h = bar(app);
    assert_eq!(shown(&h), VERTICAL, "before the toggle");
    h.get_by_label("Toggle text orientation").click();
    h.run_steps(2);
    assert!(h.state().ui.text_edit.is_some(), "still editing");
    assert_eq!(shown(&h), HORIZONTAL);
}
