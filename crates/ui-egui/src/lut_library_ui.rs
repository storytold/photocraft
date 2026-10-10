//! Color Lookup's LUT browser: one list that replaces the old "3D LUT File" dropdown. It holds
//! "None", Favorites, Recent, the built-in looks and every installed pack
//! (`photocraft_engine::lut_library`) as collapsible sections, with a nested section for each
//! folder inside a pack. Rows show the file name only (long names end in an ellipsis; the tooltip has
//! the full path and the file's details) and a star.
//!
//! - The list fills the Properties group (it grows and shrinks with the window).
//! - Up / Down move a cursor through the LUTs (and scroll to it) while the list or the filter box
//!   has focus; Enter applies the cursor's LUT, Esc drops it. Hovering a row for a moment does
//!   the same as the cursor. Both only *preview* on the canvas (no history); a click or Enter
//!   applies the LUT through Color Lookup's `file` / `lut` params, so the document embeds the table
//!   and never depends on the library afterwards.
//! - A folder or `.zip` dropped on the window while this list is showing installs as a pack (drops
//!   at any other time open as usual); files added by hand show up within a couple of seconds.
//!
//! The listing and each pack's folder tree are cached in egui memory by library revision. Only open
//! sections produce rows, and the list is virtualised, so a pack of thousands of LUTs costs nothing
//! until it is opened and little after.

mod draw;
mod install;
mod rows;
mod scans;

use self::draw::{chevron, star};
use self::install::install_dialog;
pub use self::install::{end_stale_preview, install_path, is_pack_drop, report};

use std::collections::HashMap;
use std::path::Path;

use egui::{Align2, EventFilter, FontId, Id, Key, Modifiers, Pos2, Rect, Sense, Ui, WidgetInfo, WidgetType, vec2};
use photocraft_doc::LayerId;
use photocraft_engine::lut_library::LutLibrary;
use serde_json::{Value, json};

use self::rows::{Row, Source};
use self::scans::Scans;
use crate::PhotocraftApp;
use crate::theme::Tokens;
use crate::widgets;

const ROW_H: f32 = 28.0;
const INDENT: f32 = 14.0;
const MIN_LIST_H: f32 = 112.0;
/// What the editor needs below the list (path row, Trilinear, Dither, visibility, Reset) until
/// [`finish`] has measured it, and the breathing room kept under the editor.
const BELOW_LIST_GUESS: f32 = 150.0;
const BOTTOM_MARGIN: f32 = 8.0;
const STAR_ZONE: f32 = 26.0;
const REMOVE_ZONE: f32 = 92.0;
const PREVIEW_AFTER: f64 = 0.15;

/// What the browser chose.
pub enum Pick {
    /// A built-in look (or "none"), by id.
    Look(String),
    /// An installed LUT, by absolute path.
    File(String),
}

impl Pick {
    fn params(&self) -> Value {
        match self {
            Pick::Look(id) => json!({"lut": id}),
            Pick::File(path) => json!({"file": path}),
        }
    }
}

/// The layer's current LUT, as the Color Lookup editor knows it.
pub struct Current<'a> {
    /// `"none"`, a built-in look id, or `"custom"` (a LUT loaded from a file).
    pub look: &'a str,
    /// The loaded table's name.
    pub name: &'a str,
}

fn memory<T: Clone + Send + Sync + 'static>(ui: &Ui, id: Id) -> Option<T> {
    ui.data(|d| d.get_temp::<T>(id))
}

fn remember<T: Clone + Send + Sync + 'static>(ui: &Ui, id: Id, value: T) {
    ui.data_mut(|d| d.insert_temp(id, value));
}

