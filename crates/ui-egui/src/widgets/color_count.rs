//! Integer-only Indexed Color field and a linear slider with common palette sizes.

use super::{Align2, Color32, Rect, Response, Sense, Stroke, StrokeKind, Tokens, Ui, pos2, surface, theme, vec2};

const STOPS: [u32; 6] = [8, 16, 32, 64, 128, 256];

fn parse_count(text: &str) -> Option<f64> {
    super::parse_num(text).filter(|v| *v >= 0.0 && *v <= f64::from(u32::MAX) && v.fract() == 0.0)
}

/// Where `count` sits on the track: from its left edge, or from its right edge in a
/// right-to-left UI, where the knob and the fill lead from the reading start.
fn position(track: Rect, count: u32, rtl: bool) -> f32 {
    let fraction = count.saturating_sub(2) as f32 / 254.0;
    if rtl { track.right() - fraction * track.width() } else { track.left() + fraction * track.width() }
}

/// The count under the pointer at `x`: the inverse of [`position`].
fn at_pointer(track: Rect, x: f32, rtl: bool) -> u32 {
    // Use the nearest marker: low counts are close together on the linear track.
    if let Some(stop) = STOPS.iter().copied().min_by(|&a, &b| (position(track, a, rtl) - x).abs().total_cmp(&(position(track, b, rtl) - x).abs()))
        && (position(track, stop, rtl) - x).abs() <= 6.0
    {
        return stop;
    }
    let fraction = if rtl { (track.right() - x) / track.width().max(1.0) } else { (x - track.left()) / track.width().max(1.0) };
    (2.0 + fraction.clamp(0.0, 1.0) * 254.0).round() as u32
}

/// A row in reading order: `ui.horizontal` in a left-to-right UI, its mirror in a
/// right-to-left one. The row keeps `ui.horizontal`'s height and alignment.
fn row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    let size = vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, crate::direction::row(), add)
}

