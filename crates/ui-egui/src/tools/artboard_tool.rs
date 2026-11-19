//! Artboard tool (Move group, V / ⇧V cycles with the Move tool): drag on an empty spot to draw a
//! new artboard, click a board to select it, drag it to move it with its contents, and drag the
//! selected board's edge and corner handles to resize it (⇧ on a corner keeps its proportions).
//! The "+" buttons beside the selected board's sides add a board of the same size there (⌥: a copy
//! with its contents). The options bar sets the selected board's size and switches on Add New
//! Artboard's placing mode (a click puts a board of the bar's size at the click).
//!
//! The boards, the handles and the "+" buttons are the framework's [`ArtboardOverlay`] and
//! [`AddBesideHandles`]; the pasteboard gestures are the framework's [`ArtboardTool`]. Both report
//! [`ArtboardEvent`]s, which [`apply`] turns into engine commands, so each gesture is one undo step.

// PLAN: the `craft::draw::ui::{artboard, add_beside}`, `craft::keymap::CommandRef` and
// `craft::ui::runtime::commands::CommandState` facade paths are assumed from the other `craft::` paths in this crate.

use craft::behavior::EditEvent;
use craft::behavior::kurbo::{Rect as DocRect, Size};
use craft::draw::ui::add_beside::{AddBesideHandles, AddSide};
use craft::draw::ui::artboard::{ArtboardBackground as Shown, ArtboardCommand, ArtboardEvent, ArtboardItem, ArtboardOverlay, ArtboardRules, ArtboardTool};
use craft::keymap::CommandRef;
use craft::theme::Srgba;
use craft::ui::canvas::ToolHostResponse;
use craft::ui::contract::Show as _;

use craft::ui::runtime::commands::CommandState;
use photocraft_doc::{ArtboardBackground, Layer, LayerId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::document_display::CanvasHost;
use crate::canvas::tools::{FamilyTool, ToolScope};
use crate::state::Tool;

pub(crate) mod bar;

/// Artboard tool state (serde: readable through the control channel). Every field but `adding` is
/// an option of the tool, which Reset Tool puts back to [`ArtboardToolUi::default`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArtboardToolUi {
    /// Add New Artboard's placing mode: a click on empty canvas puts a board there (#11386).
    pub adding: bool,
    /// The size of the next board, with no board selected.
    pub width: u32,
    pub height: u32,
    /// The background of a board the tool draws: `white`, `black`, `transparent` or `custom`.
    pub background: String,
    /// `#rrggbb` of the `custom` background.
    pub background_color: String,
    /// The gear menu's four entries.
    pub auto_nest: bool,
    pub auto_size_canvas: bool,
    pub keep_relative_position: bool,
    pub shrinkwrap_on_save: bool,
}

impl Default for ArtboardToolUi {
    fn default() -> Self {
        let (_, width, height) = bar::DEFAULT_SIZE;
        Self {
            adding: false,
            width,
            height,
            background: "white".to_string(),
            background_color: "#ffffff".to_string(),
            auto_nest: true,
            auto_size_canvas: true,
            keep_relative_position: true,
            shrinkwrap_on_save: true,
        }
    }
}

fn edges(r: photocraft_geom::Rect) -> [i64; 4] {
    [i64::from(r.x0), i64::from(r.y0), i64::from(r.x1), i64::from(r.y1)]
}

/// `[x, y, width, height]` for the engine's `rect` param.
fn xywh(r: [i64; 4]) -> [i64; 4] {
    [r[0], r[1], r[2].saturating_sub(r[0]), r[3].saturating_sub(r[1])]
}

/// A framework pixel rect as `[x0, y0, x1, y1]`.
fn pixel_edges(r: DocRect) -> [i64; 4] {
    [r.x0.round() as i64, r.y0.round() as i64, r.x1.round() as i64, r.y1.round() as i64]
}

/// The selected board: the active layer's artboard, with its edges.
fn active_board(app: &PhotocraftApp) -> Option<(LayerId, [i64; 4])> {
    let st = app.session.active()?;
    let id = st.doc.artboard_of(st.active_layer?)?;
    Some((id, edges(st.doc.layer(id).and_then(Layer::artboard)?.rect)))
}

