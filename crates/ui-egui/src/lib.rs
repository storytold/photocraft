//! PhotoCraft's first UI shell, built on egui/eframe.
//!
//! This crate is deliberately thin. Every action goes through
//! [`photocraft_engine::Session::execute`], and all UI state lives in [`state::UiState`] (plain
//! data). The [`control`] module exposes both to automation, so agents can drive and inspect every
//! part of the interface.
#![forbid(unsafe_code)]
// `control::inspect` builds the whole UI snapshot in one `json!`, past the default limit of 128.
#![recursion_limit = "256"]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// PhotoCraft as craft-update knows it, with its 128 px icon and accent colour: the update
/// prompt shows both, and an AppImage's menu entry gets the icon.
#[cfg(not(target_arch = "wasm32"))]
pub const UPDATE_APP: craft::update::app::App = craft::update::app::App {
    icon_png: Some(include_bytes!("../../../assets/app-icon/hicolor/128x128/apps/ai.storyteller.photocraft.png")),
    accent: Some([0x2f, 0x7b, 0xf5]),
    ..craft::update::suite::PHOTOCRAFT
};

/// Translate a string literal into the current UI language: `tl!("Image Size…")`.
macro_rules! tl {
    ($s:expr) => {
        $crate::i18n::t($s)
    };
}
mod brush_mode_menu;
pub mod canvas;
mod channel_titles;
#[cfg(not(target_arch = "wasm32"))]
pub mod control;
pub mod dialogs;
pub mod dock;
pub(crate) mod edit_session;
pub mod file_dialog;
pub mod file_open;
pub mod frame_stages;
pub mod gpu_memo;
pub mod gpu_status;
mod history_keys;
pub mod home_recent_ui;
pub mod i18n;
mod icon_data;
pub mod icons;
pub mod jobs_ui;
pub(crate) mod last_brush;
pub mod levels_clipping;
pub mod links;
pub mod memory_warning;
pub mod menu_catalog;
pub mod menus;
mod modifier_keys;
pub mod monitor_status;
pub mod notices;
pub mod os_window;
pub mod panels;
pub mod preset_files_ui;
pub mod proxy;
pub mod render_path;
pub(crate) mod scratch_reads;
pub mod state;
pub mod texel_guard;
pub mod theme;
pub mod tone;
pub(crate) mod tool_memory;
pub mod updates_ui;

use std::collections::HashMap;

use photocraft_doc::{DocId, Document};
use photocraft_engine::Session;
use serde_json::{Value, json};

#[cfg(not(target_arch = "wasm32"))]
pub use control::{ControlRequest, ControlResponse};
pub use craft::window::{
    Prefix,
    policy::{self as window_policy, Backend, Decision, Frame, TitleBarPref},
};
#[cfg(target_arch = "wasm32")]
pub use file_dialog::{FileDialogAnswer, FileDialogFn, FileDialogReply, FileDialogRequest};
pub use file_open::OsEvent;
pub use state::{Tool, UiState};

mod app;
mod input;
mod prefs;
mod services;
#[cfg(not(target_arch = "wasm32"))]
mod smart_svg_ui;
#[cfg(target_arch = "wasm32")]
#[path = "smart_svg_ui/browser.rs"]
mod smart_svg_ui;
mod text;
mod tools;

