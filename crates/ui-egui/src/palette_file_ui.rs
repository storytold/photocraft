//! Loading and saving palettes from the Indexed Color (Custom…) and Color Table dialogs: Adobe's
//! `.act` tables (768 bytes of RGB) and palette PNGs. The dialog body sets a request flag in its
//! fields; the dialog loop (which has the app) acts on it through [`take_request`], so a file
//! dialog can be asked from anywhere in the body.

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

/// The Color Table dialog's table field: a JSON array of `"#rrggbb"`.
pub const COLORS_FIELD: &str = "colors";
/// The Indexed Color dialog's Custom palette field (its `colors` param is the count).
pub const CUSTOM_FIELD: &str = "paletteColors";

/// The field holding the table for this dialog's command.
pub fn colors_field(command: &str) -> &'static str {
    if command == "image.mode.colorTable" { COLORS_FIELD } else { CUSTOM_FIELD }
}

const LOAD: &str = "__paletteLoad";
const SAVE: &str = "__paletteSave";

/// Ask the dialog loop to open a palette file into this dialog's table.
pub fn request_load(f: &mut Map<String, Value>) {
    f.insert(LOAD.into(), json!("load"));
}

/// Ask the dialog loop to save this dialog's table.
pub fn request_save(f: &mut Map<String, Value>) {
    f.insert(SAVE.into(), json!("save"));
}

/// Act on a pending request of dialog `id` (the loop calls it once per frame).
pub fn take_request(app: &mut PhotocraftApp, id: u64) -> Result<(), String> {
    let command = app.ui.dialogs.iter().find(|d| d.id == id).and_then(|d| d.fields.get("__command")).and_then(Value::as_str).unwrap_or_default().to_string();
    let pending = app.ui.dialog_mut(id).and_then(|d| d.fields.get(LOAD).cloned().or_else(|| d.fields.get(SAVE).cloned()));
    let Some(pending) = pending else { return Ok(()) };
    if let Some(d) = app.ui.dialog_mut(id) {
        d.fields.remove(LOAD);
        d.fields.remove(SAVE);
    }
    match pending.as_str() {
        Some("load") => load(app, id, &command),
        Some("save") => save(app, id, &command),
        _ => Ok(()),
    }
}

fn load(app: &mut PhotocraftApp, id: u64, _command: &str) -> Result<(), String> {
    app.pick_file_bytes_filtered(&["act", "png"], move |app, name, bytes| {
        let (colors, transparent) = parse_palette_file(&name, &bytes)?;
        if let Some(d) = app.ui.dialog_mut(id) {
            apply_loaded_palette(&mut d.fields, &colors, transparent);
        }
        Ok(Value::Null)
    })?;
    Ok(())
}

/// Apply a loaded palette (`colors` and its optional `transparent` index) to dialog `f` (Indexed
/// Color Custom or Color Table): updates the table field, switches Color Table to Custom, and
/// sets or clears `"transparent"` so a second load without transparency clears a previous one
/// (and vice versa).
pub fn apply_loaded_palette(f: &mut Map<String, Value>, colors: &[[u8; 3]], transparent: Option<u8>) {
    let command = f.get("__command").and_then(Value::as_str).unwrap_or_default();
    let field = colors_field(command);
    if command == "image.mode.colorTable" {
        f.insert("table".into(), json!("custom"));
    }
    f.insert(field.into(), json!(hex_array(colors)));
    match transparent {
        Some(t) => f.insert("transparent".into(), json!(t)),
        None => f.insert("transparent".into(), Value::Null),
    };
}

