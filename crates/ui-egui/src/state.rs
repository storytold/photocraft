//! UI state as plain data, so the control channel (and later MCP) can read and drive every
//! aspect of the interface: tools, zoom, panels, dialogs and windows.

use serde::{Deserialize, Serialize};

mod color_panel;
mod panel_state;
mod text_edit;
mod tool;
mod tool_options;
mod transform;

use std::collections::{BTreeSet, HashMap};

use craft::behavior::dock::DocumentWindows;
use craft::color_ui::ui::{SwatchKey, SwatchesState};
use craft::draw::ui::{BrushPickerState, GradientRef};
use craft::imaging::ui::scopes::ScopePanelState;
use craft::layers::ui::filter::LayerFilter;
use craft::layers::ui::options::LayersSettings;
use craft::ui::canvas::{SplitAt, ViewCommands, ViewRequest};
use craft::ui::contract::Tracked;
use craft::ui::patterns::presets::{PresetState, PresetView};
use photocraft_doc::DocId;

use crate::panels::brush_panel::brushes::BrushKey;

pub use color_panel::ColorPanelState;
pub use panel_state::{CurvesEditorState, PointCurveDrag, PointCurveState};
pub use text_edit::{TextEdit, TypeInsertionStyle, TypeTransform};
pub use tool::Tool;
pub use tool_options::{ObjectFinderMode, SampleAllLayers, ToolOptions};
use tool_options::{default_brush_tool, yes};
pub use transform::{CamTransform, ContentAwareOptions, Extras, FreeTransform, MadeLayer, SessionWarp};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Panels {
    pub options_bar: bool,
    pub status_bar: bool,
    /// The title bar with the in-window menus. An app that embeds PhotoCraft's UI and draws its own
    /// bar can hide it; the caption buttons of a custom title bar go with it.
    #[serde(default = "yes")]
    pub menu_bar: bool,
    /// Window › Contextual Task Bar (#10890): the small floating bar under the Crop tool's frame
    /// (`crop_task_bar`). Its ⋯ menu's Hide bar clears it too.
    #[serde(default = "yes")]
    pub context_task_bar: bool,
    /// Where that bar sits: following its frame, or dragged or pinned (the framework `TaskBar`).
    #[serde(default)]
    pub task_bar: TaskBarPlace,
}

pub use craft::ui::canvas_controls::TaskBarPlace;

impl Default for Panels {
    fn default() -> Self {
        Self { options_bar: true, status_bar: true, menu_bar: true, context_task_bar: true, task_bar: TaskBarPlace::default() }
    }
}

/// Per-document view (camera) state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct View {
    /// Screen (device) pixels per document pixel: the user-facing zoom (`100%` is `1.0`).
    /// Canvas geometry works in egui points and divides by `ctx.pixels_per_point`
    /// (`PhotocraftApp::point_zoom`), so neither the display scale nor Interface › UI Scale
    /// magnifies the image (#1943, #2121).
    pub zoom: f32,
    /// Document-space point shown at the canvas centre.
    pub center: [f32; 2],
    /// The view commands asked of this view's canvas (Fit on Screen placing a new view, a menu row, an action step),
    /// which its next pass runs in order (`craft::ui::canvas::Canvas::queued`); never persisted.
    #[serde(skip, default = "View::first_fit")]
    pub pending: Vec<ViewRequest>,
    /// Document size this view last showed; a change (Image/Canvas Size, crop) re-centres it.
    #[serde(default)]
    pub doc_size: [u32; 2],
    /// Canvas camera rotation in degrees, wrapped to (−180, 180]. Turns the view, not the pixels.
    #[serde(default)]
    pub rotation: f32,
    /// A discrete zoom command asks the canvas to ease to the camera above; never persisted.
    #[serde(skip)]
    pub(crate) zoom_animating: bool,
    /// Optional document/screen anchor for a pointer-centred Zoom-tool transition.
    #[serde(skip)]
    pub(crate) zoom_anchor: Option<([f64; 2], [f32; 2])>,
    /// The zoom and centre of the view when the discrete zoom above was asked for: the ease starts there.
    #[serde(skip)]
    pub(crate) zoom_from: Option<(f32, [f32; 2])>,
}