pub(crate) use app::adjustment_layer_active;
#[cfg(test)]
use app::layer_thumb_frame;
pub use canvas::artboard_ui;
pub(crate) use canvas::background_redraw;
pub(crate) use canvas::cam_preview;
pub use canvas::canvas_tool_menu;
pub use canvas::channel_view;
pub use canvas::gpu_canvas;
pub use canvas::guide_select;
pub use canvas::layer_pick_ui;
pub use canvas::move_ui;
pub use canvas::outline;
pub use canvas::paint_mouse;
pub use canvas::patch_preview;
pub use canvas::quick_pick;
pub use canvas::rulers;
pub use canvas::sampling;
pub use canvas::scroll_all_windows;
pub use canvas::select_mask_view;
pub use canvas::snap_ui;
pub use canvas::tool_feedback;
pub use dialogs::adjust_dialog;
pub use dialogs::adjust_editors;
pub use dialogs::adjust_preview;
pub(crate) use dialogs::bitmap_dialog;
pub use dialogs::blur_pins;
pub use dialogs::brush_preset_dialog;
pub use dialogs::caf_ui;
use dialogs::camera_raw_gpu;
pub use dialogs::camera_raw_ui;
pub use dialogs::channel_options_ui;
pub(crate) use dialogs::color_picker_preview;
pub use dialogs::color_picker_ui;
pub use dialogs::color_range_ui;
pub use dialogs::color_table_ui;
pub use dialogs::count_group_name_ui;
pub use dialogs::credits;
pub use dialogs::dds_options_ui;
pub use dialogs::delete_layer_prompt;
pub use dialogs::discard_ui;
pub use dialogs::disk_change_prompt;
pub use dialogs::distort_ui;
pub use dialogs::duplicate_layer_ui;
pub use dialogs::equalize_ui;
pub use dialogs::export_dialog;
pub use dialogs::file_ui;
pub use dialogs::fill_ui;
pub use dialogs::filter_dialog;
use dialogs::filter_preview_ui;
pub(crate) use dialogs::format_prompts;
pub use dialogs::gallery_ui;
use dialogs::guide_edit;
pub use dialogs::guide_layout_ui;
pub use dialogs::history_options_ui;
pub use dialogs::jpeg_options_ui;
pub use dialogs::layer_picker;
pub use dialogs::layer_style;
pub use dialogs::liquify_ui;
pub use dialogs::mask_trash_prompt;
pub use dialogs::new_doc_ui;
pub use dialogs::new_group_ui;
pub use dialogs::new_guide_ui;
pub use dialogs::new_layer_ui;
pub use dialogs::open_prompt;
pub use dialogs::palette;
pub use dialogs::paragraph_text_size;
pub(crate) use dialogs::pattern_fill_ui;
pub use dialogs::preset_group_prompt;
pub use dialogs::rasterize_prompt;
pub use dialogs::screen_picker;
pub use dialogs::select_mask_ui;
pub use dialogs::shape_dialog;
pub(crate) use dialogs::shape_fill_ui;
use dialogs::sizing;
pub use dialogs::smart_filter_dialog;
pub use dialogs::snapshot_dialog;
pub(crate) use dialogs::solid_fill_ui;
pub use dialogs::stack_dialog;
pub use dialogs::stroke_ui;
pub use dialogs::targa_options_ui;
pub use dialogs::tiff_options_ui;
pub use dialogs::vp_ui;
pub use dialogs::wide_angle_ui;
use input::alt_grab;
use input::blend_keys;
pub use input::hold_keys;
pub use input::nudge_bundle;
use input::opacity_keys;
pub use input::shortcut_dispatch;
pub use input::shortcuts;
pub use input::spring_tools;
pub use input::stylus;
use input::tool_shortcuts;
pub use input::touch_twins;
pub use input::window_keys;
pub use menus::enable_rules;
pub use menus::native_menu;
pub use menus::parity;
pub use menus::plugin_ui;
pub use menus::view_cmds;
pub use menus::workspace_ui;
pub use panels::actions;
pub use panels::adjust_ui;
pub use panels::apply_color;
pub(crate) use panels::blend_preview;
pub use panels::brush_panel;
pub use panels::brush_preview;
pub use panels::brush_sections;
pub use panels::channels_panel;
pub use panels::chrome_ui;
pub use panels::clip_line_ui;
pub use panels::color_hud;
pub use panels::color_panel_ui;
pub use panels::comps_ui;
pub use panels::doc_props_ui;
pub use panels::histogram_panel;
pub use panels::history_menu_ui;
pub use panels::layer_filter_ui;
pub use panels::layer_menu_ui;
pub use panels::layer_props_ui;
use panels::layer_reveal;
pub use panels::layer_row_ui;
use panels::layer_transfer;
pub use panels::layers_panel_ui;
use panels::mask_props_ui;
pub use panels::mask_thumbs_ui;
use panels::panel_docking;
pub use panels::preset_panels;
pub use panels::smart_ui;
pub use panels::swatches_ui;
use panels::timeline_ui;
pub use panels::type_panels_ui;
use panels::variables_ui;
pub use prefs::kys_import;
pub use prefs::prefs_ui;
use prefs::pressure_curve_ui;
#[cfg(not(target_arch = "wasm32"))]
pub use services::AgentAutomationFn;
pub use services::AutomationReadResult;
pub(crate) use services::SAVE_OPTIONS_OPEN;
#[cfg(test)]
pub(crate) use services::saved_revision_after_write;
pub use services::{
    AdoptAutosaveFn, AppendTextFn, AutomationAuthorizeFn, AutomationCommandFn, AutomationLocateFn, AutomationOpenPcraftFn, AutomationReadFn,
    AutomationServices, AutomationWriteFn, AutosaveFn, AutosaveResultsFn, CapsLockFn, ClipboardGetFn, ClipboardGetSvgFn, ClipboardSetFn, ClipboardUnreadableFn,
    CursorPosFn, DiscardAutosaveFn, ExportFn, ExportSettings, ExternalSvgEditor, ExternalSvgSession, ExternalVectorSave, ExternalVectorSource, FileStamp,
    FileStampFn, FormatChoices, ImportFn, Inbox, LoadTextFn, OpenExternalSvgFn, OpenUrlFn, OsEventsFn, PenEndFn, PhotoshopShortcutsFn, PlaceInbox,
    PrimaryDownFn, RecoverFn, Recoverable, RightButtonDownFn, SameFileFn, SaveFileFn, SaveTextFn, Services, SystemThemeFn, TabletPollFn, WindowFrameFn,
    WriteFn,
};
pub(crate) use services::{opaque_layer, saves_flat_in_place};
pub use text::cjk_fonts;
pub use text::direction;
pub use text::served_fonts;
pub use tools::adjustment_brush;
use tools::align_bar;
pub use tools::analysis;
pub use tools::artboard_tool;
pub use tools::bg_eraser_popup;
pub(crate) use tools::brush_cursor;
pub use tools::brush_picker;
pub use tools::brush_refusal;
pub use tools::brush_resize;
use tools::cam_transform;
pub use tools::cas_transform;
pub use tools::crop_presets;
pub use tools::crop_task_bar;
pub use tools::crop_ui;
pub use tools::direct_select;
pub use tools::eraser_ui;
pub use tools::eyedropper_ui;
pub use tools::frame_tool;
pub use tools::gradient_ui;
pub use tools::magnetic_lasso_ui;
pub use tools::move_box;
pub use tools::move_distance;
pub use tools::move_hover;
pub use tools::move_lock;
pub use tools::move_mods;
pub use tools::object_finder;
pub use tools::option_cells;
use tools::paint_picker;
use tools::path_components;
pub use tools::perspective_crop_ui;
pub use tools::perspective_ui;
pub use tools::puppet_ui;
pub use tools::retouch_ui;
pub use tools::shape_controls;
use tools::shape_gradient_ui;
use tools::shape_options;
use tools::shape_paint_ui;
pub use tools::shape_stroke_ui;
pub use tools::slice_ui;
pub use tools::stroke_constraint;
pub use tools::stroke_trace;
pub use tools::stroke_trail;
pub use tools::symmetry_ui;
use tools::tool_reset;
pub use tools::transform_fx;
pub use tools::transform_task_bar;
pub use tools::transform_tool;
use tools::type_box_preview;
use tools::type_font_preview;
use tools::type_path_placement;
pub use tools::type_tool;
use tools::type_transform;
pub use tools::vector_ui;

/// The Info panel's cached sample: pixel x, y, document revision, Eyedropper Sample Size and the view zoom's bits.
type InfoSampleKey = (DocId, i32, i32, u64, u32, u32);