fn report(app: &mut PhotocraftApp, r: Result<Value, String>) {
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The document's boards as the framework sees them in one pass.
#[derive(Default)]
pub(crate) struct Boards {
    /// Every board, in document order.
    pub(crate) items: Vec<ArtboardItem>,
    /// The selected board's index (Photoshop's rules select one).
    pub(crate) selected: Vec<usize>,
    /// The layer of each item.
    pub(crate) ids: Vec<LayerId>,
}

/// The framework's background of a document board.
fn shown_of(b: ArtboardBackground) -> Shown {
    match b {
        ArtboardBackground::Black => Shown::Black,
        ArtboardBackground::Transparent => Shown::Transparent,
        ArtboardBackground::Custom(c) => {
            let [r, g, b, a] = c.to_rgba8();
            Shown::Custom(Srgba::new(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, f32::from(a) / 255.0))
        }
        _ => Shown::White,
    }
}

/// The boards, rebuilt from the document each pass.
pub(crate) fn items(app: &PhotocraftApp) -> Boards {
    let mut out = Boards::default();
    let Some(st) = app.session.active() else { return out };
    let active = active_board(app).map(|b| b.0);
    for b in st.doc.artboards().iter() {
        let id = b.0;
        let r = b.2.rect;
        let name = st.doc.layer(id).map(|l| l.name.clone()).unwrap_or_default();
        let rect = DocRect::new(f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1));
        let mut item = ArtboardItem::new(name, rect);
        item.background = shown_of(b.2.background);
        if Some(id) == active {
            out.selected = vec![out.items.len()];
        }
        out.items.push(item);
        out.ids.push(id);
    }
    out
}

/// The size a click places while Add New Artboard's mode is on.
fn placing_size(app: &PhotocraftApp) -> Option<Size> {
    app.ui.artboard_tool.adding.then(|| {
        let [w, h] = bar::bar_size(app);
        Size::new(w as f64, h as f64)
    })
}

fn configure(state: &mut ArtboardTool, boards: &Boards, placing: Option<Size>) {
    state.items(&boards.items).selected(&boards.selected).rules(ArtboardRules::PHOTOSHOP).placing(placing);
}

/// The Artboard family's tool for this pass. The borrowed state is a `CanvasTool` through the framework's
/// `impl CanvasTool for &mut T`.
pub(crate) fn tool<'a>(scope: &'a ToolScope<'a>, state: &'a mut ArtboardTool, _host: CanvasHost) -> FamilyTool<'a> {
    let (boards, placing) = scope.with_app(|app| (items(app), placing_size(app)));
    configure(state, &boards, placing);
    Box::new(state)
}

/// Takes the tool's events after the host ran.
pub(crate) fn drain(app: &mut PhotocraftApp, state: &mut ArtboardTool, _response: &ToolHostResponse) {
    let events = state.take_events();
    apply(app, events);
}

/// The overlay over the boards and the "+" buttons over the selected one, shown in the Canvas body before the tool
/// runs. Its events are applied here; a "+" answer goes to the tool, which reports it with the tool's own events.
pub(crate) fn show_overlay(app: &mut PhotocraftApp, ui: &mut egui::Ui, rect: egui::Rect) {
    if app.ui.tool != Tool::Artboard || app.session.active().is_none() {
        return;
    }
    let boards = items(app);
    let overlay = ArtboardOverlay::new(tl!("Artboards"), &boards.items, &boards.selected).rules(ArtboardRules::PHOTOSHOP).show_at(ui, rect);
    let added = boards.selected.first().filter(|_| !overlay.dragging).and_then(|&i| boards.items.get(i)).and_then(|item| {
        let shown = AddBesideHandles::new(tl!("Add artboard"), item.rect.abs()).show_at(ui, rect);
        shown.added.map(|side| (side, shown.copy))
    });
    if let Some((side, copy)) = added {
        // The tool names the board the button belongs to from its own snapshot, so give it this pass's.
        let placing = placing_size(app);
        configure(&mut app.canvas_tools.artboard, &boards, placing);
        app.canvas_tools.artboard.add_beside(side, copy);
    }
    apply(app, overlay.events);
}

