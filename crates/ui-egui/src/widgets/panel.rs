use super::{Align2, Color32, CornerRadius, Rect, Response, Sense, Stroke, StrokeKind, Tokens, Ui, pos2, theme, vec2};

/// Match the Spectrum application-menu padding and blue/white hover state in
/// secondary menus (status bar, dock hamburger, layer context menus; #2187).
/// The setting is local to this popup's Ui and leaves other themes unchanged.
pub fn style_spectrum_popup_menu(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    // The 6 pt gap between rows was outside both buttons, so a drag across a menu
    // dropped the highlight (#4602). Padding stays inside the row, which is the hit target.
    ui.spacing_mut().item_spacing.y = 0.0;
    if !t.pro {
        return;
    }
    ui.spacing_mut().button_padding = vec2(10.0, 4.0);
    let v = &mut ui.style_mut().visuals;
    v.widgets.hovered.weak_bg_fill = t.accent;
    v.widgets.hovered.bg_fill = t.accent;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.inactive.corner_radius = CornerRadius::ZERO;
    v.widgets.hovered.corner_radius = CornerRadius::ZERO;
    v.widgets.active.corner_radius = CornerRadius::ZERO;
}

/// An accent insertion line on one edge of `r` while a drag hovers it. Vertical lines stand on
/// `r`'s left or right edge, where `after` is in reading order (the right edge left-to-right, the
/// left edge right-to-left); horizontal lines are never mirrored — rows stack top-down in both
/// directions — so there `after` keeps its physical meaning (bottom, else top).
pub fn drop_line(ui: &Ui, r: Rect, after: bool, vertical: bool, t: &Tokens) {
    let s = Stroke::new(2.0, t.accent);
    if vertical {
        let after = after != crate::direction::is_rtl();
        let x = if after { r.right() } else { r.left() };
        ui.painter().line_segment([pos2(x, r.top() + 2.0), pos2(x, r.bottom() - 2.0)], s);
    } else {
        let y = if after { r.bottom() } else { r.top() };
        ui.painter().line_segment([pos2(r.left() + 4.0, y), pos2(r.right() - 4.0, y)], s);
    }
}

/// Draw a bevelled box (Classic theme) or a flat rounded box.
pub fn surface(ui: &Ui, rect: Rect, fill: Color32, raised: bool) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter();
    p.rect_filled(rect, t.radius_sm, fill);
    if t.bevel {
        let (hi, lo) = if raised { (Color32::WHITE, Color32::from_gray(64)) } else { (Color32::from_gray(64), Color32::WHITE) };
        p.line_segment([rect.left_bottom(), rect.left_top()], Stroke::new(1.0, hi));
        p.line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, hi));
        p.line_segment([rect.right_top(), rect.right_bottom()], Stroke::new(1.0, lo));
        p.line_segment([rect.right_bottom(), rect.left_bottom()], Stroke::new(1.0, lo));
    }
}

/// Inner margin of a dock panel's body: a Pro panel's, or a Studio card's below its tab strip.
pub(crate) fn body_margin(pro: bool) -> egui::Margin {
    if pro { egui::Margin::same(8) } else { egui::Margin { left: 8, right: 8, top: 6, bottom: 10 } }
}

/// Height of a panel footer's bar ([`panel_footer`]), under its 1 px line.
pub const FOOTER_BAR: f32 = 30.0;

/// The height a [`panel_footer`] takes from the panel body: its line, and its bar down to the
/// body's bottom margin (the bar runs on over the margin).
pub fn footer_height(ui: &Ui) -> f32 {
    1.0 + FOOTER_BAR - body_margin(Tokens::get(ui.ctx()).pro).bottom as f32
}

