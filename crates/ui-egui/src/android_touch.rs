//! Android-specific egui input and density helpers.
//! Kept separate from the document engine and desktop UI.
#![cfg(target_os = "android")]

use egui::{Context, Event, PointerButton, Pos2, TouchPhase};

/// Convert Android display density to a readable UI scale.
/// egui's logical points already account for DPI; this is an extra
/// accessibility-friendly adjustment for compact touch screens.
pub fn configure_touch_ui(ctx: &Context, screen_width_points: f32) {
    let scale = if screen_width_points < 600.0 { 1.15 } else { 1.0 };
    ctx.set_zoom_factor(scale);
}

/// Translate the primary finger to pointer events for existing mouse-oriented
/// PhotoCraft tools. Other fingers remain available as native egui touch events.
/// The caller must retain and update primary_touch_id across events.
pub fn pointer_from_touch(
    event: &Event,
    primary_touch_id: &mut Option<egui::TouchId>,
) -> Vec<Event> {
    let Event::Touch { id, phase, pos, .. } = event else {
        return Vec::new();
    };

    match phase {
        TouchPhase::Start if primary_touch_id.is_none() => {
            *primary_touch_id = Some(*id);
            vec![
                Event::PointerMoved(*pos),
                Event::PointerButton {
                    pos: *pos,
                    button: PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        }
        TouchPhase::Move if *primary_touch_id == Some(*id) => {
            vec![Event::PointerMoved(*pos)]
        }
        TouchPhase::End | TouchPhase::Cancel if *primary_touch_id == Some(*id) => {
            *primary_touch_id = None;
            vec![
                Event::PointerButton {
                    pos: *pos,
                    button: PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
                Event::PointerGone,
            ]
        }
        _ => Vec::new(),
    }
}
