use super::*;
use craft::keymap::Mods;
use craft::ui::dialog::DialogHost;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::{Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};

pub(super) fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app
}

fn work_path(app: &mut PhotocraftApp) {
    app.run(
        "path.set",
        json!({"name": "work", "path": {"subpaths": [{"closed": true, "knots": [
            {"anchor": [2, 2], "in": [2, 2], "out": [2, 2]},
            {"anchor": [20, 2], "in": [20, 2], "out": [20, 2]},
            {"anchor": [20, 20], "in": [20, 20], "out": [20, 20]}
        ]}]}}),
    )
    .unwrap();
}

/// Opens the menu of `tool` at a fixed screen point for the document point `doc`, as a right-click would.
pub(super) fn open_here(app: &mut PhotocraftApp, tool: Tool, doc: [f64; 2]) -> bool {
    open_at(app, tool, Mods::NONE, pos2(10.0, 10.0), doc, None)
}

pub(super) fn choose_open(app: &mut PhotocraftApp, command: &str) {
    assert!(open_here(app, Tool::Pen, [10.0, 10.0]));
    pick(app, command);
}

/// The labels of `rows`, a separator as `None`.
pub(super) fn labels(rows: &[Row]) -> Vec<Option<&str>> {
    rows.iter().map(|row| row.map(|(label, _)| label)).collect()
}

/// `choose` on the app's own context.
pub(super) fn pick(app: &mut PhotocraftApp, command: &str) {
    let ctx = app.ctx().clone();
    choose(app, &ctx, command);
}

/// Whether a hosted parameter dialog is open.
pub(super) fn dialog_open(app: &PhotocraftApp) -> bool {
    app.framework.dialogs.is_open::<HostedParamDialog>()
}

/// Whether any dialog of the dialog host is open.
pub(super) fn any_dialog(app: &PhotocraftApp) -> bool {
    !app.framework.dialogs.is_empty()
}

/// Closes every open dialog.
pub(super) fn close_dialogs(app: &mut PhotocraftApp) {
    app.framework.dialogs = DialogHost::new();
}

/// The open hosted parameter dialog.
pub(super) fn hosted(app: &mut PhotocraftApp) -> &mut HostedParamDialog {
    app.framework.dialogs.get_mut::<HostedParamDialog>().expect("a hosted parameter dialog is open")
}

/// Presses OK on the open hosted parameter dialog through the app's dialog path: a family that judges a held OK
/// answers it, then its run takes the result.
pub(super) fn confirm_hosted(app: &mut PhotocraftApp, ctx: &Context) {
    try_confirm_hosted(app, ctx).expect("OK was accepted");
}

/// [`confirm_hosted`] for a run that may be refused: the result of the dialog's OK.
pub(super) fn try_confirm_hosted(app: &mut PhotocraftApp, ctx: &Context) -> Result<(), String> {
    let id = app
        .framework
        .dialogs
        .stack()
        .ids()
        .find(|id| app.framework.dialogs.get_by_id::<HostedParamDialog>(*id).is_some())
        .expect("a hosted parameter dialog is open");
    crate::dialogs::confirm(app, ctx, id)
}

/// Pen clicks at document points `at`, as on the canvas.
pub(super) fn pen_clicks(app: &mut PhotocraftApp, at: &[(f64, f64)]) {
    app.ui.tool = Tool::Pen;
    for &(x, y) in at {
        crate::canvas::testing::click(app, [x, y], Modifiers::NONE);
    }
}

pub(crate) fn harness(mut app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    app.sync_views();
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_context(app.ctx().clone()).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::default());
    h.run_steps(4);
    h
}

pub(crate) fn right_click(h: &mut Harness<'static, Box<PhotocraftApp>>, p: Pos2, modifiers: Modifiers) {
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers });
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

/// The right button goes down at `at` with `mods` held and comes up as `gesture` says.
fn secondary_press(h: &mut Harness<'static, Box<PhotocraftApp>>, at: Pos2, mods: Modifiers, gesture: Gesture) {
    let by = match gesture {
        Gesture::Still => return right_click(h, at, mods),
        Gesture::Moved => vec2(7.0, 3.0),
        Gesture::OneFrame => vec2(14.0, 6.0),
        Gesture::Drag => vec2(14.0, 7.0),
    };
    let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: mods };
    h.event(egui::Event::ModifiersChanged(mods));
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    if gesture == Gesture::OneFrame {
        // `Harness::event` runs a frame per event, so the three go straight into one frame's input.
        h.input_mut().events.extend([button(at, true), egui::Event::PointerMoved(at + by), button(at + by, false)]);
    } else {
        h.event(button(at, true));
        h.run_steps(1);
        for i in 1..=4 {
            h.event(egui::Event::PointerMoved(at + by * (i as f32 / 4.0)));
            h.run_steps(1);
        }
        h.event(button(at + by, false));
    }
    h.run_steps(2);
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
}

pub(super) fn screen_of(app: &PhotocraftApp, doc: [f32; 2]) -> Pos2 {
    crate::canvas::testing::to_screen(app, [f64::from(doc[0]), f64::from(doc[1])])
}

/// What a right-click must leave as it was. The journal counts only strokes: the type edit menu reads its spelling
/// suggestions through `edit.checkSpelling`'s `list`, a journaled read.
fn document_state(app: &PhotocraftApp) -> impl PartialEq + use<> {
    let state = app.session.active().unwrap();
    let strokes = app.session.journal.iter().filter(|(id, _)| id == "paint.stroke").count();
    (state.doc.clone(), state.revision, state.history.past_len(), state.active_layer, state.selected_layers(), strokes)
}

