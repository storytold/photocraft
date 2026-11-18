//! Slice tool and Slice Select tool (Crop group, C): the family tool over the framework's `SliceTool`,
//! the slice overlay, both tools' options bars and the Slice Options and Divide Slice dialogs. Every
//! change is an engine command (`slice.new`, `slice.set`, `slice.promote`, `slice.divide`,
//! `slice.delete`); this module only turns the framework's slice events into them.

use craft::behavior::EditEvent;
use craft::behavior::geometry::Constraint;
use craft::behavior::kurbo::Rect as DocRect;
use craft::draw::ui::slice::{
    DivideBy, DivideSlices, DivideSlicesDialog, SliceBar, SliceCommand, SliceEvent, SliceItem, SliceKind, SliceMode, SliceOptions, SliceOptionsDialog,
    SliceOrigin as ItemOrigin, SliceOverlay, SliceRules, SliceTool, SliceToolSettings,
};
use craft::theme::Srgba;
use craft::ui::canvas::ToolHostResponse;
use craft::ui::contract::Show as _;
use craft::ui::dialog::{DialogId, DialogResult};
use craft::ui::runtime::commands::{CommandRef, CommandSet as _, CommandState};
use photocraft_doc::SliceOrigin;
use photocraft_doc::slices::{self, ResolvedSlice};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::document_display::CanvasHost;
use crate::canvas::tools::{FamilyTool, ToolScope};
use crate::state::Tool;

/// Slice tool state (serde: readable through the control channel; only the selection and Hide Auto
/// Slices are saved).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SliceUi {
    /// The selected slice's display number (Slice Select tool).
    pub selected: Option<usize>,
    /// Slice Select options bar › Hide Auto Slices.
    pub hide_auto: bool,
    /// The slice a Move or Resize gesture is dragging, with its proposed rectangle: the overlay paints
    /// the items it is given, so the gesture's value is fed back until it ends.
    #[serde(skip)]
    pub preview: Option<(usize, DocRect)>,
    /// The open Slice Options dialog and the slice it is for.
    #[serde(skip)]
    pub options: Option<(DialogId, SliceTarget)>,
    /// The open Divide Slice dialog and the slice it is for.
    #[serde(skip)]
    pub divide: Option<(DialogId, SliceTarget)>,
}

/// What a dialog's command addresses: a stored slice by id, or an auto slice by its number (the
/// engine promotes it inside the same step).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SliceTarget {
    /// A stored (user or layer) slice's id.
    Stored(u32),
    /// An auto slice's display number.
    Auto(usize),
}

impl SliceTarget {
    /// The engine params that address the slice.
    fn params(self) -> Value {
        match self {
            SliceTarget::Stored(id) => json!({"slice": id}),
            SliceTarget::Auto(n) => json!({"number": n}),
        }
    }
}

fn is_slice_tool(t: Tool) -> bool {
    matches!(t, Tool::Slice | Tool::SliceSelect)
}

fn to_doc_rect(r: photocraft_geom::Rect) -> DocRect {
    DocRect::new(f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1))
}

fn origin_of(origin: SliceOrigin) -> ItemOrigin {
    match origin {
        SliceOrigin::User => ItemOrigin::User,
        SliceOrigin::Auto => ItemOrigin::Auto,
        SliceOrigin::Layer => ItemOrigin::Layer,
    }
}

/// The resolved slices of the active document, with the dragged slice at its proposed rectangle.
fn items(app: &PhotocraftApp) -> Vec<SliceItem> {
    let Some(st) = app.session.active() else { return Vec::new() };
    let preview = app.ui.slices.preview;
    slices::resolve(&st.doc)
        .iter()
        .map(|r| {
            let rect = match preview {
                Some((n, rect)) if n == r.number => rect,
                _ => to_doc_rect(r.rect),
            };
            SliceItem::new(r.number, r.id, rect, origin_of(r.origin))
        })
        .collect()
}

