//! Painting symmetry on the framework: the menu, the guides, the transform session and its tool.
use crate::{PhotocraftApp, state::Tool};
use craft::behavior::kurbo::{Point, Size};
use craft::draw::ui::symmetry::{
    Symmetry, SymmetryChoice, SymmetryCommand, SymmetryEvent, SymmetryHandles, SymmetryMenu, SymmetryMode as FwMode, SymmetryOverlay, SymmetryPathRow,
    SymmetryTool, SymmetryTransform, SymmetryWords,
};
use craft::ui::contract::{Bind, Show as _};
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::runtime::env::Env;
use photocraft_doc::Document;
use photocraft_paint::symmetry::{PresetSymmetry, SymmetryMode};
use serde::{Serialize, Serializer, ser::SerializeStruct};
use serde_json::json;
use std::sync::Arc;

/// An open Transform Symmetry: the framework session, and what ties it to the document and tool it began on.
#[derive(Clone, Debug)]
pub struct Transform {
    session: SymmetryTransform,
    document: usize,
    tool: Tool,
    source: Arc<Document>,
}

impl PartialEq for Transform {
    fn eq(&self, other: &Self) -> bool {
        self.session.original() == other.session.original()
            && self.session.preview() == other.session.preview()
            && self.document == other.document
            && self.tool == other.tool
            && Arc::ptr_eq(&self.source, &other.source)
    }
}

impl Serialize for Transform {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("Transform", 4)?;
        s.serialize_field("original", &from_framework(self.session.original()))?;
        s.serialize_field("preview", &from_framework(self.session.preview()))?;
        s.serialize_field("document", &self.document)?;
        s.serialize_field("tool", &self.tool)?;
        s.end()
    }
}

fn mode_to_framework(mode: SymmetryMode) -> FwMode {
    match mode {
        SymmetryMode::Vertical => FwMode::Vertical,
        SymmetryMode::Horizontal => FwMode::Horizontal,
        SymmetryMode::Dual => FwMode::Dual,
        SymmetryMode::Diagonal => FwMode::Diagonal,
    }
}

fn mode_from_framework(mode: FwMode) -> Option<SymmetryMode> {
    match mode {
        FwMode::Vertical => Some(SymmetryMode::Vertical),
        FwMode::Horizontal => Some(SymmetryMode::Horizontal),
        FwMode::Dual => Some(SymmetryMode::Dual),
        FwMode::Diagonal => Some(SymmetryMode::Diagonal),
        _ => None,
    }
}

fn to_framework(p: PresetSymmetry) -> Option<Symmetry> {
    Symmetry::new(mode_to_framework(p.mode), Point::new(p.center[0], p.center[1]), p.rotation)
}

fn from_framework(s: Symmetry) -> Option<PresetSymmetry> {
    PresetSymmetry::new(mode_from_framework(s.mode)?, [s.center.x, s.center.y], s.rotation)
}

fn preset(app: &PhotocraftApp) -> Option<PresetSymmetry> {
    app.session.active()?.symmetry.as_ref()?.preset()
}

/// Whether Transform Symmetry can open: a painting tool with a preset symmetry and nothing else editing.
pub fn can_transform(app: &PhotocraftApp) -> bool {
    matches!(app.ui.tool, Tool::Brush | Tool::Pencil | Tool::Eraser)
        && !crate::canvas::tools::in_gesture(app)
        && preset(app).is_some()
        && app.ui.transform.is_none()
        && app.ui.text_edit.is_none()
        && !app.dialog_open()
        && app.distort.puppet.is_none()
        && app.distort.perspective.is_none()
        && crate::jobs_ui::job_in_view(app).is_none()
}

/// Opens Transform Symmetry on the active document's preset.
pub fn begin(app: &mut PhotocraftApp) -> Result<(), String> {
    if !can_transform(app) {
        return Err("no preset symmetry available to transform".into());
    }
    let original = preset(app).and_then(to_framework).ok_or("no preset symmetry")?;
    let document = app.session.active_index().ok_or("no document")?;
    let source = app.session.active().ok_or("no document")?.doc.clone();
    app.ui.symmetry_transform = Some(Transform { session: SymmetryTransform::begin(original), document, tool: app.ui.tool, source });
    Ok(())
}

/// Stores the previewed symmetry as one command and closes the session.
pub fn commit(app: &mut PhotocraftApp) {
    if let Some(t) = app.ui.symmetry_transform.take()
        && let Some(p) = from_framework(t.session.preview())
    {
        let _ = app.run("paint.setSymmetry", json!({"mode": p.mode.id(), "center": p.center, "rotation": p.rotation}));
    }
}

/// Drops the session; the stored symmetry stays as it was.
pub fn cancel(app: &mut PhotocraftApp) {
    app.ui.symmetry_transform = None;
}

/// Whether Transform Symmetry is open (the options bar shows its commit and cancel buttons).
pub fn is_open(app: &PhotocraftApp) -> bool {
    app.ui.symmetry_transform.is_some()
}

/// The options bar's members while the transform is open, before the bar's commit and cancel buttons.
pub fn bar_members(ui: &mut egui::Ui) {
    let words = Env::of(ui).words::<SymmetryWords>();
    Text::new(words.transform.clone()).shade(Shade::Muted).show(ui);
    Divider::vertical().show(ui);
}

fn valid(app: &PhotocraftApp, t: &Transform) -> bool {
    app.session.active_index() == Some(t.document)
        && app.ui.tool == t.tool
        && can_transform(app)
        && preset(app) == from_framework(t.session.original())
        && app.session.active().is_some_and(|s| Arc::ptr_eq(&s.doc, &t.source))
}