/// A panel footer as in Photoshop's Layers and History panels: a bar of buttons under a line that
/// runs from edge to edge of the panel, the buttons centred in the bar's height and laid out from
/// the reading end (the right edge, or the left in a right-to-left UI). Drawn last in a body that
/// fills its group (leave it [`footer_height`]), the bar ends at the panel's bottom edge.
pub fn panel_footer<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    let m = body_margin(t.pro);
    let (r, _) = ui.allocate_exact_size(vec2(crate::direction::row_width(ui), footer_height(ui)), Sense::hover());
    let y = r.top() + 0.5;
    ui.painter().line_segment([pos2(r.left() - m.left as f32, y), pos2(r.right() + m.right as f32, y)], Stroke::new(1.0, t.separator));
    let bar = Rect::from_min_size(pos2(r.left(), r.top() + 1.0), vec2(r.width(), FOOTER_BAR));
    let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(crate::text::direction::trailing()));
    // The bar runs on over the body's bottom margin, past where the body's own clip ends: without
    // this its buttons were cut off there, a 26 px hover highlight drawn about 21 px tall.
    bar_ui.set_clip_rect(bar_ui.clip_rect().union(bar));
    bar_ui.spacing_mut().item_spacing.x = 2.0;
    add(&mut bar_ui)
}

/// The panel menu button (hamburger in Pro, ellipsis in Studio), matching the dock and cards.
pub fn panel_menu_button(ui: &mut Ui) -> Response {
    let t = Tokens::get(ui.ctx());
    let size = vec2(20.0, 20.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let tint = crate::icons::button_chrome(ui, rect, false, resp.hovered());
    if t.pro {
        for k in 0..3 {
            let y = rect.center().y - 3.5 + k as f32 * 3.5;
            ui.painter().line_segment([pos2(rect.center().x - 5.0, y), pos2(rect.center().x + 5.0, y)], Stroke::new(1.0, tint));
        }
    } else {
        crate::icons::paint(ui, rect, "ellipsis", 14.0, tint);
    }
    let tip = tl!("Panel menu");
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    resp.on_hover_text(tip)
}

/// The panel menu button's popup: an `egui::Popup::menu` whose body goes through
/// [`crate::text::direction::popup_body`], so a panel "…" menu mirrors in a right-to-left UI
/// by default instead of each call site remembering the scope. Left-to-right the popup's own
/// layout is used unchanged. Not yet re-exported through `widgets`, so only tests use it.
#[cfg_attr(not(test), allow(dead_code))]
pub fn panel_menu_popup<R>(button: &Response, add_contents: impl FnOnce(&mut Ui) -> R) -> Option<egui::InnerResponse<R>> {
    egui::Popup::menu(button).show(|ui| crate::text::direction::popup_body(ui, add_contents))
}

pub fn pill_tab(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let t = Tokens::get(ui.ctx());
    let translated = tl!(label);
    let font = theme::medium(12.5);
    // Pin the paragraph base direction like `direction::body_label`, so an RTL label keeps its
    // phrase order instead of shaping with an auto-detected direction. LTR keeps `layout_no_wrap`.
    let galley = if crate::text::direction::is_rtl() {
        let mut job = egui::text::LayoutJob::simple(translated.to_string(), font, t.text, f32::INFINITY);
        job.direction = crate::text::direction::text_direction();
        ui.painter().layout_job(job)
    } else {
        ui.painter().layout_no_wrap(translated.to_string(), font, t.text)
    };
    let size = vec2(galley.size().x + 20.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if selected {
        surface(ui, rect, t.hover, true);
        if !t.bevel {
            ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
        }
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, t.radius_sm, t.hover.gamma_multiply(0.6));
    }
    let color = if selected { t.text } else { t.text_dim };
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, color);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, translated));
    resp
}

/// Small caps section label.
pub fn section_label(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(tl!(text)).font(theme::medium(11.5)).color(t.text_faint));
}

/// A collapsible section header: optionally a gap and [`hairline`], then a clickable row with a
/// chevron and the title leading at the reading start (the right edge in a right-to-left UI).
/// Returns the row's response and whether the section is open. `id` keys the open state.
pub fn section_header(ui: &mut Ui, id: &str, title: &str, height: f32, hairline: bool) -> egui::InnerResponse<bool> {
    let t = Tokens::get(ui.ctx());
    let key = ui.scope_id().with(("section-header", id));
    let mut open = ui.data(|d| d.get_temp::<bool>(key)).unwrap_or(true);
    if hairline {
        ui.add_space(theme::ROW_GAP);
        self::hairline(ui);
    }
    let (r, resp) = ui.allocate_exact_size(vec2(crate::direction::row_width(ui), height), Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    crate::icons::paint(
        ui,
        Rect::from_center_size(pos2(crate::direction::start_x(r, 7.0), r.center().y), vec2(12.0, 12.0)),
        if open { "chevron-down" } else { "chevron-right" },
        11.0,
        t.text_dim,
    );
    ui.painter().text(
        pos2(crate::direction::start_x(r, 18.0), r.center().y),
        crate::direction::start_align(),
        tl!(title),
        theme::semibold(12.0),
        t.text,
    );
    if resp.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(key, open));
    }
    let label = title.to_string();
    resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::DisclosureTriangle, true, open, &label));
    if open {
        ui.add_space(theme::ROW_GAP / 2.0);
    }
    egui::InnerResponse::new(open, resp)
}