/// The Style, Width and Height of the options bar as the marquee's constraint, with Hide Auto Slices.
fn settings(app: &PhotocraftApp) -> SliceToolSettings {
    let o = &app.ui.tool_options;
    let (w, h) = (f64::from(o.slice_width), f64::from(o.slice_height));
    let style = match o.slice_style.as_str() {
        "fixedRatio" => Constraint::Ratio(w, h),
        "fixedSize" => Constraint::Size(w, h),
        _ => Constraint::Free,
    };
    let mut s = SliceToolSettings::new(style);
    s.hide_auto = app.ui.slices.hide_auto;
    s
}

/// Writes the bar's settings back to the stored keys (`tool_options.slice_*`, `slices.hide_auto`).
fn store(app: &mut PhotocraftApp, s: SliceToolSettings) {
    let o = &mut app.ui.tool_options;
    match s.style {
        Constraint::Ratio(w, h) => (o.slice_style, o.slice_width, o.slice_height) = ("fixedRatio".into(), w as f32, h as f32),
        Constraint::Size(w, h) => (o.slice_style, o.slice_width, o.slice_height) = ("fixedSize".into(), w as f32, h as f32),
        _ => o.slice_style = "normal".into(),
    }
    app.ui.slices.hide_auto = s.hide_auto;
}

/// The slice colour from Preferences › Guides, Grid & Slices.
pub fn slice_color(app: &PhotocraftApp) -> Srgba {
    hex(&app.session.prefs().guides_grid_and_slices.slice_color, Srgba::rgb(f32::from(0x38_u8) / 255.0, f32::from(0xb5_u8) / 255.0, 1.0))
}

fn hex(s: &str, fallback: Srgba) -> Srgba {
    let s = s.trim_start_matches('#');
    let b = |i: usize| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).map(|v| f32::from(v) / 255.0);
    match (s.len(), b(0), b(2), b(4)) {
        (6, Some(r), Some(g), Some(bl)) => Srgba::rgb(r, g, bl),
        _ => fallback,
    }
}

// ---------- the tool ----------

/// The framework's `SliceTool` for this pass, with the app's snapshot set on it.
pub(crate) fn tool<'a>(scope: &'a ToolScope<'a>, state: &'a mut SliceTool, _host: CanvasHost) -> FamilyTool<'a> {
    scope.with_app(|app| {
        let mode = if app.ui.tool == Tool::SliceSelect { SliceMode::Select } else { SliceMode::Slice };
        state
            .mode(mode)
            .items(&items(app))
            .selected(app.ui.slices.selected)
            .locked(app.session.file_menu.slices_locked)
            .settings(settings(app))
            .rules(SliceRules::PHOTOSHOP)
            .color(Some(slice_color(app)));
    });
    Box::new(state)
}

/// Takes the events the tool reported in the pass.
pub(crate) fn drain(app: &mut PhotocraftApp, state: &mut SliceTool, _response: &ToolHostResponse) {
    for event in state.take_events() {
        apply(app, event);
    }
}

// ---------- the overlay ----------

/// Slice outlines and numbered badges (View › Show › Slices; always while a slice tool is on) over
/// the document at `at`. Its events, and in Select mode its picks, moves and resizes, go to [`apply`].
pub(crate) fn canvas_controls(ui: &mut egui::Ui, app: &mut PhotocraftApp, at: egui::Rect) {
    let tool = app.ui.tool;
    let tool_on = is_slice_tool(tool);
    let Some(has_slices) = app.session.active().map(|d| !d.doc.slices.is_empty()) else { return };
    if !(tool_on || (app.ui.view.shows(app.ui.view.show.slices) && has_slices)) {
        return;
    }
    let items = items(app);
    let numbers = app.session.prefs().guides_grid_and_slices.show_slice_numbers;
    let mut overlay = SliceOverlay::new("Slices", &items)
        .pick(tool == Tool::SliceSelect)
        .hide_auto(!tool_on || app.ui.slices.hide_auto)
        .numbers(numbers)
        .color(slice_color(app))
        .locked(app.session.file_menu.slices_locked)
        .rules(SliceRules::PHOTOSHOP);
    if let Some(n) = app.ui.slices.selected.filter(|_| tool_on) {
        overlay = overlay.selected_slice(n);
    }
    let events = overlay.show_at(ui, at).events;
    for event in events {
        apply(app, event);
    }
}