fn save(app: &mut PhotocraftApp, id: u64, command: &str) -> Result<(), String> {
    let field = colors_field(command);
    let Some(colors) = app.ui.dialog_mut(id).and_then(|d| d.fields.get(field)).cloned().and_then(|v| hex_array_to_rgb(&v)) else {
        return Ok(()); // Nothing loaded yet: nothing to save.
    };
    // The transparent entry (when set) goes into the file: ACT trailer / PNG tRNS.
    let transparent = app.ui.dialog_mut(id).and_then(|d| d.fields.get("transparent")).cloned().and_then(|v| v.as_u64()).and_then(|t| u8::try_from(t).ok());
    app.pick_save("palette.act", move |app, path| {
        let bytes = match Path::new(&path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("png") => {
                let indices: Vec<u8> = (0..colors.len()).map(u8::try_from).collect::<Result<_, _>>().map_err(|_| "a palette has at most 256 entries")?;
                photocraft_codecs::encode_png_indexed(colors.len() as u32, 1, &indices, &colors, transparent, None, None).map_err(|e| e.to_string())?
            }
            // The trailer (entry count + transparent index) goes in only when a transparent
            // entry is set, as the dialog's Save… has always written it.
            _ => {
                photocraft_psd::act::write(&colors, transparent.map(|_| colors.len().min(256) as u16), transparent.map(u16::from)).map_err(|e| e.to_string())?
            }
        };
        let write = app.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, &bytes)?;
        Ok(json!({"path": path}))
    })?;
    Ok(())
}

/// An `.act` file's entries and transparent index, via the shared `photocraft_psd::act` parser
/// (the swatch import/export use it too). A trailer count of 0 is invalid — Photoshop reads
/// only the first `count` entries, `count >= 1` — and is treated as absent: all 256 entries.
fn act_table(bytes: &[u8]) -> Result<(Vec<[u8; 3]>, Option<u8>), String> {
    let f = photocraft_psd::act::parse(bytes).map_err(|e| e.to_string())?;
    let (colors, t) = match f.count {
        Some(0) => (f.colors.to_vec(), f.transparent),
        _ => (f.used().to_vec(), f.transparent_index().and_then(|i| u16::try_from(i).ok())),
    };
    Ok((colors, t.and_then(|t| u8::try_from(t).ok())))
}

/// A `.act` (768 bytes of RGB) or a palette PNG's table; the extension decides.
fn parse_palette_file(name: &str, bytes: &[u8]) -> Result<(Vec<[u8; 3]>, Option<u8>), String> {
    match Path::new(name).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("act") => Ok(act_table(bytes).map_err(|e| format!("{name}: {e}"))?),
        Some("png") => match photocraft_codecs::png_palette(bytes).map_err(|e| format!("{name}: {e}"))? {
            Some(p) => Ok((p.colors, p.transparent)),
            None => Err(format!("{name}: not a palette PNG (colour type 3)")),
        },
        _ => Err(format!("{name}: expected a .act or palette .png file")),
    }
}

/// Entries as `"#rrggbb"` strings (the dialog's table field).
pub fn hex_array(colors: &[[u8; 3]]) -> Vec<String> {
    colors.iter().map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])).collect()
}

/// A table field (an array of `"#rrggbb"`) back to RGB entries.
pub fn hex_array_to_rgb(v: &Value) -> Option<Vec<[u8; 3]>> {
    let a = v.as_array()?;
    a.iter()
        .map(|c| {
            c.as_str()
                .and_then(crate::color_picker_ui::parse_hex)
                .map(|[r, g, b]| [(r * 255.0).round() as u8, (g * 255.0).round() as u8, (b * 255.0).round() as u8])
        })
        .collect()
}

/// The palette entry matching `target` (0..=1 RGB): an exact match after rounding, else the
/// nearest by squared 8-bit distance (dithered edges). `None` for an empty table.
pub fn match_entry(colors: &[[u8; 3]], target: [f32; 3]) -> Option<usize> {
    if colors.is_empty() {
        return None;
    }
    let t = target.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    Some(colors.iter().position(|c| *c == t).unwrap_or_else(|| {
        let mut best = (u32::MAX, 0usize);
        for (i, c) in colors.iter().enumerate() {
            let dist = (0..3)
                .map(|k| {
                    let d = c[k] as i32 - t[k] as i32;
                    (d * d) as u32
                })
                .sum::<u32>();
            if dist < best.0 {
                best = (dist, i);
            }
        }
        best.1
    }))
}

