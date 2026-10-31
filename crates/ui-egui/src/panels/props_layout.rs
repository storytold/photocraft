//! Shared Properties-panel layout (#155): the layer header (kind icon + layer name), the one
//! collapsible section header every section uses (separator, disclosure chevron, title), column
//! widths that make field rows fill the panel, and the Quick Actions each layer kind offers.

use egui::{Rect, Sense, pos2, vec2};
use photocraft_doc::{Layer, LayerContent};

use crate::theme::{self, Tokens};
use crate::{PhotocraftApp, widgets};

/// Height of a section header row.
pub const SECTION_H: f32 = 24.0;
/// Width of the short label (or glyph) column in front of a field: "W", "tT", "VA"…
pub const LABEL_W: f32 = 22.0;
/// Horizontal gap between a label and its field.
pub const LABEL_GAP: f32 = 4.0;
/// Horizontal gap between two label+field columns.
pub const COL_GAP: f32 = 8.0;

/// A one-letter field label ("W", "H", "X", "Y") in a `w`-wide column, ending `pad` before its field. The letters
/// stacked in one column share a centre (#7314): the widest of `column` stays flush right, and the narrower ones
/// are centred on it rather than pushed against its right edge.
pub fn stacked_label(ui: &mut egui::Ui, s: &str, column: &[&str], w: f32, pad: f32) {
    let t = Tokens::get(ui.ctx());
    let font = egui::FontId::proportional(12.0);
    let widest = column.iter().map(|c| ui.painter().layout_no_wrap((*c).to_owned(), font.clone(), t.text_dim).size().x).fold(0.0, f32::max);
    let (r, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
    ui.painter().text(pos2(r.right() - pad - widest / 2.0, r.center().y), egui::Align2::CENTER_CENTER, s, font, t.text_dim);
}

/// Icon for a layer kind (the same glyphs as the Layers panel's kind filter).
pub fn kind_icon(content: &LayerContent) -> &'static str {
    match content {
        LayerContent::Adjustment(_) => "sliders-horizontal",
        LayerContent::Group(_) => "folder",
        LayerContent::Fill(_) => "paint-bucket",
        LayerContent::Text(_) => "type",
        LayerContent::Shape(_) => "pentagon",
        LayerContent::Smart(_) => "app-window",
        LayerContent::Raster(_) => "image",
    }
}

/// "Type Layer", "Curves", "Artboard"…
pub fn kind_label(layer: &Layer) -> String {
    match &layer.content {
        LayerContent::Adjustment(a) => a.label().to_string(),
        LayerContent::Group(g) if g.artboard.is_some() => "Artboard".to_string(),
        other => format!("{} Layer", other.kind_name()),
    }
}

/// Header row: kind icon, the layer's name, and its kind right-aligned in the dim text colour.
pub fn header(ui: &mut egui::Ui, layer: &Layer) {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let icon_r = Rect::from_center_size(pos2(r.left() + 10.0, r.center().y), vec2(20.0, 20.0));
    crate::icons::paint(ui, icon_r, kind_icon(&layer.content), 15.0, t.icon);
    let kind = kind_label(layer);
    let kind_g = ui.painter().layout_no_wrap(tl!(&kind).to_owned(), egui::FontId::proportional(11.0), t.text_faint);
    let kind_w = kind_g.size().x;
    // The name takes what's left between the icon and the kind (elided when long). The header
    // stays physical: the icon and name keep the left, the kind the right, in both directions.
    let name_x = r.left() + 26.0;
    let name_w = (r.right() - kind_w - 8.0 - name_x).max(0.0);
    let mut job = egui::text::LayoutJob::simple_singleline(layer.name.clone(), theme::medium(12.5), t.text);
    job.wrap = egui::text::TextWrapping::truncate_at_width(name_w);
    let name_g = ui.painter().layout_job(job);
    ui.painter().galley(pos2(name_x, r.center().y - name_g.size().y / 2.0), name_g, t.text);
    if r.width() > kind_w + 60.0 {
        ui.painter().galley(pos2(r.right() - kind_w - 2.0, r.center().y - kind_g.size().y / 2.0), kind_g, t.text_faint);
    }
    let name = layer.name.clone();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Label, true, format!("{name} — {kind}")));
}

/// A collapsible section: a separator, then a chevron + title row, leading at the reading
/// start (the right end in a right-to-left UI). Returns whether it's open.
/// Every Properties section uses this, so headers look and behave alike.
pub fn section(ui: &mut egui::Ui, id: &str, title: &str) -> bool {
    let ir = widgets::section_header(ui, id, title, SECTION_H, true);
    record(ui.ctx(), title, ir.response.rect);
    ir.inner
}