/// A group header's painted anchors at `indent` from the row's reading-start edge: chevron,
/// folder icon, name and count x, and the name's alignment.
#[derive(Debug)]
pub struct HeaderAnchors {
    pub chevron_x: f32,
    pub folder_x: f32,
    pub name_x: f32,
    pub count_x: f32,
    pub halign: Align2,
}

/// [`HeaderAnchors`] for a header row `r` indented `indent` from its reading-start edge.
pub fn header_anchors(r: Rect, indent: f32) -> HeaderAnchors {
    let sx = crate::direction::start_x(r, indent);
    let rtl = crate::direction::is_rtl();
    HeaderAnchors {
        chevron_x: if rtl { sx - 14.0 } else { sx },
        folder_x: if rtl { sx - 34.0 } else { sx + 18.0 },
        name_x: if rtl { sx - 40.0 } else { sx + 40.0 },
        count_x: crate::direction::end_x(r, 8.0),
        halign: crate::direction::start_align(),
    }
}

/// Hairline separator.
pub fn hairline(ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(crate::direction::row_width(ui), 1.0), Sense::hover());
    ui.painter().line_segment([r.left_center(), r.right_center()], Stroke::new(1.0, t.separator));
}

/// Vertical hairline for horizontal layouts.
pub fn vline(ui: &mut Ui, height: f32) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(9.0, height), Sense::hover());
    ui.painter().line_segment([r.center_top(), r.center_bottom()], Stroke::new(1.0, t.separator));
}