pub struct PhotocraftApp {
    pub session: Session,
    pub ui: UiState,
    pub services: Services,
    /// Canvas caches per (document, display): CPU textures hold monitor values; the GPU
    /// canvas state is shared (`canvas::GPU_OUTPUT`).
    canvases: HashMap<(DocId, u32), canvas::CanvasCache>,
    /// The revisions each document's canvas raster sources handed out per display (`canvas::PhotoComposite`, `canvas::ChannelOverlay`).
    raster_logs: HashMap<(DocId, u32), canvas::SourceLogs>,
    /// Navigator textures belong to open documents, not the lifetime of the egui context.
    navigator_textures: HashMap<DocId, canvas::NavigatorCache>,
    /// `canvas::display_doc`'s live-adjust and Duotone documents per document: (snapshot,
    /// preview key, the document shown).
    display_memo: HashMap<DocId, (std::sync::Weak<Document>, u64, std::sync::Arc<Document>)>,
    /// The main canvas's last GPU view of each document while it shows the plain document on
    /// the GPU canvas: the Navigator draws the same texture at its own size (#4518).
    navigator_gpu: HashMap<DocId, crate::gpu_canvas::ViewParams>,
    /// Documents whose GPU canvas texture holds the plain document (`canvas::ensure_gpu`).
    navigator_plain: std::collections::HashSet<DocId>,
    /// Viewport pages of documents too large for their GPU texture set (`canvas::viewport_pages`).
    streams: HashMap<DocId, canvas::viewport_pages::Stream>,
    /// The Info panel's Color Sampler readouts for (document, revision) (`panels::sampler_readouts`).
    info_samplers: Option<panels::SamplerReadouts>,
    /// Display profile readings (#569).
    monitors: monitor_status::State,
    pub(crate) thumbnail_cache: std::sync::Arc<std::sync::Mutex<crate::home_recent_ui::ThumbnailCache>>,
    /// The canvas tools' own state: every tool family's gesture, hover and option state
    /// (`canvas::tools`).
    pub(crate) canvas_tools: canvas::tools::CanvasTools,
    /// Brush/Eraser stroke being drawn, rendered by the engine (see `canvas::LiveStroke`).
    live_stroke: Option<canvas::LiveStroke>,
    /// A live stroke just committed with its own pixels: the canvas caches that showed it
    /// refresh only what it drew after the step they showed (`canvas::damage_since`).
    committed_stroke: Option<canvas::CommittedStroke>,
    /// Whether the last retouching stroke's commit took the pixels its live stroke drew
    /// (`"fromLive"`) instead of rendering the stroke again (`retouch_ui::finish_stroke`).
    pub(crate) retouch_from_live: bool,
    /// Footprint trail of a retouching drag (see `stroke_trail`).
    trail: Option<stroke_trail::Trail>,
    /// Remove tool strokes waiting to be applied together (Remove after each stroke off), the
    /// document, layer and channel they belong to, and their tint on the canvas (`retouch_ui`).
    /// Each as a `paint.remove` `strokes` entry: its points and the size, hardness and spacing it
    /// was drawn with, which its tint shows and its removal uses.
    pub(crate) remove_pending: Vec<serde_json::Value>,
    pub(crate) remove_pending_target: Option<retouch_ui::RemoveTarget>,
    pub(crate) remove_trail: Option<stroke_trail::Trail>,
    /// The numbers that keep apart the textures the app makes for its canvas previews (transform previews, stroke
    /// trails, Object Finder masks): a preview made again under the same name gets a number no earlier one had, so
    /// the texture store never shows the old picture for it. One counter for the whole app, since previews of a
    /// name are made in several places (a transform begins in one, re-renders in another).
    pub(crate) texture_serials: craft::ui::primitives::Serials,
    /// The menu bar's rows and the English text of its titles, rebuilt when their inputs change
    /// (`menus::MenuModel`).
    pub(crate) menu_model: Option<menus::MenuModel>,
    /// The toolbar's slots for the stored layout (`tool_commands::ToolbarSlots`).
    pub(crate) toolbar_slots: panels::tool_commands::ToolbarSlots,
    /// The dock's spec model and kept panels (`panel_docking::DockModel`).
    pub(crate) dock_model: panels::panel_docking::DockModel,
    /// The patterns and styles the preset views draw from, held while unchanged (`preset_browser::PresetArt`).
    pub(crate) preset_art: panels::preset_browser::PresetArt,
    /// Move tool drag shown live (`move_ui`).
    pub(crate) move_preview: Option<move_ui::MovePreview>,
    /// Move tool › Show Highlight on Rollover: the last hover answer (`move_hover`).
    pub(crate) move_hover: Option<move_hover::Hover>,
    /// Smart Guides distances shown with ⌘ held over the canvas: the last answer (`move_distance`).
    pub(crate) move_distance: Option<move_distance::Cache>,
    /// Object Selection › Object Finder: the hover preview (`object_finder`).
    pub(crate) object_finder: Option<object_finder::Finder>,
    /// A blend mode hovered in the Layers panel, shown live (`blend_preview`).
    pub(crate) blend_preview: Option<blend_preview::BlendPreview>,
    /// Layers panel drag: temporary reordered document displayed until mouse release.
    pub(crate) reorder_preview: Option<layers_panel_ui::ReorderPreview>,
    /// Whether the currently dragged layer is over a valid destination this frame.
    pub(crate) reorder_preview_hit: bool,
    /// Patch Tool drag: the healed document at the pointer (`patch_preview`).
    pub(crate) patch_preview: Option<patch_preview::PatchPreview>,
    /// Content-Aware Move drag or box shown with its result (`cam_preview`).
    pub(crate) cam_preview: Option<cam_preview::CamPreview>,
    /// Image › Apply Data Set…'s Preview: the set shown in the document (`apply_data_set_ui`).
    pub(crate) data_set_preview: panels::apply_data_set_ui::PreviewState,
    /// The Brush or Pencil erase key is held this frame (`hold_keys::erase_held`, #3439).
    held_erase: bool,
    /// Position and pen pressure at the last stroke's endpoint: ⇧-click interpolates from them (#178, #2827).
    last_stroke_end: Option<(DocId, [f64; 3])>,
    /// Control+Alt-drag brush resize in progress (`brush_resize`, #231).
    pub(crate) brush_resize: brush_resize::PaintResize,
    /// The layer a canvas click picked the Move tool way (its Auto-Select, or a ⌘-click from another
    /// tool) and the engine's active-layer change count right after that pick
    /// (`Session::active_layer_changes`). The Layers panel's reveal reads it when it next draws, and
    /// accepts it only if nothing changed the active layer since, however long ago (`layer_reveal`, #11381).
    pub(crate) move_pick: Option<(photocraft_doc::LayerId, u64)>,
    /// Caps Lock toggled on, read from the OS each frame (`services.caps_lock`): painting tools
    /// show the precise crosshair whatever the cursor preference (#1758).
    pub caps_lock: bool,
    /// The first digit of a two-digit opacity typed on the number keys (`opacity_keys`, #352).
    pub(crate) opacity_keys: craft::behavior::DigitKeys,
    /// The control channel: requests from a transport, the replies waiting on input, jobs and
    /// screenshots, and the screenshot queue (`app/control.rs`).
    #[cfg(not(target_arch = "wasm32"))]
    control: Option<craft::automation::ControlChannel>,
    /// Live (uncommitted) adjustment edit shown on canvas while a slider is dragged.
    pub live_adjust: Option<(photocraft_doc::LayerId, Value)>,
    /// Frames rendered (for tests and the status bar).
    pub frame: u64,
    /// What the notice stack keeps between frames: the pointer rest that gives a fresh auto-hide
    /// delay in the frame the pointer leaves (`notices::show`).
    pub(crate) notices_live: notices::Live,
    /// The low-memory warning's schedule and state (`memory_warning`).
    pub(crate) low_memory: memory_warning::Watch,
    /// Screen rect of the main canvas last frame (for overlays and the navigator).
    pub last_canvas_rect: egui::Rect,
    /// The egui layer the main canvas was drawn on last frame: preset drops target it.
    pub last_canvas_layer: egui::LayerId,
    /// Physical pixels per egui point of the canvas last frame (`ctx.pixels_per_point`, which
    /// folds in both the display scale and Interface › UI Scale). The canvas maps document pixels
    /// to physical pixels, so point-space geometry divides the view zoom by this (see
    /// [`Self::point_zoom`]).
    pub ppp: f32,
    /// The document area showing the active document's canvas last frame (not the tabs, the
    /// start screen or an opening file's card): files dropped here are placed as layers.
    pub(crate) drop_canvas_rect: Option<egui::Rect>,
    /// Where a floating document window's title bar dropped docks its document, in the main window's points, as of
    /// the last frame: the document tab strip, or a band of its height at the top of the document area (#16415).
    pub(crate) dock_drop_rect: Option<egui::Rect>,
    /// The window's one egui context, for code without one in hand: SAVED-tier reads and writes,
    /// the Frame tool's transforms from pointer events, and the mapping and panels that read the
    /// canvas's zoom ease (`document_display::shown_camera`). Read it through [`Self::ctx`].
    ctx: egui::Context,
    pub(crate) frame_preview: Option<frame_tool::Preview>,
    /// Files dropped on the canvas still to place, one Free Transform at a time, each with the
    /// document it was dropped on.
    pub(crate) drop_places: std::collections::VecDeque<(DocId, file_open::DropPlace)>,
    /// The document each of `ui.views` belongs to, as of the last [`Self::sync_views`].
    view_docs: Vec<DocId>,
    /// The document `ui.selected_path` belongs to, as of the last [`Self::sync_views`]: a Paths
    /// panel selection never carries over to a same-named path in another tab.
    path_document: Option<(DocId, u64, std::sync::Weak<Document>)>,
    pub fps: f32,
    pub(crate) last_frame_time: f64,
    /// Content bounds cached per (key, revision): scanning a 36 MP layer every frame cost ~77 ms.
    bounds_cache: HashMap<u64, (u64, photocraft_geom::Rect)>,
    /// Downsampled proxy of the active document for live previews: (doc, revision, k, proxy).
    pub(crate) proxy: Option<(DocId, u64, u32, std::sync::Arc<Document>)>,
    /// Key of the preview currently uploaded to the GPU (doc, params hash).
    pub(crate) proxy_uploaded: Option<(DocId, u64)>,
    /// Live filter preview (proxy document with the filter applied).
    pub(crate) guide_edit: Option<guide_edit::Draft>,
    pub(crate) filter_preview: Option<filter_dialog::FilterPreview>,
    /// The live preview of the parameter dialogs' own filters, on the dialog lane.
    pub(crate) filter_live: craft::imaging::ui::LivePreview,
    /// The Image › Adjustments dialog that is open: its layer and values (`adjust_dialog`).
    pub(crate) adjust_open: Option<adjust_dialog::OpenAdjust>,
    /// The Properties panel's adjustment edit in progress, with the tone editor's shown channel
    /// (`adjust_editors::PropertiesAdjust`).
    pub(crate) adjust_session: Option<adjust_editors::PropertiesAdjust>,
    /// Twice the passes the Properties panel was not in front or ran without showing the Reset session's layer: the
    /// session (`adjust_editors`) takes a layer's settings anew after the panel stopped showing it. A pass count would
    /// also move while the panel replays its memoized body, which is still showing it.
    pub(crate) properties_hidden: u64,
    /// Whether the Properties panel's run showed the Reset session's layer (`properties_hidden`).
    pub(crate) properties_tracked: bool,
    /// The Color Picker dialog asking for a screen sample: the dialog and the field that took it.
    pub(crate) dialog_dropper: Option<(craft::ui::dialog::DialogId, String)>,
    /// The child dialog (colour picker or contour editor) the open Layer Style dialog has, with that dialog's id.
    pub(crate) layer_style_child: Option<(craft::ui::dialog::DialogId, dialogs::layer_style::Child)>,
    /// What each open Color Picker edits (`color_picker_ui`).
    pub(crate) color_targets: HashMap<craft::ui::dialog::DialogId, color_picker_ui::PickTarget>,
    /// Colours a Color Picker answered with OK, waiting for their owner: (dialog, field, colour).
    pub(crate) color_answers: Vec<(craft::ui::dialog::DialogId, String, craft::theme::Srgba)>,
    /// The screen-colour pick in progress (`screen_picker`).
    pub(crate) screen_pick: craft::color_ui::ui::ScreenPick,
    /// Select and Mask / Content-Aware Fill / Liquify / Gallery / Vanishing Point sessions.
    pub(crate) caf: Option<caf_ui::CafApp>,
    pub(crate) liquify: Option<liquify_ui::LiquifyApp>,
    pub(crate) gallery: Option<gallery_ui::GalleryApp>,
    pub(crate) vanishing: Option<vp_ui::VpApp>,
    /// The Layer Style, Select and Mask and parameter dialogs' open targets, one field per family,
    /// keyed by the dialog.
    pub(crate) layer_style_target: Option<(craft::ui::dialog::DialogId, DocId, photocraft_doc::LayerId)>,
    pub(crate) select_mask_target: Option<(craft::ui::dialog::DialogId, DocId, photocraft_doc::LayerId, std::sync::Arc<select_mask_ui::PcSelectMaskSource>)>,
    /// The Guide Layout dialog's document.
    pub(crate) guide_layout_doc: Option<(craft::ui::dialog::DialogId, DocId)>,
    /// The Export As dialog that is open.
    pub(crate) export_open: Option<export_dialog::ExportOpen>,
    /// The layer the Layers panel reveals when it next draws, with the groups to open for it
    /// (`layer_reveal::{mark, file}`).
    pub(crate) pending_reveal: Option<(photocraft_doc::LayerId, craft::layers::ui::panel::Groups)>,
    /// The Pattern Fill dialog's preview and open state (`pattern_fill_ui`).
    pub(crate) pattern_fill: pattern_fill_ui::State,
    #[cfg(not(target_arch = "wasm32"))]
    filter_preview_worker: crate::dialogs::filter_preview_worker::Worker,
    /// Full CPU canvas redraws running in the background, by document (#4024).
    pub(crate) redraws: std::collections::HashMap<photocraft_doc::DocId, background_redraw::Redraw>,
    /// The last finished background redraws, for going back to a state drawn a moment ago (#6528).
    pub(crate) kept: background_redraw::KeptComposites,
    /// Select › Color Range dialog preview (proxy document + mask / image textures).
    pub(crate) color_range: Option<color_range_ui::Preview>,
    /// Image › Adjustments dialog preview through a temporary clipped adjustment layer.
    pub(crate) adjust_preview: Option<adjust_preview::AdjustPreview>,
    /// The Properties panel's live adjustment-layer drag: its damage region and coverage
    /// (`adjust_preview::live_key`).
    pub(crate) live_adjust_preview: Option<adjust_preview::LiveAdjust>,
    /// The Levels dialog's Alt-drag clipping display over the canvas.
    pub(crate) levels_clipping: Option<levels_clipping::Cache>,
    /// The active document injected input was queued for (`ui.click`, `ui.key`, …), while it is
    /// queued: a switch of the active document withdraws it (`withdraw_stale_input`).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) input_queued_for: Option<InputTarget>,
    /// True only while a pass's input carries injected automation input
    /// (`WindowHost::injected_pass`). It makes shortcut- and dialog-triggered commands pass the
    /// same policy as direct control calls.
    pub(crate) automation_input: bool,
    /// The direct control-command route keeps Actions playback synchronous and dialog-free.
    pub(crate) programmatic_playback: bool,
    /// How many `run_command` calls are on the stack. Only the outermost one writes the frame's
    /// command stage: a nested run (`file.save` → `layer.smartObjects.saveContents`,
    /// `actions.play` → its steps) is already inside the outer call's time.
    run_depth: u32,
    /// Action playbacks paused at a step dialog, by the token in that dialog's fields
    /// ([`actions::RESUME`]): what to play next and the automation gate it paused under.
    pub(crate) paused_playbacks: actions::PausedPlaybacks,
    /// Levels/Curves histogram cache: (document, adjustment layer, revision it is valid for).
    pub(crate) tone_hist: Option<(DocId, photocraft_doc::LayerId, u64, std::sync::Arc<tone::Histograms>)>,
    /// Histogram panel: the document's histogram (`histogram_panel`).
    pub(crate) doc_hist: Option<histogram_panel::PanelHist>,
    /// Histogram panel: the histogram of the adjustment previewing on the canvas.
    pub(crate) doc_hist_preview: Option<histogram_panel::PanelHist>,
    /// The panel histogram being computed on a worker thread.
    pub(crate) hist_job: Option<histogram_panel::HistJob>,
    /// The (document, revision) an Uncached Refresh asked to count at cache level 1.
    pub(crate) hist_uncached: Option<(DocId, u64)>,
    /// Memoized `tone::selection_key` of the active document's current revision
    /// (`histogram_panel::update` refreshes it once per frame; the hash walks every tile of the
    /// selection mask, while any change to the selection arrives as a new document revision).
    pub(crate) hist_sel_key: Option<(DocId, u64, u64)>,
    /// Free Transform preview (document without the moving pixels + their texture).
    pub(crate) transform_preview: Option<transform_tool::OpenPreview>,
    /// The canvas area the last Free Transform box's preview changes (`transform_tool::damage`).
    transform_area: Option<crate::transform_tool::OpenArea>,
    /// A large layer's Free Transform box being prepared in the background.
    transform_preparing: Option<crate::transform_tool::Preparing>,
    /// Move-tool ⇧/⌥ drag state (move_mods).
    pub(crate) move_mods: move_mods::MoveDrag,
    /// Cached document for a modal text Style Options color preview.
    pub(crate) text_style_preview: Option<type_panels_ui::color_picker::Preview>,
    /// The run of arrow-key nudges that share one History state (nudge_bundle).
    pub(crate) nudge_bundle: nudge_bundle::NudgeBundle,
    /// Live Layer Style dialog preview: (key over revision + style fields, preview or validation error).
    pub(crate) style_preview: Option<(u64, Result<std::sync::Arc<Document>, String>)>,
    pub(crate) solid_fill_preview: Option<solid_fill_ui::Preview>,
    /// Shape fill Color Picker preview (shape_fill_ui).
    pub(crate) shape_fill_preview: Option<shape_fill_ui::Preview>,
    /// Liquify dialog, Puppet Warp and Perspective Warp sessions (distort_ui).
    pub(crate) distort: distort_ui::Distort,
    /// Gradient tool live-mode drags and previews (gradient_ui).
    pub(crate) gradient: gradient_ui::LiveGradient,
    /// The Camera Raw session that is open: the open-time dialog or the Filter (`camera_raw_ui`).
    pub(crate) raw: Option<crate::dialogs::camera_raw_ui::RawSession>,
    /// The reply of the Camera Raw finish or cancel the result pump ran last, for the menu request that confirmed it:
    /// the request clears it when it starts and takes it when it ends (`camera_raw_ui::menu`).
    pub(crate) raw_reply: Option<Result<serde_json::Value, String>>,
    /// Raws just opened interactively, waiting in the open-time Camera Raw dialog (shown on the
    /// next frame, which has the egui context); several share it in a filmstrip.
    pub(crate) raw_shelf: Vec<camera_raw_ui::HeldRaw>,
    /// The open-time re-develops (or their Camera Raw steps) running in the background, one per
    /// raw being opened.
    pub(crate) raw_redevelop: Vec<camera_raw_ui::Redevelop>,
    /// Filter › Adaptive Wide Angle dialog (wide_angle_ui).
    pub(crate) wide_angle: Option<wide_angle_ui::WideAngleApp>,
    /// What each open Color Picker's live target was when it opened, to put back on Cancel
    /// (`color_picker_ui::live_preview`).
    pub(crate) color_picker_live: Vec<color_picker_ui::live_preview::Live>,
    /// A command clicked while Free Transform commits in the background, run when it lands.
    pub(crate) after_transform_commit: Option<(String, serde_json::Value)>,
    /// Last mirrored or imported OS image; this can predate the current internal copy.
    os_clip_sig: Option<u64>,
    /// Last observed SVG bytes, to distinguish an unchanged OS source from a newer internal copy.
    os_clip_svg: Option<std::sync::Arc<Vec<u8>>>,
    /// The session copy, if any, need not track the OS clipboard. An older OS signature is kept
    /// to recognize unchanged system contents when exporting a newer copy failed or was disabled.
    clip_unmirrored: bool,
    /// The session clipboard currently holds an image imported from the OS clipboard: it has no
    /// original position, so Paste centres it (see `menus` "edit.paste").
    pub(crate) clip_external: bool,
    /// The OS clipboard was already read for the paste in flight (Edit › Paste positions it first),
    /// so `run` doesn't read it a second time.
    pub(crate) clip_read_for_paste: bool,
    pub(crate) smart_svg: smart_svg_ui::State,
    /// Pointer position over the canvas (document px), for the Info panel and status bar.
    pub(crate) hover_doc: Option<[f64; 2]>,
    /// Layer hovered in the Layers panel, outlined on the canvas.
    pub(crate) hover_layer: Option<photocraft_doc::LayerId>,
    /// The commands run from any source (`menus::invoke`, the commands keys and surfaces fire, the control channel). Every
    /// bar's and panel's version holds it (`panels::region_versions`): a command may change anything a slot shows.
    pub(crate) commands_run: u64,
    /// A layer whose row the Layers panel opens for renaming in its next pass (Layer › Rename
    /// Layer, the row menu's Rename Layer…).
    pub(crate) layers_rename: Option<photocraft_doc::LayerId>,
    /// The Smart Filter row selected on its own, per document ([`crate::smart_ui::selected`]).
    pub(crate) smart_filter_picks: crate::smart_ui::FilterPicks,
    /// Select › Find Layers: the Layers panel's Name filter takes the focus in its next pass.
    pub(crate) find_layers_focus: bool,
    /// Layer hovered on the canvas, highlighted in the Layers panel.
    pub(crate) canvas_hover_layer: Option<photocraft_doc::LayerId>,
    pub(crate) clone_preview: Option<crate::canvas::ClonePreviewCache>,
    /// Info panel sample cache: ((x, y, revision, Sample Size), composite RGBA).
    info_sample: Option<(InfoSampleKey, [f32; 4])>,
    info_cmyk_sample: Option<(InfoSampleKey, Option<[f32; 4]>)>,
    /// Move tool marquee selecting layers (`move_box`).
    pub(crate) layer_box: Option<move_box::LayerBox>,
    /// Pen paths finished, which Undo back to their finish gives back to the Pen
    /// (`vector_ui::pen_resume_on_undo`).
    pub(crate) finished_pens: Vec<vector_ui::FinishedPen>,
    /// Type tool layout cache: ((doc, revision, layer), layout).
    pub(crate) type_layout: Option<((u64, u64, u64), std::sync::Arc<photocraft_text::TextLayout>)>,
    /// The path a Type tool click on a path would place its text on, shown before the click
    /// (`type_path_placement`).
    pub(crate) type_path_preview: Option<type_path_placement::Preview>,
    pub(crate) type_transform_preview: Option<type_transform::Preview>,
    pub(crate) type_path_damage: Option<type_path_placement::Damage>,
    /// A paragraph-box handle drag rendered in the background (`type_box_preview`).
    pub(crate) type_box_preview: Option<type_box_preview::TypeBoxPreview>,
    /// A font hovered in the Type tool's font picker, shown live (`type_font_preview`).
    pub(crate) type_font_preview: Option<type_font_preview::TypeFontPreview>,
    /// The colour under a text or gradient stop Color Picker, shown live before OK (`color_picker_preview`).
    pub(crate) color_picker_preview: Option<color_picker_preview::Preview>,
    /// The brush tip last drawn for the cursor, with the key it was built for (`brush_cursor`).
    pub(crate) brush_tip: Option<(brush_cursor::TipKey, craft::ui::canvas_controls::brush::BrushTip)>,
    /// Snap targets of the active document, derived data keyed on (document, revision,
    /// `SnapOptions`, excluded layers); `snap_ui::snap` reads and refills it through `&self`.
    pub(crate) snap_cache: std::sync::Mutex<Option<snap_ui::SnapCache>>,
    /// What the chrome derives from each open document, kept per document revision (`app::doc_queries`).
    pub(crate) doc_queries: app::doc_queries::DocQueries,
    /// The saved workspaces and the one shown (`workspace_ui`).
    pub workspaces: craft::ui::contract::Tracked<workspace_ui::Workspaces>,
    /// Autosave timing and its retry state (`prefs_ui`).
    pub(crate) autosave: craft::platform::autosave::Autosave,
    /// GPU canvas renderer, when running on the wgpu backend (see [`Self::set_wgpu`]). Dropped
    /// for the rest of the session when the device is lost (see `gpu_status`).
    gpu: Option<gpu_canvas::GpuCanvas>,
    /// The device the window itself draws with, and its health flag: the GPU canvas's, or one
    /// watched for the CPU canvas (see `gpu_status::check_window`).
    window_device: Option<(eframe::wgpu::Device, photocraft_gpu::DeviceHealth)>,
    /// That device was lost: frames draw nothing and the app closes once its work is saved (#3438).
    window_lost: bool,
    /// When (in `egui::InputState::time`) to try saving unsaved work for recovery again after a
    /// failed attempt; the app closes only once it succeeded.
    window_recovery_retry: Option<f64>,
    /// The render state the GPU canvas was built on, kept after it is dropped so Retry GPU can
    /// build a new one on the same device (see `gpu_status::choose_recovery`, #6577).
    gpu_render_state: Option<eframe::egui_wgpu::RenderState>,
    /// Why the GPU canvas was dropped (see `gpu_status::fall_back`).
    gpu_fault: Option<photocraft_gpu::Fault>,
    /// Out-of-memory errors this session after which the GPU canvas was rebuilt by itself (#6564).
    gpu_oom_retries: u32,
    /// Run once the first frames have rendered (see [`Self::on_started`]).
    started: Option<gpu_status::StartedHook>,
    /// Frame and canvas-upload timings (exposed via `ui.inspect`).
    pub perf: gpu_canvas::Perf,
    /// Per-frame stroke timings for latency reports (`PHOTOCRAFT_STROKE_TRACE`, see `stroke_trace`).
    pub stroke_trace: stroke_trace::StrokeTrace,
    /// Why each document's view is drawn on the CPU, and the history of changes (`render_path`).
    pub render_path: render_path::RenderPath,
    /// The last slow frames and their stages (`perf.slowFrames`, see `frame_stages`).
    pub slow_frames: frame_stages::SlowFrames,
    /// Preferences, autosave and snapping runtime state (see `prefs_ui`, `snap_ui`).
    pub(crate) prefs_rt: prefs_ui::Runtime,
    pub(crate) updates_rt: updates_ui::Runtime,
    /// Close, Revert or Exit parked behind the unsaved-changes prompt (see `discard_ui`).
    pub(crate) discard: Option<discard_ui::Prompt>,
    /// A save parked behind a format's options prompt (TIFF, Targa, JPEG, DDS), run when the
    /// prompt closes (`format_prompts`).
    pub(crate) parked_save: Option<format_prompts::ParkedSave>,
    /// Where each document was last saved or exported to through a dialog (#1826): its next Save
    /// As or export dialog starts there rather than beside the document. Session-only.
    pub(crate) save_dirs: HashMap<photocraft_doc::DocId, std::path::PathBuf>,
    /// Each document's own file as it was when the document was opened from it or saved to it
    /// (see [`FileStamp`]).
    pub(crate) disk_stamps: HashMap<photocraft_doc::DocId, (String, FileStamp)>,
    /// Files read but not opened yet: the first waits on its profile prompt, the rest behind it
    /// (see `open_prompt`).
    pub(crate) held_opens: std::collections::VecDeque<open_prompt::Held>,
    /// PSD files whose memory preflight requires user approval before pixel decoding (see `open_prompt`).
    pub(crate) held_memory_prompts: std::collections::VecDeque<open_prompt::MemoryPrompt>,
    pub(crate) prompt_order: std::collections::VecDeque<open_prompt::PromptKind>,
    pub(crate) held_record_again: Option<actions::record_again::Held>,
    pub(crate) held_insert_menu: Option<actions::menu_item::Held>,
    /// Set once the user has agreed to quit, so the resulting close request goes through.
    pub(crate) allow_close: bool,
    /// The craft framework's host and the framework dialogs open in it (`app/framework.rs`).
    pub(crate) framework: app::framework::Framework,
    /// The file dialog in progress (see `file_dialog`).
    pub(crate) file_dialog: Option<file_dialog::Pending>,
    /// What the last TIFF Options answer chose; it outlives a prompt.
    pub(crate) tiff_remembered: tiff_options_ui::Remembered,
    /// Pen pressure/tilt from the platform (see `stylus`).
    pub stylus: stylus::Stylus,
    /// Run long commands and file opens as background jobs with progress and Cancel (#210; see
    /// `jobs_ui`). The desktop app turns it on; off (the default), everything runs inline as
    /// before, which tests and scripts rely on.
    pub background_jobs: bool,
    /// Shortest time between frames (ms), or `None` for no cap. A present mode that doesn't wait
    /// for the display (Low-Latency Drawing) would otherwise redraw as fast as it can while
    /// anything keeps requesting repaints (a drag hovering, autoscroll, Liquify held still).
    /// After #4472 (astigmata).
    pub min_frame_ms: Option<f64>,
    last_frame_start_ms: f64,
    /// Background job bookkeeping: opening tabs, control replies waiting on a job.
    pub jobs: jobs_ui::JobsUi,
    #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
    live_tokens: theme::live::LiveTokens,
}