fn view_of(app: &PhotocraftApp) -> Option<(craft::behavior::kurbo::Point, f64)> {
    crate::canvas::document_display::active_view(app).map(|v| (v.center, v.zoom))
}

/// A type layer reading "Hello", being edited with every character selected.
fn editing_type(app: &mut PhotocraftApp) -> u64 {
    app.ui.tool = Tool::Type;
    let id = app.run("type.create", json!({"text": "Hello", "size": 12, "x": 4, "y": 20})).unwrap()["layer"].as_u64().unwrap();
    let ctx = app.ctx().clone();
    crate::menus::invoke(app, &ctx, "type.editText", json!({})).unwrap();
    id
}

fn text_of(app: &PhotocraftApp, id: u64) -> Option<photocraft_doc::TextLayer> {
    match &app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
        LayerContent::Text(t) => Some(t.clone()),
        _ => None,
    }
}

/// A committed type layer reading "Hello" over the middle of the canvas, no edit running.
fn committed_type(app: &mut PhotocraftApp) -> u64 {
    app.ui.tool = Tool::Type;
    let id = app.run("type.create", json!({"text": "Hello", "size": 14, "x": 1, "y": 22})).unwrap()["layer"].as_u64().unwrap();
    assert!(app.ui.text_edit.is_none());
    id
}

/// A 32 × 32 document with the Crop tool picked (Classic Mode, so the view stays put) and its
/// untouched default frame.
fn crop_app() -> Box<PhotocraftApp> {
    let mut app = app();
    app.ui.tool = Tool::Crop;
    app.ui.tool_options.crop.shield.classic_mode = true;
    app.ui.extras.snap = false;
    app
}

/// Draws a crop frame from (4, 4) to (28, 28): a pending crop. Returns the frame.
fn draw_crop(app: &mut PhotocraftApp) -> [f64; 4] {
    use crate::canvas::{ToolEvent, tool_event};
    tool_event(app, ToolEvent::Down { x: 4.0, y: 4.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: 16.0, y: 16.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: 28.0, y: 28.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 28.0, y: 28.0 }, Modifiers::NONE);
    assert!(crate::crop_ui::pending(app), "the drag draws a pending frame");
    frame_of(app).unwrap()
}

/// The Crop tool's frame as `[x0, y0, x1, y1]`, `None` while it shows none.
fn frame_of(app: &PhotocraftApp) -> Option<[f64; 4]> {
    crate::crop_ui::frame(app).map(|f| [f.rect.x0, f.rect.y0, f.rect.x1, f.rect.y1])
}

fn aspect(r: [f64; 4]) -> f64 {
    (r[2] - r[0]) / (r[3] - r[1])
}

// ---- the routing table ----

#[derive(Clone, Copy, PartialEq)]
enum Place {
    Image,
    Pasteboard,
    Guide,
}

#[derive(Clone, Copy, PartialEq)]
enum Held {
    Nothing,
    Space,
    CmdSpace,
    AltSpace,
}

/// How the right button goes down and comes up. `Moved` and `Drag` travel a few pixels over several frames
/// (a high-DPI mouse's click; a drag within a right-click's reach); `OneFrame` puts press, move and release into one
/// frame, as a slow frame delivers them.
#[derive(Clone, Copy, PartialEq)]
enum Gesture {
    Still,
    Moved,
    OneFrame,
    Drag,
}

#[derive(Clone, Copy)]
enum Setup {
    Plain,
    Zoomed,
    Selection,
    WorkPath,
    TextEdit,
    CommittedType,
    UntouchedCrop,
    PendingCrop,
    Guide,
}

