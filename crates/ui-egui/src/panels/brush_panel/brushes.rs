//! The Brushes panel (`brushes` registry body) on the framework's `BrushesPanel`, and the rows
//! and the event mapping the Brush Preset picker shares: rows in the engine's panel order, and one
//! mapping from preset events to `brush.presets.*` and `tools.setBrush`.

use std::hash::Hash;
use std::sync::Arc;

use craft::behavior::{EditEvent, EditEvents, GesturePhase};
use craft::draw::ui::brush_presets::{BrushCardParts, BrushPick, BrushRowPaint, BrushesPanel, brush_row};
use craft::keymap::Mods;
use craft::ui::contract::{Bind, MemoProps, MemoView, Outbox, PlainData, ReorderMove, Tracked, bind_tracked};
use craft::ui::patterns::presets::{PresetEvent, PresetFns, PresetRow, PresetSource, PresetState, PresetView};
use craft::ui::primitives::ContentPainterCx;
use craft::ui::primitives::textures::TextureCache;
use craft::ui::runtime::env::Env;
use egui::{ColorImage, Pos2, Rect, pos2};
use photocraft_engine::BrushSettings;
use photocraft_engine::brush_key_cmds::panel_layout;
use photocraft_engine::brush_preset_cmds::MAX_FOLDER_DEPTH;
use photocraft_engine::paint::{BrushPreset, MAX_BRUSH_SIZE, TipShape};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{UNGROUPED, commit_keyed, run_or_status};
use crate::PhotocraftApp;
use crate::brush_preview;
use crate::theme::Tokens;

/// What a row of the brush lists is: a preset, or a group (`folder` empty) or a folder of a group.
/// One type serves all three because their names can be equal. `group` is the group's name as the
/// engine stores it on its presets (empty for the ungrouped).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, MemoProps, PlainData)]
pub(crate) enum BrushKey {
    /// A preset, by name.
    Preset(String),
    /// A group or a folder inside one.
    Folder {
        /// The group.
        group: String,
        /// The folder path inside it; empty for the group itself.
        folder: Vec<String>,
    },
}

fn group_label(p: &BrushPreset) -> &str {
    if p.group.is_empty() { UNGROUPED } else { &p.group }
}

/// The folder path of `p`, cut at the depth the engine keeps.
fn path_of(p: &BrushPreset) -> &[String] {
    &p.folder[..p.folder.len().min(MAX_FOLDER_DEPTH)]
}

/// The group's name as its first preset stores it: presets saved without a group and presets
/// grouped under [`UNGROUPED`] share one group.
fn group_raw<'a>(presets: &'a [BrushPreset], p: &BrushPreset) -> &'a str {
    let label = group_label(p);
    presets.iter().find(|q| group_label(q) == label).map_or("", |q| q.group.as_str())
}

/// The group or folder that holds the preset `name`.
fn parent_of(presets: &[BrushPreset], name: &str) -> Option<BrushKey> {
    let p = presets.iter().find(|p| p.name == name)?;
    Some(BrushKey::Folder { group: group_raw(presets, p).to_string(), folder: path_of(p).to_vec() })
}

/// The parts the Brushes panel's rows show: a list shows tip, size, stroke and name; a grid the tips
/// with their sizes.
pub(crate) fn parts_of(view: PresetView) -> BrushCardParts {
    match view {
        PresetView::List => BrushCardParts::default(),
        _ => {
            let mut parts = BrushCardParts::default();
            parts.name = false;
            parts.stroke = false;
            parts.tip = true;
            parts
        }
    }
}

fn picture(cx: &ContentPainterCx, rect: Rect, key: impl Hash + 'static, image: impl FnOnce() -> ColorImage) {
    let texture = TextureCache::texture(cx.painter.ctx(), key, image);
    cx.picture(texture, rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), 0.0);
}

/// How one preset paints its tip and its stroke; the framework texture store caches both by signature. Both
/// signatures are taken here, once per build of the rows, so a paint only reads them.
fn paint_of(b: &BrushSettings) -> BrushRowPaint {
    let brush = b.clone();
    let stroke_sig = brush_preview::preview_sig(b);
    BrushRowPaint::new(tip_paint(&b.tip, (b.hardness, b.angle, b.roundness)), move |cx, rect| {
        let color = Tokens::get(cx.painter.ctx()).text;
        let size = [rect.width().round().clamp(1.0, 1024.0) as usize, rect.height().round().clamp(1.0, 1024.0) as usize];
        picture(cx, rect, ("brushes-stroke", stroke_sig, size, color), || brush_preview::stroke_image(&brush, size, color));
    })
}