/// The active document injected input was queued for.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InputTarget {
    /// The active document's generation: it changes on every switch of the active document, a
    /// switch away and back included, and not on an edit, so an edit does not withdraw the input.
    document: Option<u64>,
}

#[cfg(test)]
mod artboard_add_mode_tests;
#[cfg(test)]
mod artboards_to_files_background_tests;
#[cfg(test)]
mod background_eraser_tolerance_field_tests;
#[cfg(test)]
mod brush_picker_escape_tests;
#[cfg(test)]
mod burn_bar_angle_pressure_tests;
#[cfg(test)]
mod cam_no_pixels_alert_tests;
#[cfg(test)]
mod dodge_bar_angle_pressure_tests;
#[cfg(test)]
mod eyedropper_context_menu_tests;
#[cfg(test)]
mod menu_automate_groups_tests;
#[cfg(test)]
mod patch_rasterize_prompt_tests;
#[cfg(test)]
mod patch_shift_constrain_tests;
#[cfg(test)]
mod paths_scroll_tests;
#[cfg(test)]
mod pencil_pressure_toggles_tests;
#[cfg(test)]
mod smudge_alt_finger_tests;

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod stroke_frames_tests;

#[cfg(test)]
mod screenshot_tests;

/// Helpers shared by the tests of more than one module.
#[cfg(test)]
pub(crate) mod testing {
    use std::sync::Arc;
    use std::time::Duration;