/// The table grown to include index `i`: entries past the current length become black, as in
/// Photoshop (clicking empty slot N grows the table to N with `[len..N)` filled black). The
/// caller stores the result in its table field.
pub fn pad_to(colors: &[[u8; 3]], i: usize) -> Vec<[u8; 3]> {
    let mut v = colors.to_vec();
    v.resize(i.saturating_add(1), [0, 0, 0]);
    v
}

/// Linearly interpolates `colors[lo..=hi]` in place from `colors[lo]` to `colors[hi]` (Photoshop's
/// drag-range Color Table ramp: pick the first colour, pick the last colour, and every entry in
/// between steps smoothly from first to last).
pub fn lerp_range(colors: &mut [[u8; 3]], lo: usize, hi: usize) {
    let (lo, hi) = (lo.min(hi), lo.max(hi));
    let (Some(&c0), Some(&c1)) = (colors.get(lo), colors.get(hi)) else { return };
    if lo == hi {
        return;
    }
    let span = (hi - lo) as f32;
    for (offset, slot) in colors[lo..=hi].iter_mut().enumerate() {
        let t = offset as f32 / span;
        *slot = std::array::from_fn(|ch| (f32::from(c0[ch]) + (f32::from(c1[ch]) - f32::from(c0[ch])) * t).round().clamp(0.0, 255.0) as u8);
    }
}

const DRAG_START: &str = "__paletteDragStart";
const DRAG_END: &str = "__paletteDragEnd";

/// The fixed 16×16 (256-cell) palette grid shared by the Indexed Color (Custom…) and Color
/// Table dialogs. `colors` fills the first cells; entries beyond its length render as dashed
/// empties. The cell at `f["transparent"]` shows a checkerboard + X; the cell at `highlight`
/// (or the active drag / selected range) gets an accent ring. Left-clicking a single cell returns
/// `Some((i, i))`; pressing and dragging across cells returns `Some((lo, hi))` on mouse release;
/// right-clicking a filled cell offers Set/Clear Transparent, updating `f["transparent"]`.
pub fn palette_grid(ui: &mut egui::Ui, f: &mut Map<String, Value>, colors: &[[u8; 3]], highlight: Option<usize>) -> Option<(usize, usize)> {
    const CELL: f32 = 22.0; // Logical px (~45 physical at Retina 2×), as in Photoshop's ~20 px cells.
    let s = CELL / 7.0; // The grid's markers were drawn for 7 px cells; scale them with the size.
    let t = crate::theme::Tokens::get(ui.ctx());
    let transparent = f.get("transparent").and_then(Value::as_u64);
    let mut cells: Vec<(egui::Rect, bool)> = Vec::with_capacity(256);
    egui::Grid::new("palette-grid").min_col_width(CELL).min_row_height(CELL).spacing([1.0, 1.0]).show(ui, |ui| {
        for i in 0..256usize {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(CELL, CELL), egui::Sense::click_and_drag());
            if colors.get(i).is_some() {
                let parent_layer = resp.layer_id;
                resp.context_menu(|ui| {
                    // Above the dialog, as every other popup (#7113).
                    crate::press_menu::above_parent_ui(parent_layer, ui);
                    if ui.button(tl!("Set Transparent")).clicked() {
                        f.insert("transparent".into(), json!(i));
                    }
                    if ui.button(tl!("Clear Transparency")).clicked() {
                        f.insert("transparent".into(), Value::Null);
                    }
                });
            }
            cells.push((r, resp.clicked()));
            if (i + 1) % 16 == 0 {
                ui.end_row();
            }
        }
    });
    let can_interact = ui.is_enabled() && !egui::Popup::is_any_open(ui.ctx());
    let (pressed, down, pos) =
        ui.input(|inp| (inp.pointer.primary_pressed(), inp.pointer.primary_down(), inp.pointer.latest_pos().or_else(|| inp.pointer.interact_pos())));
    let hovered = pos.and_then(|p| cells.iter().position(|(r, _)| r.expand(0.6).contains(p)));
    if can_interact
        && pressed
        && let Some(i) = hovered
    {
        f.insert(DRAG_START.into(), json!(i));
        f.insert(DRAG_END.into(), json!(i));
    }
    if can_interact
        && down
        && f.contains_key(DRAG_START)
        && let Some(i) = hovered
    {
        f.insert(DRAG_END.into(), json!(i));
    }
    let drag_range =
        f.get(DRAG_START).and_then(Value::as_u64).zip(f.get(DRAG_END).and_then(Value::as_u64)).map(|(a, b)| (a.min(b) as usize, a.max(b) as usize));
    if down && drag_range.is_some() {
        ui.ctx().request_repaint();
    }
    let stored_range = f.get(crate::color_table_ui::PALETTE_RANGE).and_then(Value::as_array).and_then(|a| {
        let (lo, hi) = (a.first()?.as_u64()? as usize, a.get(1)?.as_u64()? as usize);
        Some((lo.min(hi), lo.max(hi)))
    });
    let active_range = drag_range.or(stored_range).or_else(|| highlight.map(|i| (i, i)));
    for (i, &(r, _)) in cells.iter().enumerate() {
        match colors.get(i) {
            Some(c) => {
                ui.painter().rect_filled(r, 1.0, egui::Color32::from_rgb(c[0], c[1], c[2]));
                ui.painter().rect_stroke(r, 1.0, egui::Stroke::new(0.5, t.field_border), egui::StrokeKind::Inside);
                if transparent == Some(i as u64) {
                    // Checkerboard and an X over the transparent entry.
                    crate::widgets::checker(ui.painter(), r, 3.0 * s);
                    let m = egui::vec2(2.5 * s, 2.5 * s);
                    ui.painter().line_segment([r.center() - m, r.center() + m], egui::Stroke::new(1.0, t.text));
                    ui.painter().line_segment([r.center() + egui::vec2(m.y, -m.x), r.center() - egui::vec2(m.y, -m.x)], egui::Stroke::new(1.0, t.text));
                }
            }
            None => dashed_rect(ui.painter(), r, t.field_border, s), // Empty: a hand-drawn dashed border.
        }
        if active_range.is_some_and(|(lo, hi)| (lo..=hi).contains(&i)) {
            ui.painter().rect_stroke(r.shrink(0.5), 1.0, egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
        }
    }
    if !down && let Some((lo, hi)) = drag_range {
        f.remove(DRAG_START);
        f.remove(DRAG_END);
        Some((lo, hi))
    } else if !down {
        f.remove(DRAG_START);
        f.remove(DRAG_END);
        cells.iter().position(|(_, clicked)| *clicked).map(|i| (i, i))
    } else {
        None
    }
}