// ---------- events ----------

/// Carries out one event of the tool or the overlay.
pub(crate) fn apply(app: &mut PhotocraftApp, event: SliceEvent) {
    match event {
        SliceEvent::Draw(EditEvent::Commit { after, .. } | EditEvent::Set { after, .. }) => draw(app, after),
        SliceEvent::Draw(_) => {}
        SliceEvent::Move { number, event, .. } | SliceEvent::Resize { number, event, .. } => edit_slice(app, number, event),
        SliceEvent::Select { number, .. } => app.ui.slices.selected = number,
        SliceEvent::Command(command) => run_command(app, command),
        SliceEvent::Refused(_) => {
            app.ui.slices.selected = None;
            app.ui.status = tl!("Slices are locked (View › Lock Slices)").to_string();
            app.ui.status_error = true;
        }
        _ => {}
    }
}

/// A new slice drawn to `rect`.
fn draw(app: &mut PhotocraftApp, rect: DocRect) {
    let (x, y) = (rect.x0.round(), rect.y0.round());
    let (w, h) = ((rect.x1 - rect.x0).round(), (rect.y1 - rect.y0).round());
    match app.run("slice.new", json!({"rect": [x, y, w, h]})) {
        Ok(v) => app.ui.slices.selected = v.get("number").and_then(Value::as_u64).map(|n| n as usize),
        Err(e) => report(app, e),
    }
}

/// One step of a Move or Resize of the slice `number`. The gesture's value is shown as the preview;
/// its end moves each edge of the stored slice by what the piece's edge moved.
fn edit_slice(app: &mut PhotocraftApp, number: usize, event: EditEvent<DocRect>) {
    match event {
        EditEvent::Begin { .. } => {}
        EditEvent::Update { value, .. } => app.ui.slices.preview = Some((number, value)),
        EditEvent::Cancel { .. } => app.ui.slices.preview = None,
        EditEvent::Commit { before, after, .. } | EditEvent::Set { before, after, .. } => {
            app.ui.slices.preview = None;
            commit_slice(app, number, before.same().copied(), after);
        }
        _ => {}
    }
}

/// Moves the stored slice that owns piece `number` by the change from `before` to `after`; a gesture
/// that moved no edge changes nothing.
fn commit_slice(app: &mut PhotocraftApp, number: usize, before: Option<DocRect>, after: DocRect) {
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return };
    let Some(piece) = slices::resolve(&doc).into_iter().find(|r| r.number == number) else { return };
    let Some(stored) = piece.id.and_then(|id| doc.slices.get(id)) else { return };
    let before = before.unwrap_or_else(|| to_doc_rect(piece.rect));
    let delta = |a: f64, b: f64| (a - b).round() as i32;
    let r = stored.rect;
    let moved = photocraft_geom::Rect::new(
        r.x0.saturating_add(delta(after.x0, before.x0)),
        r.y0.saturating_add(delta(after.y0, before.y0)),
        r.x1.saturating_add(delta(after.x1, before.x1)),
        r.y1.saturating_add(delta(after.y1, before.y1)),
    );
    if moved == r {
        return;
    }
    let id = stored.id;
    match app.run("slice.set", json!({"slice": id, "rect": [moved.x0, moved.y0, moved.width(), moved.height()]})) {
        Ok(_) => app.ui.slices.selected = number_of(app, id),
        Err(e) => report(app, e),
    }
}

/// The display number of the first piece of stored slice `id`.
fn number_of(app: &PhotocraftApp, id: u32) -> Option<usize> {
    let st = app.session.active()?;
    slices::resolve(&st.doc).into_iter().find(|r| r.id == Some(id)).map(|r| r.number)
}

fn report(app: &mut PhotocraftApp, error: String) {
    app.ui.status = error;
    app.ui.status_error = true;
}

// ---------- commands ----------

/// The selected slice as a command target.
fn selected_target(app: &PhotocraftApp) -> Option<SliceTarget> {
    let n = app.ui.slices.selected?;
    let r = resolved(app, n)?;
    Some(r.id.map_or(SliceTarget::Auto(n), SliceTarget::Stored))
}

