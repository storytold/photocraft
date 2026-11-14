//! The Type tool's contextual task bar: a compact floating bar under the active text layer,
//! shown while the Type tool (or a variant) is active and a text-edit session is live.
//!
//! Left to right:
//! 1. the grip (the widget's);
//! 2. font family dropdown;
//! 3. font size field;
//! 4. Bold toggle button;
//! 5. Left / Center / Right alignment buttons;
//! 6. text colour swatch;
//! 7. the ⋯ menu (the widget's);
//! 8. Commit (✓) and Cancel (✗) buttons.
//!
//! Shown when Window → Contextual Task Bar is on, a Type-family tool is active, there is an
//! active document, no dialog is open, and a text-edit session is live.

use egui::{Color32, Pos2, Rect, Stroke};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::Tool;
use crate::widgets::task_bar;

/// Returns true when the bar's conditions are met (guards `ui.inspect` too).
pub fn eligible(app: &PhotocraftApp) -> bool {
    wanted(app) && app.ui.text_edit.is_some()
}

/// The bar's pre-draw guard: tool, toggle, session and dialog.
fn wanted(app: &PhotocraftApp) -> bool {
    app.ui.tool.is_type()
        && app.ui.panels.context_task_bar
        && app.session.active().is_some()
        && app.ui.dialogs.is_empty()
}

/// Screen rect that anchors the bar: the text layer's bounding box in screen coordinates.
/// For box text, uses the box's four corners; for point text, unions all line extents.
fn text_anchor(app: &mut PhotocraftApp, xf: &ViewXform) -> Option<Rect> {
    use photocraft_doc::text::TextShape;
    use photocraft_geom::Point;

    let ed = app.ui.text_edit.clone()?;
    let id = super::LayerId(ed.layer);
    let (l, aff, _) = super::layout::layout(app, id)?;

    // text-space → screen
    let scr_t = |x: f32, y: f32| -> Pos2 {
        let p = aff.apply(Point::new(f64::from(x), f64::from(y)));
        xf.to_screen(p.x as f32, p.y as f32)
    };

    let shape = app.session.active().and_then(|s| {
        super::text_layer(&s.doc, id).map(|t| t.shape.clone())
    });

    match shape {
        Some(TextShape::Box { x, y, width, height }) => {
            let corners = [
                scr_t(x, y),
                scr_t(x + width, y),
                scr_t(x + width, y + height),
                scr_t(x, y + height),
            ];
            Some(Rect::from_points(&corners))
        }
        _ => {
            // Point text (and area text as a fallback): union of line extents in text space.
            let pts: Vec<Pos2> = l
                .lines
                .iter()
                .flat_map(|ln| {
                    let (tx0, ty0) = l.to_text(ln.x0, ln.baseline - ln.ascent);
                    let (tx1, ty1) = l.to_text(ln.x1.max(ln.x0 + 1.0), ln.baseline + ln.descent);
                    [scr_t(tx0, ty0), scr_t(tx1, ty1)]
                })
                .collect();
            if pts.is_empty() {
                None
            } else {
                Some(Rect::from_points(&pts))
            }
        }
    }
}

/// Draws the bar under the active text layer's screen bounding box.
/// Called from the canvas overlay pass for the active document's tile.
pub fn draw(app: &mut PhotocraftApp, ui: &mut egui::Ui, xf: &ViewXform) {
    if !wanted(app) {
        return;
    }
    let Some(anchor) = text_anchor(app, xf) else { return };
    let ctx = ui.ctx().clone();
    let mut place = app.ui.panels.task_bar;
    let shown = task_bar::show(&ctx, &mut place, anchor, xf.rect, |ui, more| {
        contents(app, ui, more);
    });
    app.ui.panels.task_bar = place;
    let Some(shown) = shown else { return };
    if shown.hide {
        app.ui.panels.context_task_bar = false;
        return;
    }
    let r = shown.rect;
    app.ui.task_bar_shown = Some([r.min.x, r.min.y, r.max.x, r.max.y]);
}