#[derive(Clone, Copy, Debug)]
enum Want {
    Nothing,
    Menu(&'static str),
    Picker,
    /// The brush mode menu opens, and its Edit Brush row opens the brush picker.
    EditBrush,
}

struct Case {
    name: &'static str,
    tool: Tool,
    mods: Modifiers,
    place: Place,
    setup: Setup,
    held: Held,
    want: Want,
    gesture: Gesture,
}

const fn row(name: &'static str, tool: Tool, mods: Modifiers, place: Place, setup: Setup, held: Held, want: Want) -> Case {
    Case { name, tool, mods, place, setup, held, want, gesture: Gesture::Still }
}

impl Case {
    const fn by(mut self, gesture: Gesture) -> Case {
        self.gesture = gesture;
        self
    }
}

const NO: Modifiers = Modifiers::NONE;

const ROWS: &[Case] = &[
    row("selection tool: its selection menu", Tool::Lasso, NO, Place::Image, Setup::Selection, Held::Nothing, Want::Menu("Tool")),
    row("command right-click is the layer picker", Tool::Lasso, Modifiers::COMMAND, Place::Image, Setup::Selection, Held::Nothing, Want::Menu("Layers")),
    row("quick selection: selection menu, no picker", Tool::QuickSelection, NO, Place::Image, Setup::Selection, Held::Nothing, Want::Menu("Tool")),
    row("type edit", Tool::Type, NO, Place::Image, Setup::TextEdit, Held::Nothing, Want::Menu("TypeEdit")),
    row("committed type", Tool::Type, NO, Place::Image, Setup::CommittedType, Held::Nothing, Want::Menu("TypeLayer")),
    row("marquee outside the image", Tool::RectMarquee, NO, Place::Pasteboard, Setup::Plain, Held::Nothing, Want::Menu("Pasteboard")),
    row("zoom: view menu", Tool::Zoom, NO, Place::Image, Setup::Plain, Held::Nothing, Want::Menu("View")),
    row("hand: view menu", Tool::Hand, NO, Place::Image, Setup::Plain, Held::Nothing, Want::Menu("View")),
    row("zoom outside the image", Tool::Zoom, NO, Place::Pasteboard, Setup::Plain, Held::Nothing, Want::Menu("Pasteboard")),
    row("hand outside the image", Tool::Hand, NO, Place::Pasteboard, Setup::Plain, Held::Nothing, Want::Menu("Pasteboard")),
    row("hand with Space held", Tool::Hand, NO, Place::Image, Setup::Plain, Held::Space, Want::Menu("View")),
    row("brush with Cmd+Space held", Tool::Brush, Modifiers::COMMAND, Place::Image, Setup::Plain, Held::CmdSpace, Want::Menu("Layers")),
    row("brush with Alt+Space held", Tool::Brush, Modifiers::ALT, Place::Image, Setup::Plain, Held::AltSpace, Want::Nothing),
    row("hand with Cmd+Space held", Tool::Hand, Modifiers::COMMAND, Place::Image, Setup::Plain, Held::CmdSpace, Want::Menu("Layers")),
    row("hand with Alt+Space held", Tool::Hand, Modifiers::ALT, Place::Image, Setup::Plain, Held::AltSpace, Want::Nothing),
    row("brush: picker", Tool::Brush, NO, Place::Image, Setup::Plain, Held::Nothing, Want::Picker),
    row("eraser: picker", Tool::Eraser, NO, Place::Image, Setup::Plain, Held::Nothing, Want::Picker),
    row("brush outside the image keeps its picker", Tool::Brush, NO, Place::Pasteboard, Setup::Plain, Held::Nothing, Want::Picker),
    row("brush: a press that moved still opens the picker", Tool::Brush, NO, Place::Image, Setup::Plain, Held::Nothing, Want::Picker).by(Gesture::Moved),
    row("brush: a release in the press's frame still opens the picker", Tool::Brush, NO, Place::Image, Setup::Plain, Held::Nothing, Want::Picker)
        .by(Gesture::OneFrame),
    row("brush with Shift: Edit Brush opens the picker", Tool::Brush, Modifiers::SHIFT, Place::Image, Setup::Plain, Held::Nothing, Want::EditBrush),
    row(
        "hand: a right drag within a click's reach opens the view menu and pans nothing",
        Tool::Hand,
        NO,
        Place::Image,
        Setup::Zoomed,
        Held::Nothing,
        Want::Menu("View"),
    )
    .by(Gesture::Drag),
    row("brush with Cmd: layer picker", Tool::Brush, Modifiers::COMMAND, Place::Image, Setup::Plain, Held::Nothing, Want::Menu("Layers")),
    row("move: layer picker", Tool::Move, NO, Place::Image, Setup::Plain, Held::Nothing, Want::Menu("Layers")),
    row("move on a guide: guide menu", Tool::Move, NO, Place::Guide, Setup::Guide, Held::Nothing, Want::Menu("Guide")),
    row("move on a guide: a moved right-click opens the guide menu", Tool::Move, NO, Place::Guide, Setup::Guide, Held::Nothing, Want::Menu("Guide"))
        .by(Gesture::Moved),
    row("move on a guide: a one-frame right-click opens the guide menu", Tool::Move, NO, Place::Guide, Setup::Guide, Held::Nothing, Want::Menu("Guide"))
        .by(Gesture::OneFrame),
    row("pen: path menu", Tool::Pen, NO, Place::Image, Setup::Plain, Held::Nothing, Want::Menu("Tool")),
    row("path selection: path menu", Tool::PathSelection, NO, Place::Image, Setup::WorkPath, Held::Nothing, Want::Menu("Tool")),
    row("direct selection: path menu", Tool::DirectSelection, NO, Place::Image, Setup::WorkPath, Held::Nothing, Want::Menu("Tool")),
    row("convert point: path menu", Tool::ConvertPoint, NO, Place::Image, Setup::WorkPath, Held::Nothing, Want::Menu("Tool")),
    row("untouched crop frame: no menu", Tool::Crop, NO, Place::Image, Setup::UntouchedCrop, Held::Nothing, Want::Nothing),
    row("untouched crop frame, outside: pasteboard", Tool::Crop, NO, Place::Pasteboard, Setup::UntouchedCrop, Held::Nothing, Want::Menu("Pasteboard")),
    row("pending crop: crop menu", Tool::Crop, NO, Place::Image, Setup::PendingCrop, Held::Nothing, Want::Menu("Crop")),
    row("pending crop outside the image wins over the pasteboard", Tool::Crop, NO, Place::Pasteboard, Setup::PendingCrop, Held::Nothing, Want::Menu("Crop")),
    row("count keeps its menu", Tool::Count, NO, Place::Image, Setup::Plain, Held::Nothing, Want::Menu("Tool")),
    row("content-aware move without a selection", Tool::ContentAwareMove, NO, Place::Image, Setup::Plain, Held::Nothing, Want::Menu("Tool")),
    row("content-aware move outside the selection", Tool::ContentAwareMove, NO, Place::Image, Setup::Selection, Held::Nothing, Want::Menu("Tool")),
    row("red eye: layer menu", Tool::RedEye, NO, Place::Image, Setup::Plain, Held::Nothing, Want::Menu("Tool")),
    row("red eye outside the image", Tool::RedEye, NO, Place::Pasteboard, Setup::Plain, Held::Nothing, Want::Menu("Pasteboard")),
    row("a tool without a canvas menu", Tool::Note, NO, Place::Image, Setup::Plain, Held::Nothing, Want::Nothing),
];

fn kind_name(kind: &MenuKind) -> &'static str {
    match kind {
        MenuKind::Tool => "Tool",
        MenuKind::Pasteboard => "Pasteboard",
        MenuKind::View => "View",
        MenuKind::Guide => "Guide",
        MenuKind::Crop { .. } => "Crop",
        MenuKind::TypeEdit { .. } => "TypeEdit",
        MenuKind::TypeLayer { .. } => "TypeLayer",
        MenuKind::Layers { .. } => "Layers",
        MenuKind::BrushMode { .. } => "BrushMode",
    }
}

