//! Tab strips that fit any width (#151): when the tabs don't fit beside the panel menu button
//! they shrink, eliding their labels with '…', and once they are at their minimum width the ones
//! that still don't fit move into a » chevron menu at the end of the strip. The selected tab
//! always stays on the strip, and no tab ever runs under the menu button.

use egui::{Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::theme::{self, Tokens};

pub use craft_ui::tabs::TabFit;

/// Fit tabs to a strip using the shared, bounded width policy. Local wrappers retain the
/// application's themes, translated labels and panel/document overflow menus.
pub fn fit(natural: &[f32], selected: usize, avail: f32, min_w: f32, chevron_w: f32) -> TabFit {
    craft_ui::tabs::fit(natural, selected, avail, min_w, chevron_w)
}

/// A label laid out on one row no wider than `max_w`, cut with '…' when it doesn't fit.
pub fn elided(ui: &Ui, text: &str, font: egui::FontId, color: egui::Color32, max_w: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping { max_width: max_w.max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.painter().layout_job(job)
}

/// Context-menu action chosen on a dock tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabContextAction {
    Close(usize),
    CloseGroup,
}

/// Width of the » overflow button.
pub const CHEVRON_W: f32 = 18.0;

/// Draw the » overflow button at `r`: hover tip `tip`, listing the tabs named by `labels` at the
/// indices in `overflow`, the `checked` one with a check mark as in the menu bar's menus. A picked
/// index is left in `picked`.
#[allow(clippy::too_many_arguments)]
pub fn overflow_button(ui: &mut Ui, id: egui::Id, r: Rect, tip: &str, labels: &[&str], overflow: &[usize], checked: Option<usize>, picked: &mut Option<usize>) {
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(r, id, Sense::click());
    let mut paint_ui = ui.new_child(egui::UiBuilder::new().max_rect(r));
    paint_ui.set_clip_rect(r.intersect(ui.clip_rect()));
    if resp.hovered() {
        paint_ui.painter().rect_filled(r.shrink2(vec2(1.0, 3.0)), t.radius_sm, t.hover.gamma_multiply(0.6));
    }
    crate::icons::paint(&paint_ui, r, "chevrons-right", 16.0, t.text);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tip));
    let resp = resp.on_hover_text(tip);
    // A click on the scroll bar's track is not a pick: only an entry, Escape or a click outside closes it.
    egui::Popup::menu(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(140.0);
        // Taller than the room below the popup's top (many documents open): scroll (#13189).
        let room = (ui.ctx().content_rect().bottom() - ui.min_rect().top() - 12.0).max(80.0);
        egui::ScrollArea::vertical().max_height(room).auto_shrink([true, true]).show(ui, |ui| {
            for &i in overflow {
                let Some(name) = labels.get(i) else { continue };
                let text = if checked.is_some() { format!("{} {name}", if checked == Some(i) { "✔" } else { "  " }) } else { (*name).to_owned() };
                if ui.button(text).clicked() {
                    *picked = Some(i);
                    ui.close();
                }
            }
        });
    });
}