/// The bar's contents: font family, size, bold, alignment, colour, ⋯, commit/cancel.
fn contents(app: &mut PhotocraftApp, ui: &mut egui::Ui, more: &mut task_bar::MoreMenu) {
    use super::defaults::{apply, shown_scale, type_unit};
    use super::editing::{cancel, commit_with};
    use super::fonts::{font_picker, font_picker_tip, fonts_ready, selected_style, styles};
    use super::options_bar::styles_at;

    let t = crate::theme::Tokens::get(ui.ctx());

    // Current styles — font, size, colour.
    let char_style = styles_at(app).map(|(c, _)| c);
    let shown = char_style
        .as_ref()
        .map(|c| (super::options_bar::shown_family(app, c), selected_style(c), c.size_pt * shown_scale(app), c.color));
    let o = app.ui.tool_options.clone();
    let (mut fam, mut style, size) = match &shown {
        Some((f, s, z, _)) => (
            f.clone(),
            if s.is_empty() { "Regular".into() } else { s.clone() },
            *z,
        ),
        None => (o.type_font.clone(), o.type_style.clone(), o.type_size),
    };

    // --- Font family dropdown (compact, 120 px) ---
    ui.push_id("type-task-bar", |ui| {
        if fonts_ready(ui, 120.0) {
            if let Some(picked) = font_picker(app, ui, &mut fam, 120.0) {
                app.ui.tool_options.type_font = fam.clone();
                let st = styles(&fam);
                let picked = picked.filter(|s| st.contains(s));
                style = match &picked {
                    Some(s) => s.clone(),
                    None if st.contains(&style) => style,
                    None => st.first().cloned().unwrap_or_else(|| "Regular".into()),
                };
                app.ui.tool_options.type_style = style.clone();
                match picked {
                    Some(s) => apply(app, ui.ctx(), json!({"font": fam, "fontStyle": s})),
                    None => apply(app, ui.ctx(), json!({"font": fam})),
                }
            }
            font_picker_tip(ui, tl!("Search and select fonts"));
        }

        // --- Font size field (compact, 54 px) ---
        let (unit, label) = type_unit(app);
        let mut display = size * unit;
        let field = crate::widgets::value_field(ui, &mut display, unit..=1296.0 * unit, label, 54.0)
            .on_hover_text(tl!("Set font size"));
        if field.changed() {
            let pt = display / unit;
            app.ui.tool_options.type_size = pt;
            apply(app, ui.ctx(), json!({"size": pt, "metricsAsShown": true}));
        }

        // --- Bold toggle ---
        let is_bold = char_style.as_ref().map_or_else(
            || style.contains("Bold") || o.type_style.contains("Bold"),
            |c| c.faux_bold || c.weight >= 600 || style.contains("Bold"),
        );

        let toggle_bold = crate::icons::button(ui, "bold", task_bar::BUTTON, is_bold, tl!("Bold"));
        if toggle_bold.clicked() {
            if is_bold {
                let target = if style.contains("Italic") { "Italic" } else { "Regular" };
                let new_style = styles(&fam)
                    .into_iter()
                    .find(|s| s == target)
                    .unwrap_or_else(|| target.into());
                app.ui.tool_options.type_style = new_style.clone();
                apply(app, ui.ctx(), json!({"fontStyle": new_style, "fauxBold": false}));
            } else {
                let target = if style.contains("Italic") { "Bold Italic" } else { "Bold" };
                let available = styles(&fam);
                let new_style = available
                    .iter()
                    .find(|s| *s == target)
                    .or_else(|| available.iter().find(|s| s.contains("Bold")))
                    .cloned()
                    .unwrap_or_else(|| target.into());
                app.ui.tool_options.type_style = new_style.clone();
                apply(app, ui.ctx(), json!({"fontStyle": new_style}));
            }
        }

        crate::widgets::vline(ui, 22.0);
        ui.spacing_mut().item_spacing.x = 2.0;

        // --- Alignment buttons (Left / Center / Right) ---
        let para = styles_at(app).map(|(_, p)| p);
        let shown_align = match &para {
            Some(p) => match p.align {
                photocraft_doc::text::TextAlign::Left => "left",
                photocraft_doc::text::TextAlign::Center => "center",
                photocraft_doc::text::TextAlign::Right => "right",
                _ => "",
            },
            None => o.type_align.as_str(),
        };
        let vertical = app.ui.tool != Tool::HorizontalTypeMask
            && match super::defaults::target_text(app) {
                Some(t) => t.orientation == photocraft_doc::text::Orientation::Vertical,
                None => app.ui.tool.is_vertical_type(),
            };
        let buttons = if vertical {
            [
                ("left", "vertical-align-top", tl!("Top align text")),
                ("center", "vertical-align-middle", tl!("Center text")),
                ("right", "vertical-align-bottom", tl!("Bottom align text")),
            ]
        } else {
            [
                ("left", "align-left", tl!("Left align text")),
                ("center", "align-center", tl!("Center text")),
                ("right", "align-right", tl!("Right align text")),
            ]
        };
        for (align, icon, tip) in buttons {
            if crate::icons::button(ui, icon, task_bar::BUTTON, shown_align == align, tip).clicked() {
                app.ui.tool_options.type_align = align.into();
                apply(app, ui.ctx(), json!({"align": align}));
            }
        }

        ui.spacing_mut().item_spacing.x = 8.0;
        crate::widgets::vline(ui, 22.0);

        // --- Text colour swatch ---
        let c = shown
            .as_ref()
            .map(|s| s.3)
            .map(|c| {
                let v = c.to_rgb();
                [v[0], v[1], v[2], 1.0]
            })
            .unwrap_or(app.session.tools.foreground);
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 18.0), egui::Sense::click());
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        ui.painter().rect_filled(rect, 2.0, Color32::from_rgb(q(c[0]), q(c[1]), q(c[2])));
        ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Outside);
        let resp = resp.on_hover_text(tl!("Set text color"));
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Set the text color")));
        if resp.clicked() {
            super::defaults::open_color_picker(app, [c[0], c[1], c[2]]);
        }

        // --- ⋯ menu ---
        more.show(ui);

        // --- Commit / Cancel ---
        if crate::icons::button_with_icon_size_tip(
            ui,
            "check",
            task_bar::BUTTON,
            14.0,
            false,
            &crate::i18n::fmt(
                tl!("Commit any current edits ({key})"),
                &[("key", &crate::shortcuts::pretty("Cmd+Enter"))],
            ),
            false,
        )
        .clicked()
        {
            commit_with(app, ui.input(|i| i.modifiers));
        }
        if crate::icons::button_with_icon_size_tip(ui, "ban", task_bar::BUTTON, 14.0, false, tl!("Cancel any current edits (Esc)"), false)
            .clicked()
        {
            cancel(app);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app_with_doc() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
        app.sync_views();
        app
    }

    /// `eligible` is false when the Type tool is not active.
    #[test]
    fn eligible_false_when_wrong_tool() {
        let mut app = app_with_doc();
        app.ui.tool = crate::state::Tool::Move;
        app.ui.panels.context_task_bar = true;
        // No text_edit session.
        assert!(!eligible(&app));
    }

    /// `eligible` is false when `context_task_bar` is off, even with the right tool and session.
    #[test]
    fn eligible_false_when_toggle_off() {
        let mut app = app_with_doc();
        app.ui.tool = crate::state::Tool::Type;
        app.ui.panels.context_task_bar = false;
        // Fabricate a minimal TextEdit so the only missing condition is the toggle.
        app.ui.text_edit = Some(crate::state::TextEdit {
            layer: 0,
            caret: 0,
            anchor: 0,
            session: String::new(),
            created: false,
            dragging: false,
            mask: false,
            document: None,
            resize: None,
            preedit: None,
        });
        assert!(!eligible(&app));
    }

    /// `eligible` is false when there is no active text_edit session.
    #[test]
    fn eligible_false_when_no_text_edit() {
        let mut app = app_with_doc();
        app.ui.tool = crate::state::Tool::Type;
        app.ui.panels.context_task_bar = true;
        app.ui.text_edit = None;
        assert!(!eligible(&app));
    }
}
