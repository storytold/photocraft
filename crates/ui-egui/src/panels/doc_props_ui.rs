//! Properties panel › Document: what Photoshop shows when nothing (or the Background layer) is
//! selected. Canvas size and orientation, resolution, colour mode and bit depth, and the
//! Rulers & Grids toggles. Every edit dispatches an engine or view command, so the panel stays thin.

use egui::{Align2, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_doc::{ColorMode, Document, Layer, LayerId, SampleType};
use serde_json::{Value, json};

use crate::props_layout::{control_group, stacked_label};
use crate::theme::Tokens;
use crate::{PhotocraftApp, icons, widgets};

/// Photoshop's Background layer: the locked, opaque raster layer at the bottom of the stack,
/// whatever its (localized) name ([`Document::background`]).
pub fn is_background(doc: &Document, l: &Layer) -> bool {
    doc.is_background(l.id)
}

/// Does the Properties panel show the Document view? (No layer selected, or the Background layer.)
pub fn shows_document(doc: &Document, active: Option<LayerId>) -> bool {
    match active.and_then(|id| doc.layer(id)) {
        None => true,
        Some(l) => is_background(doc, l),
    }
}

/// Image › Mode command id and label per colour mode, in Photoshop's dropdown order.
pub const MODES: &[(ColorMode, &str, &str)] = &[
    (ColorMode::Bitmap, "Bitmap", "image.mode.bitmap"),
    (ColorMode::Grayscale, "Grayscale", "image.mode.grayscale"),
    (ColorMode::Duotone, "Duotone", "image.mode.duotone"),
    (ColorMode::Indexed, "Indexed Color", "image.mode.indexedColor"),
    (ColorMode::Rgb, "RGB Color", "image.mode.rgb"),
    (ColorMode::Cmyk, "CMYK Color", "image.mode.cmyk"),
    (ColorMode::Lab, "Lab Color", "image.mode.lab"),
    (ColorMode::Multichannel, "Multichannel", "image.mode.multichannel"),
];

pub const DEPTHS: &[(SampleType, &str, &str)] = &[
    (SampleType::U8, "8 Bits/Channel", "image.mode.bits8"),
    (SampleType::U16, "16 Bits/Channel", "image.mode.bits16"),
    (SampleType::F32, "32 Bits/Channel", "image.mode.bits32"),
];

/// Ruler units offered in the Rulers & Grids dropdown (`unitsAndRulers.rulers` values).
pub const UNITS: &[(&str, &str)] = &[
    ("pixels", "Pixels"),
    ("inches", "Inches"),
    ("cm", "Centimeters"),
    ("mm", "Millimeters"),
    ("points", "Points"),
    ("picas", "Picas"),
    ("percent", "Percent"),
];

/// `image.canvasSize` params for a new width/height, keeping the aspect ratio when linked.
pub fn canvas_resize_params(old: (u32, u32), new_w: Option<f32>, new_h: Option<f32>, linked: bool) -> Option<Value> {
    let (ow, oh) = (old.0 as f32, old.1 as f32);
    let (mut w, mut h) = (new_w.unwrap_or(ow).round().max(1.0), new_h.unwrap_or(oh).round().max(1.0));
    if linked {
        if new_w.is_some() {
            h = (w * oh / ow).round().max(1.0);
        } else if new_h.is_some() {
            w = (h * ow / oh).round().max(1.0);
        }
    }
    ((w, h) != (ow, oh)).then(|| json!({"width": w, "height": h, "anchor": "center"}))
}

/// Collapsible section header with a chevron, Photoshop Properties style. Returns whether it is open.
fn section(ui: &mut egui::Ui, id: &str, title: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let key = egui::Id::unique(("doc-props-section", id));
    let mut open = ui.data(|d| d.get_temp::<bool>(key)).unwrap_or(true);
    ui.add_space(8.0);
    widgets::hairline(ui);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    icons::paint(
        ui,
        Rect::from_center_size(pos2(r.left() + 7.0, r.center().y), vec2(12.0, 12.0)),
        if open { "chevron-down" } else { "chevron-right" },
        11.0,
        t.text_dim,
    );
    ui.painter().text(pos2(r.left() + 18.0, r.center().y), Align2::LEFT_CENTER, title, crate::theme::semibold(12.0), t.text);
    if resp.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(key, open));
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::DisclosureTriangle, true, open, title));
    if open {
        ui.add_space(2.0);
    }
    open
}