/// The `layer.artboard.addBeside` name of a side.
fn side_name(side: AddSide) -> &'static str {
    match side {
        AddSide::Top => "top",
        AddSide::Right => "right",
        AddSide::Bottom => "bottom",
        AddSide::Left | _ => "left",
    }
}

/// The value a finished edit ends with: a commit, or a value that arrived whole. Begin, Update and Cancel change
/// nothing (the overlay shows the drag).
fn finished<T: Copy>(event: &EditEvent<T>) -> Option<T> {
    match event {
        EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => Some(*after),
        _ => None,
    }
}

/// Turns the framework's events into engine commands, one undo step each.
pub(crate) fn apply(app: &mut PhotocraftApp, events: Vec<ArtboardEvent>) {
    for event in events {
        let boards = items(app);
        let id_of = |i: usize| boards.ids.get(i).copied();
        let result = match event {
            ArtboardEvent::Draw(edit) => {
                let Some(rect) = finished(&edit).map(pixel_edges).filter(|r| r[2] > r[0] && r[3] > r[1]) else { continue };
                let mut params = bar::with_background(app, json!({"rect": xywh(rect)}));
                params["takeBackground"] = json!(true);
                app.run("layer.new.artboard", params)
            }
            ArtboardEvent::Place { rect, .. } => {
                if app.session.active().is_none() {
                    continue;
                }
                let r = xywh(pixel_edges(rect));
                let mut params = json!({"rect": r});
                // With no board selected the size is the tool option's, so the board keeps its preset name (the engine
                // stores it, and with an explicit `rect` the rect still decides the size).
                let preset = bar::preset_name(r[2], r[3]);
                if !preset.is_empty() && active_board(app).is_none() {
                    params["preset"] = json!(preset);
                }
                app.run("layer.new.artboard", bar::with_background(app, params))
            }
            ArtboardEvent::Move { indices, event, .. } => {
                let Some(by) = finished(&event) else { continue };
                let (dx, dy) = (by.x.round() as i64, by.y.round() as i64);
                let mut last = Ok(json!({}));
                for index in indices {
                    let (Some(id), Some(item)) = (id_of(index), boards.items.get(index)) else { continue };
                    let at = pixel_edges(item.rect);
                    if (dx, dy) == (0, 0) {
                        continue;
                    }
                    last = app.run("layer.artboard.set", json!({"layer": id.0, "x": at[0].saturating_add(dx), "y": at[1].saturating_add(dy)}));
                    if last.is_err() {
                        break;
                    }
                }
                last
            }
            ArtboardEvent::Resize { index, event, .. } => {
                let (Some(id), Some(rect)) = (id_of(index), finished(&event).map(pixel_edges)) else { continue };
                // Resizing leaves the contents where they are, as in Photoshop.
                app.run("layer.artboard.set", json!({"layer": id.0, "rect": xywh(rect), "moveContents": false}))
            }
            ArtboardEvent::AddBeside { index, side, copy, .. } => {
                let Some(id) = id_of(index) else { continue };
                app.run("layer.artboard.addBeside", json!({"layer": id.0, "side": side_name(side), "duplicate": copy}))
            }
            ArtboardEvent::Select { indices, .. } => {
                let Some(id) = indices.first().copied().and_then(id_of) else { continue };
                if active_board(app).map(|b| b.0) == Some(id) {
                    continue;
                }
                app.run("layer.select", json!({"layer": id.0}))
            }
            // PLAN: assumed `ArtboardEvent::Delete { indices, .. }` (the tool's Delete key, S-T5).
            ArtboardEvent::Delete { indices, .. } => {
                let Some(id) = indices.first().copied().and_then(id_of) else { continue };
                app.run("layer.delete", json!({"layer": id.0}))
            }
            _ => continue,
        };
        report(app, result);
    }
}

/// The states of the Artboard tool's commands: Add New Artboard is checked while the placing mode is on.
pub(crate) fn command_states(app: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    // PLAN: assumed `From<ArtboardCommand> for CommandRef` (the command derive).
    let state = CommandState::new().enabled(app.session.active().is_some()).checked(app.ui.artboard_tool.adding);
    vec![(ArtboardCommand::AddNew.into(), state)]
}