fn resolved(app: &PhotocraftApp, number: usize) -> Option<ResolvedSlice> {
    slices::resolve(&app.session.active()?.doc).into_iter().find(|r| r.number == number)
}

/// Runs a slice command of the bar, the keyboard or a double click.
fn run_command(app: &mut PhotocraftApp, command: SliceCommand) {
    let result: Result<(), String> = match command {
        SliceCommand::Delete => match selected_target(app) {
            Some(SliceTarget::Stored(id)) => app.run("slice.delete", json!({"slice": id})).map(|_| app.ui.slices.selected = None),
            // An auto slice is made from the user slices around it: it goes when they do.
            _ => Ok(()),
        },
        SliceCommand::Promote => match selected_target(app) {
            Some(target) => app.run("slice.promote", target.params()).map(|_| ()),
            None => Ok(()),
        },
        SliceCommand::Divide => open_divide(app).map(|_| ()),
        SliceCommand::Options => open_options(app).map(|_| ()),
        SliceCommand::FromGuides => app.run("slice.fromGuides", json!({})).map(|_| ()),
        _ => Ok(()),
    };
    if let Err(e) = result {
        report(app, e);
    }
}

/// The state of each slice command: gated by the selected slice's origin, the lock and the document.
pub(crate) fn command_states(app: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    let origin = app.ui.slices.selected.and_then(|n| resolved(app, n)).map(|r| origin_of(r.origin));
    let locked = app.session.file_menu.slices_locked;
    let has_doc = app.session.active().is_some();
    [SliceCommand::Delete, SliceCommand::Options, SliceCommand::Promote, SliceCommand::Divide, SliceCommand::FromGuides]
        .into_iter()
        .map(|c| (CommandRef::from(c), CommandState::new().enabled(c.enabled(origin, locked, has_doc))))
        .collect()
}

/// Runs a fired slice command; whether it was one.
pub(crate) fn run_fired(app: &mut PhotocraftApp, _ctx: &egui::Context, command: &CommandRef) -> bool {
    let Some(command) = SliceCommand::from_ref(command) else { return false };
    run_command(app, command);
    true
}

// ---------- the options bar ----------

/// Options bars of the Slice and Slice Select tools; false for another tool.
pub fn options_bar(ui: &mut egui::Ui, app: &mut PhotocraftApp, tool: Tool) -> bool {
    if !is_slice_tool(tool) {
        return false;
    }
    let mode = if tool == Tool::SliceSelect { SliceMode::Select } else { SliceMode::Slice };
    let mut current = settings(app);
    let out = SliceBar::new(mode, &mut current).locked(app.session.file_menu.slices_locked).show(ui);
    if out.settings_changed {
        store(app, current);
    }
    true
}

// ---------- dialogs ----------

fn kind_id(kind: SliceKind) -> &'static str {
    match kind {
        SliceKind::Image => "image",
        SliceKind::NoImage => "noImage",
        SliceKind::HtmlText => "table",
        _ => "image",
    }
}

/// Opens Slice Options for the selected slice (an auto slice is promoted on OK). A refusal is the
/// `Err` text.
pub(crate) fn open_options(app: &mut PhotocraftApp) -> Result<DialogId, String> {
    let n = app.ui.slices.selected.ok_or("select a slice with the Slice Select tool")?;
    let st = app.session.active().ok_or("no document")?;
    let r = slices::resolve(&st.doc).into_iter().find(|r| r.number == n).ok_or("no such slice")?;
    let v = photocraft_engine::slice_cmds::resolved_json(&st.doc, &r);
    let text = |k: &str| v[k].as_str().unwrap_or("").to_string();
    let kind = match v["kind"].as_str() {
        Some("noImage") => SliceKind::NoImage,
        Some("table") => SliceKind::HtmlText,
        _ => SliceKind::Image,
    };
    // `SliceOptions` is non-exhaustive: start from its default and set every field.
    let mut options = SliceOptions::default();
    options.kind = kind;
    options.name = text("name");
    options.url = text("url");
    options.target = text("target");
    options.message = text("message");
    options.alt = text("alt");
    options.cell_text = text("cellText");
    options.background_row = true;
    options.background = v["background"].as_str().and_then(Srgba::from_hex);
    let target = r.id.map_or(SliceTarget::Auto(n), SliceTarget::Stored);
    // HTML Text is offered only for a Table slice, so its value shows.
    let mut kinds = vec![SliceKind::Image, SliceKind::NoImage];
    if kind == SliceKind::HtmlText {
        kinds.push(SliceKind::HtmlText);
    }
    let id = app.framework.dialogs.open(SliceOptionsDialog::new(options).kinds(&kinds));
    app.ui.slices.options = Some((id, target));
    Ok(id)
}