/// A dashed rectangle border (~2 px dashes, 1 px gaps at the original 7 px cell size): egui
/// has no dash stroke. `s` scales the dashes with the cell size.
fn dashed_rect(painter: &egui::Painter, r: egui::Rect, color: egui::Color32, s: f32) {
    let dash = 2.0 * s;
    let gap = 1.0 * s;
    for (y, x0, x1) in [(r.top(), r.left(), r.right()), (r.bottom(), r.left(), r.right())] {
        let mut x = x0;
        while x < x1 {
            painter.line_segment([egui::pos2(x, y), egui::pos2((x + dash).min(x1), y)], egui::Stroke::new(1.0, color));
            x += dash + gap;
        }
    }
    for (x, y0, y1) in [(r.left(), r.top(), r.bottom()), (r.right(), r.top(), r.bottom())] {
        let mut y = y0;
        while y < y1 {
            painter.line_segment([egui::pos2(x, y), egui::pos2(x, (y + dash).min(y1))], egui::Stroke::new(1.0, color));
            y += dash + gap;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let colors = [[0, 0, 0], [255, 128, 7]];
        let hex = hex_array(&colors);
        assert_eq!(hex, vec!["#000000", "#ff8007"]);
        assert_eq!(hex_array_to_rgb(&json!(hex)).unwrap(), colors);
    }

    #[test]
    fn parse_rejects_wrong_extensions_and_shapes() {
        assert!(parse_palette_file("x.act", &[0u8; 10]).is_err());
        assert!(parse_palette_file("x.bin", &[0u8; 768]).is_err());
        // A non-palette PNG (no PLTE) is rejected, not read as a table.
        let png = b"\x89PNG\r\n\x1a\n";
        assert!(parse_palette_file("x.png", png).is_err());
    }

    #[test]
    fn parse_act_and_palette_png() {
        let bytes = photocraft_psd::act::write(&[[1, 2, 3], [4, 5, 6]], None, None).unwrap();
        let (colors, transparent) = parse_palette_file("x.act", &bytes).unwrap();
        assert_eq!(colors.len(), 256);
        assert_eq!((colors[0], colors[1]), ([1, 2, 3], [4, 5, 6]));
        assert_eq!(transparent, None);
    }

    #[test]
    fn parse_act_trailer_sets_transparent() {
        let colors: Vec<[u8; 3]> = (0..256).map(|i| [i as u8, 0, 255]).collect();
        let bytes = photocraft_psd::act::write(&colors, Some(256), Some(9)).unwrap();
        let (colors, transparent) = parse_palette_file("x.act", &bytes).unwrap();
        assert_eq!(colors.len(), 256);
        assert_eq!(transparent, Some(9));
    }

    #[test]
    fn parse_act_reads_only_the_counted_entries() {
        // A 16-colour .act (trailer count 16) loads as a 16-entry table, as in Photoshop.
        let mut bytes = vec![0u8; photocraft_psd::act::PALETTE_LEN];
        for i in 0..16 {
            bytes[i * 3] = (i + 1) as u8;
        }
        bytes.extend_from_slice(&[0, 16, 0, 2]); // count=16, tindex=2
        let (colors, transparent) = parse_palette_file("x.act", &bytes).unwrap();
        assert_eq!(colors.len(), 16);
        assert_eq!((colors[0], colors[15]), ([1, 0, 0], [16, 0, 0]));
        assert_eq!(transparent, Some(2));
    }

    #[test]
    fn parse_act_invalid_count_trailer_falls_back_to_all_entries() {
        // A trailer count of 0 is invalid (Photoshop reads the first `count` entries,
        // count >= 1): it is treated as absent — all 256 entries, index still honoured.
        // (A count past 256 clamps to the full table in `photocraft_psd::act` itself.)
        let mut bytes = vec![0u8; photocraft_psd::act::PALETTE_LEN];
        bytes[3] = 7; // entry 1 is [0, 7, 0]
        bytes.extend_from_slice(&[0, 0, 0, 1]); // count=0 (invalid), tindex=1
        let (colors, transparent) = parse_palette_file("x.act", &bytes).unwrap();
        assert_eq!(colors.len(), 256);
        assert_eq!(transparent, Some(1));
    }

    #[test]
    fn loading_act_without_then_with_transparent_updates_both_dialogs() {
        let no_trans = photocraft_psd::act::write(&[[0, 0, 0], [255, 0, 0]], None, None).unwrap();
        let with_trans = photocraft_psd::act::write(&[[0, 0, 0], [255, 0, 0]], Some(2), Some(0)).unwrap();
        for cmd in ["image.mode.indexedColor", "image.mode.colorTable"] {
            let mut f = Map::new();
            f.insert("__command".into(), json!(cmd));
            if cmd == "image.mode.colorTable" {
                f.insert("table".into(), json!("blackBody"));
            }
            // 1st load: ACT without transparent colour.
            let (c1, t1) = parse_palette_file("no_trans.act", &no_trans).unwrap();
            apply_loaded_palette(&mut f, &c1, t1);
            assert_eq!(f.get("transparent"), Some(&Value::Null), "{cmd}: 1st load has no transparent entry");
            if cmd == "image.mode.colorTable" {
                assert_eq!(f.get("table").and_then(Value::as_str), Some("custom"));
            }
            // 2nd load (without closing the dialog): ACT with transparent colour at index 0.
            let (c2, t2) = parse_palette_file("with_trans.act", &with_trans).unwrap();
            apply_loaded_palette(&mut f, &c2, t2);
            assert_eq!(f.get("transparent").and_then(Value::as_u64), Some(0), "{cmd}: 2nd load activates transparent entry 0");
            // 3rd load: ACT without transparent colour clears it again.
            apply_loaded_palette(&mut f, &c1, t1);
            assert_eq!(f.get("transparent"), Some(&Value::Null), "{cmd}: 3rd load clears transparent entry");
        }
    }

    #[test]
    fn colors_field_per_command() {
        assert_eq!(colors_field("image.mode.colorTable"), COLORS_FIELD);
        assert_eq!(colors_field("image.mode.indexedColor"), CUSTOM_FIELD);
    }

    #[test]
    fn match_entry_exact_then_nearest() {
        let colors = [[255, 0, 0], [0, 128, 0], [10, 20, 30]];
        // Exact match (after rounding).
        assert_eq!(match_entry(&colors, [1.0, 0.0, 0.0]), Some(0));
        assert_eq!(match_entry(&colors, [0.0, 128.0 / 255.0, 0.0]), Some(1));
        // Nearest by squared distance for a colour not on the table.
        assert_eq!(match_entry(&colors, [0.05, 0.1, 0.12]), Some(2));
        assert_eq!(match_entry(&colors, [0.9, 0.1, 0.1]), Some(0), "closer to red than the dark entry");
        // Out-of-range targets clamp, not panic.
        assert_eq!(match_entry(&colors, [5.0, -3.0, 0.0]), Some(0));
        // An empty table matches nothing.
        assert_eq!(match_entry(&[], [0.5, 0.5, 0.5]), None);
    }

    #[test]
    fn lerp_range_interpolates_from_first_to_last_color() {
        let mut colors = vec![[0u8, 0, 0]; 60];
        colors[10] = [0, 255, 0]; // #00ff00
        colors[50] = [255, 0, 255]; // #ff00ff
        lerp_range(&mut colors, 10, 50);
        assert_eq!(colors[9], [0, 0, 0], "entries before the range are untouched");
        assert_eq!(colors[10], [0, 255, 0], "first entry is #00ff00");
        assert_eq!(colors[30], [128, 128, 128], "midpoint of 10..=50 is #808080");
        assert_eq!(colors[50], [255, 0, 255], "last entry is #ff00ff");
        assert_eq!(colors[51], [0, 0, 0], "entries after the range are untouched");
        // Monotonic step across the whole range 10..=50.
        for k in 10..50 {
            assert!(colors[k][0] <= colors[k + 1][0] && colors[k][2] <= colors[k + 1][2]);
            assert!(colors[k][1] >= colors[k + 1][1]);
        }
    }

    #[test]
    fn mouse_drag_across_palette_grid_selects_range_on_release() {
        let colors = vec![[0u8, 0, 0]; 64];
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(500.0, 500.0)).build_ui_state(
            move |ui, state: &mut (Map<String, Value>, Option<(usize, usize)>, egui::Pos2)| {
                state.2 = ui.cursor().min;
                if let Some(sel) = palette_grid(ui, &mut state.0, &colors, None) {
                    state.1 = Some(sel);
                }
            },
            (Map::new(), None, egui::Pos2::ZERO),
        );
        h.run_steps(2);
        let origin = h.state().2;
        // Cell (col, row) center in a 16×16 grid of 22×22 cells with 1 px spacing.
        let cell_center = |idx: usize| {
            let (col, row) = ((idx % 16) as f32, (idx / 16) as f32);
            origin + egui::vec2(col * 23.0 + 11.0, row * 23.0 + 11.0)
        };
        let p10 = cell_center(10);
        let p50 = cell_center(50);
        // Move to cell 10 and press.
        h.event(egui::Event::PointerMoved(p10));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: p10, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.step();
        assert_eq!(h.state().1, None, "still holding the mouse button: {:?}", h.state().0);
        // Drag to cell 50.
        h.event(egui::Event::PointerMoved(p50));
        h.step();
        assert_eq!(h.state().1, None, "still holding at cell 50: {:?}", h.state().0);
        // Release at cell 50 -> emits Some((10, 50)).
        h.event(egui::Event::PointerButton { pos: p50, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.step();
        assert_eq!(h.state().1, Some((10, 50)));
    }
}