/// Draw the tabs of a strip in `area` (the strip minus the menu button). `paint_tab` draws one
/// tab: (ui, rect, index, label galley, response, active).
#[allow(clippy::too_many_arguments)]
fn tabs_in(
    ui: &mut Ui,
    id: egui::Id,
    area: Rect,
    panel_ids: &[&str],
    selected: &mut usize,
    font: egui::FontId,
    pad: f32,
    min_w: f32,
    active: impl Fn(usize) -> bool,
    mut paint_tab: impl FnMut(&Ui, Rect, usize, &str, std::sync::Arc<egui::Galley>, &Response, bool),
) -> Option<TabContextAction> {
    let t = Tokens::get(ui.ctx());
    // Panel names are English keys; draw them in the UI language.
    let names: Vec<&str> = panel_ids.iter().map(|n| tl!(n)).collect();
    let translated_tabs = names.as_slice();
    let natural: Vec<f32> = translated_tabs.iter().map(|n| ui.painter().layout_no_wrap((*n).to_owned(), font.clone(), t.text).size().x + pad).collect();
    let f = fit(&natural, *selected, area.width(), min_w, CHEVRON_W);
    let mut x = area.left();
    let mut context = None;
    for &(i, w) in &f.shown {
        let Some(name) = translated_tabs.get(i) else { continue };
        let panel_id = panel_ids[i];
        let r = Rect::from_min_size(pos2(x, area.top()), vec2(w, area.height()));
        // The padding gives way (down to a third) before the label is cut.
        let galley = elided(ui, name, font.clone(), t.text, (w - pad / 3.0).max(1.0));
        let cut = galley.size().x + pad + 0.5 < natural.get(i).copied().unwrap_or(0.0) && galley.size().x + pad / 3.0 >= w - 0.5;
        let resp = craft_ui::tabs::Tab::new(id.with(("tab", i)), name, i == *selected).sense(Sense::click()).focus_stroke(Stroke::new(1.0, t.accent)).show_at(
            ui,
            r,
            |ui, resp| paint_tab(ui, r, i, panel_id, galley, resp, active(i)),
        );
        let resp = if cut { resp.on_hover_text(*name) } else { resp };
        if resp.clicked() {
            *selected = i;
        }
        // The context menu belongs to the actual tab response, so its clicks are not
        // intercepted by an overlay.
        resp.context_menu(|ui| {
            if ui.button(tl!("Close")).clicked() {
                context = Some(TabContextAction::Close(i));
                ui.close();
            }
            if ui.button(tl!("Close Tab Group")).clicked() {
                context = Some(TabContextAction::CloseGroup);
                ui.close();
            }
        });
        x = r.right();
    }
    if !f.overflow.is_empty() && f.overflow_width > 0.0 {
        let r = Rect::from_min_size(pos2(x, area.top()), vec2(f.overflow_width, area.height()));
        let mut picked = None;
        overflow_button(ui, id.with("tab-overflow"), r, tl!("More panels"), translated_tabs, &f.overflow, None, &mut picked);
        if let Some(i) = picked {
            *selected = i;
        }
    }
    context
}