/// Opens Divide Slice for the selected slice. A refusal is the `Err` text.
pub(crate) fn open_divide(app: &mut PhotocraftApp) -> Result<DialogId, String> {
    let target = selected_target(app).ok_or("select a slice first")?;
    let id = app.framework.dialogs.open(DivideSlicesDialog::new(DivideSlices::new(Some(DivideBy::Count(2)), None)));
    app.ui.slices.divide = Some((id, target));
    Ok(id)
}

/// The result pump of the two dialogs: the Background colour request goes to the picker and its answer
/// back, and a finished dialog runs its engine command.
pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    take_options(app);
    take_divide(app);
}

fn take_options(app: &mut PhotocraftApp) {
    let Some((id, target)) = app.ui.slices.options else { return };
    if let Some(request) = app.framework.dialogs.get_mut_by_id::<SliceOptionsDialog>(id).and_then(SliceOptionsDialog::pick_color) {
        crate::color_picker_ui::open_for_request(app, id, request);
    }
    if let Some((key, color)) = crate::color_picker_ui::answer_for(app, id)
        && let Some(dialog) = app.framework.dialogs.get_mut_by_id::<SliceOptionsDialog>(id)
    {
        dialog.set_color(&key, color);
    }
    match app.framework.dialogs.take_by_id::<SliceOptionsDialog>(id) {
        Some(DialogResult::Ok(options)) => {
            app.ui.slices.options = None;
            let mut params = target.params();
            params["kind"] = json!(kind_id(options.kind));
            for (k, v) in [
                ("name", &options.name),
                ("url", &options.url),
                ("target", &options.target),
                ("message", &options.message),
                ("alt", &options.alt),
                ("cellText", &options.cell_text),
            ] {
                params[k] = json!(v);
            }
            params["background"] = match options.background {
                Some(c) => {
                    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                    json!(format!("#{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b)))
                }
                None => json!("none"),
            };
            match app.run("slice.set", params) {
                Ok(v) => app.ui.slices.selected = v.get("number").and_then(Value::as_u64).map(|n| n as usize),
                Err(e) => report(app, e),
            }
        }
        Some(DialogResult::Cancel) => app.ui.slices.options = None,
        Some(_) => {}
        None if app.framework.dialogs.get_by_id::<SliceOptionsDialog>(id).is_none() => app.ui.slices.options = None,
        None => {}
    }
}

fn take_divide(app: &mut PhotocraftApp) {
    let Some((id, target)) = app.ui.slices.divide else { return };
    match app.framework.dialogs.take_by_id::<DivideSlicesDialog>(id) {
        Some(DialogResult::Ok(divide)) => {
            app.ui.slices.divide = None;
            let mut params = target.params();
            for (by, count, size) in [(divide.rows, "horizontal", "horizontalSize"), (divide.columns, "vertical", "verticalSize")] {
                match by {
                    Some(DivideBy::Count(n)) => params[count] = json!(n),
                    Some(DivideBy::Size(px)) => params[size] = json!(px),
                    _ => {}
                }
            }
            if let Err(e) = app.run("slice.divide", params) {
                report(app, e);
            }
        }
        Some(DialogResult::Cancel) => app.ui.slices.divide = None,
        Some(_) => {}
        None if app.framework.dialogs.get_by_id::<DivideSlicesDialog>(id).is_none() => app.ui.slices.divide = None,
        None => {}
    }
}

#[cfg(test)]
mod tests;
