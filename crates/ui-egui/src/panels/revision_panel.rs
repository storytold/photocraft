//! Revision markers panel: red boxes over content that must be changed in a manual / 说明书,
//! turning green when detection finds the framed content changed since the marker's snapshot.
//! A marker bound to a layer follows that layer when it moves, so the box keeps framing the same
//! content. The panel drives the `revision.*` engine commands (one undo step per change).

use photocraft_compose::bounds::content_bounds;
use photocraft_compose::layer_bounds;
use serde_json::json;

use super::{PhotocraftApp, RichText, Tokens, empty};
use crate::canvas::revision_overlay::{DONE, TODO};

/// Draw the Revision panel: the toolbar (add from the selection, check all, reset) over one row
/// per marker.
pub(super) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        empty(ui, tl!("No document"));
        return;
    };
    let n = st.doc.revision_markers.len();
    ui.horizontal_wrapped(|ui| {
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
        // One undo step for the whole list; the per-row Delete removes a single marker.
        if ui.add_enabled(can, egui::Button::new(tl!("Clear All"))).clicked() {
            let _ = app.run("revision.clear", json!({}));
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
        // The rows are drawn from the live list, not to the length read before the buttons ran:
        // Delete and Clear All shrink the list while these rows are being laid out, and asking for
        // the marker at the old index after that ran off the end of the list and took PhotoCraft
        // down with it.
        let mut i = 0;
        loop {
            let len = app.session.active().map_or(0, |st| st.doc.revision_markers.len());
            if i >= len {
                break;
            }
            row(app, ui, i, &t);
            ui.add_space(2.0);
            // The row's own Delete removed this marker, so the next one slid down into its place:
            // draw it as the same index rather than stepping over it.
            if app.session.active().map_or(0, |st| st.doc.revision_markers.len()) == len {
                i += 1;
            }
        }
    });
}

/// Draw one marker: its state dot, the editable name, and the buttons that act on it (Delete,
/// Reset, Bind/Unbind).
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
        // The row's controls are laid out from the right edge inwards, Delete first: in a narrow
        // dock the row used to push the ✕ out of sight, which read as "markers cannot be deleted".
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let del = ui.add(egui::Button::new(RichText::new(tl!("Delete")).size(11.5)).small());
            if del.on_hover_text(tl!("Remove this marker")).clicked() {
                let _ = app.run("revision.remove", json!({"index": i}));
            }
            if ui.add(egui::Button::new(RichText::new(tl!("Reset")).size(11.5)).small()).clicked() {
                // Re-snapshot just this marker: its box turns red again (undo with Ctrl+Z).
                let _ = app.run("revision.snapshot", json!({"index": i}));
            }
            if let Some(name) = &bound_name {
                if ui.add(egui::Button::new(RichText::new(tl!("Unbind")).size(11.5)).small()).clicked() {
                    let _ = app.run("revision.set", json!({"index": i, "layerId": null}));
                }
                // A bound layer's name is often a whole sentence ("1.将2号和19号板材… copy 5"), and
                // drawing it in full widened the row until the buttons of the rows above and below
                // no longer lined up, which also made the list hard to scan. Show a short prefix
                // with an ellipsis and keep the whole name in the tooltip.
                let shown = shorten_name(name, BOUND_NAME_CHARS);
                ui.label(RichText::new(format!("↔ {shown}")).size(10.5).color(t.text_dim))
                    .on_hover_text(name.as_str());
            } else if ui.add(egui::Button::new(RichText::new(tl!("Bind Layer")).size(11.5)).small()).clicked() {
                // Bind what the frame actually covers. The frame's centre alone is not enough: a
                // marker over a whole row of a manual is wide and short, so its centre often lands
                // on the blank gutter between layers and used to report "no layer found" even
                // though the frame was drawn around a layer (marker 52 in the report). The layer
                // covering most of the frame wins, and a layer picked in the Layers panel binds
                // when nothing at all lies under the frame.
                let id = layer_under_frame(app, pos, size).or_else(|| selected_layer(app));
                match id {
                    Some(id) => {
                        let _ = app.run("revision.set", json!({"index": i, "layerId": id}));
                    }
                    None => {
                        app.ui.status =
                            tl!("No layer under this marker. Select the layer in the Layers panel and press Bind Layer again.").into();
                        app.ui.status_error = true;
                    }
                }
            }
            // The author's own verdict: green without re-taking the snapshot (Reset does that).
            let marked = if done {
                ui.add(egui::Button::new(RichText::new(tl!("Mark as Red")).size(11.5)).small())
                    .on_hover_text(tl!("Set this marker back to red without re-taking its snapshot"))
            } else {
                ui.add(egui::Button::new(RichText::new(tl!("Mark as Green")).size(11.5)).small())
                    .on_hover_text(tl!("Set this marker green without comparing pixels"))
            };
            if marked.clicked() {
                let _ = app.run("revision.set", json!({"index": i, "done": !done}));
            }
            // Whatever is left of the row is the marker's name.
            let mut name = name.clone();
            let edited = ui.add(egui::TextEdit::singleline(&mut name).desired_width(ui.available_width().max(48.0)));
            if edited.changed() {
                let _ = app.run("revision.set", json!({"index": i, "name": name}));
            }
        });
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

