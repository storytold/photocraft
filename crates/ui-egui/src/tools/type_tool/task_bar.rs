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

use craft::behavior::IconId;
use craft::behavior::kurbo::Point;
use craft::ui::buttons::{IconButton, ToggleButton};
use craft::ui::canvas_controls::{TaskBar, TaskBarCx};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _};
use craft::ui::display::Divider;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::text::NumberField;
use egui::{Color32, Stroke};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// Returns true when the bar's conditions are met (guards `ui.inspect` too).
pub fn eligible(app: &PhotocraftApp) -> bool {
    wanted(app) && app.ui.text_edit.is_some()
}

/// The bar's pre-draw guard: tool, toggle, session and dialog.
fn wanted(app: &PhotocraftApp) -> bool {
    app.ui.tool.is_type()
        && app.ui.panels.context_task_bar
        && app.session.active().is_some()
        && !app.dialog_open()
}

/// Screen rect that anchors the bar: the text layer's bounding box in document coordinates.
/// For box text, uses the box's four corners; for point text, unions all line extents.
fn text_anchor(app: &mut PhotocraftApp) -> Option<[Point; 4]> {
    use photocraft_doc::text::TextShape;

    let ed = app.ui.text_edit.as_ref()?;
    let id = super::LayerId(ed.layer);
    let (l, aff, _) = super::layout::layout(app, id)?;

    let to_doc = |x: f32, y: f32| -> Point {
        let p = aff.apply(photocraft_geom::Point::new(f64::from(x), f64::from(y)));
        Point::new(p.x, p.y)
    };

    let shape = app.session.active().and_then(|s| {
        super::text_layer(&s.doc, id).map(|t| t.shape.clone())
    });

    match shape {
        Some(TextShape::Box { x, y, width, height }) => {
            Some([
                to_doc(x, y),
                to_doc(x + width, y),
                to_doc(x + width, y + height),
                to_doc(x, y + height),
            ])
        }
        _ => {
            let mut min_x = f32::INFINITY;
            let mut min_y = f32::INFINITY;
            let mut max_x = f32::NEG_INFINITY;
            let mut max_y = f32::NEG_INFINITY;
            for ln in &l.lines {
                let (x0, x1) = (ln.x0, ln.x1.max(ln.x0 + 1.0));
                let (y0, y1) = (ln.baseline - ln.ascent, ln.baseline + ln.descent);
                let (tx0, ty0) = l.to_text(x0, y0);
                let (tx1, ty1) = l.to_text(x1, y1);
                min_x = min_x.min(tx0).min(tx1);
                max_x = max_x.max(tx0).max(tx1);
                min_y = min_y.min(ty0).min(ty1);
                max_y = max_y.max(ty0).max(ty1);
            }
            if min_x.is_finite() && min_y.is_finite() {
                Some([
                    to_doc(min_x, min_y),
                    to_doc(max_x, min_y),
                    to_doc(max_x, max_y),
                    to_doc(min_x, max_y),
                ])
            } else {
                None
            }
        }
    }
}

/// Shows the bar under the active text layer inside `area` (the canvas rect).
pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, area: egui::Rect) {
    if !wanted(app) {
        return;
    }
    let Some(corners) = text_anchor(app) else { return };
    let mut place = app.ui.panels.task_bar;
    let output = TaskBar::new(label(tl!("Contextual Task Bar")), &corners, Bind::bound(&mut place)).show(ui, area, |ui, cx| {
        contents(app, ui, cx);
    });
    app.ui.panels.task_bar = place;
    if output.output.hide {
        app.ui.panels.context_task_bar = false;
        return;
    }
    let r = output.output.rect;
    app.ui.task_bar_shown = Some([r.min.x, r.min.y, r.max.x, r.max.y]);
}

fn label(text: &str) -> Label {
    Label::from(text.to_string())
}

fn type_unit(app: &PhotocraftApp) -> (f32, &'static str) {
    match app.session.prefs().units_and_rulers.type_units {
        photocraft_engine::prefs::TypeUnit::Points => (1.0, "pt"),
        photocraft_engine::prefs::TypeUnit::Pixels => {
            let dpi = app.session.active().map_or(72.0, |d| d.doc.resolution_dpi.max(1.0));
            (dpi / 72.0, "px")
        }
        photocraft_engine::prefs::TypeUnit::Millimeters => (72.0 / 25.4, "mm"),
    }
}

