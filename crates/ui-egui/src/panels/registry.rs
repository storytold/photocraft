//! The panel registry: every panel PhotoCraft shows in its workspace, with its name, icon, size policy, body and
//! panel menu, and the built-in workspaces those panels are arranged in.
//!
//! The dock (`panel_docking`) reads a panel's spec and body from here; the command sources read each panel's menu
//! states and run its fired commands through [`states`] and [`run_fired`]. One arrangement serves every theme.

use craft::behavior::Axis;
use craft::behavior::dock::{Action, Column, DockEdge, Floating, Group, Layout, Node, Permissions, SplitSize, Workspace};
use craft::color_ui::ui::{ColorPanelCommand, ColorPanelMenu};
use craft::ui::contract::{Scroll, ShowContainer as _};
use craft::ui::docking::DockHeights;
use craft::ui::layout_components::Stack;
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::{CommandRef, CommandSet as _, CommandState};
use egui::{Rect, pos2, vec2};

use super::PhotocraftApp;

/// One registered panel.
pub(crate) struct PanelEntry {
    /// The stable id: the dock's panel key, the `window.panel.<id>` command suffix and the saved layouts' name.
    pub id: &'static str,
    /// The translated name: tab text, menu rows and the icon's accessible name.
    pub label: fn() -> String,
    /// The icon of the panel's iconic form.
    pub icon: &'static str,
    /// The heights its group asks for in a dock column; the first panel of each group carries them.
    pub heights: Option<DockHeights>,
    /// The least body size, width and height.
    pub min: [f32; 2],
    /// The size the panel floats at, for the panels that float by default.
    pub float_size: Option<[f32; 2]>,
    /// Draws the panel's contents.
    pub body: fn(&mut PhotocraftApp, &mut egui::Ui),
    /// The rows of the panel's menu, before the dock's own rows.
    pub rows: fn(&PhotocraftApp) -> Vec<PanelRow>,
    /// The state of each command the rows name, for a panel whose rows are a framework command set.
    pub states: fn(&PhotocraftApp) -> Vec<(CommandRef, CommandState)>,
    /// Runs a fired command of the panel's command set; `false` when the command is not the panel's.
    pub run: fn(&mut PhotocraftApp, &egui::Context, &CommandRef) -> bool,
}

/// The toolbar: a column of tool buttons that docks, floats and resizes in whole tool columns like a panel of its own.
/// Its group view (`panel_docking`) draws the rail in the group's full rect.
pub(crate) const TOOLBAR: &str = "toolbar";

/// The panel that holds the document area. It never moves, floats or closes, and panels open beside it.
pub(crate) const CANVAS: &str = "canvas";

/// The canvas panel's least body size.
pub(crate) const CANVAS_MIN: [f32; 2] = [200.0, 120.0];

/// The least width of a panel's body.
const MIN_WIDTH: f32 = 220.0;

fn no_rows(_: &PhotocraftApp) -> Vec<PanelRow> {
    Vec::new()
}

fn no_states(_: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    Vec::new()
}

/// The body of a panel whose group view draws it.
fn drawn_by_group_view(_: &mut PhotocraftApp, _: &mut egui::Ui) {}

fn runs_nothing(_: &mut PhotocraftApp, _: &egui::Context, _: &CommandRef) -> bool {
    false
}

/// A panel with no menu rows, a 220-point least width and `least` points of body height, docked by default.
const fn panel(id: &'static str, label: fn() -> String, icon: &'static str, least: f32, body: fn(&mut PhotocraftApp, &mut egui::Ui)) -> PanelEntry {
    PanelEntry { id, label, icon, heights: None, min: [MIN_WIDTH, least], float_size: None, body, rows: no_rows, states: no_states, run: runs_nothing }
}

/// A panel that floats by default at `size`.
const fn floating(id: &'static str, label: fn() -> String, icon: &'static str, size: [f32; 2], body: fn(&mut PhotocraftApp, &mut egui::Ui)) -> PanelEntry {
    PanelEntry { float_size: Some(size), ..panel(id, label, icon, LEAST, body) }
}

/// Shows `body`, whose content neither fills the group nor scrolls itself, in a stack that fills the group and scrolls
/// vertically. Bodies whose root fills the group scroll themselves and are shown as they are.
fn scrolled(app: &mut PhotocraftApp, ui: &mut egui::Ui, body: fn(&mut PhotocraftApp, &mut egui::Ui)) {
    Stack::new().fill().scroll(Scroll::vertical()).show(ui, |ui, _| body(app, ui));
}