/// A count in 2..=256. Direct entry allows every integer; only slider gestures snap to markers.
/// In a right-to-left UI the caption leads at the right, the number field follows at the left
/// and the slider's axis mirrors; the digits and the arrow keys stay as they are.
pub fn color_count_row(ui: &mut Ui, label: &str, count: &mut u32) -> Response {
    let t = Tokens::get(ui.ctx());
    *count = (*count).clamp(2, 256);
    let before = *count;
    let rtl = crate::direction::is_rtl();
    row(ui, |ui| {
        ui.label(egui::RichText::new(tl!(label)).color(t.text_dim));
        // The number field belongs at the row's reading end.
        ui.with_layout(crate::direction::trailing(), |ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(74.0, 24.0), Sense::hover());
            surface(ui, rect, t.field, false);
            if !t.bevel {
                ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
            }
            // The digits stay left-to-right; the box inherits the scope's mirrored layout
            // so the number keeps the position it had in a left-to-right UI.
            let builder = egui::UiBuilder::new().max_rect(rect.shrink2(vec2(4.0, 2.0)));
            let mut child = ui.new_child(builder);
            child.style_mut().visuals.widgets.inactive.bg_fill = Color32::TRANSPARENT;
            child.style_mut().visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
            child.style_mut().visuals.widgets.inactive.bg_stroke = Stroke::NONE;
            child.style_mut().visuals.widgets.hovered.bg_stroke = Stroke::NONE;
            child.style_mut().override_font_id = Some(theme::mono(12.0));
            child.add(egui::DragValue::new(count).range(2..=256).speed(1.0).max_decimals(0).custom_parser(parse_count)).accessible_name(tl!(label));
        });
    });
    let (rect, mut response) = ui.allocate_exact_size(vec2(ui.available_width().max(60.0), 36.0), Sense::click_and_drag());
    let track = Rect::from_min_max(pos2(rect.left() + 7.0, rect.top() + 7.5), pos2(rect.right() - 7.0, rect.top() + 10.5));
    if (response.clicked() || response.dragged())
        && let Some(pointer) = response.interact_pointer_pos()
    {
        *count = at_pointer(track, pointer.x, rtl).clamp(2, 256);
        response.request_focus();
    }
    if response.has_focus() {
        // The arrow keys stay physical: right raises the count in every direction.
        ui.input_mut(|input| {
            for key in [egui::Key::ArrowLeft, egui::Key::ArrowDown] {
                if input.consume_key(egui::Modifiers::NONE, key) {
                    *count = count.saturating_sub(1).max(2);
                }
            }
            for key in [egui::Key::ArrowRight, egui::Key::ArrowUp] {
                if input.consume_key(egui::Modifiers::NONE, key) {
                    *count = count.saturating_add(1).min(256);
                }
            }
        });
    }
    response.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), f64::from(*count), label));
    let x = position(track, *count, rtl);
    let painter = ui.painter();
    painter.rect_filled(track, t.radius_sm, t.field_border);
    // The fill grows from the leading edge: the left in LTR, the right in RTL.
    let filled = if rtl { Rect::from_min_max(pos2(x, track.top()), track.max) } else { Rect::from_min_max(track.min, pos2(x, track.bottom())) };
    painter.rect_filled(filled, t.radius_sm, t.accent);
    for stop in STOPS {
        let sx = position(track, stop, rtl);
        painter.line_segment([pos2(sx, track.bottom() + 3.0), pos2(sx, track.bottom() + 6.0)], Stroke::new(1.0, t.text_faint));
        painter.text(
            pos2(sx, track.bottom() + 9.0),
            // The last marker hugs the track's outer edge, so it does not run past it.
            if stop == 256 { if rtl { Align2::LEFT_TOP } else { Align2::RIGHT_TOP } } else { Align2::CENTER_TOP },
            stop.to_string(),
            theme::mono(10.0),
            t.text_faint,
        );
    }
    let knob = pos2(x, track.center().y);
    if t.bevel {
        surface(ui, Rect::from_center_size(knob, vec2(10.0, 16.0)), t.card, true);
    } else {
        painter.circle_filled(knob, 6.0, t.text);
        if response.hovered() || response.dragged() {
            painter.circle_stroke(knob, 8.0, Stroke::new(2.0, t.accent_soft));
        }
    }
    if *count != before {
        response.mark_changed();
    }
    ui.add_space(4.0);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Pos2;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn typed_color_counts_reject_fractions_and_allow_arbitrary_integers() {
        for (text, expected) in [("17", 17), ("256", 256), ("17.5", 64), ("17,5", 64), ("1e2", 100), ("(8+8)*2", 32), ("2^8", 256), ("5/2", 64)] {
            let mut harness = Harness::builder().with_size(vec2(320.0, 100.0)).build_ui_state(
                |ui, count: &mut u32| {
                    color_count_row(ui, "Colors", count);
                },
                64,
            );
            harness.get_by_role(egui::accesskit::Role::SpinButton).click();
            harness.run();
            harness.event(egui::Event::Text(text.into()));
            harness.key_press(egui::Key::Enter);
            harness.run();
            assert_eq!(*harness.state(), expected, "{text}");
        }
    }

    #[test]
    fn slider_clicks_snap_to_every_marker_but_allow_intermediate_counts() {
        for count in STOPS {
            let mut harness = Harness::builder().with_size(vec2(320.0, 100.0)).build_ui_state(
                |ui, count: &mut u32| {
                    color_count_row(ui, "Colors", count);
                },
                2,
            );
            let rect = harness.get_by_role(egui::accesskit::Role::Slider).rect();
            let track = rect.shrink2(vec2(7.0, 0.0));
            let point = pos2(position(track, count, false) - 4.0, rect.top() + 9.0);
            harness.event(egui::Event::PointerMoved(point));
            for pressed in [true, false] {
                harness.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                harness.run();
            }
            assert_eq!(*harness.state(), count);
            harness.key_press(egui::Key::ArrowRight);
            harness.run();
            assert_eq!(*harness.state(), (count + 1).min(256));
        }
        let track = Rect::from_min_size(Pos2::ZERO, vec2(300.0, 3.0));
        assert_eq!(at_pointer(track, position(track, 23, false), false), 23);
        assert_eq!(position(track, 129, false), track.center().x);
        assert!(((position(track, 128, false) - position(track, 64, false)) - (position(track, 96, false) - position(track, 32, false))).abs() < 0.001);
        // The axis mirrors: the same count sits at the mirrored x, and a pointer there
        // reads back the same count.
        assert_eq!(position(track, 129, true), track.center().x);
        assert_eq!(at_pointer(track, position(track, 23, true), true), 23);
    }

    #[test]
    fn the_count_field_sits_at_the_reading_end_in_rtl() {
        for (dir, at_left) in [(crate::direction::Dir::Ltr, false), (crate::direction::Dir::Rtl, true)] {
            let _guard = crate::direction::direction_scope(dir);
            let mut harness = Harness::builder().with_size(vec2(320.0, 100.0)).build_ui_state(
                |ui, count: &mut u32| {
                    color_count_row(ui, "Colors", count);
                },
                64,
            );
            harness.run();
            let field = harness.get_by_role(egui::accesskit::Role::SpinButton).rect();
            if at_left {
                assert!(field.right() < 160.0, "{dir:?}: field at the reading end, got {field:?}");
            } else {
                assert!(field.left() > 160.0, "{dir:?}: field at the reading end, got {field:?}");
            }
        }
    }

    #[test]
    fn the_number_sits_at_the_reading_end_of_its_box() {
        for (dir, at_right) in [(crate::direction::Dir::Ltr, true), (crate::direction::Dir::Rtl, false)] {
            let _guard = crate::direction::direction_scope(dir);
            let mut harness = Harness::builder().with_size(vec2(320.0, 100.0)).build_ui_state(
                |ui, count: &mut u32| {
                    color_count_row(ui, "Colors", count);
                },
                64,
            );
            harness.run();
            let spin = harness.get_by_role(egui::accesskit::Role::SpinButton).rect();
            if at_right {
                assert!(spin.right() > 300.0, "{dir:?}: number at the box's right end, got {spin:?}");
            } else {
                assert!(spin.left() < 20.0, "{dir:?}: number at the box's left end, got {spin:?}");
            }
        }
    }

    #[test]
    fn the_slider_markers_snap_from_the_reading_start_in_rtl() {
        for dir in [crate::direction::Dir::Ltr, crate::direction::Dir::Rtl] {
            let rtl = dir == crate::direction::Dir::Rtl;
            let _guard = crate::direction::direction_scope(dir);
            for count in STOPS {
                let mut harness = Harness::builder().with_size(vec2(320.0, 100.0)).build_ui_state(
                    |ui, count: &mut u32| {
                        color_count_row(ui, "Colors", count);
                    },
                    2,
                );
                let rect = harness.get_by_role(egui::accesskit::Role::Slider).rect();
                let track = rect.shrink2(vec2(7.0, 0.0));
                let point = pos2(position(track, count, rtl) - 4.0, rect.top() + 9.0);
                harness.event(egui::Event::PointerMoved(point));
                for pressed in [true, false] {
                    harness.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                    harness.run();
                }
                assert_eq!(*harness.state(), count, "{dir:?} {count}");
                // The arrow keys stay physical in every direction.
                harness.key_press(egui::Key::ArrowRight);
                harness.run();
                assert_eq!(*harness.state(), (count + 1).min(256), "{dir:?} {count}");
            }
        }
    }
}

#[cfg(test)]
mod arithmetic_tests {
    #[test]
    fn count_expression_preserves_integer_validation() {
        assert_eq!(super::parse_count("2^8"), Some(256.0));
        assert_eq!(super::parse_count("(8+8)*2"), Some(32.0));
        assert_eq!(super::parse_count("5/2"), None);
        assert_eq!(super::parse_count("1/0"), None);
        assert_eq!(super::parse_count("-2"), None);
    }
}