    use craft::settings::testing::MemoryFiles;
    use craft::settings::{Location, OpenOptions, SettingsStore};

    use crate::Services;

    /// The folder of the scratch settings store; its file lives in `files`.
    const DIR: &str = "/memory/photocraft";

    /// Services whose preferences store reads and writes `files`, so a second app over the same files reads what the
    /// first one stored. A zero debounce makes the first frame after a change write it.
    pub(crate) fn services_over(files: &Arc<MemoryFiles>) -> Services {
        let options = OpenOptions::new(Location::Dir(DIR.into())).debounce(Duration::ZERO).files(files.clone());
        Services { settings: Some(SettingsStore::<photocraft_engine::prefs::SavedPrefs>::open_with(options)), ..Default::default() }
    }

    /// Makes the input a test hands `ctx` carry its modifiers as the window's does. egui reads the held modifiers only
    /// from `Event::ModifiersChanged`; the window's integration announces each change before the press, key or wheel
    /// that carries it, and stamps every event with the held set. A kittest event carries its modifiers on the event
    /// alone, so this announces them where they differ from the held ones. It runs after the runtime's input hook and
    /// sees the input egui will read, the runtime's injected events included.
    pub(crate) fn announce_modifiers(ctx: &egui::Context) {
        ctx.add_plugin(AnnouncedModifiers::default());
    }