/// How the engine draws `tip` with the falloff, angle and roundness `shape` (a sampled tip's shape) in the theme's text
/// colour: a preset's thumbnail and the Brush Preset chip's current tip.
pub(crate) fn tip_paint(tip: &TipShape, shape: (f32, f32, f32)) -> impl Fn(&ContentPainterCx, Rect) + Send + Sync + 'static {
    let tip = tip.clone();
    let tip_key = brush_preview::tip_sig(&tip, shape.0, shape.1, shape.2, (false, false));
    move |cx, rect| {
        let color = Tokens::get(cx.painter.ctx()).text;
        picture(cx, rect, ("brushes-tip", tip_key, color), || brush_preview::tip_image(&tip, shape, 36, color));
    }
}

/// The presets of the brush lists: every preset of `presets` in the engine's panel order, each row showing the card
/// `parts`. The rows are built once per presets revision and parts.
pub(crate) fn brush_source(presets: &Tracked<Arc<Vec<BrushPreset>>>, parts: BrushCardParts) -> PresetSource<'_, BrushKey> {
    PresetSource::new((presets, parts), PresetFns::new(brush_rows))
}

/// The rows of [`brush_source`].
fn brush_rows(input: &(&Tracked<Arc<Vec<BrushPreset>>>, BrushCardParts), env: &Env) -> Vec<PresetRow<BrushKey>> {
    preset_rows(input.0.as_slice(), env, input.1)
}

/// The rows of every preset in the engine's panel order (the order `,` and `.` step through): a
/// group row before the first preset of each group and each folder, with its count.
pub(crate) fn preset_rows(presets: &[BrushPreset], env: &Env, parts: BrushCardParts) -> Vec<PresetRow<BrushKey>> {
    let mut counts: std::collections::HashMap<(&str, &[String]), usize> = std::collections::HashMap::new();
    for p in presets {
        let path = path_of(p);
        for k in 0..=path.len() {
            *counts.entry((group_label(p), &path[..k])).or_default() += 1;
        }
    }
    let mut rows = Vec::with_capacity(presets.len());
    let mut group: Option<&str> = None;
    let mut raw = "";
    let mut open: Vec<&String> = Vec::new();
    for p in panel_layout(presets).into_iter().filter_map(|i| presets.get(i)) {
        let label = group_label(p);
        if group != Some(label) {
            group = Some(label);
            raw = group_raw(presets, p);
            open.clear();
            let count = counts.get(&(label, &[][..])).copied().unwrap_or(0);
            rows.push(PresetRow::group(BrushKey::Folder { group: raw.to_string(), folder: Vec::new() }, label, 0).count(count));
        }
        let path = path_of(p);
        let common = open.iter().zip(path).take_while(|(a, b)| ***a == **b).count();
        open.truncate(common);
        for (k, name) in path.iter().enumerate().skip(common) {
            open.push(name);
            let count = counts.get(&(label, &path[..=k])).copied().unwrap_or(0);
            rows.push(PresetRow::group(BrushKey::Folder { group: raw.to_string(), folder: path[..=k].to_vec() }, name.as_str(), k + 1).count(count));
        }
        rows.push(brush_row(BrushKey::Preset(p.name.clone()), p.name.clone(), path.len() + 1, f64::from(p.brush.size), parts, env, &paint_of(&p.brush)));
    }
    rows
}

/// The current preset's row: only while a preset of that name exists, so a deleted preset does not stay picked.
pub(crate) fn current_key(tools: &photocraft_engine::ToolState) -> Option<BrushKey> {
    let name = tools.current_preset.as_deref()?;
    tools.presets.iter().find(|p| p.name.eq_ignore_ascii_case(name)).map(|p| BrushKey::Preset(p.name.clone()))
}

/// Every input of the Brushes panel's view: the presets and the brush size it shows, and its own list state.
#[derive(MemoProps)]
struct BrushesProps<'a> {
    presets: &'a Tracked<Arc<Vec<BrushPreset>>>,
    current: Option<BrushKey>,
    diameter: f64,
    state: &'a mut Tracked<PresetState<BrushKey>>,
    out: Outbox<'a, BrushesEvent>,
}

