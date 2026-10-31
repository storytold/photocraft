//! Create Rectangle / Ellipse / Triangle / Polygon: a single click (no drag) with a Shape tool
//! opens Photoshop's creation dialog instead of drawing. OK runs the same `shape.create` command as
//! a drag, with the options bar's fill and stroke, at the clicked point (or centred on it).
//! Width and height are stored in document pixels (so automation can set them with
//! `ui.dialog.set`) and shown in the ruler unit, or each in one picked from its right-click menu
//! (`widthUnit`, `heightUnit`; not remembered, as in Photoshop).

use egui::{Align2, Sense, pos2, vec2};
use serde_json::{Map, Value, json};

use photocraft_engine::prefs::Unit;

use crate::PhotocraftApp;
use crate::state::{DialogKind, Tool};
use crate::theme::Tokens;

const MARK: &str = "__createShape";
/// Largest side the dialog accepts, in pixels (Photoshop's document limit).
const MAX_PX: f64 = 300_000.0;
/// Remembered values (document pixels; sides and radii come from the options bar).
const KEYS: [&str; 4] = ["width", "height", "fromCenter", "starRatio"];

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// The tool's name in the dialog fields and the title, for the tools that have a dialog.
fn kind_of(tool: Tool) -> Option<(&'static str, &'static str)> {
    Some(match tool {
        Tool::Rectangle => ("rectangle", "Create Rectangle"),
        Tool::EllipseShape => ("ellipse", "Create Ellipse"),
        Tool::Triangle => ("triangle", "Create Triangle"),
        Tool::Polygon => ("polygon", "Create Polygon"),
        _ => return None,
    })
}

fn pref_key(kind: &str) -> String {
    format!("shape.create.{kind}")
}

fn valid(key: &str, v: &Value) -> bool {
    match key {
        "width" | "height" => v.as_f64().is_some_and(|x| x.is_finite() && (1.0..=MAX_PX).contains(&x)),
        "starRatio" => v.as_f64().is_some_and(|x| x.is_finite() && (1.0..=100.0).contains(&x)),
        _ => v.is_boolean(),
    }
}

/// True for a press and release that never left `start` by more than a few screen pixels.
pub fn is_click(start: [f64; 2], points: &[[f64; 3]], zoom: f32) -> bool {
    let slop = 3.0 / f64::from(zoom.max(0.01));
    points.iter().all(|p| (p[0] - start[0]).hypot(p[1] - start[1]) <= slop)
}

/// Open the creation dialog of `tool` for a click at `at` (document px). `None` for a tool without
/// one (Line, Custom Shape).
pub fn open(app: &mut PhotocraftApp, tool: Tool, at: [f64; 2]) -> Option<u64> {
    let (kind, label) = kind_of(tool)?;
    if !(at[0].is_finite() && at[1].is_finite()) {
        return None;
    }
    let o = &app.ui.tool_options;
    let r = f64::from(o.corner_radius.max(0.0));
    let sides = o.polygon_sides.clamp(3, 100);
    let mut f = Map::new();
    f.insert(MARK.into(), json!(kind));
    f.insert("__command".into(), json!("shape.create"));
    f.insert("__label".into(), json!(label));
    f.insert("__at".into(), json!([at[0].round(), at[1].round()]));
    f.insert("width".into(), json!(100.0));
    f.insert("height".into(), json!(100.0));
    f.insert("fromCenter".into(), json!(false));
    match kind {
        "rectangle" => {
            f.insert("radii".into(), json!([r, r, r, r]));
            f.insert("linkRadii".into(), json!(true));
        }
        "polygon" => {
            f.insert("sides".into(), json!(sides));
            f.insert("starRatio".into(), json!(100.0));
        }
        _ => {}
    }
    if let Some(Value::Object(saved)) = app.session.prefs().dialogs.get(&pref_key(kind)) {
        for key in KEYS {
            if f.contains_key(key)
                && let Some(v) = saved.get(key).filter(|v| valid(key, v))
            {
                f.insert(key.into(), v.clone());
            }
        }
    }
    Some(app.ui.open_dialog(DialogKind::Command, f))
}

