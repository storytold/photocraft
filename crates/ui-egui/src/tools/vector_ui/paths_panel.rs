use craft::behavior::geometry::SelectionOperation;
use craft::behavior::kurbo;
use craft::draw::ui::paths::{PathKind, PathRow as FwRow, PathsCommands, PathsEvent, PathsFooterButton, PathsPanel};

use super::shapes::hex;
use super::{Document, LayerContent, Path, PhotocraftApp, Value, json};
use crate::direct_select::PathRef;
use crate::theme::Tokens;

// ---------------------------------------------------------------------------------------------
// Paths panel

/// What a Paths panel row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathRow {
    Saved,
    Work,
    /// The selected layer's shape path or vector mask (temporary, italic).
    Layer,
}

/// A Paths panel row.
#[derive(Clone, Debug)]
pub struct PathEntry {
    pub name: String,
    pub path: Path,
    pub kind: PathRow,
    /// The `Layer` row is the layer's vector mask (else its shape path).
    pub vector_mask: bool,
}

/// The Paths panel's rows, top to bottom: saved paths, the work path, then the selected layer's
/// shape path ("<Layer> Shape Path") or vector mask ("<Layer> Vector Mask"), like Photoshop.
pub fn path_rows(doc: &Document, active: Option<photocraft_doc::LayerId>) -> Vec<PathEntry> {
    let mut rows: Vec<_> =
        doc.paths.iter().map(|p| PathEntry { name: p.name.clone(), path: p.path.clone(), kind: PathRow::Saved, vector_mask: false }).collect();
    if let Some(wp) = &doc.work_path {
        rows.push(PathEntry { name: "Work Path".into(), path: wp.clone(), kind: PathRow::Work, vector_mask: false });
    }
    if let Some(l) = active.and_then(|id| doc.layer(id)) {
        let layer_path = match &l.content {
            LayerContent::Shape(sh) => Some((format!("{} Shape Path", l.name), sh.path.clone(), false)),
            _ => l.vector_mask.as_ref().map(|v| (format!("{} Vector Mask", l.name), v.path.clone(), true)),
        };
        if let Some((name, path, vector_mask)) = layer_path {
            rows.push(PathEntry { name, path, kind: PathRow::Layer, vector_mask });
        }
    }
    rows
}

/// Photoshop's default name for a saved path: the first free "Path N", counting from the number of
/// saved paths.
fn free_path_name(doc: &Document) -> String {
    let mut n = doc.paths.len().saturating_add(1);
    while doc.paths.iter().any(|p| p.name == format!("Path {n}")) {
        n = n.saturating_add(1);
        if n == usize::MAX {
            break;
        }
    }
    format!("Path {n}")
}

/// The rows with their keys: the one place a row's `PathRef` is assigned. A saved path is keyed by its
/// position in `doc.paths`, which is also its position among the rows.
fn keyed_rows(doc: &Document, active: Option<photocraft_doc::LayerId>) -> Vec<(PathRef, PathEntry)> {
    let mut saved = 0;
    path_rows(doc, active)
        .into_iter()
        .map(|entry| {
            let key = match entry.kind {
                PathRow::Saved => {
                    saved += 1;
                    PathRef::Saved(saved - 1)
                }
                PathRow::Work => PathRef::Work,
                PathRow::Layer => PathRef::Layer(active.map_or(0, |id| id.0)),
            };
            (key, entry)
        })
        .collect()
}

/// Whether a layer row is the layer's vector mask (else its shape path).
fn is_vector_mask(entry: &PathEntry) -> bool {
    entry.kind == PathRow::Layer && entry.vector_mask
}

/// The framework kind of a row.
fn kind_of(entry: &PathEntry) -> PathKind {
    match entry.kind {
        PathRow::Saved => PathKind::Saved,
        PathRow::Work => PathKind::Work,
        PathRow::Layer if is_vector_mask(entry) => PathKind::VectorMask,
        PathRow::Layer => PathKind::ShapePath,
    }
}

/// The outline of a document path for a row thumbnail.
fn outline(path: &Path) -> kurbo::BezPath {
    let mut out = kurbo::BezPath::new();
    for sub in &path.subpaths {
        let Some(first) = sub.knots.first() else { continue };
        let pt = |p: photocraft_geom::Point| kurbo::Point::new(p.x, p.y);
        out.move_to(pt(first.anchor));
        for [_, c1, c2, p1] in sub.segments() {
            out.curve_to(pt(c1), pt(c2), pt(p1));
        }
        if sub.closed {
            out.close_path();
        }
    }
    out
}