/// The least body height of a group other than Layers and Brushes.
const LEAST: f32 = 50.0;

// Group heights exclude the tab strip. The dock fills each column with its last open group, which asks for its `fill`:
// Layers wants room for about ten rows at 900 points (#147).
const COLOR_HEIGHTS: DockHeights = DockHeights::new(160.0).compact(100.0).fill(LEAST);
const PROPERTIES_HEIGHTS: DockHeights = DockHeights::new(220.0).compact(130.0).fill(LEAST);
const CHARACTER_HEIGHTS: DockHeights = DockHeights::new(240.0).compact(130.0).fill(LEAST);
const NAVIGATOR_HEIGHTS: DockHeights = DockHeights::new(180.0).compact(110.0).fill(LEAST);
const HISTORY_HEIGHTS: DockHeights = DockHeights::new(170.0).compact(100.0).fill(LEAST);
const LAYERS_HEIGHTS: DockHeights = DockHeights::new(290.0).compact(170.0).fill(470.0);
const BRUSHES_HEIGHTS: DockHeights = DockHeights::new(170.0).compact(120.0).fill(70.0);

/// Every registered panel: the docked panels in their groups' one tab order, top to bottom, then the panels that
/// float or open below the canvas.
pub(crate) static PANELS: &[PanelEntry] = &[
    PanelEntry { min: [0.0, 0.0], ..panel(TOOLBAR, || tl!("Tools").to_string(), "move", 0.0, drawn_by_group_view) },
    // The Color panel sizes its field to the group and scrolls its slider modes itself (#6000).
    PanelEntry {
        heights: Some(COLOR_HEIGHTS),
        rows: |_| ColorPanelMenu::rows(),
        states: color_states,
        run: color_run,
        ..panel("color", || tl!("Color").to_string(), "palette", LEAST, super::color::panel)
    },
    PanelEntry {
        rows: super::swatches_ui::panel_rows,
        states: super::swatches_ui::command_states,
        run: super::swatches_ui::run_fired,
        ..panel("swatches", || tl!("Swatches").to_string(), "swatch_book", LEAST, super::swatches_ui::panel)
    },
    panel("gradients", || tl!("Gradients").to_string(), "blend", LEAST, super::preset_panels::gradients_panel),
    panel("patterns", || tl!("Patterns").to_string(), "grid", LEAST, super::preset_panels::patterns_panel),
    PanelEntry { heights: Some(PROPERTIES_HEIGHTS), ..panel("properties", || tl!("Properties").to_string(), "adjustments", LEAST, super::properties::panel) },
    panel("adjustments", || tl!("Adjustments").to_string(), "contrast", LEAST, |app, ui| scrolled(app, ui, super::properties::adjustments::panel)),
    PanelEntry {
        heights: Some(CHARACTER_HEIGHTS),
        ..panel(
            "character",
            || tl!("Character").to_string(),
            "type",
            LEAST,
            |app, ui| scrolled(app, ui, |app, ui| crate::type_tool::character_panel(app, ui, false)),
        )
    },
    panel(
        "paragraph",
        || tl!("Paragraph").to_string(),
        "pilcrow",
        LEAST,
        |app, ui| scrolled(app, ui, |app, ui| crate::type_tool::character_panel(app, ui, true)),
    ),
    // The Navigator's preview grows with its group, as in Photoshop (#2726).
    PanelEntry { heights: Some(NAVIGATOR_HEIGHTS), ..panel("navigator", || tl!("Navigator").to_string(), "navigation", LEAST, super::navigator::navigator) },
    PanelEntry {
        rows: super::histogram_panel::panel_rows,
        states: super::histogram_panel::command_states,
        run: super::histogram_panel::run_fired,
        ..panel("histogram", || tl!("Histogram").to_string(), "chart_line", LEAST, |app, ui| scrolled(app, ui, super::histogram_panel::show))
    },
    panel("info", || tl!("Info").to_string(), "info", LEAST, |app, ui| scrolled(app, ui, super::info::info_panel)),
    PanelEntry {
        heights: Some(HISTORY_HEIGHTS),
        rows: super::history_menu_ui::panel_rows,
        states: super::history_menu_ui::command_states,
        run: super::history_menu_ui::run_fired,
        ..panel("history", || tl!("History").to_string(), "clock", LEAST, super::history::history)
    },
    PanelEntry {
        rows: super::actions::menu::panel_rows,
        states: super::actions::menu::states,
        run: super::actions::menu::run_fired,
        ..panel("actions", || tl!("Actions").to_string(), "play", LEAST, super::actions::panel)
    },
    PanelEntry {
        rows: super::comps_ui::panel_rows,
        run: super::comps_ui::run_fired,
        ..panel("layerComps", || tl!("Layer Comps").to_string(), "copy", LEAST, super::comps_ui::panel)
    },
    PanelEntry {
        heights: Some(LAYERS_HEIGHTS),
        rows: super::layers_panel_ui::panel_rows,
        states: super::layers_panel_ui::command_states,
        run: super::layers_panel_ui::run_fired,
        ..panel("layers", || tl!("Layers").to_string(), "layers", 110.0, super::layers::layers)
    },
    PanelEntry {
        rows: super::channels_panel::panel_rows,
        states: super::channels_panel::command_states,
        run: super::channels_panel::run_fired,
        ..panel("channels", || tl!("Channels").to_string(), "squares_subtract", 110.0, super::channels_panel::show)
    },
    panel("paths", || tl!("Paths").to_string(), "pen_tool", 110.0, crate::vector_ui::paths_panel),
    PanelEntry { heights: Some(BRUSHES_HEIGHTS), ..panel("brushes", || tl!("Brushes").to_string(), "brush", 70.0, super::brush_panel::brushes::panel) },
    PanelEntry {
        rows: super::preset_browser::styles_rows,
        states: super::preset_browser::styles_states,
        run: super::preset_browser::styles_run,
        ..floating("styles", || tl!("Styles").to_string(), "sparkles", [300.0, 420.0], |app, ui| scrolled(app, ui, super::preset_panels::styles_panel))
    },
    floating("shapes", || tl!("Shapes").to_string(), "square", [300.0, 420.0], super::preset_panels::shapes_panel),
    floating("toolPresets", || tl!("Tool Presets").to_string(), "target", [300.0, 380.0], super::preset_panels::tool_presets_panel),
    floating("cloneSource", || tl!("Clone Source").to_string(), "stamp", [300.0, 360.0], |app, ui| scrolled(app, ui, super::preset_panels::clone_source_panel)),
    PanelEntry {
        rows: super::type_panels_ui::styles::panel_rows,
        run: super::type_panels_ui::styles::run_fired,
        ..floating(
            "characterStyles",
            || tl!("Character Styles").to_string(),
            "type",
            [280.0, 360.0],
            |app, ui| super::type_panels_ui::styles::panel(app, ui, false),
        )
    },
    PanelEntry {
        rows: super::type_panels_ui::styles::panel_rows,
        run: super::type_panels_ui::styles::run_fired,
        ..floating(
            "paragraphStyles",
            || tl!("Paragraph Styles").to_string(),
            "pilcrow",
            [280.0, 360.0],
            |app, ui| super::type_panels_ui::styles::panel(app, ui, true),
        )
    },
    // Glyphs and Measurement Log hold collections that scroll themselves.
    floating("glyphs", || tl!("Glyphs").to_string(), "text_cursor", [360.0, 320.0], super::type_panels_ui::glyphs::body),
    // The Timeline docks below the canvas; it lays out its own tracks.
    panel("timeline", || tl!("Timeline").to_string(), "split_horizontal", LEAST, super::timeline_ui::body),
    PanelEntry {
        rows: super::brush_panel::panel_rows,
        states: super::brush_panel::command_states,
        run: super::brush_panel::run_fired,
        ..floating("brushSettings", || tl!("Brush Settings").to_string(), "brush_cleaning", [520.0, 460.0], super::brush_settings::brush_settings)
    },
    floating("modifierKeys", || tl!("Modifier Keys").to_string(), "keyboard", [240.0, 140.0], |app, ui| scrolled(app, ui, crate::modifier_keys::panel)),
    floating("gradientEditor", || tl!("Gradient Editor").to_string(), "blend", [520.0, 440.0], |app, ui| scrolled(app, ui, crate::gradient_ui::editor::show)),
    floating("measurementLog", || tl!("Measurement Log").to_string(), "ruler", [620.0, 320.0], crate::tools::analysis::log_panel::body),
    floating("notes", || tl!("Notes").to_string(), "square_plus", [260.0, 300.0], |app, ui| scrolled(app, ui, crate::tools::analysis::notes_panel::body)),
];