/// Left-aligned label in a column measured from the translated field names.
fn field_label(ui: &mut egui::Ui, s: &str, w: f32) {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
    ui.painter().text(r.left_center(), Align2::LEFT_CENTER, s, egui::FontId::proportional(12.0), t.text_dim);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, s));
}

/// A number field that only reports a committed value (drag released, Enter, focus lost).
fn committed_field(ui: &mut egui::Ui, id: &str, name: &str, current: f32, u: crate::layer_props_ui::LenUnit, width: f32) -> Option<f32> {
    let key = egui::Id::unique(("doc-props-field", id));
    // Shown in the ruler unit (cm when the rulers are in centimeters); 100% is the current size.
    let extent = f64::from(current);
    let mut v = ui.data(|d| d.get_temp::<f32>(key)).unwrap_or_else(|| u.shown(extent, extent));
    let r = widgets::value_field(ui, &mut v, 0.0..=300_000.0, u.unit.suffix(), width).accessible_name(tl!(name));
    if r.dragged() || r.has_focus() {
        ui.data_mut(|d| d.insert_temp(key, v));
        return None;
    }
    ui.data_mut(|d| d.remove::<f32>(key));
    if !(r.drag_stopped() || r.lost_focus() || r.changed()) {
        return None;
    }
    let px = (u.px(v, extent) as f32).clamp(1.0, 300_000.0);
    ((px - current).abs() >= 0.5).then_some(px)
}

/// A disabled-looking field (Photoshop greys Canvas X/Y for documents without artboards).
fn dim_field(ui: &mut egui::Ui, name: &str, width: f32) {
    ui.add_enabled_ui(false, |ui| {
        widgets::value_field(ui, &mut 0.0, 0.0..=300_000.0, "px", width).accessible_name(name);
    });
}

/// Document properties use consistent spacing without changing the surrounding dock's style.
pub fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    crate::props_layout::panel(ui, |ui| properties_body(app, ui));
}