/// What [`command_states`] reads: whether a document is open, and the placing mode.
pub(crate) fn command_states_key(app: &PhotocraftApp) -> crate::app::framework::StatesKey {
    [u64::from(app.session.active().is_some()), u64::from(app.ui.artboard_tool.adding), 0, 0]
}

/// Runs a fired Artboard command: Add New Artboard flips the placing mode and adds no board. `ctx` is the shape every
/// command source takes.
pub(crate) fn run_fired(app: &mut PhotocraftApp, _ctx: &egui::Context, command: &CommandRef) -> bool {
    let add_new: CommandRef = ArtboardCommand::AddNew.into();
    if *command != add_new {
        return false;
    }
    app.ui.artboard_tool.adding = !app.ui.artboard_tool.adding;
    true
}

/// The Artboard tool's options bar: the bar's members, then the gear menu. Add New Artboard and the Align menu are
/// members of the framework bar.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if tool != Tool::Artboard {
        return false;
    }
    bar::show(app, ui);
    bar::gear(app, ui);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{ToolEvent, tool_event};

    fn app() -> Box<PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.sync_views();
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        app.ui.tool = Tool::Artboard;
        app
    }

    fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
        let m = egui::Modifiers::NONE;
        tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
        tool_event(app, ToolEvent::Move { x: (from[0] + to[0]) / 2.0, y: (from[1] + to[1]) / 2.0, pressure: 1.0 }, m);
        tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
        tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
    }

    fn boards(app: &PhotocraftApp) -> Vec<photocraft_geom::Rect> {
        app.session.active().unwrap().doc.artboards().iter().map(|b| b.2.rect).collect()
    }

    fn steps(app: &PhotocraftApp) -> usize {
        app.session.active().unwrap().history.past_len()
    }

    /// Drawing on empty canvas makes a board; its right-edge handle resizes it in one undo step;
    /// dragging it from inside moves it in one.
    #[test]
    fn artboard_tool_draws_resizes_and_moves_boards() {
        let mut app = app();
        let s0 = steps(&app);
        drag(&mut app, [20.0, 30.0], [120.0, 110.0]);
        assert_eq!(boards(&app), vec![photocraft_geom::Rect::new(20, 30, 120, 110)]);
        assert_eq!(steps(&app), s0 + 1);
        let st = app.session.active().unwrap();
        assert_eq!(st.active_layer, Some(st.doc.artboards()[0].0));
        // The right edge's handle.
        drag(&mut app, [120.0, 70.0], [160.0, 75.0]);
        assert_eq!(boards(&app), vec![photocraft_geom::Rect::new(20, 30, 160, 110)]);
        assert_eq!(steps(&app), s0 + 2);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(boards(&app), vec![photocraft_geom::Rect::new(20, 30, 120, 110)]);
        // From inside: the board moves.
        drag(&mut app, [60.0, 60.0], [70.0, 50.0]);
        assert_eq!(boards(&app), vec![photocraft_geom::Rect::new(30, 20, 130, 100)]);
        // A click on empty canvas draws nothing.
        drag(&mut app, [300.0, 250.0], [300.0, 250.0]);
        assert_eq!(boards(&app).len(), 1);
    }

    /// A "+" answer gives `layer.artboard.addBeside` for the side, in one undo step.
    #[test]
    fn a_plus_answer_adds_a_board_beside_the_selected_one() {
        let mut app = app();
        drag(&mut app, [20.0, 30.0], [120.0, 80.0]);
        let b = items(&app);
        configure(&mut app.canvas_tools.artboard, &b, None);
        app.canvas_tools.artboard.add_beside(AddSide::Right, false);
        let events = app.canvas_tools.artboard.take_events();
        let s0 = steps(&app);
        apply(&mut app, events);
        assert_eq!(boards(&app), vec![photocraft_geom::Rect::new(20, 30, 120, 80), photocraft_geom::Rect::new(220, 30, 320, 80)]);
        assert_eq!(steps(&app), s0 + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(boards(&app).len(), 1);
    }
}
