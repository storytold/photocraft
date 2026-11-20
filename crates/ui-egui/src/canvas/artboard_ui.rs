//! Artboards in the shell: the page frames and loose-work holes the canvas draws its pasteboard
//! from, and the Properties panel section (X/Y/W/H, preset, background). Everything that changes
//! the document is `layer.artboard.set` and friends. View › Fit Artboard on Screen is the
//! canvas's `ViewCommands::FitArtboard`, aimed at the active frame `pages` marks.

use std::sync::Arc;

use craft::behavior::EditEvent;
use craft::behavior::kurbo::{Rect as DocRect, Size};
use craft::draw::ui::artboard::{ArtboardBackground as BoardBackground, ArtboardItem, ArtboardPreset, ArtboardProperties};
use craft::theme::Srgba;
use craft::ui::canvas::PageFrame;
use photocraft_doc::{ArtboardBackground, Document, Layer};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// A document rect in the framework's document units. Each i32 coordinate widens on its own
/// before any arithmetic: a board near ±2^30, or one whose far edge saturated at `i32::MAX`,
/// overflows an i32 sum (#981).
fn doc_rect(r: photocraft_geom::Rect) -> DocRect {
    DocRect::new(f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1))
}

/// One frame per artboard of document `idx`, in `doc.artboards()` order. The active layer's board
/// is the active frame, the others are marked inactive. The frames carry no fill: the document
/// picture paints each board's background.
pub(crate) fn pages(app: &PhotocraftApp, idx: usize) -> Arc<[PageFrame]> {
    let Some(st) = app.session.documents().get(idx) else { return Arc::new([]) };
    app.doc_queries.pages(&app.session, st, || {
        let active = st.active_layer.and_then(|id| st.doc.artboard_of(id));
        st.doc.artboards().into_iter().map(|(id, name, a)| PageFrame::new(doc_rect(a.rect)).name(name).active(active == Some(id))).collect()
    })
}

/// Where visible layers outside the boards draw: Photoshop shows those over the pasteboard, so a
/// board added next to earlier work must not paint over it (#5712). The Background and any layer
/// that covers the whole canvas (or may draw anywhere, such as a fill layer) are backdrop, not
/// loose work: they keep the pasteboard. The framework subtracts these rects and the pages from
/// the pasteboard in every view.
pub(crate) fn uncovered(doc: &Document) -> Vec<DocRect> {
    let canvas = doc.bounds();
    let background = doc.background().map(|b| b.id);
    doc.layers
        .iter()
        .filter(|l| l.visible && !is_artboard(l) && Some(l.id) != background)
        .filter_map(|l| photocraft_compose::composite_bounds(l, canvas))
        .filter(|b| !b.is_empty() && !b.contains_rect(&canvas))
        .map(doc_rect)
        .collect()
}

fn is_artboard(l: &Layer) -> bool {
    matches!(&l.content, photocraft_doc::LayerContent::Group(g) if g.artboard.is_some())
}

/// The size presets the Properties section offers, from the engine's list.
fn presets() -> Vec<ArtboardPreset> {
    photocraft_engine::artboard_cmds::PRESETS.iter().map(|&(name, w, h)| ArtboardPreset::new(name, Size::new(f64::from(w), f64::from(h)))).collect()
}

/// The framework's snapshot of the artboard `layer` holds.
fn board_item(layer: &Layer, a: &photocraft_doc::Artboard) -> ArtboardItem {
    let mut item = ArtboardItem::new(layer.name.clone(), doc_rect(a.rect));
    item.background = match a.background {
        ArtboardBackground::White => BoardBackground::White,
        ArtboardBackground::Black => BoardBackground::Black,
        ArtboardBackground::Transparent => BoardBackground::Transparent,
        ArtboardBackground::Custom(c) => BoardBackground::Custom(Srgba::from_rgb_array(c.to_rgb())),
    };
    item
}