fn row_app(row: &Case) -> Box<PhotocraftApp> {
    let mut app = if matches!(row.setup, Setup::UntouchedCrop | Setup::PendingCrop) { crop_app() } else { self::app() };
    match row.setup {
        // Zoomed needs a canvas to run its view command: `context_popup_table` zooms once the canvas shows.
        Setup::Plain | Setup::UntouchedCrop | Setup::Zoomed => {}
        Setup::Selection => {
            app.run("select.rect", json!({"x": 1, "y": 1, "width": 8, "height": 8})).unwrap();
        }
        Setup::WorkPath => {
            work_path(&mut app);
            app.ui.selected_path = Some("work".into());
        }
        Setup::TextEdit => {
            editing_type(&mut app);
        }
        Setup::CommittedType => {
            committed_type(&mut app);
        }
        Setup::PendingCrop => {
            draw_crop(&mut app);
        }
        Setup::Guide => {
            app.run("view.newGuide", json!({"orientation": "vertical", "position": 16})).unwrap();
        }
    }
    app.ui.tool = row.tool;
    app
}

/// Each right-click, by tool, modifiers, place and state, opens the menu the routing in `context_popup` picks
/// (or none), and changes neither the document nor the view.
#[test]
fn context_popup_table() {
    for row in ROWS {
        let mut h = harness(row_app(row));
        if matches!(row.setup, Setup::Zoomed) {
            // A view a pan would move: zoomed in through View › 200%, as a user does; the canvas runs it in its next pass.
            let ctx = h.ctx.clone();
            crate::menus::invoke(h.state_mut(), &ctx, "view.twoHundredPercent", json!({})).unwrap();
            // The pass that applies it, then the zoom's ease to its end.
            h.run_steps(3);
        }
        if row.held != Held::Nothing {
            h.event(egui::Event::ModifiersChanged(row.mods));
            h.event(egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed: true, repeat: false, modifiers: row.mods });
            h.step();
        }
        let rect = h.state().last_canvas_rect;
        let at = match row.place {
            Place::Image => rect.center(),
            Place::Pasteboard => rect.min + vec2(5.0, 5.0),
            Place::Guide => screen_of(h.state(), [16.0, 8.0]),
        };
        let before = document_state(h.state());
        let view = view_of(h.state());
        secondary_press(&mut h, at, row.mods, row.gesture);
        let menu = h.state().ui.canvas_tool_menu.as_ref().map(|m| (kind_name(&m.kind), m.tool));
        let picker = h.state().ui.brush_picker.is_some();
        match row.want {
            Want::EditBrush => {
                assert!(!picker, "{}: no picker beside the mode menu", row.name);
                assert_eq!(menu.map(|(kind, _)| kind), Some("BrushMode"), "{}", row.name);
                let journal = h.state().session.journal.len();
                h.get_by_label("Edit Brush…").click();
                h.run_steps(2);
                assert!(h.state().ui.canvas_tool_menu.is_none(), "{}: choosing the row closes the menu", row.name);
                assert!(h.state().ui.brush_picker.is_some(), "{}: the picker opens", row.name);
                assert_eq!(h.state().session.journal.len(), journal, "{}: the row runs no command", row.name);
                h.run_steps(2);
                assert!(h.state().ui.brush_picker.is_some(), "{}: the picker stays open", row.name);
            }
            Want::Nothing => assert!(menu.is_none() && !picker, "{}: no popup, got {menu:?} picker {picker}", row.name),
            Want::Picker => assert!(menu.is_none() && picker, "{}: the brush picker, got {menu:?}", row.name),
            Want::Menu(kind) => {
                assert!(!picker, "{}: no picker beside the menu", row.name);
                let (got, tool) = menu.unwrap_or_else(|| panic!("{}: right-click opens no menu", row.name));
                assert_eq!(got, kind, "{}", row.name);
                if got == "Tool" {
                    assert_eq!(tool, row.tool, "{}: the menu is the tool's", row.name);
                }
            }
        }
        assert_eq!(h.state().ui.tool, row.tool, "{}: the tool stays", row.name);
        assert!(document_state(h.state()) == before, "{}: the right-click edits nothing", row.name);
        assert_eq!(view_of(h.state()), view, "{}: the view stays", row.name);
    }
    // A point that is not finite, or a tool without a menu, opens nothing.
    let mut app = app();
    assert!(!open_here(&mut app, Tool::Note, [1.0, 1.0]));
    assert!(!open_here(&mut app, Tool::Lasso, [f64::NAN, 1.0]));
    assert!(!open_here(&mut app, Tool::Zoom, [f64::INFINITY, 1.0]));
    assert!(app.ui.canvas_tool_menu.is_none());
    assert!(open_here(&mut app, Tool::Zoom, [1.0, 1.0]));
}

// ---- the rows ----

/// Whether `id` is a row id the menu itself handles (`engine_call` maps it).
fn menu_local(id: &str) -> bool {
    [GUIDE_EDIT, GUIDE_CLEAR, FAUX_BOLD, FAUX_ITALIC, PASTEBOARD_PICK].contains(&id)
        || ["pasteboard.", "crop.", "canvasMenu.eyedropper.size.", "colorSampler.readout."].iter().any(|p| id.starts_with(p))
        || id.starts_with(SPELLING_REPLACE)
        || id == SPELLING_ADD
        || [EYEDROPPER_COPY_HTML, EYEDROPPER_COPY_HEX].contains(&id)
}