/// Studio strip: pill tabs in `area` (the row minus the menu button). Returns the action chosen
/// from a tab's context menu.
pub fn pill_tabs(ui: &mut Ui, id: egui::Id, area: Rect, tabs: &[&str], selected: &mut usize) -> Option<TabContextAction> {
    let t = Tokens::get(ui.ctx());
    let sel = *selected;
    tabs_in(
        ui,
        id,
        area,
        tabs,
        selected,
        theme::medium(12.5),
        20.0,
        44.0,
        |i| i == sel,
        |ui, r, _, _, galley, resp, active| {
            // 2 pt between pills, as the old horizontal layout had.
            let r = Rect::from_min_max(r.min, pos2((r.right() - 2.0).max(r.left()), r.bottom()));
            if active {
                crate::widgets::surface(ui, r, t.hover, true);
                if !t.bevel {
                    ui.painter().rect_stroke(r, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
                }
            } else if resp.hovered() {
                ui.painter().rect_filled(r, t.radius_sm, t.hover.gamma_multiply(0.6));
            }
            let color = if active { t.text } else { t.text_dim };
            ui.painter().with_clip_rect(r.intersect(ui.clip_rect())).galley_with_override_text_color(r.center() - galley.size() / 2.0, galley, color);
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total(f: &TabFit) -> f32 {
        f.shown.iter().map(|(_, w)| w).sum()
    }

    #[test]
    fn tabs_that_fit_keep_their_widths() {
        let f = fit(&[60.0, 70.0, 80.0], 0, 400.0, 40.0, 18.0);
        assert_eq!(f.shown, vec![(0, 60.0), (1, 70.0), (2, 80.0)]);
        assert!(f.overflow.is_empty());
    }

    #[test]
    fn narrow_strips_shrink_the_widest_tabs_first() {
        let f = fit(&[50.0, 70.0, 120.0], 0, 200.0, 40.0, 18.0);
        assert!(f.overflow.is_empty());
        assert!((total(&f) - 200.0).abs() < 0.1, "{f:?}");
        assert_eq!(f.shown[0], (0, 50.0), "a tab narrower than the cap keeps its width");
        assert_eq!(f.shown[1], (1, 70.0));
        assert!((f.shown[2].1 - 80.0).abs() < 0.1, "only the widest shrank: {f:?}");
    }

    #[test]
    fn overflow_keeps_the_selected_tab_and_leaves_room_for_the_chevron() {
        let natural = [80.0, 90.0, 85.0, 95.0];
        for sel in 0..4 {
            let f = fit(&natural, sel, 120.0, 40.0, 18.0);
            assert!(f.shown.iter().any(|(i, _)| *i == sel), "{sel}: {f:?}");
            assert!(!f.overflow.is_empty() && !f.overflow.contains(&sel));
            assert!(total(&f) <= 120.0 - 18.0 + 1e-3, "{f:?}");
            assert_eq!(f.shown.len() + f.overflow.len(), 4);
            assert!(f.shown.windows(2).all(|w| w[0].0 < w[1].0), "tab order kept");
        }
    }

    #[test]
    fn hostile_inputs_never_panic_or_go_negative() {
        for avail in [0.0, -10.0, 5.0, f32::NAN, f32::INFINITY] {
            for natural in [&[][..], &[f32::NAN, 50.0][..], &[1e9, -5.0, 30.0][..]] {
                for sel in [0, 1, 99] {
                    let f = fit(natural, sel, avail, 40.0, 18.0);
                    assert!(f.shown.iter().all(|(_, w)| w.is_finite() && *w >= 0.0), "{avail} {natural:?} {f:?}");
                    assert_eq!(f.shown.len() + f.overflow.len(), natural.len());
                }
            }
        }
    }

    #[test]
    fn overflow_icon_and_hover_paint_stay_inside_a_tiny_button() {
        for scale in [1.0, 2.0] {
            let ctx = egui::Context::default();
            crate::PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
            ctx.set_pixels_per_point(scale);
            for width in [1.0, 5.0, 11.0, 12.0, 18.0] {
                let rect = Rect::from_min_size(pos2(20.25, 20.25), vec2(width, 26.0));
                for _ in 0..3 {
                    let mut picked = None;
                    let input = egui::RawInput { events: vec![egui::Event::PointerMoved(rect.center())], ..Default::default() };
                    let mut output = ctx.run_ui(input, |ui| {
                        overflow_button(ui, ui.scope_id().with("tiny-overflow"), rect, "More panels", &["Layers"], &[0], None, &mut picked);
                    });
                    output.textures_delta.clear();
                    let mut icons = 0;
                    for shape in &output.shapes {
                        // egui paints an unrotated image as a textured rectangle, not a mesh.
                        if matches!(&shape.shape, egui::Shape::Rect(rect) if rect.brush.is_some()) {
                            icons += 1;
                        }
                        let painted = shape.shape.visual_bounding_rect().intersect(shape.clip_rect);
                        if painted.is_positive() {
                            assert!(rect.contains_rect(painted), "{scale}x, width {width}: {painted:?}");
                        }
                    }
                    assert!(icons > 0, "the actual SVG icon was painted");
                    assert!(picked.is_none());
                }
            }
        }
    }

    /// #13189: with many documents open the » list is taller than the window; it scrolls instead
    /// of running off the screen, and a short list looks as it always did.
    mod popup {
        use super::super::*;
        use egui_kittest::{Harness, kittest::Queryable};

        const WINDOW: egui::Vec2 = egui::Vec2::new(800.0, 500.0);

        type State = (Vec<String>, Option<usize>);

        fn click_at(h: &mut Harness<'static, State>, pos: egui::Pos2) {
            h.hover_at(pos);
            h.run_steps(1);
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                h.run_steps(1);
            }
        }

        fn open(count: usize) -> Harness<'static, State> {
            let names: Vec<String> = (0..count).map(|i| format!("Document {i}")).collect();
            let mut h = Harness::builder().with_size(WINDOW).build_ui_state(
                |ui, (names, picked): &mut State| {
                    crate::PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::Pro);
                    let labels: Vec<&str> = names.iter().map(String::as_str).collect();
                    let every: Vec<usize> = (0..labels.len()).collect();
                    let r = Rect::from_min_size(pos2(760.0, 10.0), vec2(CHEVRON_W, 26.0));
                    let mut got = None;
                    overflow_button(ui, ui.scope_id().with("docs-overflow"), r, "More documents", &labels, &every, None, &mut got);
                    if got.is_some() {
                        *picked = got;
                    }
                },
                (names, None),
            );
            h.run_steps(4);
            h.get_by_label("More documents").click();
            h.run_steps(4);
            h.run_steps(4);
            h
        }

        #[test]
        fn a_long_list_scrolls_and_its_last_entry_can_be_picked() {
            let mut h = open(60);
            let last = 59;
            let first_row = h.get_by_label("Document 0").rect();
            assert!(first_row.bottom() <= WINDOW.y, "the list starts inside the window: {first_row:?}");
            assert!(h.query_by_label("Document 59").is_none_or(|n| n.rect().bottom() > WINDOW.y), "not all of it fits");
            h.hover_at(first_row.center());
            for _ in 0..40 {
                h.event(egui::Event::MouseWheel {
                    source: egui::MouseWheelSource::Wheel,
                    unit: egui::MouseWheelUnit::Line,
                    delta: vec2(0.0, -10.0),
                    modifiers: egui::Modifiers::NONE,
                    phase: egui::TouchPhase::Move,
                });
                h.run_steps(4);
            }
            let row = h.get_by_label(&format!("Document {last}")).rect();
            assert!(row.top() >= 0.0 && row.bottom() <= WINDOW.y + 0.5, "the last entry scrolled into view: {row:?}");
            h.get_by_label(&format!("Document {last}")).click();
            h.run_steps(4);
            assert_eq!(h.state().1, Some(last));
        }

        #[test]
        fn clicking_the_scrollbar_track_keeps_the_list_open() {
            let mut h = open(60);
            let first_row = h.get_by_label("Document 0").rect();
            // The popup's own rect, then the scroll bar's track: the right edge inside the frame, below the thumb.
            let popup = h.ctx.memory(|m| m.layer_ids().last().and_then(|l| m.area_rect(l.id))).expect("the open list is the topmost layer");
            assert!(popup.contains_rect(first_row), "the first row is inside the popup: {popup:?} vs {first_row:?}");
            let style = h.ctx.global_style();
            let bar = style.spacing.scroll;
            let frame_right = f32::from(egui::Frame::popup(&style).inner_margin.right);
            let x = popup.right() - frame_right - bar.bar_outer_margin - bar.bar_width / 2.0;
            let at = pos2(x, popup.bottom() - 20.0);
            assert!(popup.contains(at), "the click lands in the popup: {at:?} in {popup:?}");
            assert!(at.y > popup.top() + popup.height() * 0.5, "below the thumb at the top: {at:?} in {popup:?}");
            h.hover_at(at);
            h.run_steps(2);
            click_at(&mut h, at);
            h.run_steps(4);
            assert!(h.query_by_label("Document 0").is_some(), "the list is still open after a click on the scroll bar track");
            assert_eq!(h.state().1, None, "nothing was picked");
            assert!(h.query_by_label("Document 0").is_none_or(|n| n.rect().top() < first_row.top()), "the track click paged the list (x = {x})");
        }

        #[test]
        fn picking_an_entry_closes_the_list() {
            let mut h = open(5);
            h.get_by_label("Document 2").click();
            h.run_steps(4);
            assert_eq!(h.state().1, Some(2));
            assert!(h.query_by_label("Document 2").is_none(), "the list closed after the pick");
        }

        #[test]
        fn a_click_outside_closes_the_list() {
            let mut h = open(5);
            click_at(&mut h, pos2(100.0, 400.0));
            h.run_steps(4);
            assert!(h.query_by_label("Document 2").is_none(), "the list closed");
            assert_eq!(h.state().1, None);
        }

        #[test]
        fn escape_closes_the_list() {
            let mut h = open(5);
            h.key_press(egui::Key::Escape);
            h.run_steps(4);
            assert!(h.query_by_label("Document 2").is_none(), "the list closed");
        }

        #[test]
        fn a_short_list_shows_every_entry_without_a_scroll_bar() {
            let h = open(3);
            let rows: Vec<Rect> = (0..3).map(|i| h.get_by_label(&format!("Document {i}")).rect()).collect();
            assert!(rows.windows(2).all(|w| w[0].bottom() <= w[1].top() + 0.5 && w[1].top() - w[0].bottom() < 12.0), "{rows:?}");
            let height = rows[2].bottom() - rows[0].top();
            assert!(height < 120.0, "three entries take their natural height: {height}");
        }
    }
}
