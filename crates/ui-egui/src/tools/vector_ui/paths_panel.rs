use craft::behavior::geometry::SelectionOperation;
use craft::behavior::kurbo;
use craft::draw::ui::paths::{PathKind, PathRow as FwRow, PathsCommands, PathsEvent, PathsFooterButton, PathsPanel};

use super::shapes::hex;
use super::{Document, LayerContent, Path, PhotocraftApp, Value, json};
use crate::direct_select::PathRef;
use crate::state::{PathMenuSource, PathSelection};
use crate::theme::Tokens;

// ---------------------------------------------------------------------------------------------
// Paths panel

/// What a Paths panel row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathRow {
    Saved(usize),
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
    let mut rows: Vec<_> = doc
        .paths
        .iter()
        .enumerate()
        .map(|(i, p)| PathEntry { name: p.name.clone(), path: p.path.clone(), kind: PathRow::Saved(i), vector_mask: false })
        .collect();
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
    path_rows(doc, active)
        .into_iter()
        .map(|entry| {
            let key = match entry.kind {
                PathRow::Saved(i) => PathRef::Saved(i),
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
        PathRow::Saved(_) => PathKind::Saved,
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

/// Saved rows retain their index all the way to command dispatch.
fn name_of(doc: &Document, key: &PathRef) -> Option<PathSelection> {
    match key {
        PathRef::Saved(i) => doc.paths.get(*i).map(|_| PathSelection::Saved { index: *i }),
        PathRef::Work => Some("work".into()),
        PathRef::Layer(_) | PathRef::TypePath(_) => Some("layer".into()),
        PathRef::Pen => None,
    }
}

fn key_of_name(doc: &Document, name: &PathSelection, active: Option<photocraft_doc::LayerId>) -> Option<PathRef> {
    match name.name() {
        Some("work") => Some(PathRef::Work),
        Some("layer") => active.map(|id| PathRef::Layer(id.0)),
        _ => name.saved_index(doc).map(PathRef::Saved),
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
        Some(("path.delete", name_of(&st.doc, key)?.params()))
    }
}

/// The engine command a row command of the panel runs on `path`. Stroke Path is the engine command here; `apply`
/// asks for its tool first. Rename Path has no command: the panel opens its in-place field itself.
pub(crate) fn command_params(app: &PhotocraftApp, command: PathsCommands, path: &PathRef) -> Option<(&'static str, Value)> {
    let st = app.session.active()?;
    let name = name_of(&st.doc, path)?;
    match command {
        PathsCommands::MakeSelection => Some(("path.toSelection", name.params())),
        PathsCommands::FillPath => Some(("path.fill", name.params())),
        PathsCommands::StrokePath => Some(("path.stroke", name.params())),
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
            run(app, "path.toSelection", name.with_params(json!({"mode": mode})));
        }
        PathsEvent::Rename { key, text, .. } => {
            let Some(name) = name_of(&doc, &key) else { return };
            // The renamed (or just saved) path stays the panel's target.
            if let Some(result) = run(app, "path.rename", name.with_params(json!({"to": text}))) {
                let index = result
                    .get("pathIndex")
                    .and_then(Value::as_u64)
                    .and_then(|i| usize::try_from(i).ok())
                    // Saving Work Path appends one saved row; its engine reply remains name-only.
                    .or_else(|| (key == PathRef::Work).then_some(doc.paths.len()));
                app.ui.selected_path =
                    index.filter(|i| app.session.active().is_some_and(|st| st.doc.paths.get(*i).is_some())).map(|index| PathSelection::Saved { index });
            }
        }
        PathsEvent::Command { command, key, .. } => {
            let target = key.or_else(|| app.ui.selected_path.as_ref().and_then(|n| key_of_name(&doc, n, active)));
            let Some(target) = target else { return };
            if command == PathsCommands::StrokePath {
                // Stroke Path… asks for the tool first, as in Photoshop (#2978).
                if let Some(name) = name_of(&doc, &target) {
                    crate::canvas_tool_menu::pen_dialogs::open(app, "path.stroke", &name);
                }
            } else if let Some((cmd, params)) = command_params(app, command, &target)
                && run(app, cmd, params).is_some()
                && command == PathsCommands::SavePath
                && target == PathRef::Work
            {
                app.ui.selected_path = Some(PathSelection::Saved { index: doc.paths.len() });
            }
        }
        PathsEvent::Footer { button, rows, mods, .. } => {
            // A missing selection may use Work Path; an invalid saved selection must never
            // turn a consuming action into an operation on a different path.
            if rows.is_empty()
                && app.ui.selected_path.as_ref().is_some_and(|n| key_of_name(&doc, n, active).is_none())
                && !matches!(button, PathsFooterButton::New | PathsFooterButton::WorkPath)
            {
                return;
            }
            let selected = rows.first().cloned().or_else(|| app.ui.selected_path.as_ref().and_then(|n| key_of_name(&doc, n, active))).unwrap_or(PathRef::Work);
            // Creation does not need an existing path. All consuming actions must reject a stale row.
            let name = match name_of(&doc, &selected) {
                Some(name) => name,
                None if matches!(button, PathsFooterButton::New | PathsFooterButton::WorkPath) => "work".into(),
                None => return,
            };
            match button {
                PathsFooterButton::Fill => {
                    run(app, "path.fill", name.with_params(json!({"color": hex(app.session.tools.foreground)})));
                }
                PathsFooterButton::Stroke if mods.alt => {
                    // Alt-click opens the Stroke Path dialog, as in Photoshop (#2978).
                    crate::canvas_tool_menu::pen_dialogs::open(app, "path.stroke", &name);
                }
                PathsFooterButton::Stroke => {
                    run(app, "path.stroke", name.with_params(json!({"tool": "brush"})));
                }
                PathsFooterButton::Load => {
                    run(app, "path.toSelection", name.params());
                }
                PathsFooterButton::WorkPath => {
                    run(app, "select.toWorkPath", json!({"tolerance": 2.0}));
                }
                PathsFooterButton::VectorMask => {
                    run(app, "layer.vectorMask.fromPath", name.params());
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
                    run(app, "path.toSelection", name.params());
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

/// Deletes the selected rows without letting earlier deletions shift a later target.
fn delete_rows(app: &mut PhotocraftApp, rows: &[PathRef]) {
    // Descending saved indices keep every later deletion on its original row. Deduplicate
    // repeated targets so the next occupant of a deleted slot cannot be deleted as well.
    let mut targets = rows.to_vec();
    targets.sort_by_key(|k| match k {
        PathRef::Saved(i) => (0, usize::MAX - i),
        _ => (1, 0),
    });
    let mut seen = std::collections::HashSet::new();
    let commands: Vec<_> = targets.iter().filter(|k| seen.insert(**k)).filter_map(|k| delete_command(app, k, false)).collect();
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
    let source = PathMenuSource::capture(st);
    if app.framework.paths_source.as_ref() != Some(&source) {
        app.framework.paths_epoch = app.framework.paths_epoch.saturating_add(1);
    }
    app.framework.paths_source = Some(source.clone());
    let epoch = app.framework.paths_epoch;
    let layer = st.active_layer;
    let doc = st.doc.clone();
    let can_fill = st.active_layer.and_then(|id| doc.layer(id)).is_some_and(|l| l.surface().is_some());
    let selected = app.ui.selected_path.as_ref().and_then(|target| key_of_name(&doc, target, st.active_layer));
    let rows: Vec<FwRow<PathRef>> = keyed_rows(&doc, st.active_layer)
        .into_iter()
        .map(|(key, entry)| {
            let kind = kind_of(&entry);
            let name = if entry.kind == PathRow::Work { tl!("Work Path").to_string() } else { entry.name.clone() };
            let mut row = FwRow::new(key, kind, name, outline(&entry.path));
            row.selected = selected == Some(key);
            row
        })
        .collect();
    let size = kurbo::Size::new(doc.size.width.max(1) as f64, doc.size.height.max(1) as f64);

    // Framework rename fields, drags and popups belong to this exact document state.
    // A new source gets a new UI scope, so retained row indices cannot retarget a new path.
    let events = ui.push_id((epoch, layer), |ui| PathsPanel::new(&rows, size).can_fill(can_fill).work_name(free_path_name(&doc)).show(ui).events).inner;
    for event in events {
        if app.session.active().map(PathMenuSource::capture).as_ref() != Some(&source) {
            break;
        }
        apply(app, event);
    }
}

#[cfg(test)]
mod index_tests;