/// Small chevron for dropdowns (replaces egui's filled triangle).
pub fn chevron_icon(ui: &Ui, rect: Rect, visuals: &egui::style::WidgetVisuals, _open: bool) {
    let c = rect.center();
    let s = 3.2;
    let stroke = Stroke::new(1.3, visuals.fg_stroke.color.gamma_multiply(0.8));
    ui.painter().line_segment([c + vec2(-s, -s * 0.5), c + vec2(0.0, s * 0.5)], stroke);
    ui.painter().line_segment([c + vec2(0.0, s * 0.5), c + vec2(s, -s * 0.5)], stroke);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::{Dir, direction_scope};

    /// Render `panel_menu_popup` open over its button — a click on frame 1, then steady frames —
    /// and return the popup area's rect and the painted text galley rects.
    fn panel_menu_texts(dir: Dir) -> (egui::Rect, Vec<(String, egui::Rect)>) {
        let _scope = direction_scope(dir);
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|s| s.right_to_left = dir == Dir::Rtl);
        let mut btn_rect = egui::Rect::NOTHING;
        let mut popup = egui::Rect::NOTHING;
        let mut texts = Vec::new();
        for frame in 0..4 {
            let mut events = Vec::new();
            if frame == 1 {
                let pos = btn_rect.center();
                events.push(egui::Event::PointerMoved(pos));
                events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
            }
            if frame == 2 {
                let pos = btn_rect.center();
                events.push(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
            }
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let btn = panel_menu_button(ui);
                    btn_rect = btn.rect;
                    panel_menu_popup(&btn, |ui| {
                        ui.set_min_width(180.0);
                        crate::menu_keys::menu_row(ui, "Panel menu item", None, false, true);
                    })
                    .map(|r| popup = r.response.rect);
                },
            );
            texts = out
                .shapes
                .iter()
                .filter_map(|cs| match &cs.shape {
                    egui::Shape::Text(ts) => Some((ts.galley.text().to_string(), cs.shape.visual_bounding_rect())),
                    _ => None,
                })
                .collect();
            out.textures_delta.clear();
        }
        (popup, texts)
    }

    /// A panel menu popup's rows mirror in a right-to-left UI (their labels hug the popup's
    /// right edge); left-to-right the popup's own pinned layout keeps them at the left edge.
    #[test]
    fn panel_menu_popup_mirrors_only_in_rtl() {
        let at = |v: &[(String, egui::Rect)], s: &str| v.iter().find(|(t, _)| t.contains(s)).map(|(_, r)| *r).unwrap_or(egui::Rect::NOTHING);
        let (rtl_popup, rtl) = panel_menu_texts(Dir::Rtl);
        let (ltr_popup, ltr) = panel_menu_texts(Dir::Ltr);
        assert!(rtl_popup.is_positive() && ltr_popup.is_positive(), "the popups opened: {rtl_popup:?} {ltr_popup:?}");
        let slack = 32.0;
        let label = at(&rtl, "Panel menu item");
        assert!(label.is_positive() && rtl_popup.right() - label.right() < slack, "rtl: the row hugs the popup's right edge: {label:?} in {rtl_popup:?}");
        let label = at(&ltr, "Panel menu item");
        assert!(label.is_positive() && label.left() - ltr_popup.left() < slack, "ltr: the row keeps the popup's left edge: {label:?} in {ltr_popup:?}");
    }

    /// Paint a `drop_line` in `dir` and return the line's first point.
    fn drop_line_start(dir: Dir, after: bool, vertical: bool) -> egui::Pos2 {
        let _scope = direction_scope(dir);
        let ctx = egui::Context::default();
        let r = Rect::from_min_size(pos2(100.0, 50.0), vec2(80.0, 24.0));
        let mut p = egui::Pos2::new(f32::NAN, f32::NAN);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let t = Tokens::get(ui.ctx());
            drop_line(ui, r, after, vertical, &t);
        });
        for shape in &out.shapes {
            if let egui::Shape::LineSegment { points, .. } = &shape.shape {
                p = points[0];
            }
        }
        out.textures_delta.clear();
        p
    }

    /// A vertical drop line's `after` is in reading order: the right edge left-to-right, the left
    /// edge right-to-left; a horizontal line never mirrors.
    #[test]
    fn drop_line_after_is_in_reading_order() {
        assert!((drop_line_start(Dir::Ltr, true, true).x - 180.0).abs() < 1e-4);
        assert!((drop_line_start(Dir::Ltr, false, true).x - 100.0).abs() < 1e-4);
        assert!((drop_line_start(Dir::Rtl, true, true).x - 100.0).abs() < 1e-4, "rtl: after = the left edge");
        assert!((drop_line_start(Dir::Rtl, false, true).x - 180.0).abs() < 1e-4, "rtl: before = the right edge");
        // Horizontal lines stack top-down in both directions.
        assert!((drop_line_start(Dir::Ltr, true, false).y - 74.0).abs() < 1e-4);
        assert!((drop_line_start(Dir::Rtl, true, false).y - 74.0).abs() < 1e-4);
        assert!((drop_line_start(Dir::Rtl, false, false).y - 50.0).abs() < 1e-4);
    }

    /// `header_anchors` reproduces the hand-mirrored group-header table in both directions.
    #[test]
    fn header_anchors_lead_from_the_reading_start() {
        let r = Rect::from_min_size(pos2(100.0, 50.0), vec2(200.0, 24.0));
        let _ltr = direction_scope(Dir::Ltr);
        let a = header_anchors(r, 4.0);
        assert!(
            (a.chevron_x - 104.0).abs() < 1e-4 && (a.folder_x - 122.0).abs() < 1e-4 && (a.name_x - 144.0).abs() < 1e-4
                && (a.count_x - 292.0).abs() < 1e-4 && a.halign == Align2::LEFT_CENTER,
            "ltr: {a:?}"
        );
        drop(_ltr);
        let _rtl = direction_scope(Dir::Rtl);
        let a = header_anchors(r, 4.0);
        assert!(
            (a.chevron_x - 282.0).abs() < 1e-4 && (a.folder_x - 262.0).abs() < 1e-4 && (a.name_x - 256.0).abs() < 1e-4
                && (a.count_x - 108.0).abs() < 1e-4 && a.halign == Align2::RIGHT_CENTER,
            "rtl: {a:?}"
        );
    }

    /// Draw a `section_header` in `dir`; return its row rect and the title galley's rect.
    fn section_header_rects(dir: Dir) -> (Rect, Rect) {
        let _scope = direction_scope(dir);
        let ctx = egui::Context::default();
        let mut row = Rect::NOTHING;
        let mut out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 100.0))), ..Default::default() },
            |ui| {
                row = section_header(ui, "sec", "Brushes", 24.0, true).response.rect;
            },
        );
        let mut title = Rect::NOTHING;
        for shape in &out.shapes {
            if let egui::Shape::Text(ts) = &shape.shape
                && ts.galley.text().contains("Brushes")
            {
                title = shape.shape.visual_bounding_rect();
            }
        }
        out.textures_delta.clear();
        (row, title)
    }

    /// A section header's title leads at the reading start: 18 pt from the left edge
    /// left-to-right, hugging the right edge right-to-left.
    #[test]
    fn section_header_title_hugs_the_reading_start() {
        let (lrow, ltitle) = section_header_rects(Dir::Ltr);
        assert!(ltitle.is_positive() && (ltitle.left() - (lrow.left() + 18.0)).abs() < 2.0, "ltr: {ltitle:?} in {lrow:?}");
        let (rrow, rtitle) = section_header_rects(Dir::Rtl);
        assert!(rtitle.is_positive() && (rrow.right() - 18.0 - rtitle.right()).abs() < 2.0, "rtl: {rtitle:?} in {rrow:?}");
    }

    /// A pill tab's galley pins its paragraph base direction in a right-to-left UI (like
    /// `direction::body_label`, so an Arabic label keeps its phrase order); left-to-right keeps
    /// egui's auto-detected direction, unchanged.
    #[test]
    fn pill_tab_pins_the_galley_direction() {
        for dir in [Dir::Ltr, Dir::Rtl] {
            let _dir = direction_scope(dir);
            let ctx = egui::Context::default();
            ctx.all_styles_mut(|s| s.right_to_left = dir == Dir::Rtl);
            let mut out = ctx.run_ui(
                egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 100.0))), ..Default::default() },
                |ui| {
                    pill_tab(ui, "Toolbar", true);
                },
            );
            let mut found = None;
            for shape in &out.shapes {
                if let egui::Shape::Text(ts) = &shape.shape
                    && ts.galley.job.text.contains("Toolbar")
                {
                    found = Some(ts.galley.job.direction);
                }
            }
            let want = if dir == Dir::Rtl { egui::epaint::text::TextDirection::RightToLeft } else { egui::epaint::text::TextDirection::Auto };
            assert_eq!(found, Some(want), "{dir:?}: the pill label's galley direction");
            out.textures_delta.clear();
        }
    }

    /// Full-width body rows (footer bar, section header, hairline) read `direction::row_width`:
    /// left-to-right that is exactly `ui.available_width()` (pixel-identical LTR), right-to-left
    /// the body's max_rect width, so the computed row width equals the LTR formula in both
    /// directions and never balloons under RTL.
    #[test]
    fn body_rows_read_row_width_in_both_directions() {
        for dir in [Dir::Ltr, Dir::Rtl] {
            let _scope = direction_scope(dir);
            let ctx = egui::Context::default();
            ctx.all_styles_mut(|s| s.right_to_left = dir == Dir::Rtl);
            let mut row_w = 0.0;
            let mut avail = 0.0;
            let mut out = ctx.run_ui(
                egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 100.0))), ..Default::default() },
                |ui| {
                    ui.with_layout(crate::direction::body_layout(), |ui| {
                        row_w = crate::direction::row_width(ui);
                        avail = ui.available_width();
                        hairline(ui);
                    });
                },
            );
            out.textures_delta.clear();
            assert!((row_w - 400.0).abs() < 1e-4, "{dir:?}: row_width is the full body width, got {row_w}");
            if dir == Dir::Ltr {
                assert!((row_w - avail).abs() < 1e-4, "ltr: row_width == available_width ({row_w} vs {avail})");
            }
        }
    }
}