/// A renamed command would leave a dead row unnoticed: every row runs a menu item, an engine command or
/// a command of the runtime or of the menu itself.
#[test]
fn every_canvas_menu_row_runs_a_known_command() {
    let app = app();
    let items = crate::menus::menu_rows(&app);
    type Table<'a> = (&'a str, &'a [Option<(&'a str, &'a str)>]);
    let tables: [Table<'_>; 16] = [
        ("SELECTION_MENU", SELECTION_MENU),
        ("NO_SELECTION_MENU", NO_SELECTION_MENU),
        ("MEASURED_SELECTION_MENU", MEASURED_SELECTION_MENU),
        ("COUNT_MENU", COUNT_MENU),
        ("CAM_INSIDE_SELECTION_MENU", CAM_INSIDE_SELECTION_MENU),
        ("PEN_MENU", PEN_MENU),
        ("GUIDE_MENU", GUIDE_MENU),
        ("PASTEBOARD_MENU", PASTEBOARD_MENU),
        ("VIEW_MENU", VIEW_MENU),
        ("HAND_MENU", HAND_MENU),
        ("TYPE_EDIT_MENU", TYPE_EDIT_MENU),
        ("TYPE_LAYER_MENU", TYPE_LAYER_MENU),
        ("TYPE_LAYER_MENU_PARAGRAPH", TYPE_LAYER_MENU_PARAGRAPH),
        ("TYPE_LAYER_MENU_OTHER", TYPE_LAYER_MENU_OTHER),
        ("EYEDROPPER_MENU", EYEDROPPER_MENU),
        ("SAMPLER_MENU", SAMPLER_MENU),
    ];
    for (name, table) in tables {
        for (label, id) in table.iter().flatten() {
            assert!(
                menu_local(id)
                    || items.iter().any(|i| i.command.id == *id)
                    || crate::shortcut_dispatch::RUNTIME_COMMANDS.iter().any(|(runtime, _)| runtime == id)
                    || photocraft_engine::commands::find(id).is_some(),
                "{name}: {label}: unknown command {id}"
            );
        }
    }
}

#[test]
fn type_layer_menu_targets_the_type_under_the_pointer() {
    let mut app = app();
    let id = committed_type(&mut app);
    app.run("layer.new.layer", json!({})).unwrap();
    let ctx = app.ctx().clone();
    let active = |app: &PhotocraftApp| app.session.active().unwrap().active_layer.map(|l| l.0);
    assert_ne!(active(&app), Some(id));
    // Away from the type, or with a Type Mask tool, nothing opens and nothing is selected.
    assert!(!open_here(&mut app, Tool::Type, [30.0, 2.0]));
    assert!(!open_here(&mut app, Tool::HorizontalTypeMask, [16.0, 16.0]));
    assert!(!open_here(&mut app, Tool::Type, [f64::NAN, 16.0]));
    assert_ne!(active(&app), Some(id));
    // Over the type: its layer becomes the target, and the menu has no text edit rows.
    assert!(open_here(&mut app, Tool::Type, [16.0, 16.0]));
    assert_eq!(active(&app), Some(id));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    for edit_row in ["edit.undo", "edit.cut", "edit.copy", "edit.paste", "select.all"] {
        assert!(!available(&app, &menu, edit_row), "{edit_row} belongs to the edit menu only");
    }
    assert!(entry_enabled(&app, &menu, "type.convertToParagraphText") && !entry_enabled(&app, &menu, "type.convertToPointText"));
    choose(&mut app, &ctx, "type.rasterizeTypeLayer");
    assert!(text_of(&app, id).is_none(), "the type layer is pixels now");
    assert!(app.ui.text_edit.is_none());
}

/// Two selected type layers: the one right-clicked becomes active, and the selection stays.
#[test]
fn type_layer_menu_in_a_multiselection_acts_on_the_clicked_type() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool = Tool::Type;
    let top = app.run("type.create", json!({"text": "Top", "size": 30, "x": 10, "y": 40})).unwrap()["layer"].as_u64().unwrap();
    let bottom = app.run("type.create", json!({"text": "Low", "size": 30, "x": 10, "y": 90})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.select", json!({"layer": bottom})).unwrap();
    app.run("layer.select", json!({"layer": top, "mode": "add"})).unwrap();
    let at = [25.0, 80.0];
    assert_eq!(crate::type_tool::hit_layer(&mut app, at[0], at[1]).map(|l| l.0), Some(bottom));
    assert_eq!(app.session.active().unwrap().active_layer.map(|l| l.0), Some(top));
    let ctx = app.ctx().clone();
    assert!(open_here(&mut app, Tool::Type, at));
    let st = app.session.active().unwrap();
    assert_eq!(st.active_layer.map(|l| l.0), Some(bottom), "the clicked type is the target");
    assert_eq!(st.selected_layers().len(), 2, "the multiselection stays");
    choose(&mut app, &ctx, "type.rasterizeTypeLayer");
    assert!(text_of(&app, bottom).is_none(), "the clicked type is rasterized");
    assert!(text_of(&app, top).is_some(), "not the other selected type");
}