/// What a run of the Brushes panel's view reports.
enum BrushesEvent {
    /// The Size field's edits.
    Size(EditEvents<f64>),
    /// The picks, with every row's key in list order (a reorder is placed by them).
    Picks(Vec<BrushPick<BrushKey>>, Vec<BrushKey>),
}

/// The `brushes` registry body: the Size field over the presets, in a list or a grid. A memoized view: it runs when the
/// presets, the current preset, the size or its list state change, or when input reaches it.
pub(crate) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let mut sent = Vec::new();
    let props = BrushesProps {
        presets: &app.session.tools.presets,
        current: current_key(&app.session.tools),
        diameter: f64::from(app.session.tools.brush.size),
        state: &mut app.ui.brushes_panel,
        out: Outbox::new(&mut sent),
    };
    let _ = MemoView::new("brushes", props, show_panel).show(ui);
    for event in sent {
        match event {
            BrushesEvent::Size(events) => {
                for event in &events {
                    set_size(app, event);
                }
            }
            BrushesEvent::Picks(picks, keys) => run_picks(app, picks, &keys),
        }
    }
}

/// The Brushes panel's view.
fn show_panel(props: &mut BrushesProps<'_>, ui: &mut egui::Ui) {
    let source = brush_source(props.presets, parts_of(props.state.view));
    let mut panel =
        BrushesPanel::new(tl!("Brushes"), &source, bind_tracked!(props.state), Bind::bound(&mut props.diameter), f64::from(MAX_BRUSH_SIZE)).import(true);
    if let Some(current) = props.current.clone() {
        panel = panel.selected(current);
    }
    let out = panel.show(ui);
    props.out.send(BrushesEvent::Size(out.diameter.events));
    if !out.picks.is_empty() {
        let keys = source.rows(ui.ctx(), &Env::of(ui)).iter().map(|row| row.key.clone()).collect();
        props.out.send(BrushesEvent::Picks(out.picks, keys));
    }
}

/// One edit event of the Size field as a `tools.setBrush`: a gesture's events share a journal key,
/// a `Set` is a step of its own, `Begin` changes nothing.
fn set_size(app: &mut PhotocraftApp, event: &EditEvent<f64>) {
    let Some(&size) = event.landed() else { return };
    let before = (*app.session.tools.brush).clone();
    let mut after = before.clone();
    after.size = size as f32;
    commit_keyed(app, &before, &after, crate::history_keys::coalesce("brush", event));
}

/// The one mapping from the framework's picks to commands, for the Brushes panel and the Brush
/// Preset picker. `OpenSettings` is the picker's own.
pub(crate) fn run_picks(app: &mut PhotocraftApp, picks: Vec<BrushPick<BrushKey>>, rows: &[BrushKey]) {
    for pick in picks {
        match pick {
            BrushPick::Select(key) => apply_events(app, vec![PresetEvent::select(key, Mods::default())], rows),
            BrushPick::Activate(key) => apply_events(app, vec![PresetEvent::activate(key, Mods::default())], rows),
            BrushPick::List(event) => apply_events(app, vec![event], rows),
            BrushPick::NewPreset => {
                let _ = crate::brush_preset_dialog::open(app, None);
            }
            BrushPick::Import => {
                let _ = crate::preset_files_ui::import_brushes(app);
            }
            BrushPick::OpenSettings => {}
            _ => {}
        }
    }
}

/// The one mapping from preset events to commands. `rows` are the keys shown, in order, which a
/// reorder's position counts in. Errors go to the status line.
pub(crate) fn apply_events(app: &mut PhotocraftApp, events: Vec<PresetEvent<BrushKey>>, rows: &[BrushKey]) {
    for event in events {
        if let Some((id, params)) = command_of(app, event, rows) {
            run_or_status(app, id, params);
        }
    }
}