/// The Color panel menu's states, from the panel's own state.
fn color_states(app: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    ColorPanelMenu::states(&app.ui.color_panel.panel).into_iter().map(|(command, state)| (command.into(), state)).collect()
}

/// Runs a fired Color panel menu command; Copy Color as HTML and Copy Color's Hex Code put their text on the
/// clipboard.
fn color_run(app: &mut PhotocraftApp, ctx: &egui::Context, command: &CommandRef) -> bool {
    let Some(command) = ColorPanelCommand::from_ref(command) else { return false };
    if let Some(text) = super::color::run_menu_command(app, command) {
        ctx.copy_text(text);
    }
    true
}

/// The entry of panel `id`.
pub(crate) fn entry(id: &str) -> Option<&'static PanelEntry> {
    static BY_ID: std::sync::OnceLock<std::collections::HashMap<&'static str, &'static PanelEntry>> = std::sync::OnceLock::new();
    BY_ID
        .get_or_init(|| {
            let mut panels = std::collections::HashMap::new();
            for panel in PANELS {
                panels.entry(panel.id).or_insert(panel);
            }
            panels
        })
        .get(id)
        .copied()
}

/// The id of the panel `name` names: its id or its label, in any case. The canvas is not a registered panel.
pub(crate) fn normalize(name: &str) -> Option<&'static str> {
    let name = name.trim();
    type PanelLabels = std::sync::Arc<Vec<(&'static str, String)>>;
    static LABELS: crate::i18n::LangCache<(), PanelLabels> = crate::i18n::LangCache::new();
    let labels = LABELS.get_or_build((), || std::sync::Arc::new(PANELS.iter().map(|panel| (panel.id, (panel.label)())).collect()));
    labels.iter().find(|(id, label)| id.eq_ignore_ascii_case(name) || label.eq_ignore_ascii_case(name)).map(|(id, _)| *id)
}