#[test]
fn type_edit_menu_lists_spelling_suggestions_for_the_word_under_the_pointer() {
    let mut app = app();
    app.ui.tool = Tool::Type;
    let id = app.run("type.create", json!({"text": "Teh cat", "size": 12, "x": 4, "y": 20})).unwrap()["layer"].as_u64().unwrap();
    let ctx = app.ctx().clone();
    crate::menus::invoke(&mut app, &ctx, "type.editText", json!({})).unwrap();
    // Away from the misspelling (past the end of the line): no spelling rows.
    assert!(open_here(&mut app, Tool::Type, [600.0, 15.0]));
    assert!(spelling_rows(app.ui.canvas_tool_menu.as_ref().unwrap()).is_empty());
    // Over "Teh": its suggestions head the menu, then Add to Dictionary.
    assert!(open_here(&mut app, Tool::Type, [5.0, 15.0]));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    let rows = spelling_rows(&menu);
    assert!(rows.iter().any(|(l, _)| l == "The"), "{rows:?}");
    assert_eq!(rows.last().map(|(_, id)| id.as_str()), Some(SPELLING_ADD));
    assert!(rows.len() <= MAX_SUGGESTIONS + 1, "at most {MAX_SUGGESTIONS} suggestions and Add to Dictionary: {rows:?}");
    assert!(available(&app, &menu, &rows[0].1));
    let pick = rows.iter().find(|(l, _)| l == "The").unwrap().1.clone();
    choose(&mut app, &ctx, &pick);
    assert_eq!(text_of(&app, id).unwrap().text, "The cat");
    assert_eq!(app.ui.text_edit.as_ref().map(|e| (e.caret, e.anchor)), Some((3, 3)), "the caret lands after the replacement");
    // Add to Dictionary stops flagging a word.
    app.run("type.insertText", json!({"layer": id, "text": " Zqx"})).unwrap();
    assert!(open_here(&mut app, Tool::Type, [600.0, 15.0]));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(matches!(&menu.kind, MenuKind::TypeEdit { spelling: Some(s) } if s.word == "Zqx"));
    choose(&mut app, &ctx, SPELLING_ADD);
    assert!(app.session.prefs().user_dictionary.iter().any(|w| w == "Zqx"));
}

#[test]
fn disabled_selection_rows_do_nothing() {
    let mut app = app();
    app.ui.tool = Tool::MagicWand;
    app.run("select.rect", json!({"x": 1, "y": 1, "width": 8, "height": 8})).unwrap();
    assert!(open_here(&mut app, Tool::MagicWand, [10.0, 10.0]));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    // No filter has run and nothing can be faded yet.
    for id in ["filter.lastFilter", "edit.fade"] {
        assert!(!entry_enabled(&app, &menu, id), "{id} is greyed out");
    }
    let before = app.session.journal.len();
    pick(&mut app, "edit.fade");
    assert_eq!(app.session.journal.len(), before);
    assert!(!any_dialog(&app));
    // Rows of the other variant are not part of this menu.
    assert!(open_here(&mut app, Tool::MagicWand, [10.0, 10.0]));
    pick(&mut app, "select.all");
    assert_eq!(app.session.journal.len(), before);
}

#[test]
fn completed_pen_path_right_click_makes_selection() {
    let mut app = app();
    pen_clicks(&mut app, &[(2.0, 2.0), (20.0, 2.0), (20.0, 20.0), (2.0, 2.0)]);
    assert!(app.session.active().unwrap().doc.work_path.is_some());
    let mut h = harness(app);
    let center = h.state().last_canvas_rect.center();
    right_click(&mut h, center, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.tool == Tool::Pen && m.has_path));
    let ctx = h.ctx.clone();
    choose(h.state_mut(), &ctx, "path.toSelection");
    confirm_hosted(h.state_mut(), &ctx);
    assert!(h.state().session.active().unwrap().doc.selection.is_some());
    assert_eq!(h.state().session.journal.last().map(|(id, _)| id.as_str()), Some("path.toSelection"));
    // Like ⌘↩, it deselects and hides the work path, which stays in the document (#6079).
    assert_eq!(h.state().ui.selected_path, None);
    assert!(h.state().session.active().unwrap().doc.work_path.is_some());
    assert!(!crate::vector_ui::work_path_shown(h.state()));
    // The hidden work path is no longer the active path: a second right-click offers no path
    // commands, so Delete Path can't remove it unseen.
    right_click(&mut h, center, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.tool == Tool::Pen && !m.has_path && m.path_name.is_none()));
    choose(h.state_mut(), &ctx, "path.delete");
    assert!(h.state().session.active().unwrap().doc.work_path.is_some(), "the hidden work path survives Delete Path");
}

#[test]
fn pen_menu_finishes_in_progress_path_before_selection() {
    let mut app = app();
    pen_clicks(&mut app, &[(2.0, 2.0), (20.0, 2.0), (20.0, 20.0)]);
    assert!(app.session.active().unwrap().doc.work_path.is_none());
    assert!(open_here(&mut app, Tool::Pen, [10.0, 10.0]));
    let menu = app.ui.canvas_tool_menu.as_ref().unwrap();
    assert!(entry_enabled(&app, menu, "path.toSelection"));
    let ctx = app.ctx().clone();
    choose(&mut app, &ctx, "path.toSelection");
    confirm_hosted(&mut app, &ctx);
    assert!(app.session.active().unwrap().doc.work_path.is_some());
    assert!(app.session.active().unwrap().doc.selection.is_some());
}