/// The engine's name of a row: "work", "layer" or the saved path's name.
fn name_of(doc: &Document, key: &PathRef) -> Option<String> {
    match key {
        PathRef::Saved(i) => doc.paths.get(*i).map(|p| p.name.clone()),
        PathRef::Work => Some("work".into()),
        PathRef::Layer(_) | PathRef::TypePath(_) => Some("layer".into()),
        PathRef::Pen => None,
    }
}

/// The row an engine name stands for.
fn key_of_name(doc: &Document, name: &str, active: Option<photocraft_doc::LayerId>) -> Option<PathRef> {
    match name {
        "work" => Some(PathRef::Work),
        "layer" => active.map(|id| PathRef::Layer(id.0)),
        _ => doc.paths.iter().position(|p| p.name == name).map(PathRef::Saved),
    }
}

/// Runs an engine command; an error goes to the status line.
fn run(app: &mut PhotocraftApp, command: &str, params: Value) -> Option<Value> {
    match app.run(command, params) {
        Ok(v) => Some(v),
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
            None
        }
    }
}

/// The command that deletes a row. A shape layer's path is the shape: the context menu's Delete Path
/// deletes the layer (#6552, `shape_deletes_layer`), the footer's Delete runs `path.delete` on it.
fn delete_command(app: &PhotocraftApp, key: &PathRef, shape_deletes_layer: bool) -> Option<(&'static str, Value)> {
    let st = app.session.active()?;
    let entry = keyed_rows(&st.doc, st.active_layer).into_iter().find(|(k, _)| k == key).map(|(_, e)| e)?;
    let layer = match key {
        PathRef::Layer(id) => *id,
        _ => 0,
    };
    if is_vector_mask(&entry) {
        Some(("layer.vectorMask.delete", json!({"layer": layer})))
    } else if entry.kind == PathRow::Layer && shape_deletes_layer {
        Some(("layer.delete", json!({"layer": layer})))
    } else {
        Some(("path.delete", json!({"name": name_of(&st.doc, key)?})))
    }
}

/// The engine command a row command of the panel runs on `path`. Stroke Path is the engine command here; `apply`
/// asks for its tool first. Rename Path has no command: the panel opens its in-place field itself.
pub(crate) fn command_params(app: &PhotocraftApp, command: PathsCommands, path: &PathRef) -> Option<(&'static str, Value)> {
    let st = app.session.active()?;
    let name = name_of(&st.doc, path)?;
    match command {
        PathsCommands::MakeSelection => Some(("path.toSelection", json!({"name": name}))),
        PathsCommands::FillPath => Some(("path.fill", json!({"name": name}))),
        PathsCommands::StrokePath => Some(("path.stroke", json!({"name": name}))),
        PathsCommands::SavePath => (*path == PathRef::Work).then(|| ("path.rename", json!({"name": "work", "to": free_path_name(&st.doc)}))),
        PathsCommands::DuplicatePath => {
            let entry = keyed_rows(&st.doc, st.active_layer).into_iter().find(|(k, _)| k == path).map(|(_, e)| e)?;
            if entry.kind == PathRow::Work {
                return None;
            }
            let mut n = 1usize;
            while st.doc.paths.iter().any(|p| p.name == format!("{} copy {n}", entry.name)) {
                n = n.saturating_add(1);
                if n == usize::MAX {
                    break;
                }
            }
            let copy = format!("{} copy {n}", entry.name);
            Some(("path.set", json!({"name": copy, "path": photocraft_engine::vector_cmds::path_json(&entry.path)})))
        }
        PathsCommands::DeletePath => delete_command(app, path, true),
        _ => None,
    }
}