/// Whether the workspace may hold `panel`: the canvas or a registered panel.
pub(crate) fn registered(panel: &str) -> bool {
    panel == CANVAS || entry(panel).is_some()
}

/// What may happen to `panel`: the canvas stays docked and open and takes no tabs, but panels open beside it; the
/// toolbar never collapses to icons and never shares its column or tab group; every other panel moves, floats and
/// closes. A locked workspace narrows these itself.
pub(crate) fn permissions(panel: &str) -> Permissions {
    match panel {
        CANVAS => Permissions::PROTECTED.accepts_splits(true),
        TOOLBAR => Permissions::ALL.collapses(false).shares_column(false),
        _ => Permissions::ALL,
    }
}

/// The state of every panel's menu commands.
pub(crate) fn states(app: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    PANELS.iter().flat_map(|panel| (panel.states)(app)).collect()
}

/// Runs a fired panel menu command through the panel whose command set it belongs to; `false` when none takes it.
pub(crate) fn run_fired(app: &mut PhotocraftApp, ctx: &egui::Context, command: &CommandRef) -> bool {
    PANELS.iter().any(|panel| (panel.run)(app, ctx, command))
}

/// The groups of saved layouts written before the workspace (`panels.<group>`, `dockTabs.<group>`,
/// `dock.collapsed`): each old group key with its panels in the one tab order. Color, Properties, Character and
/// Layers are the default expanded dock column from top to bottom; Navigator, History and Brushes the default column
/// of icons.
pub(crate) const LEGACY_GROUPS: &[(&str, &[&str])] = &[
    ("color", &["color", "swatches", "gradients", "patterns"]),
    ("properties", &["properties", "adjustments"]),
    ("character", &["character", "paragraph"]),
    ("navigator", &["navigator", "histogram", "info"]),
    ("history", &["history", "actions", "layerComps"]),
    ("layers", &["layers", "channels", "paths"]),
    ("brushes", &["brushes"]),
];

/// The height of the Timeline's frame below the canvas.
const TIMELINE_HEIGHT: f32 = 220.0;

/// The window width the floating panels' default places are laid out for: the Measurement Log and Notes keep their
/// distance from the window's end edge in a window this wide.
const REFERENCE_WIDTH: f32 = 1440.0;

/// Where a panel that floats by default opens: the Measurement Log and Notes at their places near the end edge,
/// the others at (400, 140).
fn float_rect(id: &str, size: [f32; 2]) -> Rect {
    let min = match id {
        "measurementLog" => pos2(REFERENCE_WIDTH - 300.0 - size[0], 470.0),
        "notes" => pos2(REFERENCE_WIDTH - 40.0 - size[0], 300.0),
        _ => pos2(400.0, 140.0),
    };
    Rect::from_min_size(min, vec2(size[0], size[1]))
}