/// The bar's contents: font family, size, bold, alignment, colour, ⋯, commit/cancel.
fn contents(app: &mut PhotocraftApp, ui: &mut egui::Ui, cx: &mut TaskBarCx<'_>) {
    use super::defaults::{apply, shown_scale};
    use super::editing::{cancel, commit_with};
    use super::fonts::{families, selected_style, styles};
    use super::options_bar::styles_at;

    // Current styles — font, size, colour.
    let char_style = styles_at(app).map(|(c, _)| c);
    let shown = char_style
        .as_ref()
        .map(|c| (c.font_family.clone(), selected_style(c), c.size_pt * shown_scale(app), c.color));
    let o = app.ui.tool_options.clone();
    let (mut fam, mut style, size) = match &shown {
        Some((f, s, z, _)) => (
            f.clone(),
            if s.is_empty() { "Regular".into() } else { s.clone() },
            *z,
        ),
        None => (o.type_font.clone(), o.type_style.clone(), o.type_size),
    };

    // --- Font family dropdown ---
    let font_families = families();
    let entries: Vec<Entry<String>> = font_families.iter().map(|f| Entry::new(f.clone(), f.clone())).collect();
    let pick = Select::new(label(tl!("Font")), Bind::bound(&mut fam), &entries).show(ui);
    if pick.events.committed().is_some() {
        app.ui.tool_options.type_font = fam.clone();
        let st = styles(&fam);
        if !st.contains(&style) {
            style = st.first().cloned().unwrap_or_else(|| "Regular".into());
            app.ui.tool_options.type_style = style.clone();
            apply(app, json!({"font": fam, "fontStyle": style}), None);
        } else {
            apply(app, json!({"font": fam}), None);
        }
    }

    // --- Font size field ---
    let (unit, unit_label) = type_unit(app);
    let mut display = Some(f64::from(size * unit));
    let size_field = NumberField::new(label(tl!("Size")), Bind::bound(&mut display))
        .range(f64::from(unit)..=1296.0 * f64::from(unit))
        .unit(unit_label)
        .show(ui);
    if size_field.edit.events.committed().is_some() && let Some(d) = display {
        let pt = (d / f64::from(unit)) as f32;
        app.ui.tool_options.type_size = pt;
        apply(app, json!({"size": pt, "metricsAsShown": true}), None);
    }

    // --- Bold toggle ---
    let font_is_bold = char_style.as_ref().map_or_else(
        || style.contains("Bold") || o.type_style.contains("Bold"),
        |c| c.weight >= 600 || style.contains("Bold"),
    );
    let faux_bold = char_style.as_ref().is_some_and(|c| c.faux_bold);
    let is_bold = faux_bold || font_is_bold;

    let bold_button = ToggleButton::new(label(tl!("Bold")), Bind::given(is_bold))
        .icon(IconId::new("bold"))
        .icon_only()
        .show(ui);
    if bold_button.response.clicked() {
        if is_bold {
            if faux_bold && !font_is_bold {
                apply(app, json!({"fauxBold": false}), None);
            } else {
                let is_italic = char_style.as_ref().map_or_else(
                    || {
                        let s = style.to_lowercase();
                        let os = o.type_style.to_lowercase();
                        s.contains("italic") || s.contains("oblique") || os.contains("italic") || os.contains("oblique")
                    },
                    |c| c.italic || style.to_lowercase().contains("italic") || style.to_lowercase().contains("oblique"),
                );
                let new_style = resolve_bold_counterpart(&fam, &style, is_italic, false);
                app.ui.tool_options.type_style = new_style.clone();
                apply(app, json!({"fontStyle": new_style, "fauxBold": false}), None);
            }
        } else {
            let is_italic = char_style.as_ref().map_or_else(
                || {
                    let s = style.to_lowercase();
                    let os = o.type_style.to_lowercase();
                    s.contains("italic") || s.contains("oblique") || os.contains("italic") || os.contains("oblique")
                },
                |c| c.italic || style.to_lowercase().contains("italic") || style.to_lowercase().contains("oblique"),
            );
            let new_style = resolve_bold_counterpart(&fam, &style, is_italic, true);
            app.ui.tool_options.type_style = new_style.clone();
            apply(app, json!({"fontStyle": new_style}), None);
        }
    }

    Divider::vertical().show(ui);

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
        let btn = ToggleButton::new(label(tip), Bind::given(shown_align == align))
            .icon(IconId::new(icon))
            .icon_only()
            .show(ui);
        if btn.response.clicked() {
            app.ui.tool_options.type_align = align.into();
            apply(app, json!({"align": align}), None);
        }
    }

    Divider::vertical().show(ui);

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
    let border_color = Color32::from_gray(100);
    ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, border_color), egui::StrokeKind::Outside);
    let resp = resp.on_hover_text(tl!("Set text color"));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Set the text color")));
    if resp.clicked() {
        super::defaults::open_color_picker(app, [c[0], c[1], c[2]]);
    }

    // --- ⋯ menu ---
    cx.more_menu(ui);

    // --- Commit / Cancel ---
    if IconButton::new(IconId::new("check"))
        .label(tl!("Commit any current edits (Cmd+Enter)"))
        .show(ui)
        .clicked()
    {
        commit_with(app, ui.input(|i| i.modifiers));
    }
    if IconButton::new(IconId::new("x"))
        .label(tl!("Cancel any current edits (Esc)"))
        .show(ui)
        .clicked()
    {
        cancel(app);
    }
}