/// Properties panel for an artboard.
pub fn properties(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer) {
    let Some(a) = layer.artboard() else { return };
    let item = board_item(layer, a);
    let presets = presets();
    let out = ArtboardProperties::new(&item, &presets).show(ui);
    let id = layer.id.0;
    let mut edits: Vec<Value> = Vec::new();
    if let Some(i) = out.preset {
        // The engine resizes from the preset's name; the rect events of the same pick are not sent.
        if let Some(preset) = presets.get(i) {
            edits.push(json!({"layer": id, "preset": preset.name}));
        }
    } else {
        for event in out.rect.iter() {
            // A scrub's updates and its commit are one undo step under the gesture's key; a lone
            // `Set` is a step of its own.
            let (rect, coalesce) = match event {
                EditEvent::Update { id: g, value, .. } => (value, Some(g.0)),
                EditEvent::Commit { id: g, after, .. } => (after, Some(g.0)),
                EditEvent::Set { after, .. } => (after, None),
                _ => continue,
            };
            let mut p = json!({"layer": id, "x": rect.x0.round(), "y": rect.y0.round(), "width": rect.width().round(), "height": rect.height().round()});
            if let Some(g) = coalesce {
                p["coalesce"] = json!(format!("artboard-{id}-rect-{g}"));
            }
            edits.push(p);
        }
    }
    if let Some(bg) = out.background {
        let name = match bg {
            BoardBackground::White => "white",
            BoardBackground::Black => "black",
            BoardBackground::Transparent => "transparent",
            _ => "custom",
        };
        edits.push(json!({"layer": id, "background": name}));
    }
    for p in edits {
        if let Err(e) = app.run("layer.artboard.set", p) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    if let Some(request) = out.pick_color {
        let title = request.title.as_ref().map_or_else(|| crate::color_picker_ui::title_for(tl!("Color")), |t| t.as_str().to_string());
        let rgb = request.color.map_or([1.0; 3], Srgba::to_rgb_array);
        crate::color_picker_ui::open_for_command(app, &title, rgb, "layer.artboard.set", json!({"layer": id, "background": "custom"}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> (Box<PhotocraftApp>, egui::Context) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 100, "height": 50})).unwrap();
        app.sync_views();
        let ctx = app.ctx().clone();
        (app, ctx)
    }

    #[test]
    fn layers_outside_the_boards_show_over_the_pasteboard() {
        // #5712: a board drawn left of earlier work must not cover it with pasteboard.
        let (mut app, _) = app();
        app.run("layer.new.artboard", json!({"rect": [0, 0, 40, 50]})).unwrap();
        let mut doc = (*app.session.active().unwrap().doc).clone();
        let mut px = photocraft_doc::Layer::raster("Work", doc.pixel_format());
        px.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(60, 10, 80, 30), &[1.0, 0.0, 0.0, 1.0]);
        doc.layers.insert(0, px);
        assert_eq!(uncovered(&doc), vec![DocRect::new(60.0, 10.0, 80.0, 30.0)]);
    }

    #[test]
    fn full_canvas_layers_keep_the_pasteboard() {
        // The white Background (from `app()`) and a loose layer filling the whole canvas are
        // backdrop: the pasteboard still shows outside the board.
        let (mut app, _) = app();
        app.run("layer.new.artboard", json!({"rect": [0, 0, 40, 50]})).unwrap();
        let mut doc = (*app.session.active().unwrap().doc).clone();
        assert!(doc.background().is_some());
        let mut px = photocraft_doc::Layer::raster("Wash", doc.pixel_format());
        px.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.0, 0.0, 1.0, 1.0]);
        doc.layers.push(px);
        assert!(uncovered(&doc).is_empty());
    }

    #[test]
    fn pages_mark_only_the_active_layers_board() {
        let (mut app, _) = app();
        app.run("layer.new.artboard", json!({"rect": [0, 0, 40, 50]})).unwrap();
        let first = app.session.active().unwrap().doc.artboards()[0].0;
        app.run("layer.new.artboard", json!({"rect": [60, 0, 40, 50]})).unwrap();
        app.run("layer.select", json!({"layer": first.0})).unwrap();
        let i = app.session.active_index().unwrap();
        let marks: Vec<Option<bool>> = pages(&app, i).iter().map(|f| f.active).collect();
        assert_eq!(marks, vec![Some(true), Some(false)]);
    }

    #[test]
    fn comp_target_follows_selection_then_last_applied() {
        let (mut app, _) = app();
        assert_eq!(crate::comps_ui::target(&app), None);
        let a = app.run("layerComp.new", json!({})).unwrap()["comp"].as_u64().unwrap() as u32;
        let b = app.run("layerComp.new", json!({})).unwrap()["comp"].as_u64().unwrap() as u32;
        assert_eq!(crate::comps_ui::target(&app), Some(b));
        app.ui.layer_comp_selected = Some(a);
        assert_eq!(crate::comps_ui::target(&app), Some(a));
        app.ui.layer_comp_selected = Some(999);
        assert_eq!(crate::comps_ui::target(&app), Some(b));
    }
}