    /// The modifiers each viewport's input has held so far.
    #[derive(Default)]
    struct AnnouncedModifiers(std::collections::HashMap<egui::ViewportId, egui::Modifiers>);

    impl egui::Plugin for AnnouncedModifiers {
        fn debug_name(&self) -> &'static str {
            "photocraft.testing.announced_modifiers"
        }

        fn input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
            let held = self.0.entry(input.viewport_id).or_default();
            let mut events = Vec::with_capacity(input.events.len());
            for event in std::mem::take(&mut input.events) {
                match &event {
                    egui::Event::ModifiersChanged(now) => *held = *now,
                    egui::Event::WindowFocused(false) => *held = egui::Modifiers::NONE,
                    egui::Event::PointerButton { modifiers, .. } | egui::Event::Key { modifiers, .. } | egui::Event::MouseWheel { modifiers, .. }
                        if *modifiers != *held =>
                    {
                        *held = *modifiers;
                        events.push(egui::Event::ModifiersChanged(*modifiers));
                    }
                    _ => {}
                }
                events.push(event);
            }
            input.events = events;
        }
    }

    /// One pass of `ui` on `ctx` with `input`, its output taken as the window's renderer takes it: no renderer applies
    /// the texture changes in a test, so they are consumed here and the output drops clean.
    pub(crate) fn run_ui(ctx: &egui::Context, input: egui::RawInput, ui: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
        let mut output = ctx.run_ui(input, ui);
        output.textures_delta.clear();
        output
    }
}