/// `Pack/path.cube` of an installed LUT's absolute path.
fn id_of(lib: &LutLibrary, path: &str) -> Option<String> {
    Path::new(path).strip_prefix(lib.root()).ok().map(|p| p.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"))
}

fn pick_of(row: &Row, src: &Source, lib: Option<&LutLibrary>) -> Option<Pick> {
    match row {
        Row::None => Some(Pick::Look("none".into())),
        Row::Look(i) => src.builtins.get(*i).map(|(id, _)| Pick::Look((*id).into())),
        Row::Lut { pack, idx, .. } => {
            let (pack, lut) = (src.packs.get(*pack)?, src.packs.get(*pack)?.info.luts.get(*idx)?);
            Some(Pick::File(lib?.path_of(&pack.info.name, &lut.file).to_string_lossy().into_owned()))
        }
        _ => None,
    }
}

fn dup_label(of: &str) -> String {
    format!("{} {of}", tl!("Duplicate of"))
}

/// Width the "Duplicate of …" label of the pack `name` needs on the right of its header.
fn dup_zone(ctx: &egui::Context, src: &Source, name: &str) -> f32 {
    let Some(of) = src.redundant.get(name) else { return REMOVE_ZONE };
    let job = egui::text::LayoutJob::simple_singleline(dup_label(of), FontId::proportional(12.0), egui::Color32::WHITE);
    ctx.fonts_mut(|f| f.layout_job(job)).size().x + 16.0
}

/// Call after the rest of the Color Lookup editor is drawn: measures how tall everything below the
/// list is, so the list can use all the room that is left.
pub fn finish(ui: &Ui, layer: LayerId) {
    let Some(bottom) = memory::<f32>(ui, Id::new(("lutlib-list-bottom", layer.0))) else { return };
    let below = ui.min_rect().bottom() - bottom;
    if below > 0.0 && below < 600.0 {
        remember(ui, Id::new(("lutlib-below", layer.0)), below);
    }
}

/// The browser. `layer` keys the per-layer state; `builtins` are `(id, label)`.
pub fn browser(app: &mut PhotocraftApp, ui: &mut Ui, layer: LayerId, cur: &Current, builtins: &[(&str, &str)]) -> Option<Pick> {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let scans = Scans::get(&ctx);
    let has_library = app.session.lut_library.is_some();
    install::mark_shown(&ctx);

    // Files added or removed by hand show up without a restart. The check lists the whole library,
    // so a worker thread does it; this thread only starts it and reads the answer.
    if let (Some(l), Some(root)) = (rows::listing(app, ui), app.session.lut_library.as_ref().map(|lib| lib.root().to_path_buf())) {
        if scans.take_stale(l.rev) {
            let _ = app.run("lut.rescan", json!({}));
        }
        let (now, scan_id) = (ui.input(|i| i.time), Id::new("lutlib-scan"));
        if now - memory::<f64>(ui, scan_id).unwrap_or(0.0) > 2.0 {
            remember(ui, scan_id, now);
            scans.start_stale_check(&ctx, root, l.rev, l.packs.clone());
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(2));
    }
    let listing = rows::listing(app, ui);

    let open_id = Id::new("lutlib-open");
    let mut open: HashMap<String, bool> = memory(ui, open_id).unwrap_or_default();
    let filter_id = Id::new("lutlib-filter");
    let mut filter: String = memory(ui, filter_id).unwrap_or_default();
    let loaded_id = Id::new(("lutlib-loaded", layer.0));
    let (mut loaded, mut loaded_name): (String, String) = memory(ui, loaded_id).unwrap_or_default();
    if cur.look != "custom" {
        loaded.clear();
    } else if !loaded.is_empty() && loaded_name.is_empty() {
        loaded_name = cur.name.to_string();
    } else if loaded_name != cur.name {
        loaded.clear();
    }
    let confirm_id = Id::new("lutlib-confirm");
    let mut confirm: String = memory(ui, confirm_id).unwrap_or_default();
    let cursor_id = Id::new(("lutlib-cursor", layer.0));
    // The keyboard cursor: the LUT's key and the row it is on (a LUT can be listed twice).
    let (mut cursor, mut cursor_row): (String, usize) = memory(ui, cursor_id).unwrap_or_default();

    // Filter and Install Pack… share one row.
    let mut install_clicked = false;
    let filter_resp = ui
        .horizontal(|ui| {
            let button_w = if has_library { 116.0 } else { 0.0 };
            let r = ui.add(egui::TextEdit::singleline(&mut filter).hint_text(tl!("Filter")).desired_width((ui.available_width() - button_w - 8.0).max(60.0)));
            if has_library {
                install_clicked = widgets::secondary_button(ui, tl!("Install Pack…"), 108.0).clicked();
            }
            r
        })
        .inner;

    let lib = app.session.lut_library.as_ref();
    let redundant = lib.map(|l| scans.redundant(&ctx, l.root(), l.rev)).unwrap_or_default();
    let (favorites, recent): (Vec<String>, Vec<String>) = lib.map(|l| (l.favorites().to_vec(), l.recent().to_vec())).unwrap_or_default();
    let empty = (std::sync::Arc::new(Vec::new()), std::sync::Arc::new(HashMap::new()));
    let (packs, ids) = listing.as_ref().map_or((&empty.0, &empty.1), |l| (&l.packs, &l.ids));
    let src = Source {
        packs: packs.as_slice(),
        ids,
        builtins,
        custom_shown: cur.look == "custom" && loaded.is_empty(),
        needle: filter.trim().to_lowercase(),
        open: &open,
        favorites: &favorites,
        recent: &recent,
        redundant: &redundant,
    };
    let rows = src.rows();
    let applied_key: Option<String> = match cur.look {
        "none" => Some("none".into()),
        "custom" => lib.and_then(|l| id_of(l, &loaded)).map(|id| format!("lut:{id}")),
        look => Some(format!("look:{look}")),
    };
    let (list_id, scroll_id, state_id) = (Id::new(("lutlib-list", layer.0)), Id::new(("lutlib-scroll", layer.0)), Id::new(("lutlib-scroll-state", layer.0)));

    // Arrow keys move the cursor while the list or the filter box has focus.
    let focused = ui.memory(|m| m.has_focus(list_id)) || filter_resp.has_focus();
    let mut commit: Option<Pick> = None;
    let mut scroll_to: Option<usize> = None;
    if focused {
        let (down, up, enter, esc) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if esc {
            cursor.clear();
        }
        if down || up {
            let selectable: Vec<(usize, String)> = rows.iter().enumerate().filter_map(|(i, r)| src.key_of(r).map(|k| (i, k))).collect();
            // From the applied LUT, start at its row in the pack (the last one) rather than in Recent.
            let base = if cursor.is_empty() {
                selectable.iter().rposition(|(_, k)| Some(k) == applied_key.as_ref())
            } else {
                selectable.iter().position(|(i, k)| *i == cursor_row && *k == cursor).or_else(|| selectable.iter().position(|(_, k)| *k == cursor))
            };
            let next = match (base, down) {
                (Some(p), true) => p + 1,
                (Some(p), false) => p.saturating_sub(1),
                (None, true) => 0,
                (None, false) => selectable.len().saturating_sub(1),
            };
            if let Some((i, k)) = selectable.get(next.min(selectable.len().saturating_sub(1))) {
                cursor = k.clone();
                cursor_row = *i;
                scroll_to = Some(*i);
            }
        }
        if enter && !cursor.is_empty() {
            commit = rows.get(cursor_row).and_then(|r| pick_of(r, &src, lib));
        }
    }

    // The list fills what the Properties group has left: it follows the window, not a fixed size.
    // The height of what the editor draws below the list is measured by `finish` (the frame
    // before), so nothing here depends on a guessed layout.
    let below: f32 = memory(ui, Id::new(("lutlib-below", layer.0))).unwrap_or(BELOW_LIST_GUESS);
    let list_h = (ui.clip_rect().bottom() - ui.cursor().min.y - below - BOTTOM_MARGIN).max(MIN_LIST_H);
    let (saved_offset, saved_h): (f32, f32) = memory(ui, state_id).unwrap_or((0.0, list_h));
    let well = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), list_h));
    ui.painter().rect(well, t.radius_sm, t.field, egui::Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    let mut area = egui::ScrollArea::vertical().id_salt(scroll_id).max_height(list_h).min_scrolled_height(list_h).auto_shrink([false, false]);
    if let Some(i) = scroll_to {
        let top = i as f32 * ROW_H;
        let offset = if top < saved_offset {
            top
        } else if top + ROW_H > saved_offset + saved_h {
            top + ROW_H - saved_h
        } else {
            saved_offset
        };
        area = area.vertical_scroll_offset(offset.max(0.0));
    }
    let mut toggled: Option<(String, bool)> = None;
    let (mut remove, mut starred, mut hovered): (Option<String>, Option<String>, Option<usize>) = (None, None, None);
    let mut clicked_row = false;
    let out = ui
        .scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            area.show_rows(ui, ROW_H, rows.len(), |ui, range| {
                for i in range {
                    let Some(row) = rows.get(i) else { continue };
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                    let key = src.key_of(row);
                    let (applied, at_cursor) =
                        (key.is_some() && key == applied_key, i == cursor_row && key.as_deref() == Some(cursor.as_str()) && !cursor.is_empty());
                    if applied {
                        ui.painter().rect_filled(rect, t.radius_sm, t.row_selected);
                    } else if at_cursor {
                        ui.painter().rect_filled(rect, t.radius_sm, t.accent_soft);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, t.radius_sm, t.hover);
                    }
                    let (text, depth) = match row {
                        Row::None => (tl!("None").to_string(), 0),
                        Row::Custom => (cur.name.to_string(), 0),
                        Row::Look(k) => (tl!(builtins.get(*k).map_or("", |b| b.1)).to_string(), 1),
                        Row::Lut { pack, idx, depth } => {
                            (src.packs.get(*pack).and_then(|p| p.info.luts.get(*idx)).map(|l| l.name.clone()).unwrap_or_default(), *depth)
                        }
                        Row::Header { title, depth, .. } => (title.clone(), *depth),
                    };
                    let mut x = rect.left() + 8.0 + f32::from(depth) * INDENT;
                    let mid = rect.center().y;
                    if let Row::Header { open, .. } = row {
                        chevron(ui, Pos2::new(x + 4.0, mid), *open, t.text_dim);
                        x += 14.0;
                    }
                    let is_header = matches!(row, Row::Header { .. });
                    // Leave room on the right for the star or the pack's label; long names end in "…".
                    let reserve = match row {
                        Row::Lut { .. } => STAR_ZONE + 6.0,
                        Row::Header { pack: Some(name), .. } if src.redundant.contains_key(name) => dup_zone(&ctx, &src, name),
                        Row::Header { pack: Some(_), .. } => REMOVE_ZONE + 8.0,
                        _ => 8.0,
                    };
                    let mut job =
                        egui::text::LayoutJob::simple_singleline(text.clone(), FontId::proportional(13.0), if is_header { t.text_dim } else { t.text });
                    job.wrap = egui::text::TextWrapping {
                        max_width: (rect.right() - reserve - x).max(24.0),
                        max_rows: 1,
                        break_anywhere: true,
                        overflow_character: Some('…'),
                    };
                    let galley = ctx.fonts_mut(|f| f.layout_job(job));
                    ui.painter().galley(Pos2::new(x, mid - galley.size().y / 2.0), galley, t.text);
                    let info = if is_header {
                        WidgetInfo::labeled(WidgetType::CollapsingHeader, true, &text)
                    } else {
                        WidgetInfo::selected(WidgetType::SelectableLabel, true, applied, &text)
                    };
                    resp.widget_info(move || info.clone());

                    let mut in_star = false;
                    let mut in_remove = false;
                    match row {
                        Row::Lut { pack, idx, .. } => {
                            let id = src.lut_id(*pack, *idx).unwrap_or_default();
                            let path = lib
                                .map(|l| {
                                    l.path_of(
                                        src.packs.get(*pack).map_or("", |p| &p.info.name),
                                        src.packs.get(*pack).and_then(|p| p.info.luts.get(*idx)).map_or("", |l| &l.file),
                                    )
                                    .to_string_lossy()
                                    .into_owned()
                                })
                                .unwrap_or_default();
                            let fav = favorites.contains(&id);
                            if fav || resp.hovered() {
                                star(ui, Pos2::new(rect.right() - STAR_ZONE / 2.0 - 2.0, mid), fav, if fav { t.accent_text } else { t.text_faint });
                            }
                            in_star = resp.interact_pointer_pos().is_some_and(|p| p.x > rect.right() - STAR_ZONE);
                            let tip_id = id.clone();
                            resp.clone().on_hover_ui(|ui| {
                                ui.label(tip_id);
                                if let Some(h) = scans.header(&path) {
                                    let kind = match h.kind {
                                        photocraft_engine::lut_library::meta::Kind::Cube3d => tl!("3D cube"),
                                        photocraft_engine::lut_library::meta::Kind::Cube1d => tl!("1D cube"),
                                        photocraft_engine::lut_library::meta::Kind::ThreeDl => "3DL",
                                        photocraft_engine::lut_library::meta::Kind::Look => tl!("LUT look"),
                                    };
                                    ui.label(if h.size > 0 {
                                        crate::i18n::fmt(tl!("{kind}, size {n}"), &[("kind", kind), ("n", &h.size.to_string())])
                                    } else {
                                        kind.to_string()
                                    });
                                    if !h.title.is_empty() {
                                        ui.label(crate::i18n::fmt(tl!("Title: {title}"), &[("title", &h.title)]));
                                    }
                                    if h.custom_domain {
                                        ui.label(tl!("Declares an input range other than 0 to 1"));
                                    }
                                }
                                if photocraft_engine::lut_library::meta::looks_like_conversion(&text) {
                                    ui.label(tl!("The name suggests a technical conversion, not a creative look"));
                                }
                            });
                        }
                        Row::Header { pack: Some(name), .. } => {
                            let dup = src.redundant.get(name);
                            let label = if confirm == *name {
                                Some((tl!("Delete").to_string(), t.danger))
                            } else if resp.hovered() {
                                Some((tl!("Remove Pack").to_string(), t.text_faint))
                            } else {
                                dup.map(|of| (dup_label(of), t.accent_text))
                            };
                            if let Some((label, c)) = label {
                                ui.painter().text(Pos2::new(rect.right() - 8.0, mid), Align2::RIGHT_CENTER, label, FontId::proportional(12.0), c);
                            }
                            let zone = if dup.is_some() { dup_zone(&ctx, &src, name) } else { REMOVE_ZONE };
                            in_remove = resp.interact_pointer_pos().is_some_and(|p| p.x > rect.right() - zone);
                        }
                        _ => {}
                    }
                    if resp.hovered() && key.is_some() {
                        hovered = Some(i);
                        ctx.request_repaint_after(std::time::Duration::from_millis(160));
                    }
                    if resp.clicked() {
                        clicked_row = true;
                        match row {
                            Row::Lut { pack, idx, .. } if in_star => starred = src.lut_id(*pack, *idx),
                            Row::Header { pack: Some(name), .. } if in_remove => {
                                if confirm == *name {
                                    remove = Some(name.clone());
                                } else {
                                    confirm = name.clone();
                                }
                            }
                            Row::Header { key, open, .. } => toggled = Some((key.clone(), !open)),
                            _ => commit = commit.take().or_else(|| pick_of(row, &src, lib)),
                        }
                    }
                }
            })
        })
        .inner;
    remember(ui, state_id, (out.state.offset.y, out.inner_rect.height()));
    remember(ui, Id::new(("lutlib-list-bottom", layer.0)), out.inner_rect.bottom());
    // The list takes keyboard focus when clicked; Up / Down must not move focus away from it.
    let list_resp = ui.interact(out.inner_rect, list_id, Sense::focusable_noninteractive());
    if clicked_row {
        list_resp.request_focus();
    }
    if ui.memory(|m| m.has_focus(list_id)) {
        ui.memory_mut(|m| m.set_focus_lock_filter(list_id, EventFilter { vertical_arrows: true, ..EventFilter::default() }));
    }

    // Preview what the cursor or a resting pointer is on, without touching history.
    let rested = ui.input(|i| i.pointer.time_since_last_movement()) as f64 > PREVIEW_AFTER;
    let target = if !cursor.is_empty() {
        rows.get(cursor_row)
    } else if rested {
        hovered.and_then(|i| rows.get(i))
    } else {
        None
    };
    let preview = if commit.is_none() { target.filter(|r| src.key_of(r) != applied_key).and_then(|r| pick_of(r, &src, lib)) } else { None };
    let frame = ctx.cumulative_frame_nr();
    let previewing: (u64, u64) = ui.data(|d| d.get_temp(Id::new("lutlib-previewing"))).unwrap_or_default();
    if let Some(p) = &preview {
        app.live_adjust = Some((layer, p.params()));
        ctx.data_mut(|d| d.insert_temp(Id::new("lutlib-previewing"), (layer.0, frame)));
    } else if previewing.0 == layer.0 {
        if app.live_adjust.as_ref().is_some_and(|(l, _)| *l == layer) {
            app.live_adjust = None;
        }
        ctx.data_mut(|d| d.insert_temp(Id::new("lutlib-previewing"), (0u64, 0u64)));
    }
    if previewing.0 == layer.0 && preview.is_some() {
        ctx.data_mut(|d| d.insert_temp(Id::new("lutlib-previewing"), (layer.0, frame)));
    }

    if let Some((key, state)) = toggled {
        open.insert(key, state);
    }
    if install_clicked {
        install_dialog(app);
    }
    if let Some(name) = remove {
        confirm.clear();
        let _ = app.run("lut.removePack", json!({"name": name}));
    }
    if let Some(id) = starred {
        let _ = app.run("lut.favorite", json!({"id": id}));
    }
    if let Some(Pick::File(path)) = &commit {
        loaded = path.clone();
        loaded_name.clear();
        if let Some(id) = app.session.lut_library.as_ref().and_then(|l| id_of(l, path)) {
            let _ = app.run("lut.used", json!({"id": id}));
        }
    }
    if commit.is_some() {
        cursor.clear();
    }
    remember(ui, open_id, open);
    remember(ui, filter_id, filter);
    remember(ui, confirm_id, confirm);
    remember(ui, cursor_id, (cursor, cursor_row));
    remember(ui, loaded_id, (loaded, loaded_name));
    commit
}

#[cfg(test)]
#[path = "lut_library_ui_tests.rs"]
mod tests;