/// #5703: the Pen options bar's Make Selection (Path mode) opens the same dialog for the path
/// being drawn (finished into the Work Path first), and OK selects inside it; with no path at
/// all it reports that instead.
#[test]
fn pen_options_bar_makes_a_selection_from_the_path() {
    let mut app = app();
    let ctx = app.ctx().clone();
    app.ui.tool = Tool::Pen;
    assert!(pen_dialogs::make_selection(&mut app).is_err(), "no path yet");
    assert!(!dialog_open(&app));
    pen_clicks(&mut app, &[(2.0, 2.0), (20.0, 2.0), (20.0, 20.0)]);
    pen_dialogs::make_selection(&mut app).unwrap();
    confirm_hosted(&mut app, &ctx);
    let st = app.session.active().unwrap();
    assert!(st.doc.work_path.is_some() && st.doc.selection.is_some());
    // That hid the Work Path (#6079): it is not the active path until it is selected in the
    // Paths panel again, and then it is, with no pen path in progress.
    app.run("select.deselect", json!({})).unwrap();
    assert!(pen_dialogs::make_selection(&mut app).is_err(), "the hidden Work Path is not the active path");
    assert!(!dialog_open(&app));
    app.ui.selected_path = Some("work".into());
    pen_dialogs::make_selection(&mut app).unwrap();
    confirm_hosted(&mut app, &ctx);
    assert!(app.session.active().unwrap().doc.selection.is_some());
    // Drawn with a saved path selected, the path is added to that one (#4721): the dialog is
    // for it, not for a Work Path.
    app.run("path.rename", json!({"name": "work", "to": "Cutout"})).unwrap();
    app.ui.selected_path = Some("Cutout".into());
    pen_clicks(&mut app, &[(4.0, 24.0), (12.0, 24.0), (12.0, 30.0)]);
    pen_dialogs::make_selection(&mut app).unwrap();
    assert_eq!(hosted(&mut app).fields().text("name"), Some("Cutout"));
}

/// A document with the Pen tool picked and a Work Path. Every route to the Stroke Path dialog ends in
/// `pen_dialogs::open`, so driving that covers the Paths panel's routes.
fn stroke_app() -> Box<PhotocraftApp> {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    work_path(&mut app);
    app.ui.selected_path = Some("work".into());
    app
}

fn open_stroke(app: &mut PhotocraftApp) {
    pen_dialogs::open(app, "path.stroke", "work");
    assert!(dialog_open(app), "Stroke Path… opens a dialog");
    assert_eq!(hosted(app).fields().text("name"), Some("work"), "the Work Path is stroked");
}

/// The Simulate Pressure the freshly opened dialog starts with; the dialog is closed again.
fn pressure_on_open(app: &mut PhotocraftApp) -> Option<bool> {
    open_stroke(app);
    let on = hosted(app).fields().bool("simulatePressure");
    let ctx = app.ctx().clone();
    assert!(app.framework.dialogs.cancel::<HostedParamDialog>(&ctx));
    on
}

fn set_pressure(app: &mut PhotocraftApp, on: bool) {
    hosted(app).set_field("simulatePressure", on.into());
}

fn strokes(app: &PhotocraftApp) -> usize {
    app.session.journal.iter().filter(|(id, _)| id == "path.stroke").count()
}

/// OK after a stroke that ran remembers Simulate Pressure for the next opening; Cancel and a failed run
/// remember nothing.
#[test]
fn stroke_path_reopens_with_the_simulate_pressure_that_was_applied() {
    let mut app = stroke_app();
    let ctx = app.ctx().clone();
    assert_eq!(pressure_on_open(&mut app), Some(false), "no remembered choice: off");
    open_stroke(&mut app);
    set_pressure(&mut app, true);
    confirm_hosted(&mut app, &ctx);
    assert_eq!(strokes(&app), 1, "OK stroked the path");
    assert_eq!(pressure_on_open(&mut app), Some(true), "the applied choice opens next time");
    // Cancel applies nothing and leaves the remembered choice alone.
    open_stroke(&mut app);
    set_pressure(&mut app, false);
    assert!(app.framework.dialogs.cancel::<HostedParamDialog>(&ctx));
    assert_eq!(strokes(&app), 1, "Cancel ran no stroke");
    assert_eq!(pressure_on_open(&mut app), Some(true), "Cancel remembers nothing");
    // A run that fails (the path went while the dialog was open) remembers nothing either.
    open_stroke(&mut app);
    set_pressure(&mut app, false);
    app.run("path.delete", json!({"name": "work"})).unwrap();
    assert!(try_confirm_hosted(&mut app, &ctx).is_err(), "the run has no path to stroke");
    close_dialogs(&mut app);
    assert_eq!(strokes(&app), 1, "the failed run stroked nothing");
    work_path(&mut app);
    app.ui.selected_path = Some("work".into());
    assert_eq!(pressure_on_open(&mut app), Some(true), "a failed confirmation remembers nothing");
}

#[test]
fn pen_shape_menu_enables_shape_ops_and_style_copy_only_when_applicable() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    app.run(
        "shape.create",
        json!({"kind":"path", "path":{"subpaths":[
        {"closed":true,"knots":[[2,2],[20,2],[20,20]]},
        {"closed":true,"knots":[[5,5],[10,5],[10,10]]}
    ]}, "fill":"#ff0000", "stroke":{"width":2,"color":"#000000"}}),
    )
    .unwrap();
    assert!(open_here(&mut app, Tool::Pen, [10.0, 10.0]));
    let menu = app.ui.canvas_tool_menu.as_ref().unwrap();
    assert!(entry_enabled(&app, menu, "layer.combineShapes.unite"));
    assert!(entry_enabled(&app, menu, "path.style.copyFill"));
    assert!(!entry_enabled(&app, menu, "path.style.pasteFill"));
    assert!(!entry_enabled(&app, menu, "path.fill"));
    pick(&mut app, "path.style.copyFill");
    assert!(app.session.path_fill_clipboard.is_some());
    assert!(open_here(&mut app, Tool::Pen, [10.0, 10.0]));
    assert!(entry_enabled(&app, app.ui.canvas_tool_menu.as_ref().unwrap(), "path.style.pasteFill"));
}