#[cfg(test)]
mod navigator_gpu_tests;

#[cfg(test)]
mod color_swatch_tests;
#[cfg(test)]
mod dialog_title_ellipsis_tests;

#[cfg(test)]
mod profile_open_tests;

#[cfg(test)]
mod input_tests;

#[cfg(test)]
mod home_chrome_tests;

#[cfg(test)]
mod pen_variants_tests;

#[cfg(test)]
mod color_replacement_digit_keys_tests;
#[cfg(test)]
mod color_replacement_tests;
#[cfg(test)]
mod home_recent_gate_tests;
#[cfg(test)]
mod layers_panel_nodes;
#[cfg(test)]
mod live_retouch_tests;
#[cfg(test)]
mod lock_layers_shortcut_tests;
#[cfg(test)]
mod pencil_tests;
#[cfg(test)]
mod smart_objects_menu_tests;
#[cfg(test)]
mod tool_shift_cycle_order_tests;
#[cfg(test)]
mod tool_shortcut_tests;
#[cfg(test)]
mod toolbar_custom_group_layout_tests;
#[cfg(test)]
mod toolbar_group_order_tests;

#[cfg(test)]
mod pixel_aspect_tests;

#[cfg(test)]
mod transform_controls_pointer_tests;
#[cfg(test)]
mod transform_fx_tests;

