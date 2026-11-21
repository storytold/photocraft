//! The Type tool's contextual task bar: a compact floating bar under the active text layer,
//! shown while the Type tool (or a variant) is active and a text-edit session is live.
//!
//! Left to right:
//! 1. the grip (the widget's);
//! 2. font family dropdown;
//! 3. font size field;
//! 4. text colour swatch;
//! 5. Left / Center / Right alignment buttons;
//! 6. Warp Text button;
//! 7. the ⋯ menu (the widget's).
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

/// The bar's contents matching the reference application:
/// font family, size, colour, three alignments, warp text, ⋯ menu.
fn contents(app: &mut PhotocraftApp, ui: &mut egui::Ui, cx: &mut TaskBarCx<'_>) {
    use super::defaults::{apply, shown_scale};
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

    // 1. Font family dropdown
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

    // 2. Font size field
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

    Divider::vertical().show(ui);

    // 3. Text colour swatch
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

    Divider::vertical().show(ui);

    // 4. Alignment buttons (Left / Center / Right)
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

    // 5. Warp Text button
    let can_warp = crate::menus::is_enabled(app, "type.warpText");
    if IconButton::new(IconId::new("text_warp"))
        .label(tl!("Create warped text"))
        .enabled(can_warp)
        .show(ui)
        .clicked()
    {
        let ctx = ui.ctx().clone();
        if let Err(e) = crate::menus::invoke(app, &ctx, "type.warpText", json!({})) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }

    // 6. ⋯ menu
    cx.more_menu(ui);
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

    /// The Type contextual task bar shows only while a text-edit session is live,
    /// and not for the Type tool with no session or for other tools.
    #[test]
    fn bar_shows_only_while_text_edit_is_live() {
        let mut app = app_with_doc();
        app.ui.panels.context_task_bar = true;

        // 1. Type tool without an active text-edit session: not eligible.
        app.ui.tool = Tool::Type;
        app.ui.text_edit = None;
        assert!(!eligible(&app));

        // 2. Other tool (e.g. Move), even if a text_edit state existed: not eligible.
        app.ui.tool = Tool::Move;
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

        // 3. Various other tools: not eligible.
        for other_tool in [Tool::RectMarquee, Tool::Lasso, Tool::Crop, Tool::Brush, Tool::Eraser, Tool::Hand] {
            app.ui.tool = other_tool;
            assert!(!eligible(&app), "must not show for {other_tool:?}");
        }

        // 4. Type tool with active text-edit session: eligible.
        app.ui.tool = Tool::Type;
        assert!(eligible(&app));

        // 5. Window -> Contextual Task Bar toggle off: not eligible.
        app.ui.panels.context_task_bar = false;
        assert!(!eligible(&app));
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

        app.ui.tool = Tool::Type;
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

    /// Warp Text command is enabled while editing a text layer.
    #[test]
    fn warp_text_is_enabled_for_text_layer() {
        let mut app = app_with_doc();
        let created = app
            .run(
                "type.create",
                json!({
                    "text": "Warp Me",
                    "size": 32,
                    "x": 50,
                    "y": 50,
                }),
            )
            .unwrap();
        let layer = created["layer"].as_u64().unwrap();
        app.sync_views();
        app.ui.tool = Tool::Type;
        app.ui.text_edit = Some(crate::state::TextEdit {
            layer,
            caret: 0,
            anchor: 0,
            session: String::new(),
            created: false,
            mask: false,
            document: None,
        });
        assert!(crate::menus::is_enabled(&app, "type.warpText"));
    }

    /// `inspect` reports the Type task bar rectangle when eligible.
    #[test]
    fn inspect_reports_type_task_bar_when_eligible() {
        let mut app = app_with_doc();
        app.ui.tool = Tool::Type;
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