/// Section headers drawn this pass, top to bottom (title, rect), for tests and `ui.inspect`.
pub fn sections_drawn(ctx: &egui::Context) -> Vec<(String, Rect)> {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<(u64, Vec<(String, Rect)>)>(egui::Id::unique("props-sections")))
        .filter(|(p, _)| *p + 1 >= pass)
        .map(|(_, v)| v)
        .unwrap_or_default()
}

fn record(ctx: &egui::Context, title: &str, r: Rect) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        let e = d.get_temp_mut_or_default::<(u64, Vec<(String, Rect)>)>(egui::Id::unique("props-sections"));
        if e.0 != pass {
            *e = (pass, Vec::new());
        }
        e.1.push((title.to_string(), r));
    });
}

/// Width of each field when `cols` label+field pairs (labels `label_w` wide) share the row.
pub fn field_width(avail: f32, cols: usize, label_w: f32) -> f32 {
    let n = cols.max(1) as f32;
    ((avail - n * (label_w + LABEL_GAP) - (n - 1.0) * COL_GAP) / n).max(36.0)
}

/// The Quick Actions a layer kind offers, as (label, command id), Photoshop-style: pixel layers get
/// subject selection, type layers its conversions, shapes and smart objects their own.
pub fn quick_actions(content: &LayerContent) -> &'static [(&'static str, &'static str)] {
    match content {
        LayerContent::Raster(_) => &[("Remove Background", "layer.removeBackground"), ("Select Subject", "select.subject")],
        LayerContent::Text(_) => &[
            ("Convert to Shape", "type.convertToShape"),
            ("Convert to Paragraph Text", "type.convertToParagraphText"),
            ("Convert to Point Text", "type.convertToPointText"),
            ("Create Warped Text", "type.warpText"),
            ("Rasterize Type", "type.rasterizeTypeLayer"),
        ],
        LayerContent::Shape(_) => &[("Rasterize Shape", "layer.rasterize.shape"), ("Convert to Smart Object", "layer.smartObjects.convertToSmartObject")],
        LayerContent::Smart(_) => &[
            ("Edit Contents", "layer.smartObjects.editContents"),
            ("Convert to Layers", "layer.smartObjects.convertToLayers"),
            ("Rasterize Smart Object", "layer.rasterize.smartObject"),
        ],
        LayerContent::Group(_) => &[("Ungroup Layers", "layer.ungroupLayers"), ("Convert to Smart Object", "layer.smartObjects.convertToSmartObject")],
        LayerContent::Fill(_) => &[("Rasterize Fill Content", "layer.rasterize.fillContent")],
        LayerContent::Adjustment(_) => &[],
    }
}

/// The Quick Actions shown right now: the kind's list, limited to commands that exist and are
/// enabled (Convert to Paragraph Text only for point type, and vice versa).
pub fn visible_quick_actions(app: &PhotocraftApp, content: &LayerContent) -> Vec<(&'static str, &'static str)> {
    quick_actions(content).iter().copied().filter(|(_, id)| crate::menus::is_live(id) && crate::menus::is_enabled(app, id)).collect()
}