impl View {
    /// A new view's queue: its first pass places it fitted.
    fn first_fit() -> Vec<ViewRequest> {
        vec![ViewRequest::Place(ViewCommands::FitOnScreen)]
    }

    /// Asks this view's canvas to run `command`, a command the user gave, on its next pass, after the commands already
    /// asked: it eases under Animated Zoom.
    pub fn request(&mut self, command: ViewCommands) {
        self.pending.push(ViewRequest::Command(command));
    }

    /// Asks this view's canvas to place the view with `command` (a new or cropped document's fit) on its next pass: the
    /// view jumps there.
    pub fn place(&mut self, command: ViewCommands) {
        self.pending.push(ViewRequest::Place(command));
    }

    /// Drops the commands asked of this view's canvas: the view set now stands.
    pub fn clear_pending(&mut self) {
        self.pending.clear();
    }

    /// Whether `command` waits for this view's canvas.
    pub fn asks(&self, command: ViewCommands) -> bool {
        self.pending.iter().any(|request| request.command() == command)
    }

    /// Asks the canvas to ease the discrete zoom about to be made, from the zoom and centre the view has now, when
    /// Animated Zoom is enabled; otherwise the zoom jumps. Called before the zoom or centre changes.
    pub(crate) fn request_zoom_animation(&mut self, enabled: bool) {
        self.zoom_animating = enabled;
        self.zoom_anchor = None;
        self.zoom_from = enabled.then_some((self.zoom, self.center));
    }
}

impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            center: [0.0, 0.0],
            pending: View::first_fit(),
            doc_size: [0, 0],
            rotation: 0.0,
            zoom_animating: false,
            zoom_anchor: None,
            zoom_from: None,
        }
    }
}