/// A coordinate to one decimal, without a trailing `.0` when it is a whole number.
fn round1(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r.fract().abs() < 1e-9 {
        format!("{}", r as i64)
    } else {
        format!("{r:.1}")
    }
}

/// Characters of a bound layer's name the row shows before an ellipsis. Layer names in a manual are
/// whole sentences, so the cap keeps every row the same width whatever the layer is called.
const BOUND_NAME_CHARS: usize = 16;

/// The first `max` characters of `name`, with `…` when anything was dropped. Counted in characters,
/// not bytes, so a Chinese name is not cut in the middle of a glyph.
fn shorten_name(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        return name.to_owned();
    }
    let mut out: String = name.chars().take(max).collect();
    out.push('…');
    out
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
    let name = next_marker_name(app);
    let mut params = json!({"x": x, "y": y, "width": w, "height": h, "name": name});
    if let Some(id) = layer_under_frame(app, [x, y], [w, h]) {
        params["layerId"] = json!(id);
    }
    let added = app.run("revision.add", params).is_ok();
    if added && app.session.active().is_some_and(|st| st.doc.selection.is_some()) {
        let _ = app.run("select.deselect", json!({}));
    }
}

/// The next marker name: the highest number already on screen, plus one. Naming by the list's
/// length gave two markers the same number as soon as one in the middle was deleted (a 15-row list
/// whose marker 8 was removed names the next marker "Marker 15" while "Marker 15" is still there).
///
/// The number is read off the end of the existing names rather than after the current `Marker`
/// word: a document that was numbered while the interface was in another language carries names
/// like "标记 7" next to "Marker 7", and only counting the current word's names made the new marker
/// repeat a number that was already on the canvas.
pub(crate) fn next_marker_name(app: &PhotocraftApp) -> String {
    let prefix = tl!("Marker");
    let used: Vec<&str> = app
        .session
        .active()
        .map(|st| st.doc.revision_markers.iter().map(|m| m.name.as_str()).collect())
        .unwrap_or_default();
    let mut highest = 0usize;
    for name in &used {
        let tail: String = name.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<String>().chars().rev().collect();
        if let Ok(number) = tail.parse::<usize>() {
            highest = highest.max(number);
        }
    }
    let mut n = highest + 1;
    let mut name = format!("{prefix} {n}");
    while used.contains(&name.as_str()) {
        n += 1;
        name = format!("{prefix} {n}");
    }
    name
}

/// The layer a marker frame should follow: the topmost visible, non-background pixel layer covering
/// most of the frame. Groups (folders) are skipped, so binding tracks the layer under the frame and
/// not the folder above it. Falls back to a pure containment test so a small frame behaves exactly
/// as before.
pub(crate) fn layer_under_frame(app: &PhotocraftApp, pos: [f64; 2], size: [f64; 2]) -> Option<u64> {
    let st = app.session.active()?;
    let doc = &st.doc;
    let canvas = doc.bounds();
    let (x0, y0) = (pos[0] as i32, pos[1] as i32);
    let (x1, y1) = ((pos[0] + size[0]) as i32, (pos[1] + size[1]) as i32);
    let mut best: Option<(i64, u64)> = None;
    for (_, _, layer) in doc.walk().into_iter().rev() {
        if doc.is_background(layer.id) || !layer.visible || layer.is_group() {
            continue;
        }
        let b = layer_bounds(layer, canvas);
        if b.is_empty() {
            continue;
        }
        let (ox, oy) = ((b.x1.min(x1) - b.x0.max(x0)).max(0), (b.y1.min(y1) - b.y0.max(y0)).max(0));
        let overlap = i64::from(ox) * i64::from(oy);
        if overlap == 0 {
            continue;
        }
        if best.is_none_or(|(area, _)| overlap > area) {
            best = Some((overlap, layer.id.0));
        }
    }
    best.map(|(_, id)| id)
}

/// The layer chosen in the Layers panel when exactly one is picked. Shapes are stored as groups in
/// this document model, and a folder can be the thing that moves a whole block of a manual, so the
/// old "pixel or shape layer only" test refused most of the layers a user would pick here and the
/// Bind Layer button appeared to do nothing.
fn selected_layer(app: &PhotocraftApp) -> Option<u64> {
    let st = app.session.active()?;
    let doc = &st.doc;
    let ids = st.selected_layers();
    let id = ids.first().copied()?;
    let layer = doc.layer(id)?;
    (!doc.is_background(id) && layer.visible).then_some(id.0)
}

#[cfg(test)]
mod tests {
    use super::shorten_name;

    /// A short name is drawn in full, a long one keeps its first characters and gains an ellipsis,
    /// and the ellipsis counts as one character rather than as three bytes of the name.
    #[test]
    fn long_layer_names_are_shortened_with_an_ellipsis() {
        assert_eq!(shorten_name("A29 板材", 16), "A29 板材", "a name that fits is left alone");
        let long = "1.将2号和19号板材如上图一样安装在一起。";
        let shown = shorten_name(long, 16);
        assert_eq!(shown.chars().count(), 17, "16 characters plus the ellipsis");
        assert!(shown.ends_with('…'), "the tail is an ellipsis");
        let prefix: String = shown.chars().take(16).collect();
        assert!(long.starts_with(&prefix), "the shown text is the start of the full name");
    }
}