/// The one mapping from the panel's events to commands. A row command takes its row from the event, and the selected
/// row when the event has none.
pub(crate) fn apply(app: &mut PhotocraftApp, event: PathsEvent<PathRef>) {
    let Some(st) = app.session.active() else { return };
    let (doc, active) = (st.doc.clone(), st.active_layer);
    match event {
        PathsEvent::Select(None) => app.ui.selected_path = None,
        PathsEvent::Select(Some(key)) => {
            // Selecting the Work Path ends the hide from Make Selection (#6079).
            if key == PathRef::Work {
                app.ui.hidden_work_path = None;
            }
            app.ui.selected_path = name_of(&doc, &key);
        }
        PathsEvent::LoadSelection { key, operation, .. } => {
            let Some(name) = name_of(&doc, &key) else { return };
            let mode = match operation {
                SelectionOperation::Add => "add",
                SelectionOperation::Subtract => "subtract",
                SelectionOperation::Intersect => "intersect",
                _ => "replace",
            };
            run(app, "path.toSelection", json!({"name": name, "mode": mode}));
        }
        PathsEvent::Rename { key, text, .. } => {
            let Some(name) = name_of(&doc, &key) else { return };
            // The renamed (or just saved) path stays the panel's target.
            if run(app, "path.rename", json!({"name": name, "to": text})).is_some() {
                app.ui.selected_path = Some(text);
            }
        }
        PathsEvent::Command { command, key, .. } => {
            let target = key.or_else(|| app.ui.selected_path.as_deref().and_then(|n| key_of_name(&doc, n, active)));
            let Some(target) = target else { return };
            if command == PathsCommands::StrokePath {
                // Stroke Path… asks for the tool first, as in Photoshop (#2978).
                if let Some(name) = name_of(&doc, &target) {
                    crate::canvas_tool_menu::pen_dialogs::open(app, "path.stroke", &name);
                }
            } else if let Some((cmd, params)) = command_params(app, command, &target) {
                run(app, cmd, params);
            }
        }
        PathsEvent::Footer { button, rows, mods, .. } => {
            let selected =
                rows.first().cloned().or_else(|| app.ui.selected_path.as_deref().and_then(|n| key_of_name(&doc, n, active))).unwrap_or(PathRef::Work);
            let name = name_of(&doc, &selected).unwrap_or_else(|| "work".into());
            match button {
                PathsFooterButton::Fill => {
                    run(app, "path.fill", json!({"name": name, "color": hex(app.session.tools.foreground)}));
                }
                PathsFooterButton::Stroke if mods.alt => {
                    // Alt-click opens the Stroke Path dialog, as in Photoshop (#2978).
                    crate::canvas_tool_menu::pen_dialogs::open(app, "path.stroke", &name);
                }
                PathsFooterButton::Stroke => {
                    run(app, "path.stroke", json!({"name": name, "tool": "brush"}));
                }
                PathsFooterButton::Load => {
                    run(app, "path.toSelection", json!({"name": name}));
                }
                PathsFooterButton::WorkPath => {
                    run(app, "select.toWorkPath", json!({"tolerance": 2.0}));
                }
                PathsFooterButton::VectorMask => {
                    run(app, "layer.vectorMask.fromPath", json!({"name": name}));
                }
                PathsFooterButton::New => {
                    run(app, "path.set", json!({"name": free_path_name(&doc), "path": {"subpaths": []}}));
                }
                PathsFooterButton::Delete => {
                    let targets = if rows.is_empty() { vec![selected] } else { rows };
                    delete_rows(app, &targets);
                }
                _ => {}
            }
        }
        PathsEvent::FooterDrop { button, rows, .. } => match button {
            PathsFooterButton::Delete => delete_rows(app, &rows),
            PathsFooterButton::Load => {
                if let Some(name) = rows.first().and_then(|k| name_of(&doc, k)) {
                    run(app, "path.toSelection", json!({"name": name}));
                }
            }
            PathsFooterButton::New => {
                let commands: Vec<_> = rows.iter().filter_map(|k| command_params(app, PathsCommands::DuplicatePath, k)).collect();
                for (cmd, params) in commands {
                    run(app, cmd, params);
                }
            }
            _ => {}
        },
        _ => {}
    }
}

/// Deletes `rows`. Every row becomes a command by name (or layer id) before the first runs: a saved path's key is its
/// position, which the first deletion shifts.
fn delete_rows(app: &mut PhotocraftApp, rows: &[PathRef]) {
    let commands: Vec<_> = rows.iter().filter_map(|k| delete_command(app, k, false)).collect();
    for (cmd, params) in commands {
        run(app, cmd, params);
    }
}

pub fn paths_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(st) = app.session.active() else {
        let t = Tokens::get(ui.ctx());
        ui.label(egui::RichText::new(tl!("No document")).color(t.text_faint));
        return;
    };
    let doc = st.doc.clone();
    let can_fill = st.active_layer.and_then(|id| doc.layer(id)).is_some_and(|l| l.surface().is_some());
    let selected = app.ui.selected_path.clone();
    let rows: Vec<FwRow<PathRef>> = keyed_rows(&doc, st.active_layer)
        .into_iter()
        .map(|(key, entry)| {
            let kind = kind_of(&entry);
            let engine_name = name_of(&doc, &key);
            let name = if entry.kind == PathRow::Work { tl!("Work Path").to_string() } else { entry.name.clone() };
            let mut row = FwRow::new(key, kind, name, outline(&entry.path));
            row.selected = selected.is_some() && selected == engine_name;
            row
        })
        .collect();
    let size = kurbo::Size::new(doc.size.width.max(1) as f64, doc.size.height.max(1) as f64);

    let events = PathsPanel::new(&rows, size).can_fill(can_fill).work_name(free_path_name(&doc)).show(ui).events;
    for event in events {
        apply(app, event);
    }
}