/// The old groups the default expanded column holds, top to bottom; the rest are the column of icons.
const EXPANDED_GROUPS: [&str; 4] = ["color", "properties", "character", "layers"];

/// The width of the Start dock's split: the toolbar's dock takes its column's own width.
const TOOLBAR_DOCK_WIDTH: f32 = 60.0;

/// The old group `key` as a dock group of its panels in the one tab order.
fn legacy_group(key: &str) -> Group<String> {
    let panels: &[&str] = LEGACY_GROUPS.iter().find(|(group, _)| *group == key).map_or(&[], |(_, panels)| *panels);
    Group::new(panels.iter().map(|panel| (*panel).to_owned()).collect(), 0)
}

/// Every panel laid out: the toolbar's column in a dock at the start edge, the canvas with the Timeline below it, an
/// end dock of two columns (from the window edge inward: the expanded groups, then the column of icons Photoshop keeps
/// beside the canvas), and each floating panel in its own window. The built-in workspaces close what they do not show,
/// so every closed panel keeps the place it comes back to.
fn full_layout() -> Layout<String> {
    let toolbar = Column::new(vec![Group::new(vec![TOOLBAR.to_owned()], 0)]).with_steps(1);
    let expanded = Column::new(EXPANDED_GROUPS.iter().map(|key| legacy_group(key)).collect());
    let icons =
        Column::new(LEGACY_GROUPS.iter().map(|(key, _)| *key).filter(|key| !EXPANDED_GROUPS.contains(key)).map(legacy_group).collect()).with_iconic(true);
    let canvas =
        Node::split(Axis::Y, SplitSize::FixedSecond(TIMELINE_HEIGHT), Node::tabs(vec![CANVAS.to_owned()], 0), Node::tabs(vec!["timeline".to_owned()], 0));
    let beside = Node::split(Axis::X, SplitSize::Ratio(0.8), canvas, Node::dock(DockEdge::End, vec![expanded, icons]));
    let root = Node::split(Axis::X, SplitSize::FixedFirst(TOOLBAR_DOCK_WIDTH), Node::dock(DockEdge::Start, vec![toolbar]), beside);
    // Ids from the table order, so the built-in layout's windows are the same on every build of it.
    let floating = PANELS
        .iter()
        .filter_map(|panel| panel.float_size.map(|size| (panel.id, size)))
        .zip(1_u64..)
        .map(|((id, size), window)| Floating::single(vec![id.to_owned()], 0, float_rect(id, size)).with_id(window))
        .collect();
    Layout::new(Some(root), floating)
}

/// The panels a built-in workspace shows, by name; any other name is Essentials.
fn preset_panels(name: &str) -> Vec<&'static str> {
    let groups: &[&str] = match name {
        "Photography" => &["navigator", "history", "properties", "layers"],
        "Painting" => &["color", "layers"],
        // Room for typography beside Properties and Layers.
        "Graphic and Web" => &["properties", "character", "layers"],
        // Color includes Swatches; the Navigator keeps the pixel canvas easy to inspect.
        "Pixel Art" => &["color", "navigator", "layers"],
        "Motion" => &["properties", "layers"],
        _ => &["color", "properties", "layers", "navigator", "history", "brushes"],
    };
    let mut shown: Vec<&'static str> = vec![TOOLBAR];
    shown.extend(LEGACY_GROUPS.iter().filter(|(key, _)| groups.contains(key)).flat_map(|(_, panels)| panels.iter().copied()));
    if name == "Motion" {
        shown.push("timeline");
    }
    shown
}

/// The built-in workspace `name` (Essentials for any other name): the full layout with the panels it does not show
/// closed, so each of them reopens at its place.
pub(crate) fn preset(name: &str) -> Workspace<String> {
    let shown = preset_panels(name);
    let closes = PANELS.iter().map(|panel| panel.id).filter(|id| !shown.contains(id)).map(|id| Action::close(id.to_owned()));
    let mut workspace = Workspace::new(full_layout());
    // The built-in layout holds every registered panel once and each of them may close, so a refusal does not arise; the
    // full layout with every panel shown is the fallback.
    if workspace.apply_all(closes, false, |panel: &String| permissions(panel)).is_err() {
        workspace = Workspace::new(full_layout());
    }
    workspace
}

/// The workspace a first launch and Reset show: Essentials.
pub(crate) fn default_workspace() -> Workspace<String> {
    preset("Essentials")
}