/// An extra OS window showing a document ("Window → Arrange → New Window for Document").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocWindow {
    pub id: u64,
    pub document: usize,
    pub view: View,
    pub open: bool,
    /// This window owns the document's view; the document is removed from the main canvas.
    #[serde(default)]
    pub floating: bool,
    /// Transient focus edge used to activate the document once when this viewport gains focus.
    #[serde(skip)]
    pub focused: bool,
    /// Where the window opens on screen (outer top-left, in points); `None` lets the OS place it.
    /// A tab dragged off the tab strip opens its window where it was dropped (#3733).
    #[serde(default)]
    pub position: Option<[f32; 2]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiState {
    pub tool: Tool,
    /// The tools used so far, most recent first and each one once. The first tool of a shortcut
    /// group (the tools sharing a key, [`Tool::key`]) is the one its key brings back, and the first
    /// tool of a toolbar slot is the one the slot shows, as in Photoshop (#2608, #3569). One list
    /// answers both because they don't line up: the Marquee slot holds tools with and without a
    /// key, the Hand slot two keys. Kept between launches in `Preferences::group_tools`.
    #[serde(default)]
    pub group_tools: Vec<Tool>,
    /// A tool letter still held (spring-loaded tool, #2672): the tool to give back on release.
    /// Exposed to automation, never restored from settings.
    #[serde(default, skip_deserializing)]
    pub spring: Option<crate::spring_tools::Spring>,
    /// Recently opened file paths, most-recent first (File › Open Recent). Capped; de-duplicated.
    #[serde(default)]
    pub recent_files: craft::ui::contract::Tracked<Vec<String>>,
    /// Recently picked type families, most-recent first (the font menu's Recent group). Capped by
    /// Preferences ▸ Type ▸ Recent Fonts; de-duplicated.
    #[serde(default)]
    pub recent_fonts: Vec<String>,
    /// Inline type editing session, if any.
    #[serde(default)]
    pub text_edit: Option<TextEdit>,
    /// Whether the text being edited overflows its paragraph box (the box's lower-right handle
    /// shows the overflow "+").
    /// Read by automation while a session edits that box; never written to settings.
    #[serde(skip)]
    pub type_box_overflow: bool,
    /// Font for subsequent typing at this caret, separate from existing character runs.
    #[serde(default)]
    pub type_insertion_style: Option<TypeInsertionStyle>,
    /// Tool to restore when an inline type edit session commits or cancels (e.g. Move tool, #1132).
    #[serde(skip)]
    pub text_edit_return_tool: Option<Tool>,
    /// Ctrl/Cmd Type-tool gesture; never saved or restored.
    #[serde(skip)]
    pub type_transform: Option<TypeTransform>,
    /// Reference point for the current type editing session (document coordinates).
    #[serde(skip)]
    pub type_transform_pivot: Option<[f64; 2]>,
    /// Content-Aware Move's Transform On Drop box, waiting for ↩ / Esc.
    #[serde(default, skip_deserializing)]
    pub cam_transform: Option<CamTransform>,
    /// Free Transform session, if any. Written as its record (`FreeTransform::record`) for
    /// automation; never restored.
    #[serde(default, skip_deserializing)]
    pub transform: Option<FreeTransform>,
    /// Temporary symmetry-axis editing, exposed to automation but never restored from settings.
    #[serde(default, skip_deserializing)]
    pub symmetry_transform: Option<crate::symmetry_ui::Transform>,
    /// The Properties panel's armed Levels/Curves eyedropper on an adjustment layer (`layer_picker`),
    /// exposed to automation but never restored from settings.
    #[serde(default, skip_deserializing)]
    pub layer_picker: Option<crate::layer_picker::LayerPicker>,
    /// Clone Stamp / Healing source point (⌥-click) and the aligned offset once a stroke started.
    #[serde(default)]
    pub clone_source: Option<[f64; 2]>,
    #[serde(default)]
    pub clone_offset: Option<[f64; 2]>,
    /// Pattern Stamp aligned origin in document pixels, kept across strokes.
    #[serde(default)]
    pub pattern_stamp_phase: Option<[f32; 2]>,
    /// Painting targets the active layer's mask instead of its pixels.
    #[serde(default)]
    pub mask_target: bool,
    /// The active layer's vector mask is targeted (its Layers thumbnail is bracketed): the path
    /// tools edit it (#196). Never set together with `mask_target`.
    #[serde(default)]
    pub vector_mask_target: bool,
    /// The smart object (layer id) whose filter mask is targeted (its thumbnail on the Smart
    /// Filters row clicked): painting, fills, filters and adjustments edit that mask
    /// (FILE-215-3). Never set together with `mask_target` or `vector_mask_target`.
    #[serde(default)]
    pub filter_mask_target: Option<u64>,
    /// The adjustment or fill layer (id) whose own settings Properties shows although its mask
    /// is targeted: set when that layer is created, as in Photoshop (#3369). Targeting the mask
    /// thumbnail, the layer's pixels or another layer clears it.
    #[serde(default)]
    pub properties_layer: Option<u64>,
    /// Brush Preset picker opened by a right-click on the canvas or the options-bar brush chip:
    /// its screen position (points).
    #[serde(default)]
    pub brush_picker: Option<[f32; 2]>,
    /// The Brush Preset picker's list: search, collapsed groups, view, size and the parts shown.
    #[serde(skip)]
    pub(crate) brush_picker_state: Tracked<BrushPickerState<BrushKey>>,
    /// Photoshop's HUD Color Picker (#6098), open while Shift+Alt+right-click is held on the
    /// canvas with a painting tool (`crate::color_hud`).
    #[serde(default)]
    pub color_hud: Option<crate::color_hud::Hud>,
    /// Selection-tool context menu opened by a plain canvas right-click.
    #[serde(default)]
    pub canvas_tool_menu: Option<crate::canvas_tool_menu::CanvasToolMenu>,
    /// The guide a Move-tool click selected (`guide_select`): drawn blue, Delete removes it.
    #[serde(default)]
    pub selected_guide: Option<crate::guide_select::SelectedGuide>,
    /// The brush is a per-tool option (the Brush, the Eraser and every retouching tool each keep
    /// their own size, hardness, tip and dynamics, as in Photoshop): the tool whose brush the
    /// session brush holds, and the other tools' saved brushes.
    #[serde(default = "default_brush_tool")]
    pub brush_tool: Tool,
    #[serde(default)]
    pub tool_brushes: Vec<(Tool, photocraft_engine::paint::BrushSettings)>,
    /// Pen path under construction. Not saved: its `EditSubpath` has no serde.
    #[serde(skip)]
    pub pen: Option<crate::vector_ui::PenPath>,
    /// The document the pen path is drawn on; switching away commits it to that Work Path.
    #[serde(default)]
    pub pen_doc: Option<photocraft_doc::DocId>,
    /// The shape options' Stroke Options popover is open.
    #[serde(default)]
    pub shape_stroke_open: bool,
    /// The folders open in the shape Fill / Stroke swatch lists; kept for the session.
    #[serde(skip)]
    pub paint_folders: PresetState<SwatchKey>,
    /// The shape Fill / Stroke gradient lists' view and open groups, by their place in this
    /// session's lists; kept for the session.
    #[serde(skip)]
    pub shape_gradient_state: PresetState<GradientRef>,
    /// The arrow line (document id, layer id) whose arrowheads are the Line tool's default
    /// (Properties › Set as default): later edits to it are copied to the tool defaults too.
    #[serde(skip)]
    pub arrow_default: Option<(u64, u64)>,
    /// The path alignment popover's Align To choice (an index into its list).
    #[serde(skip)]
    pub path_align_to: usize,
    /// Direct Selection tool: selected anchors and the drag in progress (#790).
    #[serde(default)]
    pub direct_selection: crate::direct_select::DirectSelection,
    #[serde(skip)]
    pub type_path_geometry_cache: crate::type_path_placement::GeometryCache,
    /// Selected row in the Paths panel ("work" or a saved path name).
    #[serde(default)]
    pub selected_path: Option<String>,
    /// The document and Work Path deselected by making a selection from it with the Pen or Path
    /// Selection tool (#6079): the canvas hides that path until a path is selected again or the
    /// Work Path changes.
    #[serde(default)]
    pub hidden_work_path: Option<(u64, photocraft_doc::Path)>,
    /// The Layers panel filter: what it filters by (kind, name, effect, mode, attribute, colour,
    /// smart object) and each type's value (#5048, #5898, #10880, #10881).
    #[serde(default)]
    pub layer_filter: LayerFilter,
    /// The Layers panel options: thumbnail size and contents, the filter row and the other
    /// Panel Options. Kept with the preferences, not here.
    #[serde(skip)]
    pub layers_settings: LayersSettings,
    /// The Actions panel state, of which only Button Mode and the collapsed sets are saved. The
    /// list lives on the session.
    #[serde(default)]
    pub actions: crate::actions::ActionsUi,
    /// Layer Comps panel: the selected comp (by comp id).
    #[serde(default)]
    pub layer_comp_selected: Option<u32>,
    /// Preset panels (Gradients, Patterns, Styles, Shapes, Tool Presets, Clone Source).
    #[serde(default)]
    pub presets_ui: crate::preset_panels::PresetUi,
    /// Character/Paragraph Styles, Glyphs and Check Spelling (see `type_panels_ui`).
    #[serde(default)]
    pub type_panels: crate::type_panels_ui::TypePanelsUi,
    /// The font menus' filter bar (classes, favourites, variable, similar); kept for the session.
    #[serde(skip)]
    pub(crate) font_filters: craft::font_manager::picker::FontFilters,
    /// Ruler/Count/Note tools, Measurement Log and Notes panels (see `analysis`).
    #[serde(default)]
    pub analysis: crate::analysis::AnalysisUi,
    pub timeline: crate::timeline_ui::TimelineUi,
    /// Slice and Slice Select tools (see `slice_ui`).
    #[serde(default)]
    pub slices: crate::slice_ui::SliceUi,
    /// Frame tool (see `frame_tool`).
    #[serde(default)]
    pub frame_tool: crate::frame_tool::FrameToolUi,
    /// The contextual task bar as drawn in the last frame: its screen rect `[x0, y0, x1, y1]` in
    /// points, `None` while no bar shows (`ui.inspect` › `taskBar`).
    #[serde(skip)]
    pub task_bar_shown: Option<[f32; 4]>,
    /// Artboard tool (see `artboard_tool`).
    pub artboard_tool: crate::artboard_tool::ArtboardToolUi,
    /// Modifier Keys panel, custom pixel aspect ratios, workspace dialogs (see `workspace_ui`).
    #[serde(default)]
    pub shell: crate::workspace_ui::ShellUi,
    /// View extras: rulers (⌘R), grid (⌘'), guides (⌘;), snapping (⇧⌘;), locked guides (⌥⌘;).
    #[serde(default)]
    pub extras: Extras,
    /// View / Window / Type preferences: screen mode, Extras, Show and Snap To, flip, arrangement.
    #[serde(default)]
    pub view: crate::view_cmds::ViewOptions,
    pub panels: Panels,
    /// Views per open document (index-aligned with the session's documents).
    pub views: Vec<View>,
    pub windows: Vec<DocWindow>,
    /// The document groups: the docked ones the document area tiles and the floating windows,
    /// each with its tabs and the document it shows.
    #[serde(default)]
    pub document_windows: DocumentWindows<DocId>,
    /// The divider positions of the document area's tiled groups.
    #[serde(default)]
    pub split_at: SplitAt,
    /// A New Window's own view, keyed by its group id; a group without one shows its document's view.
    #[serde(default)]
    pub window_views: HashMap<u64, View>,
    /// The theme on screen, taken from the preferences and the suite each frame.
    #[serde(skip)]
    pub theme: crate::theme::AppTheme,
    /// The command palette: whether it is open and its recent picks.
    #[serde(default)]
    pub palette: crate::palette::PaletteState,
    /// The panel layout: every panel's place, the docks, the floating panels and where each one
    /// docks back to.
    /// One `Arc` across frames: the dock keys what it derives on the workspace's revision.
    #[serde(default = "default_docking")]
    pub docking: std::sync::Arc<craft::behavior::dock::Workspace<String>>,
    /// The panel open in the flyout of a column collapsed to icons (`Layout::opens_in_flyout`); the dock area shows it.
    /// Not saved: a flyout is closed at launch.
    #[serde(skip)]
    pub flyout: Option<String>,
    /// Which chip the Color panel edits, and how it shows the colour.
    #[serde(default)]
    pub color_panel: ColorPanelState,
    /// The Color panel options as last synced with the preferences: a stored value that differs
    /// from it was changed outside the panel (`prefs.set`, a reset) and is taken, not overwritten.
    #[serde(skip)]
    pub color_panel_synced: Option<serde_json::Value>,
    /// The Swatches panel's selection, open folders and view; kept for the session.
    #[serde(skip)]
    pub swatches: SwatchesState,
    /// Histogram panel options (view, channel, statistics, colours, source) and the tonal range
    /// dragged across its graph. The options are kept with the preferences
    /// (`histogram_panel::persist_options`), not here.
    #[serde(skip)]
    pub histogram: ScopePanelState,
    /// The History panel's selected snapshot: its document, the snapshot id and where the history
    /// stood when it was clicked (`history::selected_snapshot`).
    #[serde(skip)]
    pub history_snapshot: Option<(DocId, u64, crate::panels::history::HistorySpot)>,
    /// The History row to scroll into view on the next pass (`history::reveal`).
    #[serde(skip)]
    pub history_reveal: Option<craft::ui::patterns::panels::HistoryKey>,
    /// The toolbar's Edit Toolbar drawer is open.
    #[serde(skip)]
    pub toolbar_drawer: bool,
    /// The Properties panel's Transform link: W and H change together. On at start; not saved.
    #[serde(skip, default = "yes")]
    pub properties_linked: bool,
    /// Brush Settings: selected section (0 = Brush Tip Shape).
    #[serde(default)]
    pub brush_section: usize,
    /// The Brush Settings dynamic controls whose curve editor is open.
    #[serde(skip)]
    pub brush_curves_open: BTreeSet<&'static str>,
    /// Brushes panel: list or grid, zoom, collapsed groups and the search filter. Starts as a list.
    #[serde(default = "brushes_list")]
    pub(crate) brushes_panel: Tracked<PresetState<BrushKey>>,
    /// Marquee options-bar mode: 0 new, 1 add, 2 subtract, 3 intersect (modifier keys override).
    #[serde(default)]
    pub selection_mode: u8,
    #[serde(default)]
    pub tool_options: ToolOptions,
    /// The shape Fill / Stroke swatches' recently used gradients, newest first (the Gradient
    /// tab's Recent row).
    #[serde(default)]
    pub recent_gradients: Vec<photocraft_doc::Fill>,
    pub next_id: u64,
    /// Last status message (errors from commands, hints).
    pub status: String,
    /// The status message is an error (shown in the warning colour).
    #[serde(default)]
    pub status_error: bool,
    /// Notices (import/export warnings, files that couldn't open) posted since the last pass and
    /// not yet in the window's toast queue, newest last.
    #[serde(default)]
    pub notices: Vec<crate::notices::Notice>,
    /// Non-modal release-check window; readable and controllable through `ui.inspect`/`ui.set`.
    #[serde(default)]
    pub updates_open: bool,
    /// Pending GPU fallback warning, visible to automation.
    #[serde(default)]
    pub gpu_fallback_notice: Option<String>,
    /// Documents (ids) whose slow full refresh on the CPU compositor has had its notice.
    #[serde(default)]
    pub slow_refresh_noticed: Vec<u64>,
    /// Status bar info field, Home screen (see `chrome_ui`).
    #[serde(default)]
    pub chrome: crate::chrome_ui::ChromeState,
}