/// What the painting symmetry button and its menu show: the current symmetry, the paths it can follow and which
/// commands are open.
#[derive(Clone, Debug, PartialEq, craft::ui::contract::MemoProps)]
pub(crate) struct MenuInput {
    current: SymmetryChoice,
    rows: Vec<SymmetryPathRow>,
    can_transform: bool,
    can_set: bool,
}

/// The symmetry menu's input as the app holds it now.
pub(crate) fn menu_input(app: &PhotocraftApp, ui: &egui::Ui) -> MenuInput {
    let words = Env::of(ui).words::<SymmetryWords>();
    let current = match app.session.active().and_then(|s| s.symmetry.as_ref()) {
        None => SymmetryChoice::Off,
        Some(s) => match (s.preset(), s.path_source()) {
            (Some(p), _) => SymmetryChoice::Mode(mode_to_framework(p.mode)),
            (None, Some(key)) => SymmetryChoice::Path(key.to_owned()),
            (None, None) => SymmetryChoice::Off,
        },
    };
    let rows: Vec<SymmetryPathRow> = app
        .session
        .active()
        .map(|st| {
            crate::vector_ui::path_rows(&st.doc, st.active_layer)
                .into_iter()
                .map(|row| match row.kind {
                    crate::vector_ui::PathRow::Saved => SymmetryPathRow::new(row.name.clone(), row.name),
                    crate::vector_ui::PathRow::Work => SymmetryPathRow::new("work", words.work_path.as_str().to_owned()),
                    crate::vector_ui::PathRow::Layer => SymmetryPathRow::new("layer", row.name),
                })
                .collect()
        })
        .unwrap_or_default();
    MenuInput { current, rows, can_transform: can_transform(app), can_set: crate::menus::is_enabled(app, "paint.setSymmetry") }
}

/// The painting symmetry button and its menu over `input`, shown in the options bar of the painting tools. Returns
/// what the menu picked; [`pick`] runs it.
pub(crate) fn show_menu(input: &MenuInput, ui: &mut egui::Ui) -> Option<SymmetryCommand> {
    let words = Env::of(ui).words::<SymmetryWords>();
    SymmetryMenu::new(words.group.clone(), &input.current).paths(&input.rows).can_transform(input.can_transform).can_set(input.can_set).show(ui).command
}

/// Runs what the menu picked. The one place a symmetry command is mapped to the app's commands.
pub(crate) fn pick(app: &mut PhotocraftApp, ctx: &egui::Context, command: SymmetryCommand) {
    let result = match command {
        SymmetryCommand::Off => app.run("paint.symmetryDisable", json!({})),
        SymmetryCommand::Mode(m) => match mode_from_framework(m) {
            Some(mode) => app.run("paint.setSymmetry", json!({"mode": mode.id()})),
            None => return,
        },
        SymmetryCommand::FromPath(key) => app.run("paint.symmetryFromPath", json!({"name": key})),
        SymmetryCommand::Transform => crate::menus::invoke(app, ctx, "ui.symmetryTransform", json!({})),
        _ => return,
    };
    if let Err(e) = result {
        app.ui.status = e;
    }
}

/// The guides and, in a session, the handles, inside the canvas body at `at`. A session whose window lost focus is
/// dropped; a second pass in the frame repeats that and applies only its own input.
pub fn canvas_controls(ui: &mut egui::Ui, app: &mut PhotocraftApp, at: egui::Rect) {
    if !ui.ctx().input(|i| i.focused) {
        cancel(app);
    }
    let words = Env::of(ui).words::<SymmetryWords>();
    let Some(st) = app.session.active() else {
        return;
    };
    let page = Size::new(f64::from(st.doc.size.width), f64::from(st.doc.size.height));
    let session = app.ui.symmetry_transform.as_ref().filter(|t| app.session.active_index() == Some(t.document)).map(|t| t.session.preview());
    if let Some(mut preview) = session {
        SymmetryOverlay::new(words.group.clone(), &preview, page).editing(true).show_at(ui, at);
        let out = SymmetryHandles::new(words.center.clone(), Bind::bound(&mut preview), page).session().show_at(ui, at);
        if let Some(t) = &mut app.ui.symmetry_transform {
            for event in out.edit.events.iter() {
                t.session.apply(event);
            }
        }
        if out.apply {
            commit(app);
        } else if out.discard {
            cancel(app);
        }
    } else if matches!(app.ui.tool, Tool::Brush | Tool::Pencil | Tool::Eraser)
        && let Some(symmetry) = preset(app).and_then(to_framework)
    {
        SymmetryOverlay::new(words.group.clone(), &symmetry, page).show_at(ui, at);
    }
}

/// The canvas tool over a session taken out of the app for the pass, while it belongs to the active document. The
/// canvas runs it ahead of the painting tool, so a press away from the handles never paints. The tool borrows the
/// session only, so the pass can take the app.
pub fn session_tool<'a>(app: &PhotocraftApp, t: &'a Transform) -> Option<SymmetryTool<'a>> {
    (app.session.active_index() == Some(t.document)).then(|| SymmetryTool::new(&t.session, valid(app, t)))
}

/// Applies what the session's tool reported: `Apply` stores the symmetry, `Discard` drops the session.
pub fn finish_tool(app: &mut PhotocraftApp, events: Vec<SymmetryEvent>) {
    for event in events {
        match event {
            SymmetryEvent::Apply(_) => commit(app),
            _ => cancel(app),
        }
    }
}

#[cfg(test)]
mod tests;
