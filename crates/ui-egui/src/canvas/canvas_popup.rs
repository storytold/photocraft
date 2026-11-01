//! Outside-click dismissal shared by canvas menus and the Brush Preset picker.

use egui::{Context, Id, Rect};

pub(crate) fn mark_open(ctx: &Context, menu: Id) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(menu.with("opening-frame"), frame));
}

pub(crate) fn outside_pressed(ctx: &Context, menu: Id, rect: Rect) -> bool {
    // A quick tap's opening press can fall outside the menu after physical-pixel rounding.
    // Use the frame, rather than the pass, so a layout retry cannot dismiss it either.
    let opened = ctx.data(|d| d.get_temp::<u64>(menu.with("opening-frame")));
    opened != Some(ctx.cumulative_frame_nr()) && ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::{Dir, direction_scope};
    use egui::{Event, Modifiers, PointerButton, RawInput, pos2, vec2};

    fn outside_press() -> RawInput {
        RawInput {
            events: vec![
                Event::PointerMoved(pos2(5.75, 5.75)),
                Event::PointerButton { pos: pos2(5.75, 5.75), button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE },
                Event::PointerButton { pos: pos2(5.75, 5.75), button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn opening_frame_survives_layout_retries_and_later_presses_close() {
        let ctx = Context::default();
        let id = Id::unique("menu");
        let rect = Rect::from_min_size(pos2(6.0, 6.0), vec2(100.0, 100.0));
        for _ in 0..3 {
            let mut passes = 0;
            let mut opening_pointer = None;
            let mut decisions = Vec::new();
            ctx.run_ui(outside_press(), |ui| {
                let ctx = ui.ctx();
                if passes == 0 {
                    opening_pointer = Some(ctx.input(|i| i.pointer.clone()));
                    mark_open(ctx, id);
                    ctx.request_discard("test the menu's extra layout pass");
                } else {
                    // egui consumes events before retrying. Replay the opening press so this
                    // assertion tests the frame guard, rather than the absence of input.
                    ctx.input_mut(|i| i.pointer = opening_pointer.clone().unwrap());
                }
                let outside_input = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p)));
                decisions.push((outside_input, outside_pressed(ctx, id, rect)));
                passes += 1;
            })
            .drop_without_applying_deltas();
            assert_eq!(passes, 2);
            assert_eq!(decisions, [(true, false), (true, false)], "both passes must protect an outside opening press");
            let mut later_outside = false;
            ctx.run_ui(outside_press(), |ui| later_outside = outside_pressed(ui.ctx(), id, rect)).drop_without_applying_deltas();
            assert!(later_outside, "a later frame's outside press still dismisses the menu");
        }
    }

    #[test]
    fn opening_one_menu_does_not_protect_another() {
        let ctx = Context::default();
        let first = Id::unique("first");
        let second = Id::unique("second");
        let rect = Rect::from_min_size(pos2(6.0, 6.0), vec2(100.0, 100.0));
        ctx.run_ui(RawInput::default(), |ui| mark_open(ui.ctx(), first)).drop_without_applying_deltas();
        ctx.run_ui(outside_press(), |ui| {
            let ctx = ui.ctx();
            mark_open(ctx, second);
            assert!(!outside_pressed(ctx, second, rect));
            assert!(outside_pressed(ctx, first, rect));
        })
        .drop_without_applying_deltas();
    }

    // The canvas context menus (rulers.rs:550, tabs.rs, start_screen.rs) open through egui's
    // `Popup::context_menu`, whose anchor is the pointer: under RTL the fork picks BOTTOM_END
    // (the popup's right edge at the anchor, growing leftward), under LTR BOTTOM_START, and
    // `find_best_align` flips the side when the default one would leave the screen.
    fn context_menu_rect(dir: Dir, anchor: egui::Pos2) -> egui::Rect {
        let _scope = direction_scope(dir);
        let ctx = Context::default();
        ctx.all_styles_mut(|s| s.right_to_left = dir == Dir::Rtl);
        let screen = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let raw = |pressed: Option<bool>| RawInput {
            screen_rect: Some(screen),
            events: vec![Event::PointerMoved(anchor)]
                .into_iter()
                .chain(pressed.map(|p| Event::PointerButton { pos: anchor, button: PointerButton::Secondary, pressed: p, modifiers: Modifiers::NONE }))
                .collect(),
            ..Default::default()
        };
        let id = egui::Id::unique("canvas-ctx");
        let mut popup_id = None;
        // Hover first (a press on the very first frame finds no hovered widget, so egui never
        // registers the click), then press, release, and let the open popup measure and paint.
        for input in [raw(None), raw(Some(true)), raw(Some(false)), raw(None), raw(None)] {
            ctx.run_ui(input, |ui| {
                let resp = ui.interact(screen, id, egui::Sense::click());
                let popup = egui::Popup::context_menu(&resp);
                popup_id = Some(popup.get_id());
                popup.show(|ui| {
                    ui.set_min_size(egui::vec2(200.0, 140.0));
                    ui.label("row");
                });
            })
            .drop_without_applying_deltas();
        }
        ctx.memory(|m| m.area_rect(popup_id.expect("a popup id"))).expect("the popup is shown")
    }

    #[test]
    fn canvas_context_menu_anchor_side_flips_with_direction() {
        let anchor = egui::pos2(500.0, 100.0);
        let rtl = context_menu_rect(Dir::Rtl, anchor);
        assert!((rtl.right() - anchor.x).abs() < 0.5, "rtl: the popup's right edge sits at the anchor: {rtl:?}");
        assert!((rtl.top() - anchor.y).abs() < 0.5, "rtl: the popup hangs below the anchor: {rtl:?}");
        let ltr = context_menu_rect(Dir::Ltr, anchor);
        assert!((ltr.left() - anchor.x).abs() < 0.5, "ltr: the popup's left edge sits at the anchor: {ltr:?}");
        assert!((ltr.top() - anchor.y).abs() < 0.5, "ltr: the popup hangs below the anchor: {ltr:?}");
    }

    #[test]
    fn canvas_context_menu_stays_clamped_inside_the_screen_in_both_directions() {
        let screen = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        // Near the reading-start screen edge the default side does not fit, so egui flips to
        // the other side; either way the whole popup stays inside the screen.
        let rtl = context_menu_rect(Dir::Rtl, pos2(50.0, 100.0));
        assert!(screen.contains_rect(rtl), "rtl near the left edge: {rtl:?}");
        assert!((rtl.left() - 50.0).abs() < 0.5, "rtl near the left edge flips to rightward growth: {rtl:?}");
        let ltr = context_menu_rect(Dir::Ltr, pos2(790.0, 100.0));
        assert!(screen.contains_rect(ltr), "ltr near the right edge: {ltr:?}");
        assert!((ltr.right() - 790.0).abs() < 0.5, "ltr near the right edge flips to leftward growth: {ltr:?}");
    }
}