fn command_of(app: &PhotocraftApp, event: PresetEvent<BrushKey>, rows: &[BrushKey]) -> Option<(&'static str, Value)> {
    let presets = app.session.tools.presets.as_ref();
    match event {
        PresetEvent::Select { key: BrushKey::Preset(name), .. } => Some(("tools.setBrush", json!({ "preset": name }))),
        // The double click's first click already picked it.
        PresetEvent::Activate { key: BrushKey::Preset(name), .. } => {
            let current = app.session.tools.current_preset.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(&name));
            (!current).then(|| ("tools.setBrush", json!({ "preset": name })))
        }
        PresetEvent::Rename { key, text, .. } => {
            let text = text.trim().to_string();
            match key {
                BrushKey::Preset(name) => (!text.is_empty() && text != name).then(|| ("brush.presets.rename", json!({ "name": name, "newName": text }))),
                BrushKey::Folder { group, folder } => {
                    let old = folder.last().unwrap_or(&group);
                    if text.is_empty() || text == *old {
                        return None;
                    }
                    let mut p = json!({ "group": group, "newName": text });
                    if !folder.is_empty() {
                        p["folder"] = json!(folder);
                    }
                    Some(("brush.presets.renameGroup", p))
                }
            }
        }
        PresetEvent::Delete(BrushKey::Preset(name)) => Some(("brush.presets.delete", json!({ "name": name }))),
        PresetEvent::Delete(BrushKey::Folder { group, folder }) => {
            let mut p = json!({ "group": group });
            if !folder.is_empty() {
                p["folder"] = json!(folder);
            }
            Some(("brush.presets.deleteGroup", p))
        }
        PresetEvent::MoveTo { key: BrushKey::Preset(name), group: BrushKey::Folder { group, folder }, .. } => {
            Some(("brush.presets.move", json!({ "name": name, "group": group, "folder": folder })))
        }
        // A drag is applied once, on its commit (or as a single `Set`).
        PresetEvent::Reorder(edit) if matches!(edit.phase(), Some(GesturePhase::Commit) | None) => reorder_command(presets, edit.value()?, rows),
        _ => None,
    }
}

/// Whether `k` is `root` or lies inside it.
fn inside(presets: &[BrushPreset], k: &BrushKey, root: &BrushKey) -> bool {
    let BrushKey::Folder { group: g, folder: f } = root else { return false };
    let at = match k {
        BrushKey::Folder { .. } => Some(k.clone()),
        BrushKey::Preset(name) => parent_of(presets, name),
    };
    matches!(at, Some(BrushKey::Folder { group, folder }) if group == *g && folder.starts_with(f))
}

/// The command of a reorder. `to` counts the rows left once the moved ones (a moved folder with
/// its presets) are taken out.
fn reorder_command(presets: &[BrushPreset], mv: &ReorderMove<BrushKey>, rows: &[BrushKey]) -> Option<(&'static str, Value)> {
    let remaining: Vec<&BrushKey> = rows.iter().filter(|k| !mv.keys.iter().any(|m| *k == m || inside(presets, k, m))).collect();
    let to = mv.to.min(remaining.len());
    match &mv.key {
        BrushKey::Preset(name) => {
            let parent = mv.parent.as_ref()?;
            let BrushKey::Folder { group, folder } = parent else { return None };
            let head = &remaining[..to];
            let at = head.iter().rposition(|k| **k == *parent)?;
            let index = head[at + 1..].iter().filter(|k| matches!(k, BrushKey::Preset(n) if parent_of(presets, n).as_ref() == Some(parent))).count();
            Some(("brush.presets.move", json!({ "name": name, "group": group, "folder": folder, "index": index })))
        }
        BrushKey::Folder { group, folder } => {
            let tail = &remaining[to..];
            let mut p = json!({ "group": group });
            if !folder.is_empty() {
                p["folder"] = json!(folder);
            }
            let held_by = folder.split_last().map(|(_, up)| BrushKey::Folder { group: group.clone(), folder: up.to_vec() });
            match mv.parent.as_ref() {
                None => {
                    let before = tail.iter().find_map(|k| match k {
                        BrushKey::Folder { group, folder } if folder.is_empty() => Some(group.clone()),
                        _ => None,
                    });
                    if folder.is_empty() {
                        p["before"] = json!(before);
                    } else {
                        // A nested folder goes out to the top level.
                        p["into"] = json!({});
                        if let Some(before) = before {
                            p["before"] = json!(before);
                        }
                    }
                }
                Some(parent @ BrushKey::Folder { group: pg, folder: pf }) => {
                    let before = tail.iter().find_map(|k| match k {
                        BrushKey::Folder { group, folder } if group == pg && folder.len() == pf.len() + 1 && folder.starts_with(pf) => folder.last().cloned(),
                        _ => None,
                    });
                    if held_by.as_ref() == Some(parent) {
                        p["before"] = json!(before);
                    } else {
                        p["into"] = json!({ "group": pg, "folder": pf });
                        if let Some(before) = before {
                            p["before"] = json!(before);
                        }
                    }
                }
                Some(BrushKey::Preset(_)) => return None,
            }
            Some(("brush.presets.moveGroup", p))
        }
    }
}