/// The panel layout at start and for a saved state without one.
fn default_docking() -> std::sync::Arc<craft::behavior::dock::Workspace<String>> {
    std::sync::Arc::new(crate::panels::registry::default_workspace())
}

/// The Brushes panel's state at start and for a saved state without one: the list view.
fn brushes_list() -> Tracked<PresetState<BrushKey>> {
    let mut state = PresetState::new();
    state.view = PresetView::List;
    Tracked::new(state)
}

impl Default for UiState {
    /// Creates the default UI state with default tools and empty drag state.
    fn default() -> Self {
        Self {
            tool: Tool::Move,
            group_tools: Vec::new(),
            spring: None,
            recent_files: Default::default(),
            recent_fonts: Vec::new(),
            text_edit: None,
            type_box_overflow: false,
            type_insertion_style: None,
            text_edit_return_tool: None,
            type_transform: None,
            type_transform_pivot: None,
            cam_transform: None,
            transform: None,
            symmetry_transform: None,
            layer_picker: None,
            mask_target: false,
            vector_mask_target: false,
            filter_mask_target: None,
            properties_layer: None,
            brush_picker: None,
            brush_picker_state: Tracked::default(),
            color_hud: None,
            canvas_tool_menu: None,
            selected_guide: None,
            brush_tool: Tool::Brush,
            tool_brushes: Vec::new(),
            clone_source: None,
            clone_offset: None,
            pattern_stamp_phase: None,
            extras: Extras::default(),
            view: Default::default(),
            actions: Default::default(),
            layer_comp_selected: None,
            presets_ui: Default::default(),
            type_panels: Default::default(),
            font_filters: Default::default(),
            analysis: Default::default(),
            timeline: Default::default(),
            slices: Default::default(),
            frame_tool: Default::default(),
            task_bar_shown: None,
            artboard_tool: Default::default(),
            shell: Default::default(),
            layer_filter: LayerFilter::default(),
            layers_settings: LayersSettings::default(),
            pen: None,
            pen_doc: None,
            shape_stroke_open: false,
            paint_folders: PresetState::new(),
            shape_gradient_state: PresetState::new(),
            arrow_default: None,
            path_align_to: 0,
            direct_selection: Default::default(),
            type_path_geometry_cache: Default::default(),
            selected_path: None,
            hidden_work_path: None,
            panels: Panels::default(),
            views: Vec::new(),
            windows: Vec::new(),
            document_windows: DocumentWindows::default(),
            split_at: SplitAt::default(),
            window_views: HashMap::new(),
            theme: crate::theme::AppTheme::default(),
            palette: Default::default(),
            docking: default_docking(),
            flyout: None,
            color_panel: Default::default(),
            color_panel_synced: None,
            swatches: SwatchesState::default(),
            histogram: ScopePanelState::default(),
            history_snapshot: None,
            history_reveal: None,
            toolbar_drawer: false,
            properties_linked: true,
            brush_section: 0,
            brush_curves_open: BTreeSet::new(),
            brushes_panel: brushes_list(),
            selection_mode: 0,
            tool_options: ToolOptions::default(),
            recent_gradients: Vec::new(),
            next_id: 1,
            status: String::new(),
            status_error: false,
            notices: Vec::new(),
            updates_open: false,
            gpu_fallback_notice: None,
            slow_refresh_noticed: Vec::new(),
            chrome: Default::default(),
        }
    }
}