fn num(f: &Map<String, Value>, key: &str) -> Option<f64> {
    f.get(key).and_then(Value::as_f64).filter(|v| v.is_finite())
}

/// `shape.create` geometry for the dialog's values (no fill or stroke).
pub fn geometry(f: &Map<String, Value>) -> Result<Value, String> {
    let kind = f.get(MARK).and_then(Value::as_str).ok_or("not a Create Shape dialog")?;
    let at = f.get("__at").and_then(Value::as_array).ok_or("the dialog has no position")?;
    let (Some(x), Some(y)) = (at.first().and_then(Value::as_f64), at.get(1).and_then(Value::as_f64)) else {
        return Err("the dialog has no position".into());
    };
    let side = |key: &str, label: &str| -> Result<f64, String> {
        num(f, key).filter(|v| (1.0..=MAX_PX).contains(v)).ok_or_else(|| format!("{label} must be between 1 and {MAX_PX} pixels"))
    };
    let (w, h) = (side("width", "Width")?, side("height", "Height")?);
    let center = f.get("fromCenter").and_then(Value::as_bool).unwrap_or(false);
    if !(x.abs() <= MAX_PX && y.abs() <= MAX_PX) {
        return Err("the shape's position is out of range".into());
    }
    let (x0, y0) = if center { (x - w / 2.0, y - h / 2.0) } else { (x, y) };
    let rect = json!([x0, y0, w, h]);
    Ok(match kind {
        "rectangle" => {
            let mut radii = [0.0; 4];
            if let Some(a) = f.get("radii").and_then(Value::as_array) {
                for (r, v) in radii.iter_mut().zip(a) {
                    *r = v.as_f64().filter(|v| v.is_finite()).unwrap_or(0.0).clamp(0.0, w.min(h) / 2.0);
                }
            }
            if radii.iter().any(|r| *r > 0.0) { json!({"kind": "roundedRect", "rect": rect, "radii": radii}) } else { json!({"kind": "rect", "rect": rect}) }
        }
        "ellipse" => json!({"kind": "ellipse", "rect": rect}),
        "triangle" => json!({"kind": "polygon", "rect": rect, "sides": 3}),
        "polygon" => {
            let sides = num(f, "sides").map_or(5.0, |s| s.round().clamp(3.0, 100.0));
            let ratio = num(f, "starRatio").map_or(100.0, |r| r.clamp(1.0, 100.0));
            json!({"kind": "polygon", "rect": rect, "sides": sides, "starRatio": ratio / 100.0})
        }
        other => return Err(format!("unknown shape `{other}`")),
    })
}

/// OK: create the shape and remember the values for the next click.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let result = geometry(f).and_then(|g| crate::vector_ui::create_shape(app, g));
    match &result {
        Ok(_) => {
            if let Some(kind) = f.get(MARK).and_then(Value::as_str) {
                let saved: Map<String, Value> = KEYS.iter().filter_map(|k| f.get(*k).map(|v| (k.to_string(), v.clone()))).collect();
                app.session.prefs.edit(|p| p.dialogs.insert(pref_key(kind), Value::Object(saved)));
            }
        }
        Err(e) => {
            app.ui.status = e.clone();
            app.ui.status_error = true;
        }
    }
    result
}

/// A row in reading order: `ui.horizontal` in a left-to-right dialog, its mirror in a
/// right-to-left one. The row keeps `ui.horizontal`'s height and alignment.
///
/// Not `direction::dialog_row()`: that helper's `Align::Min` cross-align and full-height
/// allocation change the LTR row metrics.
fn row<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    let size = vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, crate::direction::row(), add)
}