/// Resolves the bold or non-bold counterpart style for a font family, preserving italic/oblique styling.
pub(crate) fn resolve_bold_counterpart(fam: &str, current_style: &str, is_italic: bool, want_bold: bool) -> String {
    let available = super::fonts::styles(fam);
    let lower_curr = current_style.to_lowercase();
    let is_oblique = lower_curr.contains("oblique")
        || (!lower_curr.contains("italic") && available.iter().any(|s| s.to_lowercase().contains("oblique")));

    let target = match (is_italic, is_oblique, want_bold) {
        (true, true, true) => "Bold Oblique",
        (true, true, false) => "Oblique",
        (true, false, true) => "Bold Italic",
        (true, false, false) => "Italic",
        (false, _, true) => "Bold",
        (false, _, false) => "Regular",
    };

    if let Some(s) = available.iter().find(|s| s.eq_ignore_ascii_case(target)) {
        return s.clone();
    }

    let faces = photocraft_text::try_shared()
        .and_then(|shared| shared.lock().ok().map(|mut e| e.fonts.faces(fam)))
        .unwrap_or_default();

    let style_info = |s: &str| -> (bool, f32) {
        if let Some(f) = faces.iter().find(|f| f.style.eq_ignore_ascii_case(s)) {
            (f.italic, f.weight)
        } else {
            let lower = s.to_lowercase();
            let it = lower.contains("italic") || lower.contains("oblique");
            let weight = if lower.contains("thin") || lower.contains("hairline") {
                100.0
            } else if lower.contains("extralight") || lower.contains("ultralight") {
                200.0
            } else if lower.contains("light") {
                300.0
            } else if lower.contains("medium") {
                500.0
            } else if lower.contains("semibold") || lower.contains("demibold") {
                600.0
            } else if lower.contains("extrabold") || lower.contains("ultrabold") {
                800.0
            } else if lower.contains("black") || lower.contains("heavy") {
                900.0
            } else if lower.contains("bold") {
                700.0
            } else {
                400.0
            };
            (it, weight)
        }
    };

    let target_weight = if want_bold { 700.0 } else { 400.0 };

    let best = available
        .iter()
        .filter(|s| {
            let (it, w) = style_info(s);
            let is_bd = w >= 600.0;
            it == is_italic && is_bd == want_bold
        })
        .min_by(|a, b| {
            let (_, wa) = style_info(a);
            let (_, wb) = style_info(b);
            (wa - target_weight).abs().total_cmp(&(wb - target_weight).abs())
        });

    if let Some(s) = best {
        return s.clone();
    }

    target.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app_with_doc() -> Box<PhotocraftApp> {
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
            mask: false,
            document: None,
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

    /// Preserving oblique styling when toggling bold.
    #[test]
    fn resolve_bold_counterpart_preserves_oblique() {
        // Toggling bold on oblique selects Bold Oblique
        assert_eq!(resolve_bold_counterpart("CustomFamily", "Oblique", true, true), "Bold Oblique");
        // Toggling bold off on Bold Oblique selects Oblique
        assert_eq!(resolve_bold_counterpart("CustomFamily", "Bold Oblique", true, false), "Oblique");
        // Regular / Bold fallback
        assert_eq!(resolve_bold_counterpart("CustomFamily", "Regular", false, true), "Bold");
        assert_eq!(resolve_bold_counterpart("CustomFamily", "Bold", false, false), "Regular");
        // Bundled Inter: toggling bold picks Bold (or SemiBold if Bold absent)
        let bold_style = resolve_bold_counterpart("Inter", "Regular", false, true);
        assert!(bold_style == "Bold" || bold_style == "SemiBold");
        assert_eq!(resolve_bold_counterpart("Inter", &bold_style, false, false), "Regular");
    }

    /// Preserving italic styling when toggling bold.
    #[test]
    fn resolve_bold_counterpart_preserves_italic() {
        assert_eq!(resolve_bold_counterpart("CustomFamily", "Italic", true, true), "Bold Italic");
        assert_eq!(resolve_bold_counterpart("CustomFamily", "Bold Italic", true, false), "Italic");
    }

    /// `text_anchor` produces four corners for the active text layer.
    #[test]
    fn text_anchor_produces_four_corners() {
        let mut app = app_with_doc();
        let created = app
            .run(
                "type.create",
                json!({
                    "text": "PhotoCraft Text",
                    "size": 32,
                    "x": 100,
                    "y": 100,
                }),
            )
            .unwrap();
        let layer = created["layer"].as_u64().unwrap();
        app.sync_views();

        app.ui.tool = crate::state::Tool::Type;
        app.ui.panels.context_task_bar = true;
        app.ui.text_edit = Some(crate::state::TextEdit {
            layer,
            caret: 0,
            anchor: 0,
            session: String::new(),
            created: false,
            mask: false,
            document: None,
        });

        let corners = text_anchor(&mut app).expect("anchor rect exists");
        assert_eq!(corners.len(), 4);
    }

    /// When faux bold alone makes the button active, clearing it preserves the underlying face (e.g. Light).
    #[test]
    fn clear_faux_bold_alone_preserves_underlying_face() {
        let mut app = app_with_doc();
        let created = app
            .run(
                "type.create",
                json!({
                    "text": "Light Text",
                    "size": 24,
                    "fontStyle": "Light",
                    "fauxBold": true,
                }),
            )
            .unwrap();
        let layer = super::super::LayerId(created["layer"].as_u64().unwrap());
        app.sync_views();

        let t = super::super::text_layer(&app.session.active().unwrap().doc, layer).unwrap();
        let c = &t.char_runs()[0].style;
        assert!(c.faux_bold);
        assert_eq!(c.font_style, "Light");

        // Verify font_is_bold is false and faux_bold is true
        let font_is_bold = c.weight >= 600 || c.font_style.contains("Bold");
        assert!(!font_is_bold);
        assert!(c.faux_bold);

        // Clearing faux bold without replacing fontStyle keeps "Light"
        app.ui.text_edit = Some(crate::state::TextEdit {
            layer: layer.0,
            caret: 0,
            anchor: 0,
            session: String::new(),
            created: false,
            mask: false,
            document: None,
        });
        super::super::defaults::apply(&mut app, json!({"fauxBold": false}), None);
        let t_after = super::super::text_layer(&app.session.active().unwrap().doc, layer).unwrap();
        let c_after = &t_after.char_runs()[0].style;
        assert!(!c_after.faux_bold);
        assert_eq!(c_after.font_style, "Light");
    }

    /// `inspect` reports the Type task bar rectangle when eligible.
    #[test]
    fn inspect_reports_type_task_bar_when_eligible() {
        let mut app = app_with_doc();
        app.ui.tool = crate::state::Tool::Type;
        app.ui.panels.context_task_bar = true;
        app.ui.text_edit = Some(crate::state::TextEdit {
            layer: 0,
            caret: 0,
            anchor: 0,
            session: String::new(),
            created: false,
            mask: false,
            document: None,
        });
        app.ui.task_bar_shown = Some([10.0, 20.0, 110.0, 50.0]);

        let ctx = egui::Context::default();
        let val = crate::control::inspect(&app, &ctx);
        assert_eq!(val["taskBar"]["enabled"], true);
        assert_eq!(val["taskBar"]["visible"], true);
        assert_eq!(val["taskBar"]["rect"], json!([10.0, 20.0, 110.0, 50.0]));
    }
}
