//! Shared Properties-panel layout (#155): the layer header (kind icon + layer name), the one
//! collapsible section header every section uses (separator, disclosure chevron, title), column
//! widths that make field rows fill the panel, and the Quick Actions each layer kind offers.

use egui::{Rect, Sense, pos2, vec2};
use photocraft_doc::{Layer, LayerContent, SmartSource};

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
        LayerContent::Shape(_) => "vector-shape",
        LayerContent::Smart(_) => "app-window",
        LayerContent::Raster(_) => "image",
    }
}

/// "Type Layer", "Curves", "Artboard"…
pub fn kind_label(layer: &Layer) -> String {
    if layer.frame.is_some() {
        return tl!("Frame").into();
    }
    match &layer.content {
        LayerContent::Adjustment(a) => a.label().to_string(),
        LayerContent::Group(g) if g.artboard.is_some() => "Artboard".to_string(),
        // Photoshop tells a linked smart object apart in the header (#14121).
        content if linked_file(content).is_some() => "Linked Smart Object".to_string(),
        other => format!("{} Layer", other.kind_name()),
    }
}

/// The file a linked smart object follows (File › Place Linked, Relink to File, Convert to
/// Linked), as stored in the document; `None` for any other layer. A placed layer read from a PSD
/// is not one: its path is the uuid of an entry in the PSD's own file table, which PhotoCraft
/// reads its source from, until a relink or Convert to Linked gives it a file (and drops the PSD
/// placed-layer data, `psd_raw`).
pub fn linked_file(content: &LayerContent) -> Option<&str> {
    match content {
        LayerContent::Smart(sm) if sm.psd_raw.is_none() => match &sm.source {
            SmartSource::Linked { path } => Some(path),
            SmartSource::Embedded { .. } => None,
        },
        _ => None,
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
    // The name takes what's left between the icon and the kind (elided when long).
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

/// A collapsible section: a separator, then a chevron + title row. Returns whether it's open.
/// Every Properties section uses this, so headers look and behave alike.
pub fn section(ui: &mut egui::Ui, id: &str, title: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let key = egui::Id::unique(("props-section", id));
    let mut open = ui.data(|d| d.get_temp::<bool>(key)).unwrap_or(true);
    ui.add_space(theme::ROW_GAP);
    widgets::hairline(ui);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), SECTION_H), Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    crate::icons::paint(
        ui,
        Rect::from_center_size(pos2(r.left() + 7.0, r.center().y), vec2(12.0, 12.0)),
        if open { "chevron-down" } else { "chevron-right" },
        11.0,
        t.text_dim,
    );
    ui.painter().text(pos2(r.left() + 18.0, r.center().y), egui::Align2::LEFT_CENTER, tl!(title), theme::semibold(12.0), t.text);
    if resp.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(key, open));
    }
    let label = title.to_string();
    resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::DisclosureTriangle, true, open, &label));
    record(ui.ctx(), title, r);
    if open {
        ui.add_space(theme::ROW_GAP / 2.0);
    }
    open
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
/// subject selection, type layers its conversions, shapes and smart objects their own. A linked
/// smart object offers Edit Contents, Embed and Convert to Layers, in that order (#14121).
pub fn quick_actions(content: &LayerContent) -> &'static [(&'static str, &'static str)] {
    if linked_file(content).is_some() {
        return &[("Edit Contents", "layer.smartObjects.editContents"), (EMBED.0, EMBED.1), ("Convert to Layers", "layer.smartObjects.convertToLayers")];
    }
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

/// A linked smart object's Embed Quick Action: Layer › Smart Objects › Embed Linked.
pub const EMBED: (&str, &str) = ("Embed", "layer.smartObjects.convertToEmbedded");

/// Whether the Quick Action `id` can't run on `content` right now though its command is enabled:
/// Embed while the linked file is missing (the last look at it found nothing), as Photoshop greys
/// it, so the button stays, greyed, until the file is back.
fn greyed(app: &PhotocraftApp, content: &LayerContent, id: &str) -> bool {
    id == EMBED.1 && linked_file(content).is_some_and(|path| app.session.missing_links().contains(path))
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
                let on = !greyed(app, content, id);
                let resp = ui.add_enabled_ui(on, |ui| widgets::secondary_button(ui, label, w)).inner;
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, on, *label));
                if on && resp.clicked() {
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