fn label(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(96.0, 22.0), Sense::hover());
    // The caption hugs its control: right of it in a left-to-right row, left of it in a
    // right-to-left one.
    let (pos, anchor) = if crate::direction::is_rtl() {
        (pos2(r.left() + 6.0, r.center().y), Align2::LEFT_CENTER)
    } else {
        (pos2(r.right() - 6.0, r.center().y), Align2::RIGHT_CENTER)
    };
    ui.painter().text(pos, anchor, text, egui::FontId::proportional(12.0), t.text_dim);
}

pub fn body(app: &PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.spacing_mut().item_spacing.y = 6.0;
    let kind = f.get(MARK).and_then(Value::as_str).unwrap_or("").to_string();
    // Width and height each in its own unit (percent of the document's side): the ruler unit, or
    // the one picked from that field's right-click menu, as in Photoshop (#7786).
    let ur = &app.session.prefs().units_and_rulers;
    let (rulers, ppi) = (ur.rulers, ur.point_size.per_inch());
    let (dpi, size) = app
        .session
        .active()
        .map_or((72.0, [1.0, 1.0]), |d| (f64::from(d.doc.resolution_dpi.max(1.0)), [f64::from(d.doc.size.width), f64::from(d.doc.size.height)]));
    for (key, unit_key, text, extent) in [("width", "widthUnit", tl!("Width:"), size[0]), ("height", "heightUnit", tl!("Height:"), size[1])] {
        let unit = f.get(unit_key).and_then(Value::as_str).and_then(Unit::parse).unwrap_or(rulers);
        row(ui, |ui| {
            label(ui, text);
            let px = num(f, key).unwrap_or(100.0);
            let to_unit = |px: f64| unit.from_px(px, dpi, extent, ppi) as f32;
            let mut v = to_unit(px);
            let places = unit.decimals() as u32;
            let resp = crate::widgets::value_field_prec(ui, &mut v, to_unit(1.0)..=to_unit(MAX_PX), unit.suffix(), 110.0, places).accessible_name(text);
            if resp.changed() {
                let px = unit.to_px(f64::from(v), dpi, extent, ppi).clamp(1.0, MAX_PX);
                f.insert(key.into(), json!((px * 1000.0).round() / 1000.0));
            }
            // Right-click: the length units for this field (the sizes stay in pixels).
            resp.context_menu(|ui| {
                crate::menus::popup_body(ui, |ui| {
                    if let Some(name) = crate::rulers::unit_menu_of(ui, unit, &crate::rulers::LENGTH_UNITS) {
                        f.insert(unit_key.into(), json!(name));
                    }
                });
            });
        });
    }
    match kind.as_str() {
        "rectangle" => {
            let mut radii = [0.0f32; 4];
            if let Some(a) = f.get("radii").and_then(Value::as_array) {
                for (r, v) in radii.iter_mut().zip(a) {
                    *r = v.as_f64().unwrap_or(0.0) as f32;
                }
            }
            let mut linked = f.get("linkRadii").and_then(Value::as_bool).unwrap_or(true);
            let mut changed = None;
            // `radii` is [top-left, top-right, bottom-right, bottom-left].
            row(ui, |ui| {
                label(ui, tl!("Radius:"));
                ui.vertical(|ui| {
                    // All four fields show "Radius:"; the accessible name says which corner
                    // (radii are top-left, top-right, bottom-right, bottom-left).
                    let corners = ["Top Left", "Top Right", "Bottom Right", "Bottom Left"];
                    for pair in [[0usize, 1], [3, 2]] {
                        row(ui, |ui| {
                            for i in pair {
                                let name = format!("{} {}", tl!(corners[i]), tl!("Radius:"));
                                if crate::widgets::value_field(ui, &mut radii[i], 0.0..=10000.0, "px", 72.0).accessible_name(name).changed() {
                                    changed = Some(i);
                                }
                            }
                        });
                    }
                });
            });
            row(ui, |ui| {
                label(ui, "");
                if crate::widgets::checkbox(ui, &mut linked, tl!("Link radii")).changed() {
                    f.insert("linkRadii".into(), json!(linked));
                }
            });
            if let Some(i) = changed {
                if linked {
                    radii = [radii[i]; 4];
                }
                f.insert("radii".into(), json!(radii.map(|r| f64::from(r.max(0.0)))));
            }
        }
        "polygon" => {
            row(ui, |ui| {
                label(ui, tl!("Sides:"));
                let mut s = num(f, "sides").unwrap_or(5.0) as f32;
                if crate::widgets::value_field(ui, &mut s, 3.0..=100.0, "", 72.0).accessible_name(tl!("Sides:")).changed() {
                    f.insert("sides".into(), json!(s.round().clamp(3.0, 100.0) as u32));
                }
            });
            row(ui, |ui| {
                label(ui, tl!("Star Ratio:"));
                let mut r = num(f, "starRatio").unwrap_or(100.0) as f32;
                if crate::widgets::value_field(ui, &mut r, 1.0..=100.0, "%", 72.0).accessible_name(tl!("Star Ratio:")).changed() {
                    f.insert("starRatio".into(), json!(f64::from(r.clamp(1.0, 100.0))));
                }
            });
        }
        _ => {}
    }
    row(ui, |ui| {
        label(ui, "");
        let mut center = f.get("fromCenter").and_then(Value::as_bool).unwrap_or(false);
        if crate::widgets::checkbox(ui, &mut center, tl!("From Center")).changed() {
            f.insert("fromCenter".into(), json!(center));
        }
    });
}