#[test]
fn unfinished_shape_path_disables_actions_that_would_target_the_previous_layer() {
    let mut app = app();
    app.ui.tool_options.vector_mode = craft::draw::ui::VectorMode::Shape;
    pen_clicks(&mut app, &[(2.0, 2.0), (20.0, 2.0), (20.0, 20.0)]);
    assert!(open_here(&mut app, Tool::Pen, [10.0, 10.0]));
    let menu = app.ui.canvas_tool_menu.as_ref().unwrap();
    for id in ["layer.vectorMask.fromPath", "path.delete", "path.fill", "path.stroke", "path.clippingPath.set", "view.newGuidesFromShape"] {
        assert!(!entry_enabled(&app, menu, id), "{id} must not act on the layer replaced by the pending shape");
    }
    assert!(entry_enabled(&app, menu, "path.toSelection"));
}

#[test]
fn crop_menu_rows_act_like_the_options_bar() {
    use craft::behavior::geometry::{CropGuide, GuideShow};
    use craft::draw::ui::crop::{CropLength, CropRatio, CropSettings};
    let mut app = crop_app();
    let ctx = app.ctx().clone();
    draw_crop(&mut app);
    let reopen = |app: &mut PhotocraftApp| {
        assert!(open_here(app, Tool::Crop, [10.0, 10.0]));
        app.ui.canvas_tool_menu.clone().unwrap()
    };
    // Nothing to save with a free ratio, nothing to delete without saved presets.
    let menu = reopen(&mut app);
    assert!(!entry_enabled(&app, &menu, CROP_NEW) && !entry_enabled(&app, &menu, CROP_DELETE));
    assert!(mode_checked(&app, &menu, "crop.ratio."), "free Ratio is checked, as in the dropdown");
    // A size preset fills W x H x Resolution, which the menu checks, as the dropdown does.
    choose(&mut app, &ctx, "crop.ratio.preset:4x5in");
    let menu = reopen(&mut app);
    assert_eq!(app.ui.tool_options.crop.ratio, CropRatio::Size);
    assert!(mode_checked(&app, &menu, "crop.ratio.preset:4x5in") && !mode_checked(&app, &menu, "crop.ratio.whr"));
    assert!((aspect(frame_of(&app).unwrap()) - 0.8).abs() < 1e-9);
    assert!(entry_enabled(&app, &menu, CROP_NEW));
    // Clear empties W x H x Resolution, as the bar's Clear.
    choose(&mut app, &ctx, CROP_CLEAR);
    assert_eq!(app.ui.tool_options.crop.size.width, None);
    // Overlays: another overlay starts at its first orientation; when it shows.
    app.ui.tool_options.crop.guide = CropGuide::GoldenSpiral;
    app.ui.tool_options.crop.guide_orientation = 2;
    reopen(&mut app);
    choose(&mut app, &ctx, "crop.overlay.triangle");
    assert_eq!((app.ui.tool_options.crop.guide, app.ui.tool_options.crop.guide_orientation), (CropGuide::Triangle, 0));
    reopen(&mut app);
    choose(&mut app, &ctx, "crop.overlayShow.never");
    assert_eq!(app.ui.tool_options.crop.guide_show, GuideShow::Never);
    let menu = reopen(&mut app);
    assert!(mode_checked(&app, &menu, "crop.overlay.triangle") && mode_checked(&app, &menu, "crop.overlayShow.never"));
    // A saved preset is listed by name; picking it refits the frame to its W x H, though the
    // ratio key is "whr" before and after.
    app.ui.canvas_tool_menu = None;
    let mut banner_settings = CropSettings::default();
    banner_settings.ratio = CropRatio::Size;
    banner_settings.size.width = Some(CropLength::new(300.0, UnitKind::Pixels));
    banner_settings.size.height = Some(CropLength::new(100.0, UnitKind::Pixels));
    banner_settings.size.resolution = "72".into();
    let saved = json!({"crop": banner_settings});
    app.run("tool.presets.new", json!({"name": "Banner", "tool": "crop", "options": {"toolOptions": saved}})).unwrap();
    assert!(crate::crop_presets::user_presets(&app).iter().any(|p| p.name == "Banner" && p.ratio == CropRatio::Size));
    let menu = reopen(&mut app);
    let banner = crop_rows(&app, &menu).into_iter().flatten().find(|(l, _)| l == "Banner").expect("the saved preset is a row").1;
    assert!(entry_enabled(&app, &menu, CROP_DELETE));
    choose(&mut app, &ctx, &banner);
    assert_eq!(app.ui.tool_options.crop.size.width, Some(CropLength::new(300.0, UnitKind::Pixels)));
    assert!((aspect(frame_of(&app).unwrap()) - 3.0).abs() < 1e-9, "the frame follows the preset: {:?}", frame_of(&app));
    let menu = reopen(&mut app);
    assert!(mode_checked(&app, &menu, &banner));
    // Crop commits as ↵ does: the preset's size.
    choose(&mut app, &ctx, CROP_COMMIT);
    let size = app.session.active().unwrap().doc.size;
    assert_eq!((size.width, size.height), (300, 100));
    // Cancel drops a pending crop.
    draw_crop(&mut app);
    reopen(&mut app);
    choose(&mut app, &ctx, CROP_CANCEL);
    assert!(!crate::crop_ui::pending(&app));
    // ...and, as Esc does, hides the frame without cropping (#10370).
    assert_eq!(frame_of(&app), None);
    assert_eq!(app.session.active().unwrap().doc.size, size);
}