/// Responsive canvas fields and view controls; edits retain their existing command routes.
pub(crate) fn properties_body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else { return };
    let (w, h, mode, depth, dpi) = (st.doc.size.width, st.doc.size.height, st.doc.mode, st.doc.depth, st.doc.resolution_dpi);
    let has_bg = st.doc.layers.first().is_some_and(|l| is_background(&st.doc, l));
    let lu = crate::layer_props_ui::LenUnit::of(app);
    let mut run: Vec<(String, Value)> = Vec::new();
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(22.0, 26.0), Sense::hover());
        icons::paint(ui, r, "file", 16.0, t.icon);
        ui.label(RichText::new(tl!("Document")).font(crate::theme::semibold(12.0)).color(t.text));
    });
    let label_w = [tl!("Orientation"), tl!("Resolution"), tl!("Mode"), tl!("Depth"), tl!("Fill")]
        .iter()
        .map(|s| ui.painter().layout_no_wrap((*s).to_owned(), egui::FontId::proportional(12.0), t.text_dim).size().x)
        .fold(0.0, f32::max)
        .ceil()
        + 6.0;
    let narrow = ui.available_width() < 340.0;
    if section(ui, "canvas", tl!("Canvas")) {
        control_group(ui, |ui| {
            let link_key = egui::Id::unique("doc-props-link");
            let linked = ui.data(|d| d.get_temp::<bool>(link_key)).unwrap_or(false);
            let field_w = ((ui.available_width() - 28.0 - 6.0 - 2.0 * 18.0 - 3.0 * 6.0) / 2.0).max(36.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.add_space(14.0);
                    if icons::button(ui, if linked { "link" } else { "unlink" }, 28.0, linked, tl!("Link width and height")).clicked() {
                        ui.data_mut(|d| d.insert_temp(link_key, !linked));
                    }
                });
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        stacked_label(ui, "W", &["W", "H"], 18.0, 4.0);
                        if let Some(v) = committed_field(ui, "w", "Width", w as f32, lu, field_w) {
                            run.extend(canvas_resize_params((w, h), Some(v), None, linked).map(|p| ("image.canvasSize".to_string(), p)));
                        }
                        stacked_label(ui, "X", &["X", "Y"], 18.0, 4.0);
                        dim_field(ui, "X", field_w);
                    });
                    ui.horizontal(|ui| {
                        stacked_label(ui, "H", &["W", "H"], 18.0, 4.0);
                        if let Some(v) = committed_field(ui, "h", "Height", h as f32, lu, field_w) {
                            run.extend(canvas_resize_params((w, h), None, Some(v), linked).map(|p| ("image.canvasSize".to_string(), p)));
                        }
                        stacked_label(ui, "Y", &["X", "Y"], 18.0, 4.0);
                        dim_field(ui, "Y", field_w);
                    });
                });
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                field_label(ui, tl!("Orientation"), label_w);
                let portrait = h > w;
                if icons::button(ui, "rectangle-vertical", 26.0, portrait, tl!("Portrait")).clicked() && !portrait && w != h {
                    run.push(("image.canvasSize".into(), json!({"width": h, "height": w, "anchor": "center"})));
                }
                if icons::button(ui, "rectangle-horizontal", 26.0, !portrait, tl!("Landscape")).clicked() && portrait {
                    run.push(("image.canvasSize".into(), json!({"width": h, "height": w, "anchor": "center"})));
                }
            });
            ui.horizontal(|ui| {
                field_label(ui, tl!("Resolution"), label_w);
                ui.label(RichText::new(format!("{} {}", widgets::fmt_num(dpi as f64), tl!("Pixels/Inch"))).color(t.text));
            });
            ui.horizontal(|ui| {
                field_label(ui, tl!("Mode"), label_w);
                let mut m = mode;
                let opts: Vec<(ColorMode, &str)> = MODES.iter().map(|(m, l, _)| (*m, *l)).collect();
                if widgets::dropdown(ui, "doc-props-mode", tl!("Color Mode"), &mut m, &opts, ui.available_width())
                    && let Some((_, _, id)) = MODES.iter().find(|(x, _, _)| *x == m)
                {
                    run.push(((*id).to_string(), Value::Null));
                }
            });
            ui.horizontal(|ui| {
                field_label(ui, tl!("Depth"), label_w);
                let mut d = depth;
                let opts: Vec<(SampleType, &str, &str)> =
                    DEPTHS.iter().map(|(d, l, _)| (*d, *l, if *d == SampleType::F32 { "Floating point" } else { "Integer" })).collect();
                if widgets::dropdown_with_tooltips(ui, "doc-props-depth", tl!("Depth"), &mut d, &opts, ui.available_width())
                    && let Some((_, _, id)) = DEPTHS.iter().find(|(x, _, _)| *x == d)
                {
                    run.push(((*id).to_string(), Value::Null));
                }
            });
            if narrow {
                field_label(ui, tl!("Fill"), label_w);
            }
            ui.horizontal(|ui| {
                if !narrow {
                    field_label(ui, tl!("Fill"), label_w);
                }
                let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                ui.painter().rect_stroke(r, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
                // The canvas fill is only editable for documents without a Background layer.
                ui.add_enabled_ui(!has_bg, |ui| {
                    let mut f = 0u8;
                    widgets::dropdown(ui, "doc-props-fill", tl!("Background"), &mut f, &[(0u8, tl!("Background Color"))], ui.available_width());
                });
            });
        });
    }
    if section(ui, "rulers", tl!("Rulers & Grids")) {
        control_group(ui, |ui| {
            ui.horizontal(|ui| {
                let pixel_grid = crate::view_cmds::checked(app, "view.show.pixelGrid").unwrap_or(false);
                for (icon, tip, id, on) in [
                    ("ruler", crate::shortcuts::tip_label(app, "Rulers", "view.rulers"), "view.rulers", app.ui.extras.rulers),
                    ("grid-3x3", crate::shortcuts::tip_label(app, "Grid", "view.show.grid"), "view.show.grid", app.ui.extras.grid),
                    ("grid-2x2", crate::shortcuts::tip_label(app, "Pixel Grid", "view.show.pixelGrid"), "view.show.pixelGrid", pixel_grid),
                ] {
                    if icons::button(ui, icon, 26.0, on, &tip).clicked() {
                        run.push((id.to_string(), Value::Null));
                    }
                }
                ui.add_space(4.0);
                let mut unit = app.session.prefs().units_and_rulers.rulers.name().to_string();
                let opts: Vec<(String, &str)> = UNITS.iter().map(|(k, l)| (k.to_string(), *l)).collect();
                if widgets::dropdown(ui, "doc-props-units", tl!("Units"), &mut unit, &opts, ui.available_width()) {
                    run.push(("prefs.set".into(), json!({"path": "unitsAndRulers.rulers", "value": unit})));
                }
            });
        });
    }
    if section(ui, "guides", tl!("Guides")) {
        control_group(ui, |ui| {
            let action_width = widgets::button_width(ui, &[tl!("New Guide…")]) + widgets::button_width(ui, &[tl!("Clear")]) + 6.0;
            let wrap_actions = narrow || ui.available_width() < 64.0 + action_width;
            ui.horizontal(|ui| {
                for (icon, tip, id) in [
                    ("eye", crate::shortcuts::tip_label(app, "Show Guides", "view.show.guides"), "view.show.guides"),
                    ("lock", crate::shortcuts::tip_label(app, "Lock Guides", "view.lockGuides"), "view.lockGuides"),
                ] {
                    let on = if id == "view.lockGuides" { app.ui.extras.lock_guides } else { app.ui.extras.guides };
                    if icons::button(ui, icon, 26.0, on, &tip).clicked() {
                        run.push((id.to_string(), Value::Null));
                    }
                }
                if !wrap_actions {
                    guide_actions(app, ui, &mut run, false);
                }
            });
            if wrap_actions {
                ui.horizontal_wrapped(|ui| guide_actions(app, ui, &mut run, true));
            }
        });
    }
    let ctx = ui.ctx().clone();
    for (id, params) in run {
        let params = if params.is_null() { json!({}) } else { params };
        if let Err(e) = crate::menus::invoke(app, &ctx, &id, params) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// Keep guide actions visible, disabling unavailable commands and filling narrow rows evenly.
fn guide_actions(app: &PhotocraftApp, ui: &mut egui::Ui, run: &mut Vec<(String, Value)>, stretch: bool) {
    let clear_width = widgets::button_width(ui, &[tl!("Clear")]);
    let new_width = if stretch { (ui.available_width() - clear_width - 6.0).max(0.0) } else { 0.0 };
    for (label, id, width) in [(tl!("New Guide…"), "view.newGuide", new_width), (tl!("Clear"), "view.clearGuides", clear_width)] {
        ui.add_enabled_ui(crate::menus::is_enabled(app, id), |ui| {
            if (if stretch { widgets::fitted_secondary_button(ui, label, width) } else { widgets::secondary_button(ui, label, width) }).clicked() {
                run.push((id.to_string(), Value::Null));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::Document;

    /// Render the actual panel with a translated, populated document at the requested width.
    fn panel_harness(width: f32, language: &str) -> egui_kittest::Harness<'static, PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":40,"height":60,"background":"white"})).unwrap();
        let lang = crate::i18n::Lang::from_pref(language);
        let mut h = egui_kittest::Harness::builder().with_size(vec2(width, 760.0)).build_ui_state(
            move |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    crate::i18n::with_language(lang, || properties(app, ui));
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(4);
        h
    }

    /// Long translated labels and every control remain inside both narrow and wide panels.
    #[test]
    fn translated_document_controls_fit_the_panel_without_clipped_labels() {
        use egui_kittest::kittest::Queryable;
        for language in ["en", "fr", "de"] {
            let lang = crate::i18n::Lang::from_pref(language);
            let text = |s| crate::i18n::with_language(lang, || crate::i18n::t(s).to_owned());
            for width in [250.0, 290.0, 560.0] {
                let h = panel_harness(width, language);
                let bounds = Rect::from_min_max(pos2(8.0, 0.0), pos2(width - 8.0, 760.0)).expand(1.0);
                for label in ["Orientation", "Resolution", "Mode", "Depth", "Fill"] {
                    let translated = text(label);
                    let r = h.get_by_role_and_label(egui::Role::Label, &translated).rect();
                    assert!(bounds.contains_rect(r), "{language}/{width}: {label} {r:?}");
                    let painted = h
                        .output()
                        .shapes
                        .iter()
                        .find_map(|s| match &s.shape {
                            egui::Shape::Text(t) if t.galley.text() == translated => Some((Rect::from_min_size(t.pos, t.galley.size()), s.clip_rect)),
                            _ => None,
                        })
                        .unwrap();
                    assert!(r.expand(1.0).contains_rect(painted.0) && painted.1.contains_rect(painted.0), "{language}/{width}: {label} is clipped");
                }
                for (role, labels) in
                    [(egui::Role::ComboBox, &["Color Mode", "Depth", "Background", "Units"][..]), (egui::Role::Button, &["New Guide…", "Clear"][..])]
                {
                    for label in labels {
                        let r = h.get_by_role_and_label(role, &text(label)).rect();
                        assert!(bounds.contains_rect(r), "{language}/{width}: {label} overflows: {r:?}");
                    }
                }
                let mode = h.get_by_role_and_label(egui::Role::ComboBox, &text("Color Mode")).rect();
                let depth = h.get_by_role_and_label(egui::Role::ComboBox, &text("Depth")).rect();
                assert!((mode.left() - depth.left()).abs() < 0.5 && (mode.right() - depth.right()).abs() < 0.5);
                assert_eq!(h.state().session.active().unwrap().doc.size, photocraft_doc::Size::new(40, 60));
            }
        }
    }

    /// Widening a dock only stretches the headers, not the form controls.
    #[test]
    fn wide_document_controls_stop_growing() {
        use egui_kittest::kittest::Queryable;
        let normal = panel_harness(560.0, "en");
        let wide = panel_harness(1000.0, "en");
        for (role, labels) in [
            (egui::Role::SpinButton, &["Width", "Height", "X", "Y"][..]),
            (egui::Role::ComboBox, &["Color Mode", "Depth", "Background", "Units"][..]),
            (egui::Role::Button, &["New Guide…", "Clear"][..]),
        ] {
            for label in labels {
                let a = normal.get_by_role_and_label(role, label).rect();
                let b = wide.get_by_role_and_label(role, label).rect();
                assert!((a.width() - b.width()).abs() < 0.5, "{label} grows with the dock");
                assert!(b.right() <= 373.0, "{label} exceeds the compact control group: {b:?}");
            }
        }
        let a = normal.get_by_role_and_label(egui::Role::DisclosureTriangle, "Canvas").rect();
        let b = wide.get_by_role_and_label(egui::Role::DisclosureTriangle, "Canvas").rect();
        assert!((b.width() - a.width() - 440.0).abs() < 0.5, "section header must span the dock");
    }

    /// Reflowed controls still dispatch orientation, view and guide commands.
    #[test]
    fn narrow_document_controls_keep_their_commands() {
        use egui_kittest::kittest::Queryable;
        let mut h = panel_harness(250.0, "en");
        h.get_by_label("Landscape").click();
        h.run_steps(3);
        assert_eq!(h.state().session.active().unwrap().doc.size, photocraft_doc::Size::new(60, 40));
        let before = h.state().ui.extras.rulers;
        let tip = crate::shortcuts::tip_label(h.state(), "Rulers", "view.rulers");
        h.get_by_role_and_label(egui::Role::Button, &tip).click();
        h.run_steps(3);
        assert_eq!(h.state().ui.extras.rulers, !before);
        h.get_by_label("New Guide…").click();
        h.run_steps(3);
        assert_eq!(h.state().ui.dialogs.len(), 1);
    }

    fn doc_with_background() -> photocraft_engine::Session {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 40, "height": 20, "background": "white"})).unwrap();
        s
    }

    #[test]
    fn background_selection_shows_document() {
        let mut s = doc_with_background();
        let st = s.active().unwrap();
        let bg = st.doc.layers[0].id;
        assert!(is_background(&st.doc, &st.doc.layers[0]));
        assert!(shows_document(&st.doc, Some(bg)));
        assert!(shows_document(&st.doc, None));
        s.execute("layer.new.layer", json!({"name": "Ink"})).unwrap();
        let st = s.active().unwrap();
        let ink = st.active_layer.unwrap();
        assert!(!shows_document(&st.doc, Some(ink)));
        assert!(!is_background(&st.doc, st.doc.layer(ink).unwrap()));
    }

    #[test]
    fn transparent_documents_have_no_background() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 40, "height": 20, "background": "transparent"})).unwrap();
        let st = s.active().unwrap();
        let d: &Document = &st.doc;
        assert!(d.layers.iter().all(|l| !is_background(d, l)));
    }

    #[test]
    fn canvas_resize_keeps_ratio_when_linked() {
        assert_eq!(canvas_resize_params((2400, 1500), Some(1200.0), None, true), Some(json!({"width": 1200.0, "height": 750.0, "anchor": "center"})));
        assert_eq!(canvas_resize_params((2400, 1500), None, Some(3000.0), true), Some(json!({"width": 4800.0, "height": 3000.0, "anchor": "center"})));
        assert_eq!(canvas_resize_params((2400, 1500), Some(1000.0), None, false), Some(json!({"width": 1000.0, "height": 1500.0, "anchor": "center"})));
        assert_eq!(canvas_resize_params((2400, 1500), Some(2400.2), None, false), None);
    }

    /// #5661: a 1 x 1 px document at 300 ppi is 0.0033 in wide. With the rulers in inches the
    /// Width/Height fields clamped that to 0.01 in (3 px) and resized the document unasked.
    #[test]
    fn tiny_document_in_inches_is_not_resized_by_showing_it() {
        use egui_kittest::Harness;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 1, "height": 1, "resolution": 300, "background": "white"})).unwrap();
        app.session.prefs.edit(|p| p.units_and_rulers.rulers = photocraft_engine::prefs::Unit::Inches);
        let undo = app.session.active().unwrap().history.entries().len();
        let mut h = Harness::builder().with_size(vec2(320.0, 600.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    properties(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(3);
        let st = h.state().session.active().unwrap();
        assert_eq!((st.doc.size.width, st.doc.size.height), (1, 1));
        assert_eq!(st.history.entries().len(), undo, "no history step");
    }

    /// Horizontal centre of every single-letter label the panel drew, by letter.
    fn letter_centres(h: &egui_kittest::Harness<'_, PhotocraftApp>) -> Vec<(String, f32)> {
        h.output()
            .shapes
            .iter()
            .filter_map(|s| if let egui::Shape::Text(t) = &s.shape { Some(t) } else { None })
            .filter(|t| t.galley.text().chars().count() == 1)
            .map(|t| (t.galley.text().to_string(), t.pos.x + t.galley.size().x * 0.5))
            .collect()
    }

    /// #7314: the Canvas section's W over H (and X over Y) share a centre; a narrow "H" used to sit
    /// flush right under the wider "W".
    #[test]
    fn canvas_size_labels_are_centred_on_each_other() {
        use egui_kittest::Harness;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 40, "height": 20, "background": "white"})).unwrap();
        let mut h = Harness::builder().with_size(vec2(320.0, 600.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    properties(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(3);
        let centres = letter_centres(&h);
        let at = |l: &str| centres.iter().find(|(s, _)| s == l).map(|(_, x)| *x).unwrap_or_else(|| panic!("no {l} label in {centres:?}"));
        assert!((at("W") - at("H")).abs() < 0.01, "W and H: {centres:?}");
        assert!((at("X") - at("Y")).abs() < 0.01, "X and Y: {centres:?}");
    }

    #[test]
    fn document_dropdowns_map_to_live_mode_commands() {
        for id in MODES.iter().map(|m| m.2).chain(DEPTHS.iter().map(|d| d.2)) {
            assert!(photocraft_engine::commands::find(id).is_some(), "{id} is not an engine command");
        }
    }

    #[test]
    fn image_mode_depth_labels_match_sample_types() {
        assert_eq!(
            DEPTHS.iter().map(|(sample, label, _)| (*sample, *label)).collect::<Vec<_>>(),
            [(SampleType::U8, "8 Bits/Channel"), (SampleType::U16, "16 Bits/Channel"), (SampleType::F32, "32 Bits/Channel"),]
        );
    }
}