#[cfg(test)]
mod tests;

/// The dialog reads from the right in a right-to-left UI: each caption sits at the reading start
/// (the right) with its field to the left, and the four corner radii mirror with it.
#[cfg(test)]
mod rtl_tests {
    use super::*;

    /// One frame of the dialog body: the texts it draws, each with the rect it was painted in.
    fn texts(app: &PhotocraftApp, fields: &mut Map<String, Value>) -> Vec<(String, egui::Rect)> {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| body(app, ui, fields));
        out.textures_delta.clear();
        out.shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(t) => Some((t.galley.text().to_string(), t.galley.rect.translate(t.pos.to_vec2()))),
                _ => None,
            })
            .collect()
    }

    /// A caption and the value drawn just after it.
    fn pair(texts: &[(String, egui::Rect)], caption: &str) -> (egui::Rect, egui::Rect) {
        let i = texts.iter().position(|(t, _)| t == caption).unwrap_or_else(|| panic!("{caption:?} is not drawn"));
        let value = texts.get(i + 1).map(|(_, r)| *r).unwrap_or_else(|| panic!("{caption:?} has no value"));
        (texts[i].1, value)
    }

    /// The four corner radii, in the order they were created: top-left, top-right, bottom-left,
    /// bottom-right. Each field paints its value and its unit separately, so the units are skipped.
    fn radii(texts: &[(String, egui::Rect)]) -> Vec<egui::Rect> {
        let i = texts.iter().position(|(t, _)| t == "Radius:").unwrap_or_else(|| panic!("no radius caption"));
        texts[i + 1..].iter().filter(|(t, _)| t != "px").take(4).map(|(_, r)| *r).collect()
    }

    #[test]
    fn the_body_reads_from_the_right_under_rtl() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let mut fields = Map::new();
        fields.insert(MARK.into(), json!("rectangle"));
        fields.insert("width".into(), json!(100.0));
        fields.insert("height".into(), json!(100.0));
        fields.insert("fromCenter".into(), json!(false));
        fields.insert("radii".into(), json!([0.0, 0.0, 0.0, 0.0]));
        fields.insert("linkRadii".into(), json!(true));

        let ltr = {
            let _ltr = crate::text::direction::direction_scope(crate::text::direction::Dir::Ltr);
            texts(&app, &mut fields.clone())
        };
        let rtl = {
            let _rtl = crate::text::direction::direction_scope(crate::text::direction::Dir::Rtl);
            texts(&app, &mut fields.clone())
        };

        let (ltr_caption, ltr_value) = pair(&ltr, "Width:");
        assert!(ltr_caption.right() < ltr_value.left(), "LTR: the caption is left of its field: {ltr_caption:?} {ltr_value:?}");
        let (rtl_caption, rtl_value) = pair(&rtl, "Width:");
        assert!(rtl_caption.left() > rtl_value.right(), "RTL: the caption is right of its field: {rtl_caption:?} {rtl_value:?}");

        let ltr_radii = radii(&ltr);
        let rtl_radii = radii(&rtl);
        assert_eq!((ltr_radii.len(), rtl_radii.len()), (4, 4), "the four corner radii are drawn: {ltr_radii:?}");
        assert!(ltr_radii[0].left() < ltr_radii[1].left(), "LTR: top-left is left of top-right: {ltr_radii:?}");
        assert!(rtl_radii[0].left() > rtl_radii[1].left(), "RTL: top-left is right of top-right: {rtl_radii:?}");
        assert!(ltr_radii[2].left() < ltr_radii[3].left(), "LTR: bottom-left is left of bottom-right: {ltr_radii:?}");
        assert!(rtl_radii[2].left() > rtl_radii[3].left(), "RTL: bottom-left is right of bottom-right: {rtl_radii:?}");
    }

    /// The polygon variant's own rows mirror too.
    #[test]
    fn the_polygon_body_reads_from_the_right_under_rtl() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let mut fields = Map::new();
        fields.insert(MARK.into(), json!("polygon"));
        fields.insert("sides".into(), json!(5.0));
        fields.insert("starRatio".into(), json!(100.0));

        let rtl = {
            let _rtl = crate::text::direction::direction_scope(crate::text::direction::Dir::Rtl);
            texts(&app, &mut fields)
        };
        let (caption, value) = pair(&rtl, "Sides:");
        assert!(caption.left() > value.right(), "RTL: the caption is right of its field: {caption:?} {value:?}");
    }

    /// The right-click units menu's rows: RTL puts the checked unit's label at the popup's right
    /// edge with its mark in the reading-start gutter; LTR is unchanged.
    #[test]
    fn the_units_context_menu_mirrors_only_under_rtl() {
        let menu_texts = |dir: crate::text::direction::Dir| {
            let _scope = crate::text::direction::direction_scope(dir);
            let ctx = egui::Context::default();
            ctx.all_styles_mut(|s| s.right_to_left = dir == crate::text::direction::Dir::Rtl);
            let mut popup = egui::Rect::NOTHING;
            let mut texts = Vec::new();
            for _ in 0..3 {
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                    let btn = ui.button("field");
                    let _ = egui::Popup::menu(&btn)
                        .open_memory(Some(egui::SetOpenCommand::Bool(true)))
                        .show(|ui| {
                            crate::menus::popup_body(ui, |ui| {
                                let _ = crate::rulers::unit_menu_of(ui, Unit::Pixels, &crate::rulers::LENGTH_UNITS);
                            });
                        })
                        .inspect(|r| popup = r.response.rect);
                });
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
        };
        let (rtl_popup, rtl) = menu_texts(crate::text::direction::Dir::Rtl);
        let (ltr_popup, ltr) = menu_texts(crate::text::direction::Dir::Ltr);
        assert!(rtl_popup.is_positive() && ltr_popup.is_positive(), "the popups opened");
        let at = |v: &[(String, egui::Rect)], s: &str| v.iter().find(|(t, _)| t.contains(s)).map(|(_, r)| *r).unwrap_or(egui::Rect::NOTHING);
        let px = at(&rtl, "Pixels");
        assert!(rtl_popup.right() - px.right() < 32.0, "rtl: the row label hugs the popup's right edge: {px:?} in {rtl_popup:?}");
        let px = at(&ltr, "Pixels");
        assert!(px.left() - ltr_popup.left() < 32.0, "ltr: the row label hugs the popup's left edge: {px:?} in {ltr_popup:?}");
    }
}