#[cfg(test)]
mod transform_commit_click_tests;
#[cfg(test)]
mod transform_drag_start_tests;

#[cfg(test)]
mod artboard_from_group_label_tests;
#[cfg(test)]
mod layer_props_ui_tests;
#[cfg(test)]
mod notice_text_tests;
#[cfg(test)]
mod transform_rotate90_bounds_tests;
#[cfg(test)]
mod transform_switch_tests;
#[cfg(test)]
mod transform_task_bar_tests;
#[cfg(test)]
mod transform_undo_tests;

#[cfg(test)]
mod transform_type_tests;

#[cfg(test)]
mod star_tool_tests;

#[cfg(test)]
mod new_doc_bitmap_mode_tests;

#[cfg(test)]
mod new_doc_recent_items_tests;

#[cfg(test)]
mod transform_several_tests;

#[cfg(test)]
mod transform_selection_rules_tests;

#[cfg(test)]
mod psd_open_tests;
#[cfg(test)]
mod save_identity_tests;

#[cfg(test)]
mod move_shift_click_tests;

#[cfg(test)]
mod move_target_default_tests;

#[cfg(test)]
mod wand_press_inside_selection_tests;

#[cfg(test)]
mod move_history_names_tests;
#[cfg(test)]
mod move_shift_diagonal_tests;

#[cfg(test)]
mod layers_footer_tests;
#[cfg(test)]
mod layers_rows_tests;
#[cfg(test)]
mod layers_thumbnail_tests;

#[cfg(test)]
mod color_panel_tests;
#[cfg(test)]
mod color_tab_theme_tests;

#[cfg(test)]
mod new_layer_button_tests;

#[cfg(test)]
mod smart_filter_rows_tests;

#[cfg(test)]
mod blur_pins_tests;

#[cfg(test)]
mod filter_preview_locator_tests;

#[cfg(test)]
mod move_outline_tests;

#[cfg(test)]
mod move_bar_distribute_gear_tests;

#[cfg(test)]
mod brush_picker_memo_trials;
#[cfg(test)]
mod brush_settle_trials;
#[cfg(test)]
mod chrome_settle_trials;
#[cfg(test)]
mod command_state_trials;
#[cfg(test)]
mod doc_revision_trials;
#[cfg(test)]
mod doc_tracked_trials;
#[cfg(test)]
mod drag_settle_trials;
#[cfg(test)]
mod layer_thumb_bounds_tests;
#[cfg(test)]
mod mask_thumb_refresh_tests;
#[cfg(test)]
mod menu_floor_trials;
#[cfg(test)]
mod scratch_read_keys_tests;

#[cfg(test)]
mod new_doc_remember_tests;

#[cfg(test)]
mod hidden_layer_tests;

#[cfg(test)]
mod warp_text_dialog_tests;

#[cfg(test)]
mod options_bar_mode_wheel_tests;

#[cfg(test)]
mod field_wheel_app_tests;

#[cfg(test)]
mod brush_shift_drag_axis_tests;

#[cfg(test)]
mod marquee_tests;

#[cfg(test)]
mod single_line_marquee_move_tests;

#[cfg(test)]
mod guide_cursor_tests;

#[cfg(test)]
mod canvas_menu_tests;

#[cfg(test)]
mod nudge_bundle_tests;

#[cfg(test)]
mod caps_lock_tests;

#[cfg(test)]
mod view_sync_tests;

#[cfg(test)]
mod stamp_tests;

#[cfg(test)]
mod alt_click_tests;
#[cfg(test)]
mod artboard_first_board_tests;
#[cfg(test)]
mod brush_resize_overlay_tests;
#[cfg(test)]
mod stroke_timing_tests;

#[cfg(test)]
mod patch_reset_tool_tests;
#[cfg(test)]
mod patch_tool_history_names_tests;
#[cfg(test)]
mod polygon_lasso_tests;

#[cfg(test)]
mod window_frame_tests;

#[cfg(test)]
mod artboards_export_options_dialog_tests;
#[cfg(test)]
mod artboards_overlap_dialog_tests;
#[cfg(test)]
mod artboards_to_files_types_tests;
#[cfg(test)]
mod artboards_to_pdf_reverse_tests;
#[cfg(test)]
mod browse_field_tests;
#[cfg(test)]
mod export_quality_tests;
#[cfg(test)]
mod select_and_mask_button_tests;
#[cfg(test)]
mod wand_tolerance_field_tests;

#[cfg(test)]
mod saved_path_edit_tests;

#[cfg(test)]
mod pen_path_ownership_tests;

#[cfg(test)]
mod artboard_options_bar_tests;
#[cfg(test)]
mod artboards_include_name_dialog_tests;

#[cfg(test)]
mod doc_tab_menu_tests;
#[cfg(test)]
mod preferences_submenu_tests;
#[cfg(test)]
mod selection_move_step_tests;
#[cfg(test)]
mod view_guides_submenu_tests;

#[cfg(test)]
mod lasso_click_tests;
#[cfg(test)]
mod reset_tip_tests;
#[cfg(test)]
mod rotate_steps_tests;
#[cfg(test)]
mod zoom_mode_tests;

#[cfg(test)]
mod red_eye_context_menu_tests;

#[cfg(test)]
mod clone_stamp_indexed_bar_tests;

#[cfg(test)]
mod place_visible_centre_tests;