impl UiState {
    /// Records `tool` as the last one used in its shortcut group and toolbar slot.
    pub fn remember_group_tool(&mut self, tool: Tool) {
        if self.group_tools.first() != Some(&tool) {
            self.group_tools.retain(|t| *t != tool);
            self.group_tools.insert(0, tool);
        }
    }

    /// The last tool used in the shortcut group of `key`, if any.
    #[cfg(test)]
    pub fn group_tool(&self, key: char) -> Option<Tool> {
        self.group_tools.iter().copied().find(|t| t.key() == key)
    }

    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_all_layers_is_per_tool_and_old_prefs_load() {
        // The single shared flag of older preferences is dropped: every tool starts off (#5524).
        let o: ToolOptions = serde_json::from_str(r#"{"sample_all_layers": true, "feather": 3.0}"#).unwrap();
        assert_eq!((o.sample_all, o.feather), (SampleAllLayers::default(), 3.0));
        let mut o = ToolOptions::default();
        o.sample_all.magic_wand = true;
        let back: ToolOptions = serde_json::from_value(serde_json::to_value(&o).unwrap()).unwrap();
        assert!(back.sample_all.magic_wand && !back.sample_all.get(Tool::Blur));
        assert!(!back.sample_all.get(Tool::Brush), "a tool without the option reads false");
        let mut s = SampleAllLayers::default();
        assert!(s.slot(Tool::Brush).is_none());
        for t in [Tool::MagicWand, Tool::PaintBucket, Tool::Blur, Tool::Sharpen, Tool::Smudge, Tool::Remove] {
            *s.slot(t).unwrap() = true;
            assert!(s.get(t), "{t:?}");
        }
        assert!(!s.get(Tool::QuickSelection));
    }

    #[test]
    fn tool_names_parse() {
        basic_and_retouch_tool_names_parse();
        vector_tool_names_parse();
    }

    fn basic_and_retouch_tool_names_parse() {
        assert_eq!(Tool::from_name("brush"), Some(Tool::Brush));
        assert_eq!(Tool::from_name("Rect"), Some(Tool::RectMarquee));
        assert_eq!(Tool::from_name("RectMarquee"), Some(Tool::RectMarquee));
        assert_eq!(Tool::from_name("Eraser Tool"), Some(Tool::Eraser));
        assert_eq!(Tool::from_name("mixerBrush"), Some(Tool::MixerBrush));
        assert_eq!(Tool::from_name("Mixer Brush Tool"), Some(Tool::MixerBrush));
        assert_eq!(Tool::from_name("colorReplacement"), Some(Tool::ColorReplacement));
        assert_eq!(Tool::from_name("Color Replacement Tool"), Some(Tool::ColorReplacement));
        assert_eq!(Tool::from_name("redEye"), Some(Tool::RedEye));
        assert_eq!(Tool::from_name("Red Eye Tool"), Some(Tool::RedEye));
        assert_eq!(Tool::from_name("artHistoryBrush"), Some(Tool::ArtHistoryBrush));
        assert_eq!(Tool::from_name("Art History Brush"), Some(Tool::ArtHistoryBrush));
        assert_eq!(Tool::from_name("Art History Brush Tool"), Some(Tool::ArtHistoryBrush));
        assert_eq!(Tool::RedEye.key(), 'J');
        assert!(!Tool::RedEye.is_brushlike());
        assert_eq!(Tool::from_name("patternStamp"), Some(Tool::PatternStamp));
        assert_eq!(Tool::from_name("Pattern Stamp Tool"), Some(Tool::PatternStamp));
        assert_eq!(Tool::ALL.len(), 69);
    }

    fn vector_tool_names_parse() {
        assert_eq!(Tool::from_name("Vertical Type Mask Tool"), Some(Tool::VerticalTypeMask));
        assert_eq!(Tool::from_name("addAnchorPoint"), Some(Tool::AddAnchorPoint));
        assert_eq!(Tool::from_name("Delete Anchor Point Tool"), Some(Tool::DeleteAnchorPoint));
        assert_eq!(Tool::DeleteAnchorPoint.key(), ' ');
        assert_eq!(Tool::from_name("convertPoint"), Some(Tool::ConvertPoint));
        assert_eq!(Tool::from_name("Convert Point Tool"), Some(Tool::ConvertPoint));
        assert_eq!(Tool::ConvertPoint.key(), ' ', "P never reaches it");
        assert!(!Tool::ConvertPoint.is_brushlike());
        assert_eq!(Tool::from_name("Remove Tool"), Some(Tool::Remove));
        assert_eq!(Tool::Remove.key(), 'J');
        assert!(Tool::Remove.is_brushlike());
        assert_eq!(Tool::from_name("RotateView"), Some(Tool::RotateView));
        assert_eq!(Tool::from_name("Rotate View Tool"), Some(Tool::RotateView));
        assert_eq!(Tool::RotateView.key(), 'R');
        assert_eq!(Tool::ArtHistoryBrush.key(), 'Y');
        assert!(Tool::ArtHistoryBrush.is_brushlike());
        assert_eq!(Tool::from_name("nope"), None);
    }

    /// The README's "Everything in the box" tool heading and list name exactly the tools a user can
    /// pick, `Tool::ALL` (#996).
    #[test]
    fn readme_tool_list_matches_tool_all() {
        let readme = include_str!("../../../README.md");
        let (_, rest) = readme.split_once("<h4>🧰 ").unwrap();
        let (count, rest) = rest.split_once(" tools</h4>").unwrap();
        assert_eq!(count.parse::<usize>().unwrap(), Tool::ALL.len(), "README.md tool heading");
        let list = rest.trim_start().lines().next().unwrap();
        // "Rectangular and Elliptical Marquee" names two tools: "Rectangular Marquee", "Elliptical Marquee".
        let mut named: Vec<String> = Vec::new();
        for entry in list.split(" · ") {
            match entry.split_once(" and ").and_then(|(first, rest)| Some((first, rest.split_once(' ')?))) {
                Some((first, (second, noun))) => named.extend([format!("{first} {noun}"), format!("{second} {noun}")]),
                None => named.push(entry.to_owned()),
            }
        }
        named.sort();
        let mut labels: Vec<String> = Tool::ALL.iter().map(|tool| tool.label().trim_end_matches(" Tool").to_owned()).collect();
        labels.sort();
        assert_eq!(named, labels, "README.md tool list");
    }

    #[test]
    fn ui_state_serializes() {
        let s = UiState::default();
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["tool"], "Move");
        let back: UiState = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }
}
