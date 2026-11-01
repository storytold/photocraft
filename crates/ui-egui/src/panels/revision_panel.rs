//! Revision markers panel: red boxes over content that must be changed in a manual / 说明书,
//! turning green when detection finds the framed content changed since the marker's snapshot.
//! A marker bound to a layer follows that layer when it moves, so the box keeps framing the same
//! content. The panel drives the `revision.*` engine commands (one undo step per change).

use photocraft_compose::bounds::content_bounds;
use photocraft_compose::layer_bounds;
use serde_json::json;

use super::{PhotocraftApp, RichText, Tokens, empty};
use crate::canvas::revision_overlay::{DONE, TODO};

pub(super) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        empty(ui, tl!("No document"));
        return;
    };
    let n = st.doc.revision_markers.len();
    ui.horizontal(|ui| {
        if ui.button(tl!("Add from Selection")).clicked() {
            add_from_selection(app);
        }
        let can = n > 0;
        if ui.add_enabled(can, egui::Button::new(tl!("Check All"))).clicked() {
            let _ = app.run("revision.detect", json!({}));
        }
        // Reset = re-snapshot every marker: the current content becomes the new baseline and the
        // box turns red again (use after duplicating a finished file, to re-arm the checklist).
        // Undo (Ctrl+Z) restores the previous snapshots and green state.
        if ui.add_enabled(can, egui::Button::new(tl!("Reset All to Red"))).clicked() {
            let _ = app.run("revision.snapshot", json!({}));
        }
    });
    if n == 0 {
        ui.add_space(4.0);
        ui.label(
            RichText::new(tl!("Select the content to change with the Rectangular Marquee, then press Add from Selection."))
                .size(11.5)
                .color(t.text_dim),
        );
        return;
    }
    ui.separator();
    egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
        for i in 0..n {
            row(app, ui, i, &t);
            ui.add_space(2.0);
        }
    });
}

fn row(app: &mut PhotocraftApp, ui: &mut egui::Ui, i: usize, t: &Tokens) {
    // Read the marker's row data up front and drop the document borrow, so the button closures
    // below can call `app.run` (which borrows the app mutably) without a conflict.
    let (name, layer_id, done, pos, size) = {
        let Some(st) = app.session.active() else { return };
        let m = &st.doc.revision_markers[i];
        (m.name.clone(), m.layer_id, m.done, m.pos, m.size)
    };
    let bound_name = layer_id.and_then(|id| app.session.active()?.doc.layer(id).map(|l| l.name.clone()));
    let color = if done { DONE } else { TODO };
    ui.horizontal(|ui| {
        ui.colored_label(color, if done { "✓" } else { "●" });
        let mut name = name.clone();
        let edited = ui.add(egui::TextEdit::singleline(&mut name).desired_width(90.0));
        if edited.changed() {
            let _ = app.run("revision.set", json!({"index": i, "name": name}));
        }
        if let Some(name) = &bound_name {
            ui.label(RichText::new(format!("↔ {name}")).size(11.0).color(t.text_dim));
            if ui.small_button(tl!("Unbind")).clicked() {
                let _ = app.run("revision.set", json!({"index": i, "layerId": null}));
            }
        } else if ui.small_button(tl!("Bind Layer")).clicked() {
            // Bind the layer under the marker's own frame, not whatever layer happens to be
            // selected: each marker follows the content it actually frames.
            let id = layer_at_point(app, [pos[0] + size[0] / 2.0, pos[1] + size[1] / 2.0]);
            match id {
                Some(id) => {
                    let _ = app.run("revision.set", json!({"index": i, "layerId": id}));
                }
                None => {
                    app.ui.status = tl!("No layer found under this marker; select a layer and bind it manually.").into();
                    app.ui.status_error = true;
                }
            }
        }
        if ui.small_button(tl!("Reset")).clicked() {
            // Re-snapshot just this marker: its box turns red again (undo with Ctrl+Z).
            let _ = app.run("revision.snapshot", json!({"index": i}));
        }
        if ui.small_button("✕").clicked() {
            let _ = app.run("revision.remove", json!({"index": i}));
            return;
        }
    });
    let pos = format!(
        "x {}  y {}  {} × {}",
        round1(pos[0]),
        round1(pos[1]),
        round1(size[0]),
        round1(size[1])
    );
    ui.label(RichText::new(pos).size(10.5).color(t.text_dim));
}

fn round1(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r.fract().abs() < 1e-9 {
        format!("{}", r as i64)
    } else {
        format!("{r:.1}")
    }
}

/// Add a marker from the current selection (Rectangular Marquee), or a centred default box when
/// nothing is selected. The layer under the frame is bound automatically and the selection is
/// cleared once the marker exists.
fn add_from_selection(app: &mut PhotocraftApp) {
    let Some(st) = app.session.active() else { return };
    let doc = &st.doc;
    let (x, y, w, h) = match doc.selection.as_ref().map(content_bounds) {
        Some(r) if !r.is_empty() => (f64::from(r.x0), f64::from(r.y0), f64::from(r.width()), f64::from(r.height())),
        _ => {
            let (cx, cy) = (f64::from(doc.size.width) / 2.0, f64::from(doc.size.height) / 2.0);
            (cx - 100.0, cy - 30.0, 200.0, 60.0)
        }
    };
    let name = format!("{} {}", tl!("Marker"), doc.revision_markers.len() + 1);
    let mut params = json!({"x": x, "y": y, "width": w, "height": h, "name": name});
    if let Some(id) = layer_at_point(app, [x + w / 2.0, y + h / 2.0]) {
        params["layerId"] = json!(id);
    }
    let added = app.run("revision.add", params).is_ok();
    if added && app.session.active().is_some_and(|st| st.doc.selection.is_some()) {
        let _ = app.run("select.deselect", json!({}));
    }
}

/// Topmost visible, non-background pixel layer whose content bounds contain `pt`, if any. Groups
/// (folders) are skipped so binding tracks the actual layer under the frame, not the folder.
fn layer_at_point(app: &PhotocraftApp, pt: [f64; 2]) -> Option<u64> {
    let st = app.session.active()?;
    let doc = &st.doc;
    let canvas = doc.bounds();
    let (x, y) = (pt[0].round() as i32, pt[1].round() as i32);
    for (_, _, layer) in doc.walk().into_iter().rev() {
        if doc.is_background(layer.id) || !layer.visible || layer.is_group() {
            continue;
        }
        let b = layer_bounds(layer, canvas);
        if !b.is_empty() && b.contains(x, y) {
            return Some(layer.id.0);
        }
    }
    None
}