/// Quick Actions section body: full-width buttons, two per row when the panel is wide enough.
/// Returns the command to run (through the menus, so ones with dialogs open them).
pub fn quick_actions_ui(app: &PhotocraftApp, ui: &mut egui::Ui, content: &LayerContent) -> Option<&'static str> {
    let actions = visible_quick_actions(app, content);
    if actions.is_empty() || !section(ui, "quick", "Quick Actions") {
        return None;
    }
    let avail = ui.available_width();
    let cols = if avail >= 360.0 { 2 } else { 1 };
    let mut out = None;
    for row in actions.chunks(cols) {
        // A short last row (or a lone action) still spans the panel.
        let w = ((avail - (row.len() - 1) as f32 * COL_GAP) / row.len() as f32).floor();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = COL_GAP;
            for (label, id) in row {
                let resp = widgets::secondary_button(ui, label, w);
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, *label));
                if resp.clicked() {
                    out = Some(*id);
                }
            }
        });
        ui.add_space(theme::ROW_GAP);
    }
    out
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod rtl_tests {
    use super::*;
    use crate::direction::{Dir, direction_scope};
    use egui_kittest::Harness;
    use serde_json::json;

    /// The painted box of the text with this content.
    fn painted_text(h: &Harness<'_, ()>, text: &str) -> egui::Rect {
        h.output()
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.job.text == text => Some(t.galley.rect.translate(t.pos.to_vec2())),
                _ => None,
            })
            .next()
            .unwrap_or_else(|| panic!("the painted `{text}`"))
    }

    /// The painted box of the icon image inside `within`: egui paints an image as a
    /// textured rect (or a textured mesh when rotated).
    fn painted_icon(h: &Harness<'_, ()>, within: egui::Rect) -> egui::Rect {
        fn find_icon(shape: &egui::Shape, within: egui::Rect) -> Option<egui::Rect> {
            match shape {
                egui::Shape::Rect(r) if r.brush.is_some() => r.rect.intersects(within).then_some(r.rect),
                egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => {
                    let r = m.calc_bounds();
                    r.intersects(within).then_some(r)
                }
                // SVG icons tessellate into vector paths, not textured rects.
                egui::Shape::Path(p) => {
                    let r = p.visual_bounding_rect();
                    r.intersects(within).then_some(r)
                }
                egui::Shape::Vec(v) => v.iter().find_map(|s| find_icon(s, within)),
                _ => None,
            }
        }
        h.output().shapes.iter().find_map(|cs| find_icon(&cs.shape, within)).unwrap_or_else(|| panic!("the painted icon"))
    }

    /// A section header drawn in `dir`: its title box, its chevron box, and the row it sits in.
    fn section_header(dir: Dir) -> (egui::Rect, egui::Rect, egui::Rect) {
        let _dir = direction_scope(dir);
        let mut h = Harness::builder().with_size(vec2(300.0, 120.0)).build_ui_state(
            |ui, _state: &mut ()| {
                // The fonts are installed after the harness is built: the first frame has
                // none, and the painted captions would panic without them.
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    section(ui, "appearance", "Appearance");
                }
            },
            (),
        );
        crate::PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.run_steps(4);
        let row = sections_drawn(&h.ctx).into_iter().find(|(title, _)| title == "Appearance").map(|(_, r)| r).unwrap_or_else(|| panic!("the section row"));
        (painted_text(&h, "Appearance"), painted_icon(&h, row), row)
    }

    /// The painted text boxes of a draw call in `dir`.
    fn painted_in(dir: Dir, draw: impl Fn(&mut egui::Ui)) -> Vec<(String, egui::Rect)> {
        let _dir = direction_scope(dir);
        let mut h = Harness::builder().with_size(vec2(300.0, 120.0)).build_ui_state(
            move |ui, _state: &mut ()| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    draw(ui);
                }
            },
            (),
        );
        crate::PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.run_steps(4);
        h.output()
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(t) => Some((t.galley.job.text.clone(), t.galley.rect.translate(t.pos.to_vec2()))),
                _ => None,
            })
            .collect()
    }

    /// A layer to draw the header of.
    fn layer() -> Layer {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
        let id = s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap()["layer"].as_u64().unwrap();
        s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().clone()
    }

    /// The section header reads from the right in a right-to-left UI: the title ends 18 px
    /// before the row's right edge, where the chevron leads it.
    #[test]
    fn section_header_reads_from_the_right_in_rtl() {
        let (ltr_title, ltr_chev, ltr_row) = section_header(Dir::Ltr);
        assert!((ltr_title.left() - (ltr_row.left() + 18.0)).abs() < 1.0, "LTR: the title starts 18 px into its row: {ltr_title:?} {ltr_row:?}");
        assert!((ltr_chev.center().x - (ltr_row.left() + 7.0)).abs() < 1.0, "LTR: the chevron leads 7 px into its row: {ltr_chev:?} {ltr_row:?}");
        assert!(ltr_chev.right() <= ltr_title.left(), "LTR: the chevron stays before the title: {ltr_chev:?} {ltr_title:?}");

        let (rtl_title, rtl_chev, rtl_row) = section_header(Dir::Rtl);
        assert!((rtl_title.right() - (rtl_row.right() - 18.0)).abs() < 1.0, "RTL: the title ends 18 px before its row's right edge: {rtl_title:?} {rtl_row:?}");
        assert!(
            (rtl_chev.center().x - (rtl_row.right() - 7.0)).abs() < 1.0,
            "RTL: the chevron leads 7 px before its row's right edge: {rtl_chev:?} {rtl_row:?}"
        );
        assert!(rtl_chev.left() >= rtl_title.right(), "RTL: the chevron stays after the title: {rtl_chev:?} {rtl_title:?}");
    }

    /// The layer header's icon and name stay on the physical left and the stacked label on the
    /// physical right: the fields around them mirror, these do not.
    #[test]
    fn the_layer_header_and_the_stacked_label_stay_physical() {
        let layer = layer();
        let draw = |ui: &mut egui::Ui| {
            header(ui, &layer);
            stacked_label(ui, "W", &["W", "H"], LABEL_W, 2.0);
        };
        let ltr = painted_in(Dir::Ltr, draw);
        let rtl = painted_in(Dir::Rtl, draw);
        for text in ["Paint", "Pixel Layer", "W"] {
            let ltr_box = ltr.iter().find(|(t, _)| t == text).map(|(_, r)| *r).unwrap_or_else(|| panic!("the LTR `{text}`"));
            let rtl_box = rtl.iter().find(|(t, _)| t == text).map(|(_, r)| *r).unwrap_or_else(|| panic!("the RTL `{text}`"));
            assert_eq!(ltr_box, rtl_box, "the `{text}` keeps its physical position");
        }
    }
}
