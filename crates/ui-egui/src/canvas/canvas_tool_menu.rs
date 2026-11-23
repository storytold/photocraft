//! Canvas context menus for the selection tools, the Pen, the Line tool in Shape mode, the Zoom and
//! Hand tools, text being edited with a Type tool, committed type under a Type tool, the pasteboard
//! (outside the image), a guide, a pending crop, the layer list and the painting tools' mode list.
//! The menu is state in `UiState::canvas_tool_menu`; [`run`] shows it on the framework's menu layer
//! every pass.

use craft::behavior::IconId;
use craft::draw::ui::crop::{CROP_RATIOS, CROP_SIZES, CropBarWords, CropPresetChoice, CropRatio, CropSettings, CropSizeFields};
use craft::keymap::Mods;
use craft::ui::canvas::{CanvasOutput, MomentaryTool, ToolHostResponse};
use craft::ui::canvas_controls::GuidesOutput;
use craft::ui::contract::overlay::{Anchor, Trigger};
use craft::ui::contract::{Bind, CommandSet, Entry};
use craft::ui::menu::Menu;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, UnitKind};
use egui::{Context, Pos2};
use photocraft_doc::{ColorMode, LayerContent};
use photocraft_engine::prefs::{self, CanvasColor};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::direct_select::PathRef;
use crate::{PhotocraftApp, state::Tool};
use craft::draw::ui::BezierHit;

pub(crate) mod pen_dialogs;

/// Which menu is open. The kind decides the rows, the checks and what a pick runs; the pen, sampler,
/// eyedropper, count and Content-Aware Move menus are [`MenuKind::Tool`] and are told apart by the
/// menu's `tool`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum MenuKind {
    /// The tool's own actions.
    #[default]
    Tool,
    /// Opened outside the image: the pasteboard colour.
    Pasteboard,
    /// The Zoom and Hand tools' view menu.
    View,
    /// Opened on a guide: the guide menu (`guide_select`).
    Guide,
    /// Opened on a pending crop, listing the saved crop presets by the names they had when it opened.
    Crop {
        /// The saved crop presets' names.
        presets: Vec<String>,
    },
    /// Opened while a Type tool edits text: the text and type rows.
    TypeEdit {
        /// The misspelled word under the pointer when the menu opened: its suggestions head the menu.
        spelling: Option<Spelling>,
    },
    /// Opened with a Type tool over committed type (no edit running): the type layer rows.
    TypeLayer {
        /// The shape of the committed text, which picks the Convert row (#12043).
        shape: TypeShape,
    },
    /// The layers with visible pixels under the pointer, topmost first (id, name).
    Layers {
        /// The layers listed.
        layers: Vec<(u64, String)>,
    },
    /// A painting tool's Shift+right-click mode list (`brush_mode_menu`).
    BrushMode { plain: bool },
}

fn no_canvas() -> egui::Id {
    egui::Id::NULL
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CanvasToolMenu {
    pub pos: [f32; 2],
    pub tool: Tool,
    /// Which menu this is.
    #[serde(default)]
    pub kind: MenuKind,
    /// The canvas that shows the menu: the one that opened it. `Id::NULL` until a canvas's [`run`]
    /// adopts a menu opened outside it.
    #[serde(skip, default = "no_canvas")]
    pub canvas: egui::Id,
    pub has_selection: bool,
    #[serde(default)]
    pub has_path: bool,
    #[serde(default)]
    pub path_name: Option<String>,
    /// The selected tool when Space holds the Hand over it (`tool` is then the Hand): the menu
    /// closes when that tool changes (#11271).
    #[serde(default)]
    pub under: Option<Tool>,
    /// The Color Sampler menu's sampler: the one under the pointer when it opened (its index),
    /// `None` off a sampler, where every row is greyed.
    #[serde(default)]
    pub sampler: Option<usize>,
    /// Where that sampler was when the menu opened: the active document's index and the sampler's
    /// position. Once a control command deletes, moves or renumbers samplers, or another document
    /// becomes active, the index may name a different sampler, so the menu closes and Delete greys.
    #[serde(default)]
    pub sampler_at: Option<(usize, [f64; 2])>,
    /// The Eyedropper's menu (#11747): the composite colour under the right-click, averaged with
    /// the Sample Size, which its Copy Color rows put on the clipboard. `None` over transparency.
    #[serde(default)]
    pub color: Option<[f32; 3]>,
    /// Opened with the Content-Aware Move tool inside the active selection (#12194).
    #[serde(default)]
    pub inside_selection: bool,
    /// Opened with the Delete Anchor Point or Convert Point tool over an anchor or a segment of the
    /// targeted path (#12771, #12744): the engine command that entry runs and its params. It heads
    /// the path menu, above a separator. The third part is the document id and revision it was read
    /// from: the knot index means something only there, so the entry expires once either moves on.
    #[serde(default)]
    pub point: Option<(String, Value, (u64, u64))>,
}

/// A misspelled word in the type layer being edited (`edit.checkSpelling`'s `list` entry).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spelling {
    pub layer: u64,
    /// The word's characters `start..end` in the layer's text.
    pub start: usize,
    pub end: usize,
    pub word: String,
    pub suggestions: Vec<String>,
}

/// Most suggestions the type edit menu lists.
const MAX_SUGGESTIONS: usize = 3;
/// The spelling rows' ids: `spelling.replace.<n>` picks the n-th suggestion.
const SPELLING_REPLACE: &str = "spelling.replace.";
const SPELLING_NONE: &str = "spelling.none";
const SPELLING_ADD: &str = "spelling.add";

/// One menu row: label and command id. `None` is a separator.
pub type Row<S = &'static str> = Option<(S, S)>;

/// The two rows of the transform menu that belong to the app: Content-Aware Scale and Puppet Warp.
/// They show only while a box is open, where they are greyed. The transform family passes them to
/// `TransformOverlay::menu_rows`.
pub fn transform_app_rows() -> [Entry<String>; 2] {
    [
        Entry::new("edit.contentAwareScale".to_string(), tl!("Content-Aware Scale").to_string()).enabled(false),
        Entry::new("edit.puppetWarp".to_string(), tl!("Puppet Warp").to_string()).enabled(false),
    ]
}

/// Photoshop's pasteboard menu (right-click outside the image): the pasteboard colour, the choice
/// Preferences › Interface also offers. The ids are this menu's own: [`choose`] turns them into
/// `prefs.set` (`pasteboard.<CanvasColor name>`) or opens the Color Picker.
pub const PASTEBOARD_MENU: &[Row] = &[
    Some(("Default", "pasteboard.default")),
    Some(("Black", "pasteboard.black")),
    Some(("Dark Gray", "pasteboard.darkGray")),
    Some(("Medium Gray", "pasteboard.mediumGray")),
    Some(("Light Gray", "pasteboard.lightGray")),
    Some(("Custom", "pasteboard.custom")),
    None,
    Some(("Select Custom Color…", PASTEBOARD_PICK)),
];

/// Photoshop's right-click menu inside text being edited: the edit rows act on the text (its
/// selection, the OS clipboard), the type rows on the layer, as the Type menu's do. Faux Bold and
/// Faux Italic are the Character panel's toggles (this menu's own ids, run as `type.setStyle`).
/// Convert to Paragraph Text and Convert to Point Text are both listed; the one that doesn't apply
/// greys out. The exact Photoshop placement is inferred (docs/context-menu-parity.md).
pub const TYPE_EDIT_MENU: &[Row] = &[
    Some(("Undo", "edit.undo")),
    None,
    Some(("Cut", "edit.cut")),
    Some(("Copy", "edit.copy")),
    Some(("Paste", "edit.paste")),
    Some(("Select All", "select.all")),
    None,
    Some(("Check Spelling…", "edit.checkSpelling")),
    Some(("Find and Replace Text…", "edit.findAndReplaceText")),
    None,
    Some(("Faux Bold", FAUX_BOLD)),
    Some(("Faux Italic", FAUX_ITALIC)),
    None,
    Some(("Rasterize Type", "type.rasterizeTypeLayer")),
    Some(("Create Work Path", "type.createWorkPath")),
    Some(("Convert to Shape", "type.convertToShape")),
    None,
    Some(("Horizontal", "type.orientation.horizontal")),
    Some(("Vertical", "type.orientation.vertical")),
    None,
    Some(("None", "type.antiAlias.none")),
    Some(("Sharp", "type.antiAlias.sharp")),
    Some(("Crisp", "type.antiAlias.crisp")),
    Some(("Strong", "type.antiAlias.strong")),
    Some(("Smooth", "type.antiAlias.smooth")),
    Some(("Windows LCD", "type.antiAlias.windowsLcd")),
    Some(("Windows", "type.antiAlias.windows")),
    None,
    Some(("Convert to Paragraph Text", "type.convertToParagraphText")),
    Some(("Convert to Point Text", "type.convertToPointText")),
    Some(("Warp Text…", "type.warpText")),
];

/// Right-click with a Type tool over type that isn't being edited (#7324): the type layer rows,
/// in the reference menu's order (#12043) — Edit Type… on top, Faux Bold and Faux Italic for the
/// whole layer, Layer Style… last, and only the Convert row the layer's text shape allows. Its
/// text-edit rows (Undo, Cut, Copy, Paste, Select All) stay in [`TYPE_EDIT_MENU`]: away from an
/// edit they would act on the document instead of the text.
macro_rules! type_layer_menu {
    ($($convert:expr),*) => {
        &[
            Some(("Edit Type…", "type.editText")),
            None,
            Some(("Check Spelling…", "edit.checkSpelling")),
            Some(("Find and Replace Text…", "edit.findAndReplaceText")),
            None,
            Some(("Rasterize Type", "type.rasterizeTypeLayer")),
            Some(("Create Work Path", "type.createWorkPath")),
            Some(("Convert to Shape", "type.convertToShape")),
            None,
            Some(("Horizontal", "type.orientation.horizontal")),
            Some(("Vertical", "type.orientation.vertical")),
            None,
            Some(("None", "type.antiAlias.none")),
            Some(("Sharp", "type.antiAlias.sharp")),
            Some(("Crisp", "type.antiAlias.crisp")),
            Some(("Strong", "type.antiAlias.strong")),
            Some(("Smooth", "type.antiAlias.smooth")),
            Some(("Windows LCD", "type.antiAlias.windowsLcd")),
            Some(("Windows", "type.antiAlias.windows")),
            None,
            Some(("Faux Bold", FAUX_BOLD)),
            Some(("Faux Italic", FAUX_ITALIC)),
            None,
            $($convert,)*
            None,
            Some(("Warp Text…", "type.warpText")),
            None,
            Some(("Layer Style…", LAYER_STYLE)),
        ]
    };
}

/// Point text: the only Convert row that applies turns it into paragraph text.
pub const TYPE_LAYER_MENU: &[Row] = type_layer_menu!(Some(("Convert to Paragraph Text", "type.convertToParagraphText")));
/// Paragraph (box) text: the mirror case, per the measured reference (#12043).
pub const TYPE_LAYER_MENU_PARAGRAPH: &[Row] = type_layer_menu!(Some(("Convert to Point Text", "type.convertToPointText")));
/// Path and area text: how the layer could be converted was not measured, so both rows stay as
/// before, each greyed unless the engine takes the layer.
pub const TYPE_LAYER_MENU_OTHER: &[Row] =
    type_layer_menu!(Some(("Convert to Paragraph Text", "type.convertToParagraphText")), Some(("Convert to Point Text", "type.convertToPointText")));

/// The shape of the committed text a type layer menu was opened on, which decides the Convert
/// rows it lists (#12043). Captured when the menu opens and refreshed by [`close_stale`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TypeShape {
    /// Point text: the menu lists Convert to Paragraph Text ([`TYPE_LAYER_MENU`]).
    Point,
    /// Paragraph (box) text: the menu lists Convert to Point Text ([`TYPE_LAYER_MENU_PARAGRAPH`]).
    Paragraph,
    /// Path or area text: both rows, greyed unless applicable ([`TYPE_LAYER_MENU_OTHER`]).
    Other,
}

/// The type layer menu's rows for the shape the menu captured.
fn type_layer_rows(menu: &CanvasToolMenu) -> &'static [Row] {
    match &menu.kind {
        MenuKind::TypeLayer { shape: TypeShape::Point } => TYPE_LAYER_MENU,
        MenuKind::TypeLayer { shape: TypeShape::Paragraph } => TYPE_LAYER_MENU_PARAGRAPH,
        _ => TYPE_LAYER_MENU_OTHER,
    }
}

/// The type edit menu's Faux Bold and Faux Italic rows. In the type layer menu they toggle the
/// style of the whole layer instead of the characters being edited (#12043).
pub const FAUX_BOLD: &str = "type.fauxBold";
pub const FAUX_ITALIC: &str = "type.fauxItalic";
/// The type layer menu's Layer Style… row: the Layer Style dialog on the target layer, as the
/// Layer menu's Blending Options… (#12043).
pub const LAYER_STYLE: &str = "layer.layerStyle.blendingOptions";

/// Type edit rows that act on the whole layer (it may stop being type): the edit ends first.
const ENDS_TYPE_EDIT: [&str; 5] =
    ["type.rasterizeTypeLayer", "type.createWorkPath", "type.convertToShape", "type.convertToParagraphText", "type.convertToPointText"];

/// The pasteboard menu's Select Custom Color… row.
pub const PASTEBOARD_PICK: &str = "pasteboard.selectCustomColor";

/// The guide menu's Edit Selected Guides row: New Guide's dialog on the selected guide.
pub const GUIDE_EDIT: &str = "view.editSelectedGuides";
/// The guide menu's Clear Selected Guides row.
pub const GUIDE_CLEAR: &str = "view.clearSelectedGuides";

/// Photoshop's guide menu (right-click on a guide), in its order. Its two Selected Guides rows act
/// on the guide the right-click selected; the rest are the View menu's.
pub const GUIDE_MENU: &[Row] = &[
    Some(("Edit Selected Guides", GUIDE_EDIT)),
    None,
    Some(("Lock Guides", "view.lockGuides")),
    Some(("Clear Guides", "view.clearGuides")),
    Some(("Clear Canvas Guides", "view.clearCanvasGuides")),
    Some(("Clear Selected Guides", GUIDE_CLEAR)),
    Some(("Clear Selected Artboard Guides", "view.clearSelectedArtboardGuides")),
];

/// The crop menu's row ids. A ratio row is `crop.ratio.<key>`, the options bar's preset dropdown
/// key; a saved preset is `crop.preset.<n>`, the n-th name the menu opened with; an overlay row is
/// `crop.overlay.<id>` or `crop.overlayShow.<id>`.
const CROP_RATIO: &str = "crop.ratio.";
const CROP_PRESET: &str = "crop.preset.";
const CROP_OVERLAY: &str = "crop.overlay.";
const CROP_SHOW: &str = "crop.overlayShow.";
pub const CROP_CLEAR: &str = "crop.clear";
pub const CROP_NEW: &str = "crop.newPreset";
pub const CROP_DELETE: &str = "crop.deletePreset";
pub const CROP_CANCEL: &str = "crop.cancel";
pub const CROP_COMMIT: &str = "crop.commit";

/// The saved crop presets the open crop menu lists; `None` for any other menu.
fn crop_presets(menu: &CanvasToolMenu) -> Option<&[String]> {
    match &menu.kind {
        MenuKind::Crop { presets } => Some(presets),
        _ => None,
    }
}

/// The preset list's entries the menu offers, in the options bar's order: Ratio, W x H x
/// Resolution, the ratios, the sizes, the `users` saved presets, Front Image.
fn crop_choices(users: usize) -> impl Iterator<Item = CropPresetChoice> {
    [CropPresetChoice::Free, CropPresetChoice::Size]
        .into_iter()
        .chain((0..CROP_RATIOS.len()).map(CropPresetChoice::Ratio))
        .chain((0..CROP_SIZES.len()).map(CropPresetChoice::SizePreset))
        .chain((0..users).map(CropPresetChoice::User))
        .chain([CropPresetChoice::FrontImage])
}

/// The name of a built-in preset-list entry, as the options bar's list shows it: the ratio words, or a size preset's values
/// named as New Crop Preset… would name them.
fn choice_name(words: &CropBarWords, choice: CropPresetChoice, units: &craft::ui::runtime::env::Units) -> String {
    match choice {
        CropPresetChoice::Free => words.ratio.as_str().to_owned(),
        CropPresetChoice::Size => words.size_mode.as_str().to_owned(),
        CropPresetChoice::Ratio(i) => match i {
            0 => words.ratio_original.as_str(),
            1 => words.ratio_square.as_str(),
            2 => words.ratio_4_5.as_str(),
            3 => words.ratio_5_7.as_str(),
            4 => words.ratio_2_3.as_str(),
            _ => words.ratio_16_9.as_str(),
        }
        .to_owned(),
        CropPresetChoice::SizePreset(i) => {
            let Some(preset) = CROP_SIZES.get(i) else { return String::new() };
            let mut size = CropSizeFields::default();
            size.width = Some(preset.width);
            size.height = Some(preset.height);
            size.resolution = preset.resolution.to_owned();
            let mut settings = CropSettings::default();
            settings.ratio = CropRatio::Size;
            settings.size = size;
            settings.preset_name(words, units).unwrap_or_default()
        }
        CropPresetChoice::FrontImage => words.front_image.as_str().to_owned(),
        CropPresetChoice::Selection => words.selection.as_str().to_owned(),
        _ => String::new(),
    }
}

/// The overlay's name in the bar's words.
fn guide_name(words: &CropBarWords, guide: craft::behavior::geometry::CropGuide) -> &str {
    use craft::behavior::geometry::CropGuide;
    match guide {
        CropGuide::Grid => words.grid.as_str(),
        CropGuide::Diagonal => words.diagonal.as_str(),
        CropGuide::Triangle => words.triangle.as_str(),
        CropGuide::GoldenRatio => words.golden_ratio.as_str(),
        CropGuide::GoldenSpiral => words.golden_spiral.as_str(),
        _ => words.thirds.as_str(),
    }
}

/// The show rule's name in the bar's words.
fn show_name(words: &CropBarWords, show: craft::behavior::geometry::GuideShow) -> &str {
    use craft::behavior::geometry::GuideShow;
    match show {
        GuideShow::Always => words.show_always.as_str(),
        GuideShow::Never => words.show_never.as_str(),
        _ => words.show_auto.as_str(),
    }
}

/// The overlay's glyph, the one the options bar's overlay menu draws.
fn guide_icon(guide: craft::behavior::geometry::CropGuide) -> IconId {
    use craft::behavior::geometry::CropGuide;
    IconId::new(match guide {
        CropGuide::Grid => "crop_overlay_grid",
        CropGuide::Diagonal => "crop_overlay_diagonal",
        CropGuide::Triangle => "crop_overlay_triangle",
        CropGuide::GoldenRatio => "crop_overlay_golden_ratio",
        CropGuide::GoldenSpiral => "crop_overlay_golden_spiral",
        _ => "crop_overlay_thirds",
    })
}

/// The key of a built-in entry in its row id, the options bar's dropdown key since before the
/// bar was the framework's, so saved scripts keep working: `""` Ratio, `whr`, `original`, `4:5`,
/// `preset:4x5in`, `preset:1024x768px`, `front`, `selection`.
fn crop_key(choice: CropPresetChoice) -> String {
    match choice {
        CropPresetChoice::Size => "whr".into(),
        CropPresetChoice::Ratio(i) => match CROP_RATIOS.get(i) {
            Some(CropRatio::Sides { width, height, .. }) => format!("{width}:{height}"),
            _ => "original".into(),
        },
        CropPresetChoice::SizePreset(i) => CROP_SIZES.get(i).map_or_else(String::new, |p| {
            let unit = if p.width.unit == UnitKind::Inches { "in" } else { "px" };
            format!("preset:{}x{}{unit}", p.width.value, p.height.value)
        }),
        CropPresetChoice::FrontImage => "front".into(),
        CropPresetChoice::Selection => "selection".into(),
        _ => String::new(),
    }
}

/// The row id of a preset-list entry: `crop.preset.<n>` for a saved preset, else `crop.ratio.<key>`.
fn crop_id(choice: CropPresetChoice) -> String {
    match choice {
        CropPresetChoice::User(i) => format!("{CROP_PRESET}{i}"),
        choice => format!("{CROP_RATIO}{}", crop_key(choice)),
    }
}

/// The crop menu (right-click on a pending crop, #6561): the options bar's preset dropdown, in its
/// order, and its Clear; the overlay and when it shows; then Cancel and Crop, the bar's ⊘ and ✓.
/// Built from the bar's own entries and words, so the two cannot disagree. The labels are shown
/// as given. Empty for any other menu.
pub fn crop_rows(app: &PhotocraftApp, menu: &CanvasToolMenu) -> Vec<Row<String>> {
    use craft::behavior::geometry::{CropGuide, GuideShow};
    let Some(users) = crop_presets(menu) else { return Vec::new() };
    let env = Env::of_root(app.ctx());
    let words = env.words::<CropBarWords>();
    let units = env.units();
    let mut rows: Vec<Row<String>> = crop_choices(users.len())
        .map(|choice| {
            let label = match choice {
                CropPresetChoice::User(i) => users.get(i).cloned().unwrap_or_default(),
                choice => choice_name(&words, choice, &units),
            };
            Some((label, crop_id(choice)))
        })
        .collect();
    rows.push(Some((words.clear.as_str().to_owned(), CROP_CLEAR.into())));
    rows.push(None);
    rows.push(Some((words.new_preset.as_str().to_owned(), CROP_NEW.into())));
    rows.push(Some((words.delete_preset.as_str().to_owned(), CROP_DELETE.into())));
    rows.push(None);
    rows.extend(CropGuide::ALL.iter().map(|k| Some((guide_name(&words, *k).to_owned(), format!("{CROP_OVERLAY}{}", serde_id(k))))));
    rows.push(None);
    rows.extend(GuideShow::ALL.iter().map(|k| Some((show_name(&words, *k).to_owned(), format!("{CROP_SHOW}{}", serde_id(k))))));
    rows.push(None);
    rows.push(Some((tl!("Cancel").to_string(), CROP_CANCEL.into())));
    rows.push(Some((tl!("Crop").to_string(), CROP_COMMIT.into())));
    rows
}

/// Is `command` one of the crop menu's rows?
fn is_crop(app: &PhotocraftApp, menu: &CanvasToolMenu, command: &str) -> bool {
    crop_rows(app, menu).iter().flatten().any(|(_, id)| id == command)
}

/// The crop menu's checkmarks, as the options bar shows them: the dropdown's entry (a saved
/// preset the fields hold, else the mode or ratio), the overlay and when it shows.
fn crop_checked(app: &PhotocraftApp, command: &str) -> bool {
    let crop = &app.ui.tool_options.crop;
    if command.starts_with(CROP_RATIO) || command.starts_with(CROP_PRESET) {
        return command == crop_id(crop.choice(&crate::crop_presets::user_presets(app)));
    }
    if let Some(k) = command.strip_prefix(CROP_OVERLAY) {
        return k == serde_id(&crop.guide);
    }
    command.strip_prefix(CROP_SHOW).is_some_and(|k| k == serde_id(&crop.guide_show))
}

/// The id of a crop overlay or show rule: its serde name, the one the settings store.
fn serde_id<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(id)) => id,
        _ => String::new(),
    }
}

/// The value a [`serde_id`] names, if any.
fn from_serde_id<T: serde::de::DeserializeOwned>(id: &str) -> Option<T> {
    serde_json::from_value(Value::String(id.to_owned())).ok()
}

fn crop_enabled(app: &PhotocraftApp, menu: &CanvasToolMenu, command: &str) -> bool {
    match command {
        CROP_NEW => {
            let env = Env::of_root(app.ctx());
            app.ui.tool_options.crop.preset_name(&env.words::<CropBarWords>(), &env.units()).is_some()
        }
        CROP_DELETE => crop_presets(menu).is_some_and(|u| !u.is_empty()),
        c => is_crop(app, menu, c),
    }
}

/// A crop row: each runs what its options-bar twin runs.
fn choose_crop(app: &mut PhotocraftApp, menu: &CanvasToolMenu, command: &str) -> Result<(), String> {
    use craft::behavior::geometry::{CropGuide, GuideShow};
    let users = crop_presets(menu).map_or(0, <[String]>::len);
    match command {
        CROP_CANCEL => crate::crop_ui::discard(app),
        CROP_COMMIT => {
            crate::crop_ui::apply(app);
        }
        CROP_NEW => {
            crate::crop_presets::open_new(app);
        }
        CROP_DELETE => {
            crate::crop_presets::open_delete(app);
        }
        _ => {
            if let Some(choice) = crop_choices(users).find(|c| crop_id(*c) == command) {
                let saved = crate::crop_presets::user_presets(app);
                let Some(document) = crate::crop_ui::document(app) else { return Ok(()) }; // PLAN: assumed `crop_ui::document(app) -> Option<CropDocument>`, the document the bar and the tool run over (tools-w45)
                app.ui.tool_options.crop.choose(choice, &document, &saved);
            } else if command == CROP_CLEAR {
                app.ui.tool_options.crop.clear();
            } else if let Some(k) = command.strip_prefix(CROP_OVERLAY).and_then(from_serde_id::<CropGuide>) {
                app.ui.tool_options.crop.set_guide(k);
            } else if let Some(k) = command.strip_prefix(CROP_SHOW).and_then(from_serde_id::<GuideShow>) {
                app.ui.tool_options.crop.guide_show = k;
            }
            // As the options bar does: a ratio or W x H change refits the frame.
            crate::crop_ui::refit(app, false);
        }
    }
    Ok(())
}

/// The pasteboard colours in the menu's order, which Space+F steps through.
const CANVAS_COLORS: [CanvasColor; 6] =
    [CanvasColor::Default, CanvasColor::Black, CanvasColor::DarkGray, CanvasColor::MediumGray, CanvasColor::LightGray, CanvasColor::Custom];

/// Photoshop's Space+F and ⇧Space+F: F while the Hand key is held steps the pasteboard colour in place of Cycle Screen
/// Mode. The keys are the held Hand's scope (`canvasHand`, which `MomentaryCommands::Hand` holds), so they work only
/// while Space is down.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "canvasColor", scope = "canvasHand")]
pub(crate) enum CanvasColorCommands {
    #[command(label = "Next Pasteboard Color", default = "F")]
    Next,
    #[command(label = "Previous Pasteboard Color", default = "Shift+F")]
    Previous,
}

impl CanvasColorCommands {
    pub(crate) const ALL: [CanvasColorCommands; 2] = [CanvasColorCommands::Next, CanvasColorCommands::Previous];
}

/// Runs a fired [`CanvasColorCommands`]; whether `command` is one.
pub(crate) fn run_canvas_color(app: &mut PhotocraftApp, _ctx: &Context, command: &CommandRef) -> bool {
    let Some(step) = CanvasColorCommands::from_ref(command) else { return false };
    if let Err(e) = cycle_canvas_color(app, step == CanvasColorCommands::Previous) {
        app.ui.status = e;
    }
    true
}

/// The next pasteboard colour, round and round (`backward`: the previous). Saved like a choice in the pasteboard menu
/// (`interface.canvasColor`).
pub fn cycle_canvas_color(app: &mut PhotocraftApp, backward: bool) -> Result<(), String> {
    let n = CANVAS_COLORS.len();
    let at = CANVAS_COLORS.iter().position(|c| *c == app.session.prefs().interface.canvas_color).unwrap_or(0);
    let next = if backward { (at + n - 1) % n } else { (at + 1) % n };
    let name = CANVAS_COLORS.get(next).map_or("default", |c| c.name());
    app.run("prefs.set", json!({"values": {"interface.canvasColor": name}})).map(|_| ())
}

/// The Zoom tool's context menu: the fixed zooms of the View menu (plus Fill Screen), then the
/// zoom steps last, in Photoshop's order (#6005). Photoshop has no View menu command for Fill
/// Screen; its row asks the canvas for its own command.
pub const VIEW_MENU: &[Row] = &[
    Some(("Fit on Screen", "view.fitOnScreen")),
    Some(("Fill Screen", "view.fillScreen")),
    Some(("100%", "view.actualPixels")),
    Some(("200%", "view.twoHundredPercent")),
    Some(("Print Size", "view.printSize")),
    None,
    Some(("Zoom In", "view.zoomIn")),
    Some(("Zoom Out", "view.zoomOut")),
];

/// The Hand tool's context menu: only the fixed zooms, as measured in the reference application (#12022).
pub const HAND_MENU: &[Row] = &[
    Some(("Fit on Screen", "view.fitOnScreen")),
    Some(("100%", "view.actualPixels")),
    Some(("200%", "view.twoHundredPercent")),
    Some(("Print Size", "view.printSize")),
];

/// A selection menu: the rows every selection menu shares, with `$fills` (the rows between
/// Transform Selection and Last Filter) in its own order.
macro_rules! selection_menu {
    ($($fills:expr),* $(,)?) => {
        &[
            Some(("Deselect", "select.deselect")),
            Some(("Select Inverse", "select.inverse")),
            Some(("Feather…", "select.modify.feather")),
            Some(("Select and Mask…", "select.selectAndMask")),
            None,
            Some(("Save Selection…", "select.saveSelection")),
            Some(("Make Work Path…", "select.toWorkPath")),
            None,
            Some(("Layer Via Copy", "layer.new.layerViaCopy")),
            Some(("Layer Via Cut", "layer.new.layerViaCut")),
            Some(("New Layer…", "layer.new.layer")),
            None,
            Some(("Free Transform", "edit.freeTransform")),
            Some(("Transform Selection", "select.transformSelection")),
            $($fills,)*
            None,
            Some(("Last Filter", "filter.lastFilter")),
            None,
            Some(("Fade…", "edit.fade")),
        ]
    };
}

/// Photoshop's selection-tool context menu with an active selection, in its order. Its Generative
/// Fill row has no PhotoCraft command and is left out. Rows stay visible and grey out exactly like
/// their menu-bar twins.
pub const SELECTION_MENU: &[Row] = selection_menu![
    Some(("Distort", "edit.transform.distort")),
    Some(("Perspective", "edit.transform.perspective")),
    None,
    Some(("Delete and Fill Selection", "edit.deleteAndFillSelection")),
    Some(("Fill…", "edit.fill")),
    Some(("Stroke…", "edit.stroke")),
    Some(("Content-Aware Fill…", "edit.contentAwareFill")),
];

/// The selection tools' context menu with nothing selected: making a selection.
pub const NO_SELECTION_MENU: &[Row] = &[
    Some(("Select All", "select.all")),
    Some(("Reselect", "select.reselect")),
    Some(("Color Range…", "select.colorRange")),
    None,
    Some(("Load Selection…", "select.loadSelection")),
];

/// Photoshop's generic layer menu, in its order for a pixel layer without a selection: the Count
/// tool's canvas menu on a marker or off one (#11772), and the Red Eye and Object Selection
/// (#12866) tools' without a selection. Each row is enabled as the Layers panel menu enables it
/// for the active layer (`menus::is_enabled`) and runs like the menu item.
pub const COUNT_MENU: &[Row] = &[
    Some(("Blending Options…", "layer.layerStyle.blendingOptions")),
    None,
    Some(("Duplicate Layer…", "layer.duplicate")),
    Some(("Delete Layer", "layer.delete")),
    Some(("Group from Layers…", "layer.new.groupFromLayers")),
    None,
    Some(("Convert to Smart Object", "layer.smartObjects.convertToSmartObject")),
    Some(("Convert to Layers", "layer.smartObjects.convertToLayers")),
    None,
    Some(("Rasterize Layer", "layer.rasterize.layer")),
    Some(("Rasterize Layer Style", "layer.rasterize.layerStyle")),
    None,
    Some(("Free Transform", "edit.freeTransform")),
    None,
    Some(("Color Range…", "select.colorRange")),
    Some(("Load Selection…", "select.loadSelection")),
    Some(("Reselect", "select.reselect")),
];

/// Right-click with the Content-Aware Move tool inside an active selection (#12194).
pub const CAM_INSIDE_SELECTION_MENU: &[Row] = &[Some(("Color Range…", "select.colorRange")), Some(("Load Selection…", "select.loadSelection"))];

/// The selection menu as measured in the reference application with the Red Eye tool (#12577) and
/// the Object Selection tool (#12866): no Distort or Perspective, and Delete and Fill Selection
/// after the fills.
pub const MEASURED_SELECTION_MENU: &[Row] = selection_menu![
    None,
    Some(("Fill…", "edit.fill")),
    Some(("Stroke…", "edit.stroke")),
    Some(("Content-Aware Fill…", "edit.contentAwareFill")),
    Some(("Delete and Fill Selection", "edit.deleteAndFillSelection")),
];

/// The Eyedropper's copy rows (#11747).
pub const EYEDROPPER_COPY_HTML: &str = "canvasMenu.eyedropper.copyHtml";
pub const EYEDROPPER_COPY_HEX: &str = "canvasMenu.eyedropper.copyHex";

/// The Eyedropper's canvas menu, in the reference application's order (#11747): the Sample Sizes
/// (the current one checked), then the two Copy Color rows for the colour under the pointer.
pub const EYEDROPPER_MENU: &[Row] = &[
    Some(("Point Sample", "canvasMenu.eyedropper.size.1")),
    Some(("3 by 3 Average", "canvasMenu.eyedropper.size.3")),
    Some(("5 by 5 Average", "canvasMenu.eyedropper.size.5")),
    Some(("11 by 11 Average", "canvasMenu.eyedropper.size.11")),
    Some(("31 by 31 Average", "canvasMenu.eyedropper.size.31")),
    Some(("51 by 51 Average", "canvasMenu.eyedropper.size.51")),
    Some(("101 by 101 Average", "canvasMenu.eyedropper.size.101")),
    None,
    Some(("Copy Color as HTML", EYEDROPPER_COPY_HTML)),
    Some(("Copy Color's Hex Code", EYEDROPPER_COPY_HEX)),
];

/// The Sample Size an Eyedropper menu row sets, in pixels.
fn eyedropper_size(command: &str) -> Option<u32> {
    command.strip_prefix("canvasMenu.eyedropper.size.")?.parse().ok()
}

/// Is `menu` the Eyedropper's own menu (not the pasteboard, guide or crop menu it can also open)?
fn is_eyedropper(menu: &CanvasToolMenu) -> bool {
    menu.tool == Tool::Eyedropper && menu.kind == MenuKind::Tool
}

/// Photoshop's Pen context menu order. Rows whose operation is not applicable to the current
/// path or layer stay visible and disabled.
pub const PEN_MENU: &[Row] = &[
    Some(("Create Vector Mask", "layer.vectorMask.fromPath")),
    Some(("Delete Path", "path.delete")),
    None,
    Some(("Define Custom Shape…", "edit.defineCustomShape")),
    None,
    Some(("Make Selection…", "path.toSelection")),
    Some(("New Guides From Shape", "view.newGuidesFromShape")),
    Some(("Fill Path…", "path.fill")),
    Some(("Stroke Path…", "path.stroke")),
    None,
    Some(("Clipping Path…", "path.clippingPath.set")),
    None,
    Some(("Free Transform Path", "path.transform")),
    None,
    Some(("Unite Shapes", "layer.combineShapes.unite")),
    Some(("Subtract Front Shape", "layer.combineShapes.subtractFrontShape")),
    Some(("Unite Shapes at Overlap", "layer.combineShapes.intersectShapeAreas")),
    Some(("Subtract Shapes at Overlap", "layer.combineShapes.excludeOverlappingShapes")),
    None,
    Some(("Copy Fill", "path.style.copyFill")),
    Some(("Copy Complete Stroke", "path.style.copyStroke")),
    None,
    Some(("Paste Fill", "path.style.pasteFill")),
    Some(("Paste Complete Stroke", "path.style.pasteStroke")),
    None,
    Some(("Isolate Layers", "select.isolateLayers")),
    None,
    Some(("Make Symmetry Path", "paint.symmetryFromPath")),
    Some(("Disable Symmetry Path", "paint.symmetryDisable")),
];

/// Is a path with at least two points being drawn with the Pen?
pub(crate) fn pen_pending(app: &PhotocraftApp) -> bool {
    app.ui.pen.as_ref().is_some_and(|p| p.draft.knots.len() >= 2)
}

/// The shape tools' canvas menu in Shape mode (#13249), measured with the Line tool: the Pen's
/// path rows without Create Vector Mask, Delete Path, Clipping Path… or the Symmetry rows, plus
/// Blending Options… and the Rasterize rows. `$transform` is the transform row: Free Transform with
/// no path, which runs the menu bar's, Free Transform Path on a path or shape layer.
macro_rules! shape_menu {
    ($transform:expr) => {
        &[
            Some(("Define Custom Shape…", "edit.defineCustomShape")),
            None,
            Some(("Blending Options…", "layer.layerStyle.blendingOptions")),
            None,
            Some(("Make Selection…", "path.toSelection")),
            Some(("New Guides From Shape", "view.newGuidesFromShape")),
            Some(("Fill Path…", "path.fill")),
            Some(("Stroke Path…", "path.stroke")),
            None,
            Some(("Rasterize Layer", "layer.rasterize.layer")),
            Some(("Rasterize Layer Style", "layer.rasterize.layerStyle")),
            Some(("Rasterize Vector Mask", "layer.rasterize.vectorMask")),
            None,
            $transform,
            None,
            Some(("Unite Shapes", "layer.combineShapes.unite")),
            Some(("Subtract Front Shape", "layer.combineShapes.subtractFrontShape")),
            Some(("Unite Shapes at Overlap", "layer.combineShapes.intersectShapeAreas")),
            Some(("Subtract Shapes at Overlap", "layer.combineShapes.excludeOverlappingShapes")),
            None,
            Some(("Copy Fill", "path.style.copyFill")),
            Some(("Copy Complete Stroke", "path.style.copyStroke")),
            None,
            Some(("Paste Fill", "path.style.pasteFill")),
            Some(("Paste Complete Stroke", "path.style.pasteStroke")),
            None,
            Some(("Isolate Layers", "select.isolateLayers")),
        ]
    };
}

/// [`shape_menu!`] with a path or a shape layer: Free Transform Path, as the Pen's.
pub const SHAPE_MENU: &[Row] = shape_menu!(Some(("Free Transform Path", "path.transform")));

/// [`shape_menu!`] with no path: Free Transform, the menu bar's, on the active layer.
pub const SHAPE_MENU_NO_PATH: &[Row] = shape_menu!(Some(("Free Transform", "edit.freeTransform")));

/// The shape menu's rows that act on the layer, not the path: enabled and run as the menu bar's.
fn shape_layer_row(command: &str) -> bool {
    matches!(
        command,
        "layer.layerStyle.blendingOptions"
            | "layer.rasterize.layer"
            | "layer.rasterize.layerStyle"
            | "layer.rasterize.vectorMask"
            | "edit.freeTransform"
            | "view.newGuidesFromShape"
    )
}

/// Shape tools whose Shape-mode right-click opens [`SHAPE_MENU`] (#13249). The Ellipse (#13232)
/// and the other shape tools join here once measured.
pub fn shape_tool_menu(tool: Tool) -> bool {
    matches!(tool, Tool::Line)
}

/// Does a right-click with a shape tool open its menu? Only where measured: Shape mode, over
/// a pixel or shape layer. Path and Pixels mode, and other layer kinds, keep opening nothing.
fn shape_menu_measured(app: &PhotocraftApp) -> bool {
    app.ui.tool_options.shape_mode == craft::draw::ui::VectorMode::Shape
        && app.session.active().is_some_and(|st| {
            st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| matches!(l.content, LayerContent::Raster(_) | LayerContent::Shape(_)))
        })
}

/// The label a row shows, untranslated. The Pen's transform row names what it acts on, as in
/// Photoshop (#12913): the points of the path being drawn, or nothing when the document has no
/// path at all. A path that exists but isn't active, and the Path Selection and Direct Selection
/// menus, keep the table's label. The Add Anchor Point menu names the selected anchors (#12710).
pub(crate) fn row_label(app: &PhotocraftApp, menu: &CanvasToolMenu, label: &'static str, command: &str) -> &'static str {
    if menu.tool == Tool::AddAnchorPoint {
        return add_anchor_row_label(app, menu, label, command);
    }
    if command != "path.transform" || menu.tool != Tool::Pen {
        return label;
    }
    if pen_pending(app) {
        return "Free Transform Points";
    }
    let any_path = app.session.active().is_some_and(|st| {
        st.doc.work_path.is_some()
            || !st.doc.paths.is_empty()
            || st.doc.walk().iter().any(|(_, _, l)| l.vector_mask.is_some() || matches!(l.content, LayerContent::Shape(_)))
    });
    if any_path { label } else { "Free Transform" }
}

/// The Add Anchor Point menu's Fill, Stroke and transform rows, labelled as measured (#12710): with
/// anchors of the targeted path selected they read **Fill Subpath…** / **Stroke Subpath…** for one
/// subpath, **Fill Subpaths…** / **Stroke Subpaths…** for several, and **Free Transform Points**.
/// With no path targeted the transform row reads **Free Transform**. Only the labels follow the
/// selection: `path.fill`, `path.stroke` and `path.transform` take no subpath or anchor selection
/// yet, so the rows still act on the whole path.
fn add_anchor_row_label(app: &PhotocraftApp, menu: &CanvasToolMenu, label: &'static str, command: &str) -> &'static str {
    if !menu.has_path {
        return if command == "path.transform" { "Free Transform" } else { label };
    }
    // The other rows keep their label: return before looking at the selection, once per row.
    if !matches!(command, "path.fill" | "path.stroke" | "path.transform") {
        return label;
    }
    let ds = &app.ui.direct_selection;
    let Some([first, _]) = ds.anchors.first() else { return label };
    if !crate::direct_select::still_valid(app, ds) {
        return label;
    }
    let several = ds.anchors.iter().any(|[s, _]| s != first);
    match command {
        "path.fill" if several => "Fill Subpaths…",
        "path.fill" => "Fill Subpath…",
        "path.stroke" if several => "Stroke Subpaths…",
        "path.stroke" => "Stroke Subpath…",
        "path.transform" => "Free Transform Points",
        _ => label,
    }
}

/// The Color Sampler menu's Delete row: removes the sampler the menu opened on.
pub const SAMPLER_DELETE: &str = "view.colorSamplers.delete";

/// The Color Sampler's canvas menu (#11733), in the reference application's order: Delete, then
/// the readout modes for the sampler under the pointer. The readout rows are this menu's own ids
/// and stay greyed until samplers have their own readout mode.
pub const SAMPLER_MENU: &[Row] = &[
    Some(("Delete", SAMPLER_DELETE)),
    None,
    Some(("Actual Color", "colorSampler.readout.actual")),
    Some(("Proof Color", "colorSampler.readout.proof")),
    None,
    Some(("Grayscale", "colorSampler.readout.gray")),
    Some(("RGB Color", "colorSampler.readout.rgb")),
    Some(("Web Color", "colorSampler.readout.web")),
    Some(("HSB Color", "colorSampler.readout.hsb")),
    Some(("HSL Color", "colorSampler.readout.hsl")),
    Some(("CMYK Color", "colorSampler.readout.cmyk")),
    Some(("Lab Color", "colorSampler.readout.lab")),
    None,
    Some(("Grayscale 16 bit", "colorSampler.readout.gray16")),
    Some(("RGB 16 bit", "colorSampler.readout.rgb16")),
    Some(("CMYK 16 bit", "colorSampler.readout.cmyk16")),
    Some(("Lab 16 bit", "colorSampler.readout.lab16")),
    None,
    Some(("Total Ink", "colorSampler.readout.totalInk")),
];

/// Does the Color Sampler menu's index still name the sampler it opened on, in the same document?
/// `true` off a sampler, where there is nothing to go stale.
fn sampler_current(app: &PhotocraftApp, menu: &CanvasToolMenu) -> bool {
    let Some(i) = menu.sampler else { return true };
    let now = app.session.active_index().zip(app.session.active().and_then(|d| d.color_samplers.get(i).copied()));
    now.is_some() && now == menu.sampler_at
}

/// Is `menu` the Color Sampler's own menu (not the pasteboard or guide menu opened with it)?
fn sampler_menu(menu: &CanvasToolMenu) -> bool {
    menu.tool == Tool::ColorSampler && menu.kind == MenuKind::Tool
}

/// Is `menu` the Content-Aware Move tool's menu opened inside the active selection (#12194)?
pub fn inside_selection(menu: &CanvasToolMenu) -> bool {
    menu.tool == Tool::ContentAwareMove && menu.kind == MenuKind::Tool && menu.has_selection && menu.inside_selection
}

/// Content-Aware Move's canvas context menu requires an open document and leaves Bitmap, Duotone
/// and Multichannel documents on their existing no-menu path (#12194).
fn cam_allowed(app: &PhotocraftApp) -> bool {
    app.session.active().is_some_and(|s| !matches!(s.doc.mode, ColorMode::Bitmap | ColorMode::Duotone | ColorMode::Multichannel))
}

/// Is document point `d` on the image and inside the active selection?
fn selected_at(app: &PhotocraftApp, d: [f64; 2]) -> bool {
    d.iter().all(|v| v.is_finite())
        && d[0] >= 0.0
        && d[1] >= 0.0
        && app.session.active().is_some_and(|s| {
            d[0] < f64::from(s.doc.size.width)
                && d[1] < f64::from(s.doc.size.height)
                && s.doc.selection.as_ref().is_some_and(|sel| sel.sample_channel(d[0].floor() as i32, d[1].floor() as i32, 0) >= 0.5)
        })
}

/// Does a Red Eye right-click at `doc` open a menu (#12577)? Only where the reference application
/// was measured: an RGB document whose one layer is a pixel layer, with no selection or the pointer
/// inside it. Elsewhere it keeps opening nothing.
fn red_eye_measured(app: &PhotocraftApp, doc: Option<[f64; 2]>) -> bool {
    app.session.active().is_some_and(|s| {
        s.doc.mode == ColorMode::Rgb
            && matches!(s.doc.layers.as_slice(), [l] if matches!(l.content, LayerContent::Raster(_)) && l.frame.is_none())
            && (s.doc.selection.is_none() || doc.is_some_and(|d| selected_at(app, d)))
    })
}

/// Tools whose plain canvas right-click opens a menu: the selection tools' actions, the
/// Content-Aware Move menu, the path menu, the Color Sampler's, the Red Eye tool's and the shape
/// tools' (in Shape mode only: [`open`] checks it).
pub fn applies(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::RectMarquee
            | Tool::EllipseMarquee
            | Tool::SingleRowMarquee
            | Tool::SingleColumnMarquee
            | Tool::Lasso
            | Tool::PolygonLasso
            | Tool::MagneticLasso
            | Tool::MagicWand
            | Tool::ObjectSelection
            // Its brush is set from the options bar's chip; the canvas right-click is the
            // selection menu, as in Photoshop (#7268).
            | Tool::QuickSelection
            | Tool::ColorSampler
            // The layer menu (#11772).
            | Tool::Count
            // Sample Size and Copy Color (#11747).
            | Tool::Eyedropper
            | Tool::ContentAwareMove
            // The layer or selection menu, where measured (#12577).
            | Tool::RedEye
    ) || path_menu(tool)
        || shape_tool_menu(tool)
}

/// A Magnetic Lasso border is being drawn on the document in front: a right-click opens no menu
/// over it, and the border stays live, as in Photoshop (#11689). A border left on another
/// document doesn't count, and neither does an ⌥-drag's freehand segment (not measured).
pub fn magnetic_drawing(app: &PhotocraftApp) -> bool {
    app.canvas_tools.selection.outline_open()
}

/// Tools whose canvas right-click opens the path menu ([`PEN_MENU`]): the Pen, and the Path
/// Selection and Direct Selection tools, whose Photoshop menu offers the same whole-path actions
/// for the selected path (#1056).
pub fn path_menu(tool: Tool) -> bool {
    matches!(tool, Tool::Pen | Tool::PathSelection | Tool::DirectSelection) || point_menu(tool)
}

/// The anchor tools whose path menu also heads with the entry for the point under the right-click
/// (#12771, #12744, #12710).
fn point_menu(tool: Tool) -> bool {
    matches!(tool, Tool::AddAnchorPoint | Tool::DeleteAnchorPoint | Tool::ConvertPoint)
}

/// The Add Anchor Point tool's right-click on anchor `knot` of `target` selects it, as measured
/// (#12710); an anchor already among the selected ones keeps the selection. The caller passes only
/// an anchor that got a point entry, so the Pen draft and paths the anchor tools ignore never get here.
fn select_right_clicked_anchor(app: &mut PhotocraftApp, target: PathRef, knot: [usize; 2]) {
    let owner = crate::direct_select::Owner::current(app);
    let ds = &app.ui.direct_selection;
    if ds.target != Some(target) || !crate::direct_select::still_valid(app, ds) {
        app.ui.direct_selection = crate::direct_select::DirectSelection { target: Some(target), ..Default::default() };
    }
    let ds = &mut app.ui.direct_selection;
    ds.owner = owner;
    ds.segment = None;
    if !ds.anchors.contains(&knot) {
        ds.anchors = vec![knot];
    }
}

/// The entry for the anchor or segment `hit` on path `target`, as the anchor tools hit it: Delete
/// Anchor Point on an anchor, Add Anchor Point on a segment, at the clicked point. `None` for a
/// handle, or the Pen draft, which the engine doesn't edit.
fn point_at(app: &PhotocraftApp, target: PathRef, hit: BezierHit) -> Option<(String, Value, (u64, u64))> {
    let (command, mut params) = match hit {
        BezierHit::Anchor { knot, .. } => (POINT_DELETE, json!({"subpath": knot.subpath, "knot": knot.knot})),
        BezierHit::Segment { segment, t, .. } => (POINT_ADD, json!({"subpath": segment.subpath, "knot": segment.knot, "t": t})),
        // A handle, or a hit the framework adds later: no point entry.
        _ => return None,
    };
    if target == PathRef::Pen || crate::direct_select::ignores(app, &target) {
        return None;
    }
    if let (Some(o), Some(t)) = (params.as_object_mut(), target.params(app).as_object()) {
        o.extend(t.clone());
    }
    let stamp = app.session.active().map(|st| (st.doc.id.0, st.revision))?;
    Some((command.to_string(), params, stamp))
}

const POINT_DELETE: &str = "path.deleteAnchor";
const POINT_ADD: &str = "path.addAnchor";

/// The label of a menu's top point entry, spelled out so `cargo xtask i18n-coverage` lists it.
fn point_label(command: &str) -> String {
    if command == POINT_DELETE { tl!("Delete Anchor Point").to_string() } else { tl!("Add Anchor Point").to_string() }
}

/// Is `command` the open menu's top point entry?
fn is_point(menu: &CanvasToolMenu, command: &str) -> bool {
    menu.point.as_ref().is_some_and(|(c, ..)| c == command)
}

/// Is the open menu's point entry still about the path as it is? The document and its revision are
/// the ones the entry was read from; an edit meanwhile (say a control-channel `path.addAnchor`)
/// shifts the knot indexes, so a stale entry runs nothing.
fn point_current(app: &PhotocraftApp, menu: &CanvasToolMenu) -> bool {
    menu.point.as_ref().is_some_and(|(.., stamp)| app.session.active().map(|st| (st.doc.id.0, st.revision)) == Some(*stamp))
}

/// Tools whose plain canvas right-click offers the view menu.
pub fn is_view_tool(tool: Tool) -> bool {
    matches!(tool, Tool::Zoom | Tool::Hand)
}

/// The selection tools' rows, with or without an active selection.
pub fn selection_rows(has_selection: bool) -> &'static [Row] {
    if has_selection { SELECTION_MENU } else { NO_SELECTION_MENU }
}

/// The rows of an open menu. The crop, layer and brush-mode menus build theirs per menu: empty here.
pub fn rows(menu: &CanvasToolMenu) -> &'static [Row] {
    match &menu.kind {
        MenuKind::Crop { .. } | MenuKind::Layers { .. } | MenuKind::BrushMode { .. } => &[],
        MenuKind::Guide => GUIDE_MENU,
        MenuKind::Pasteboard => PASTEBOARD_MENU,
        MenuKind::View => {
            if menu.tool == Tool::Hand {
                HAND_MENU
            } else {
                VIEW_MENU
            }
        }
        MenuKind::TypeEdit { .. } => TYPE_EDIT_MENU,
        MenuKind::TypeLayer { .. } => type_layer_rows(menu),
        MenuKind::Tool => {
            if path_menu(menu.tool) {
                PEN_MENU
            } else if shape_tool_menu(menu.tool) {
                if menu.has_path { SHAPE_MENU } else { SHAPE_MENU_NO_PATH }
            } else if sampler_menu(menu) {
                SAMPLER_MENU
            } else if menu.tool == Tool::Count {
                COUNT_MENU
            } else if matches!(menu.tool, Tool::RedEye | Tool::ObjectSelection) {
                if menu.has_selection { MEASURED_SELECTION_MENU } else { COUNT_MENU }
            } else if menu.tool == Tool::Eyedropper {
                EYEDROPPER_MENU
            } else if inside_selection(menu) {
                CAM_INSIDE_SELECTION_MENU
            } else {
                selection_rows(menu.has_selection)
            }
        }
    }
}

/// Is `command` one of the open menu's rows, and enabled?
pub fn available(app: &PhotocraftApp, menu: &CanvasToolMenu, command: &str) -> bool {
    (is_point(menu, command) && point_current(app, menu))
        || ((rows(menu).iter().flatten().any(|(_, id)| *id == command) || is_spelling(menu, command) || is_crop(app, menu, command))
            && entry_enabled(app, menu, command))
}

/// Is `command` checked in the open menu: the crop bar's choice, a locked guide, the pasteboard
/// colour, the type styles or the Eyedropper's Sample Size?
pub(crate) fn mode_checked(app: &PhotocraftApp, menu: &CanvasToolMenu, command: &str) -> bool {
    match &menu.kind {
        MenuKind::Crop { .. } => crop_checked(app, command),
        MenuKind::Guide => command == "view.lockGuides" && app.ui.extras.lock_guides,
        MenuKind::Pasteboard => pasteboard_choice(command) == Some(app.session.prefs().interface.canvas_color),
        MenuKind::TypeEdit { .. } | MenuKind::TypeLayer { .. } => type_checked(app, command),
        _ if is_eyedropper(menu) => eyedropper_size(command).is_some_and(|n| app.ui.tool_options.eyedropper.size.pixels() == n),
        _ => false,
    }
}

/// The type edit menu's checkmarks: the faux styles at the selection, the layer's orientation and
/// anti-aliasing.
fn type_checked(app: &PhotocraftApp, command: &str) -> bool {
    use photocraft_doc::text::Orientation;
    let Some(t) = crate::type_tool::target_text(app) else { return false };
    match command {
        FAUX_BOLD | FAUX_ITALIC => crate::type_tool::styles_at(app).is_some_and(|(c, _)| if command == FAUX_BOLD { c.faux_bold } else { c.faux_italic }),
        "type.orientation.horizontal" => t.orientation == Orientation::Horizontal,
        "type.orientation.vertical" => t.orientation == Orientation::Vertical,
        c => c.strip_prefix("type.antiAlias.") == Some(photocraft_engine::type_extra_cmds::aa_name(t.antialias)),
    }
}

/// Is the Type tool editing text, so that a right-click opens the type edit menu?
pub fn editing_text(app: &PhotocraftApp, tool: Tool) -> bool {
    tool.is_type() && app.ui.text_edit.is_some()
}

/// The committed type layer under document point `doc` when a right-click there with a Type tool
/// opens the type layer menu (#7324). The Type Mask tools are left out: to them existing type is
/// just pixels.
pub fn type_layer_at(app: &mut PhotocraftApp, tool: Tool, doc: [f64; 2]) -> Option<photocraft_doc::LayerId> {
    if !tool.is_type() || tool.is_type_mask() || app.ui.text_edit.is_some() || !doc.iter().all(|v| v.is_finite()) {
        return None;
    }
    crate::type_tool::hit_layer(app, doc[0], doc[1])
}

/// Make the right-clicked type layer `id` the active layer, which the structural rows (Rasterize
/// Type, Convert to Shape…) act on and the checks show. Inside a multiselection the selection stays,
/// so the formatting rows still act on every selected type layer, as the Type menu's do; outside it
/// the type replaces the selection, as a click with the Type tool does.
fn target_type_layer(app: &mut PhotocraftApp, id: photocraft_doc::LayerId) -> bool {
    let Some(st) = app.session.active() else { return false };
    let selected = st.selected_layers();
    if st.active_layer == Some(id) {
        true
    } else if selected.contains(&id) {
        let selected = selected.to_vec();
        photocraft_engine::layer_multi_cmds::set_selection(&mut app.session, selected, Some(id), Some(id)).is_ok()
    } else {
        app.session.select_layer(id).is_ok()
    }
}

/// Are the type layer menu's faux rows live? They run `type.setStyle` on the selected type
/// layers, so they are offered exactly when that command takes them (#12043 review): at least
/// one selected type layer, and none of them under Lock All - `check_text_unlocked`'s one
/// rejection. A position lock styles fine, as the command does.
fn faux_rows_enabled(app: &PhotocraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    let targets: Vec<photocraft_doc::LayerId> =
        st.selected_layers().iter().copied().filter(|id| st.doc.layer(*id).is_some_and(|l| matches!(l.content, LayerContent::Text(_)))).collect();
    !targets.is_empty() && targets.iter().all(|id| !st.doc.effective_locks(*id).all)
}

/// The text shape of the menu's target layer, which is active after [`target_type_layer`]:
/// point or paragraph text get their single Convert row, anything else the pair (#12043).
fn active_type_shape(app: &PhotocraftApp) -> TypeShape {
    let Some(st) = app.session.active() else { return TypeShape::Other };
    match st.active_layer.and_then(|id| st.doc.layer(id)) {
        Some(l) => match &l.content {
            LayerContent::Text(t) => match t.shape {
                photocraft_doc::text::TextShape::Point => TypeShape::Point,
                photocraft_doc::text::TextShape::Box { .. } => TypeShape::Paragraph,
                _ => TypeShape::Other,
            },
            _ => TypeShape::Other,
        },
        None => TypeShape::Other,
    }
}

/// The misspelled word of the edited text under document point `doc`, with its suggestions.
fn spelling_at(app: &mut PhotocraftApp, doc: [f64; 2]) -> Option<Spelling> {
    let layer = app.ui.text_edit.as_ref()?.layer;
    if !doc.iter().all(|v| v.is_finite()) {
        return None;
    }
    let at = crate::type_tool::offset_at(app, photocraft_doc::LayerId(layer), doc[0], doc[1]);
    let found = app.session.execute("edit.checkSpelling", json!({"action": "list", "layer": layer, "suggestions": MAX_SUGGESTIONS})).ok()?;
    let hit = found["misspellings"].as_array()?.iter().find(|m| {
        let (a, b) = (m["start"].as_u64().unwrap_or(u64::MAX) as usize, m["end"].as_u64().unwrap_or(0) as usize);
        a <= at && at <= b
    })?;
    Some(Spelling {
        layer,
        start: hit["start"].as_u64()? as usize,
        end: hit["end"].as_u64()? as usize,
        word: hit["word"].as_str()?.to_string(),
        suggestions: hit["suggestions"].as_array()?.iter().filter_map(|s| s.as_str().map(str::to_string)).take(MAX_SUGGESTIONS).collect(),
    })
}

/// The spelling rows heading the type edit menu (label, id): the suggestions, or a greyed note
/// when there are none, then Add to Dictionary. Empty away from a misspelled word.
pub fn spelling_rows(menu: &CanvasToolMenu) -> Vec<(String, String)> {
    let MenuKind::TypeEdit { spelling: Some(sp) } = &menu.kind else { return Vec::new() };
    let mut rows: Vec<(String, String)> = sp.suggestions.iter().enumerate().map(|(i, s)| (s.clone(), format!("{SPELLING_REPLACE}{i}"))).collect();
    if rows.is_empty() {
        rows.push(("No Spelling Suggestions".into(), SPELLING_NONE.into()));
    }
    rows.push(("Add to Dictionary".into(), SPELLING_ADD.into()));
    rows
}

/// Is `command` one of the menu's spelling rows?
fn is_spelling(menu: &CanvasToolMenu, command: &str) -> bool {
    spelling_rows(menu).iter().any(|(_, id)| id == command)
}

/// Does a spelling row's label come from the UI (translated) rather than from the dictionary?
pub fn spelling_label_is_ui(id: &str) -> bool {
    matches!(id, SPELLING_NONE | SPELLING_ADD)
}

/// The pasteboard colour a pasteboard menu row picks (`None` for Select Custom Color… or a row
/// of another menu).
fn pasteboard_choice(command: &str) -> Option<CanvasColor> {
    command.strip_prefix("pasteboard.").and_then(CanvasColor::parse)
}

/// The engine command (and params) a pasteboard row runs, for the automation policy: every row
/// changes a preference.
pub fn engine_call(app: &PhotocraftApp, menu: &CanvasToolMenu, command: &str) -> (String, Value) {
    if matches!(menu.kind, MenuKind::Crop { .. }) && command == CROP_COMMIT {
        return ("image.crop".into(), json!({}));
    }
    if menu.kind == MenuKind::Guide {
        let selected = crate::guide_select::selected(app);
        return match command {
            GUIDE_EDIT => ("view.editGuide".into(), json!({})),
            GUIDE_CLEAR => ("view.deleteGuide".into(), selected.map_or(json!({}), |(v, i)| crate::guide_select::delete_params(v, i))),
            _ => (command.into(), json!({})),
        };
    }
    if sampler_menu(menu) {
        return (command.into(), menu.sampler.map_or(json!({}), |i| json!({"index": i})));
    }
    let typing = matches!(menu.kind, MenuKind::TypeEdit { .. } | MenuKind::TypeLayer { .. });
    match pasteboard_choice(command) {
        Some(c) if menu.kind == MenuKind::Pasteboard => ("prefs.set".into(), json!({"values": {"interface.canvasColor": c.name()}})),
        _ if menu.kind == MenuKind::Pasteboard => ("prefs.set".into(), json!({})),
        _ if typing && matches!(command, FAUX_BOLD | FAUX_ITALIC) => ("type.setStyle".into(), json!({})),
        _ if is_spelling(menu, command) => spelling_call(menu, command),
        _ => (command.into(), json!({})),
    }
}

/// The `edit.checkSpelling` call a spelling row makes: replace the word with a suggestion, or add
/// it to the user dictionary.
fn spelling_call(menu: &CanvasToolMenu, command: &str) -> (String, Value) {
    let MenuKind::TypeEdit { spelling: Some(sp) } = &menu.kind else { return ("edit.checkSpelling".into(), json!({})) };
    let replace = command.strip_prefix(SPELLING_REPLACE).and_then(|n| n.parse::<usize>().ok()).and_then(|n| sp.suggestions.get(n));
    match replace {
        Some(to) => {
            ("edit.checkSpelling".into(), json!({"action": "change", "layer": sp.layer, "start": sp.start, "end": sp.end, "word": sp.word, "replace": to}))
        }
        None => ("edit.checkSpelling".into(), json!({"action": "addToDictionary", "word": sp.word})),
    }
}

/// Does a right-click with `tool` at document point `doc` open the pasteboard menu? Outside the
/// image, for the tools whose right-click has no job there: not the painting tools (their Brush
/// Preset picker or right-button erase), the Pen (its path menu) or text being edited.
pub fn pasteboard_at(app: &PhotocraftApp, tool: Tool, doc: [f64; 2]) -> bool {
    let Some(st) = app.session.active() else { return false };
    let (w, h) = (f64::from(st.doc.size.width), f64::from(st.doc.size.height));
    let outside = doc.iter().all(|v| v.is_finite()) && (doc[0] < 0.0 || doc[1] < 0.0 || doc[0] >= w || doc[1] >= h);
    outside && !path_menu(tool) && !crate::paint_mouse::has_brush_picker(tool) && !crate::paint_mouse::right_erases(app, tool) && !editing_text(app, tool)
}

/// A guide row: every row is a View › Guides command, the two Selected Guides ones included.
fn choose_guide(app: &mut PhotocraftApp, ctx: &Context, command: &str) -> Result<(), String> {
    crate::menus::invoke(app, ctx, command, json!({})).map(|_| ())
}

/// A pasteboard row: set the colour, or open the Color Picker on the custom one.
fn choose_pasteboard(app: &mut PhotocraftApp, menu: &CanvasToolMenu, command: &str) -> Result<(), String> {
    if command == PASTEBOARD_PICK {
        let rgb = prefs::parse_hex(&app.session.prefs().interface.canvas_custom_color).map_or([0.5; 3], |c| c.map(|v| f32::from(v) / 255.0));
        crate::color_picker_ui::open_for_target(app, "pasteboard", "Color Picker (Custom Canvas Color)", rgb)?;
        return Ok(());
    }
    let (command, params) = engine_call(app, menu, command);
    app.run(&command, params).map(|_| ())
}

pub fn entry_enabled(app: &PhotocraftApp, menu: &CanvasToolMenu, command: &str) -> bool {
    match &menu.kind {
        MenuKind::Crop { .. } => return crop_enabled(app, menu, command),
        MenuKind::Guide => {
            return match command {
                GUIDE_EDIT | GUIDE_CLEAR => crate::guide_select::selected(app).is_some(),
                _ => GUIDE_MENU.iter().flatten().any(|(_, id)| *id == command) && crate::menus::is_enabled(app, command),
            };
        }
        MenuKind::Pasteboard => return PASTEBOARD_MENU.iter().flatten().any(|(_, id)| *id == command),
        MenuKind::View if command == "view.fillScreen" => return app.session.active().is_some(),
        MenuKind::View => return crate::menus::is_enabled(app, command),
        MenuKind::TypeEdit { .. } => {
            return match command {
                SPELLING_NONE => false,
                c if is_spelling(menu, c) => app.ui.text_edit.is_some(),
                FAUX_BOLD | FAUX_ITALIC => app.ui.text_edit.is_some(),
                c => TYPE_EDIT_MENU.iter().flatten().any(|(_, id)| *id == c) && crate::menus::is_enabled(app, c),
            };
        }
        MenuKind::TypeLayer { .. } => {
            return match command {
                // #12043: over committed type the faux rows toggle the whole layer: they run as
                // `type.setStyle` on its type layers, so they are live exactly when that command
                // takes them.
                FAUX_BOLD | FAUX_ITALIC => faux_rows_enabled(app),
                c => type_layer_rows(menu).iter().flatten().any(|(_, id)| *id == c) && crate::menus::is_enabled(app, c),
            };
        }
        MenuKind::Layers { .. } | MenuKind::BrushMode { .. } => return true,
        MenuKind::Tool => {}
    }
    if is_eyedropper(menu) {
        return eyedropper_size(command).is_some() || (matches!(command, EYEDROPPER_COPY_HTML | EYEDROPPER_COPY_HEX) && menu.color.is_some());
    }
    // Off a sampler every row is greyed. The readout rows wait for per-sampler readout modes.
    if sampler_menu(menu) {
        return command == SAMPLER_DELETE && menu.sampler.is_some() && sampler_current(app, menu);
    }
    // Inside the Content-Aware Move selection menu (#12194), Load Selection… stays greyed until a
    // selection channel (non-spot alpha channel) has been saved.
    if inside_selection(menu) && command == "select.loadSelection" {
        return app.session.active().is_some_and(|st| st.doc.channels.iter().any(|c| c.spot.is_none())) && crate::menus::is_enabled(app, command);
    }
    if !(path_menu(menu.tool) || shape_tool_menu(menu.tool)) {
        return crate::menus::is_enabled(app, command);
    }
    if shape_tool_menu(menu.tool) {
        if !rows(menu).iter().flatten().any(|(_, id)| *id == command) {
            return false;
        }
        // Measured enabled over a pixel layer too, where the menu bar greys it (#13249).
        if command == "view.newGuidesFromShape" {
            return app.session.active().is_some();
        }
        if shape_layer_row(command) {
            return crate::menus::is_enabled(app, command);
        }
    }
    let Some(st) = app.session.active() else { return false };
    let layer = st.active_layer.and_then(|id| st.doc.layer(id));
    let locked = layer.is_some_and(|l| st.doc.effective_locks(l.id).all);
    let can_paint = layer.is_some_and(|l| matches!(l.content, LayerContent::Raster(_))) && !locked;
    let shape_layer = layer.and_then(|l| match &l.content {
        LayerContent::Shape(sh) => Some(sh),
        _ => None,
    });
    let shape = shape_layer.is_some();
    let can_mask = layer.is_some_and(|l| !matches!(l.content, LayerContent::Shape(_))) && !locked;
    let saved = menu.path_name.as_deref().is_some_and(|n| n != "work" && n != "layer");
    let has_path = menu.has_path;
    let pending = pen_pending(app);
    let pending_layer = pending && (app.ui.tool_options.vector_mode == craft::draw::ui::VectorMode::Shape || app.ui.vector_mask_target);
    match command {
        "layer.vectorMask.fromPath" => has_path && can_mask && !pending_layer,
        "path.delete" => has_path && !pending_layer && (pending || menu.path_name.as_deref() != Some("layer")),
        "edit.defineCustomShape" | "path.toSelection" | "path.transform" | "paint.symmetryFromPath" => has_path,
        "view.newGuidesFromShape" => shape && !pending,
        "path.fill" | "path.stroke" => has_path && can_paint && !pending_layer,
        "path.clippingPath.set" => saved && !pending,
        "layer.combineShapes.unite"
        | "layer.combineShapes.subtractFrontShape"
        | "layer.combineShapes.intersectShapeAreas"
        | "layer.combineShapes.excludeOverlappingShapes" => !pending && shape_layer.is_some_and(|sh| sh.path.subpaths.len() > 1),
        "path.style.copyFill" => !pending && shape_layer.is_some_and(|sh| sh.fill.is_some()),
        "path.style.copyStroke" => !pending && shape_layer.is_some_and(|sh| sh.stroke.is_some()),
        "path.style.pasteFill" => !pending && shape && app.session.path_fill_clipboard.is_some(),
        "path.style.pasteStroke" => !pending && shape && app.session.path_stroke_clipboard.is_some(),
        "select.isolateLayers" => layer.is_some(),
        "paint.symmetryDisable" => st.symmetry.is_some(),
        _ => false,
    }
}

/// A right-click with `tool` and `mods` at screen point `at`, over document point `doc`: asks the
/// canvas's router (`paint_mouse::context_popup`) what opens, then opens it. `false` when nothing
/// opens.
pub fn open_at(app: &mut PhotocraftApp, tool: Tool, mods: Mods, at: Pos2, doc: [f64; 2], point: Option<(PathRef, BezierHit)>) -> bool {
    use crate::paint_mouse::ContextPopup; // PLAN: assumed `context_popup(app, tool, mods, doc) -> Option<ContextPopup>` with `Menu(MenuKind)` and `BrushPicker` (cw.P8)
    match crate::paint_mouse::context_popup(app, tool, mods, doc) {
        Some(ContextPopup::Menu(kind)) => open(app, tool, kind, at, doc, point),
        Some(ContextPopup::BrushPicker) => {
            crate::brush_picker::open(app, [at.x, at.y]);
            true
        }
        None => false,
    }
}

/// Opens the menu of `kind` for `tool` at screen point `at`, over document point `doc`, gathering
/// what the rows need: the misspelled word, the type layer, the saved crop presets, the layers
/// under the point, the sampler, the eyedropper's colour, the point entry. `false` (and nothing
/// open) when the menu has nothing to show: a non-finite point, a crop that isn't pending, no layers,
/// a tool without the menu, Content-Aware Move outside RGB, Gray and Lab, an unmeasured Red Eye.
pub fn open(app: &mut PhotocraftApp, tool: Tool, kind: MenuKind, at: Pos2, doc: [f64; 2], point: Option<(PathRef, BezierHit)>) -> bool {
    if !at.x.is_finite() || !at.y.is_finite() || !doc.iter().all(|v| v.is_finite()) {
        return false;
    }
    let mut menu = CanvasToolMenu {
        pos: [at.x, at.y],
        tool,
        kind: MenuKind::Tool,
        canvas: egui::Id::NULL,
        has_selection: false,
        has_path: false,
        path_name: None,
        under: None,
        sampler: None,
        sampler_at: None,
        color: None,
        inside_selection: false,
        point: None,
    };
    match kind {
        MenuKind::BrushMode { .. } if crate::brush_mode_menu::items(tool).is_empty() => {
            return false;
        }
        MenuKind::Pasteboard | MenuKind::Guide => {
            // Space's Hand keeps the menu over the selected tool (#11271); a held zoom key's goes with
            // the key, as before.
            if kind == MenuKind::Pasteboard {
                menu.under = (tool == Tool::Hand && tool != app.ui.tool).then_some(app.ui.tool);
            }
            menu.kind = kind;
        }
        MenuKind::BrushMode { .. } => {
            // The checked row reads the tool's own brush.
            crate::paint_mouse::sync_tool_brush(app);
            menu.kind = kind;
        }
        MenuKind::View => {
            if !is_view_tool(tool) {
                return false;
            }
            menu.under = (tool != app.ui.tool).then_some(app.ui.tool);
            menu.kind = kind;
        }
        MenuKind::Crop { .. } => {
            if tool != Tool::Crop || !crate::crop_ui::pending(app) {
                return false;
            }
            let presets = crate::crop_presets::user_presets(app).into_iter().map(|p| p.name).collect(); // PLAN: assumed `CropUserPreset { name, .. }` (tools-w46)
            menu.kind = MenuKind::Crop { presets };
        }
        MenuKind::Layers { .. } => {
            let layers = crate::layer_pick_ui::layers_at(app, doc[0], doc[1]);
            if layers.is_empty() {
                return false;
            }
            menu.kind = MenuKind::Layers { layers };
        }
        MenuKind::TypeEdit { .. } => {
            if !editing_text(app, tool) {
                return false;
            }
            menu.kind = MenuKind::TypeEdit { spelling: spelling_at(app, doc) };
            capture(app, &mut menu, doc, None);
        }
        MenuKind::TypeLayer { .. } => {
            let Some(id) = type_layer_at(app, tool, doc) else { return false };
            if !target_type_layer(app, id) {
                return false;
            }
            // The clicked layer is the menu's target; its shape picks the Convert row (#12043).
            menu.kind = MenuKind::TypeLayer { shape: active_type_shape(app) };
            capture(app, &mut menu, doc, None);
        }
        MenuKind::Tool => {
            if !applies(tool) {
                return false;
            }
            // Without an active document or in Bitmap/Duotone/Multichannel modes, Content-Aware Move
            // opens no menu (#12194); release any non-held control-channel HUD that would otherwise
            // stay open.
            if tool == Tool::ContentAwareMove && !cam_allowed(app) {
                crate::color_hud::release(&mut app.ui);
                return false;
            }
            if tool == Tool::RedEye && !red_eye_measured(app, Some(doc)) {
                return false;
            }
            if shape_tool_menu(tool) && !shape_menu_measured(app) {
                return false;
            }
            capture(app, &mut menu, doc, point);
            // The Eyedropper reads the colour now, under the right-click: all layers, with the Sample
            // Size, whatever its Sample setting (#11747). Picking a row later copies this colour.
            if tool == Tool::Eyedropper {
                menu.color = crate::canvas::composite_color(app, doc[0], doc[1]);
            }
        }
    }
    crate::brush_picker::close(app);
    app.ui.canvas_tool_menu = Some(menu);
    true
}

/// What a tool or type menu captures at the right-click: the selection, the path, the point
/// entry (`point`, the anchor or segment the caller hit), the Content-Aware Move selection and the
/// Color Sampler's sampler.
fn capture(app: &mut PhotocraftApp, menu: &mut CanvasToolMenu, doc: [f64; 2], point: Option<(PathRef, BezierHit)>) {
    menu.has_selection = app.session.active().is_some_and(|s| s.doc.selection.is_some());
    menu.path_name = crate::vector_ui::active_path_name(app);
    menu.has_path = menu.path_name.is_some() || pen_pending(app);
    if point_menu(menu.tool) && menu.has_path {
        menu.point = point.and_then(|(target, hit)| point_at(app, target, hit));
        // Add Anchor Point: the anchor the menu offers to delete is selected too (#12710).
        if menu.tool == Tool::AddAnchorPoint
            && menu.point.is_some()
            && let Some((target, BezierHit::Anchor { knot, .. })) = point
        {
            select_right_clicked_anchor(app, target, [knot.subpath, knot.knot]);
        }
    }
    menu.inside_selection = menu.tool == Tool::ContentAwareMove && selected_at(app, doc);
    if menu.tool == Tool::ColorSampler {
        menu.sampler = crate::analysis::sampler_at(app, doc); // PLAN: assumed name `analysis::sampler_at` (tools-w53)
        menu.sampler_at = menu.sampler.and_then(|i| Some((app.session.active_index()?, *app.session.active()?.color_samplers.get(i)?)));
    }
}

/// A view menu row: the View menu's item, which asks the canvas for its own command when the row is one (zoom, fit,
/// 100%, Fill Screen: the command its chord fires, zooming about the centre).
fn choose_view(app: &mut PhotocraftApp, ctx: &Context, command: &str) -> Result<(), String> {
    crate::menus::invoke(app, ctx, command, json!({})).map(|_| ())
}

/// Runs a row of the open menu and closes it.
pub fn choose(app: &mut PhotocraftApp, ctx: &Context, command: &str) {
    // The layer and brush-mode menus pick by id; the brush-mode menu reads itself while open.
    match app.ui.canvas_tool_menu.as_ref().map(|m| &m.kind) {
        Some(MenuKind::BrushMode { .. }) => {
            match crate::brush_mode_menu::find(app, command) {
                Ok(item) => crate::brush_mode_menu::choose(app, item),
                Err(e) => app.ui.status = e,
            }
            return;
        }
        Some(MenuKind::Layers { .. }) => {
            if let Some(layer) = command.strip_prefix(crate::layer_pick_ui::ROW).and_then(|n| n.parse::<u64>().ok()) {
                crate::layer_pick_ui::choose(app, layer);
            }
            return;
        }
        _ => {}
    }
    let Some(menu) = app.ui.canvas_tool_menu.take() else { return };
    if !available(app, &menu, command) {
        return;
    }
    let result = if is_point(&menu, command) {
        menu.point.as_ref().map_or(Ok(()), |(id, params, _)| app.run(id, params.clone()).map(|_| ()))
    } else {
        match &menu.kind {
            MenuKind::Crop { .. } => choose_crop(app, &menu, command),
            MenuKind::Guide => choose_guide(app, ctx, command),
            MenuKind::Pasteboard => choose_pasteboard(app, &menu, command),
            MenuKind::View => choose_view(app, ctx, command),
            MenuKind::TypeEdit { .. } if is_spelling(&menu, command) => choose_spelling(app, &menu, command),
            MenuKind::TypeEdit { .. } | MenuKind::TypeLayer { .. } => choose_type(app, ctx, command),
            MenuKind::Layers { .. } | MenuKind::BrushMode { .. } => Ok(()),
            // The shape menu's path rows run as the Pen's; its layer rows as the menu bar's, below.
            MenuKind::Tool if path_menu(menu.tool) || (shape_tool_menu(menu.tool) && !shape_layer_row(command)) => choose_pen(app, ctx, &menu, command),
            MenuKind::Tool if sampler_menu(&menu) => {
                let (id, params) = engine_call(app, &menu, command);
                app.run(&id, params).map(|_| ())
            }
            MenuKind::Tool if is_eyedropper(&menu) => choose_eyedropper(app, ctx, &menu, command),
            // Photoshop's Make Work Path… asks for the tolerance first.
            MenuKind::Tool if command == "select.toWorkPath" => {
                pen_dialogs::open(app, command, "");
                Ok(())
            }
            MenuKind::Tool => crate::menus::invoke(app, ctx, command, json!({})).map(|_| ()),
        }
    };
    if let Err(e) = result {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// An Eyedropper row (#11747): a size sets the Sample Size (shared with the options bar); a Copy
/// Color row puts the colour read at the right-click on the clipboard, as the Color panel's menu
/// does. Neither touches the foreground colour or the history.
fn choose_eyedropper(app: &mut PhotocraftApp, ctx: &Context, menu: &CanvasToolMenu, command: &str) -> Result<(), String> {
    if let Some(size) = eyedropper_size(command).and_then(craft::color_ui::ui::SampleSize::from_pixels) {
        app.ui.tool_options.eyedropper.size = size;
        return Ok(());
    }
    let rgb = menu.color.ok_or("No colour under the pointer")?;
    use craft::color_ui::ui::{ColorPanelCommand, ColorPanelMenu, ColorPanelState};
    let copy = if command == EYEDROPPER_COPY_HTML { ColorPanelCommand::CopyHtml } else { ColorPanelCommand::CopyHex };
    let color = craft::theme::Srgba::rgb(rgb[0], rgb[1], rgb[2]);
    if let Some(text) = ColorPanelMenu::run_command(&mut ColorPanelState::default(), copy, color) {
        ctx.copy_text(text);
    }
    Ok(())
}

/// A spelling row: replace the word under the pointer (the caret lands after the replacement), or
/// add it to the user dictionary.
fn choose_spelling(app: &mut PhotocraftApp, menu: &CanvasToolMenu, command: &str) -> Result<(), String> {
    let (id, params) = engine_call(app, menu, command);
    let result = app.run(&id, params)?;
    if let Some(end) = result["end"].as_u64().map(|e| e as usize)
        && let Some(e) = app.ui.text_edit.as_mut()
    {
        (e.caret, e.anchor) = (end, end);
    }
    Ok(())
}

/// A type edit row. Undo, Cut, Copy, Paste and Select All run as their Edit and Select menu items,
/// which act on the text while it is being edited (Paste asks the window for the clipboard text).
fn choose_type(app: &mut PhotocraftApp, ctx: &Context, command: &str) -> Result<(), String> {
    match command {
        FAUX_BOLD | FAUX_ITALIC => {
            let bold = command == FAUX_BOLD;
            let on = !type_checked(app, command);
            // While editing, the selection's characters are styled; over a committed type layer
            // the same run takes the whole layer, which is what formatting_params scopes to with
            // no selection (#12043). Either way one `type.setStyle` run is one history state.
            let mut p = crate::type_tool::formatting_params(app).ok_or("No type to format")?;
            p[if bold { "fauxBold" } else { "fauxItalic" }] = json!(on);
            app.run("type.setStyle", p)?;
        }
        _ => {
            if ENDS_TYPE_EDIT.contains(&command) {
                crate::type_tool::commit(app);
            }
            crate::menus::invoke(app, ctx, command, json!({}))?;
        }
    }
    Ok(())
}

fn choose_pen(app: &mut PhotocraftApp, ctx: &Context, menu: &CanvasToolMenu, command: &str) -> Result<(), String> {
    if command == "select.isolateLayers" {
        crate::menus::invoke(app, ctx, command, json!({}))?;
        return Ok(());
    }
    if command == "paint.symmetryDisable" {
        app.run(command, json!({}))?;
        return Ok(());
    }
    let pending = pen_pending(app);
    let pending_layer = pending && (app.ui.tool_options.vector_mode == craft::draw::ui::VectorMode::Shape || app.ui.vector_mask_target);
    if pending && !crate::vector_ui::pen_commit(app, false) {
        return Err(app.ui.status.clone());
    }
    let name = if pending { Some(if pending_layer { "layer" } else { "work" }.to_string()) } else { menu.path_name.clone() }
        .ok_or_else(|| "No path is available for this action".to_string())?;
    match command {
        "edit.defineCustomShape" | "path.toSelection" | "path.fill" | "path.stroke" | "path.clippingPath.set" | "path.transform" => {
            pen_dialogs::open(app, command, &name);
        }
        _ => {
            // The path name goes only to the commands whose doc takes one (Copy Fill and the
            // other style rows work on the shape layer and take no parameters).
            let takes_name =
                photocraft_engine::commands::find(command).and_then(photocraft_engine::param_schema::of).is_none_or(|p| p.open() || p.has_key("name"));
            app.run(command, if takes_name { json!({"name": name}) } else { json!({}) })?;
        }
    }
    Ok(())
}

/// Closes the open menu once what it was opened for is gone: the tool changed (`under` while
/// Space holds the Hand), the text edit it listed ended (or began, for the type layer menu), the
/// crop is no longer pending, or the sampler it names moved. A type layer menu follows its layer's
/// text shape, which a Convert or its Undo changes while the menu is open (#12043 review).
pub fn close_stale(app: &mut PhotocraftApp) {
    let Some(menu) = app.ui.canvas_tool_menu.as_ref() else { return };
    let stale = app.ui.tool != menu.under.unwrap_or(menu.tool)
        || match &menu.kind {
            MenuKind::TypeEdit { .. } => app.ui.text_edit.is_none(),
            MenuKind::TypeLayer { .. } => app.ui.text_edit.is_some(),
            MenuKind::Crop { .. } => !crate::crop_ui::pending(app),
            MenuKind::Tool => sampler_menu(menu) && !sampler_current(app, menu),
            _ => false,
        };
    let type_layer = matches!(menu.kind, MenuKind::TypeLayer { .. });
    if stale {
        app.ui.canvas_tool_menu = None;
        return;
    }
    if type_layer {
        let fresh = active_type_shape(app);
        if let Some(MenuKind::TypeLayer { shape }) = app.ui.canvas_tool_menu.as_mut().map(|m| &mut m.kind) {
            *shape = fresh;
        }
    }
}

/// The rows of `menu` for the framework menu, each row's id the command [`choose`] takes: the
/// spelling suggestions, the crop rows, the point entry, then the table's rows. The layer menu is
/// `layer_pick_ui::entries`, the brush-mode menu `brush_mode_menu::entries`.
pub fn entries(app: &PhotocraftApp, menu: &CanvasToolMenu) -> Vec<Entry<String>> {
    match &menu.kind {
        MenuKind::Layers { layers } => {
            return crate::layer_pick_ui::entries(layers, app.session.active().and_then(|s| s.active_layer).map(|l| l.0));
        }
        MenuKind::BrushMode { plain } => return crate::brush_mode_menu::entries(app, menu.tool, *plain),
        _ => {}
    }
    let mut out = Vec::new();
    let spelling = spelling_rows(menu);
    for (label, command) in &spelling {
        let text = if spelling_label_is_ui(command) { tl!(label).to_string() } else { label.clone() };
        out.push(Entry::new(command.clone(), text).enabled(entry_enabled(app, menu, command)));
    }
    if !spelling.is_empty() {
        out.push(Entry::separator());
    }
    for row in crop_rows(app, menu) {
        let Some((label, command)) = row else {
            out.push(Entry::separator());
            continue;
        };
        let mut entry = Entry::new(command.clone(), label).enabled(entry_enabled(app, menu, &command)).checked(mode_checked(app, menu, &command));
        // The overlay rows carry their own glyph (#10889); the rest stay text-only.
        if let Some(kind) = command.strip_prefix(CROP_OVERLAY).and_then(from_serde_id::<craft::behavior::geometry::CropGuide>) {
            entry = entry.icon(guide_icon(kind));
        }
        out.push(entry);
    }
    if let Some((command, ..)) = &menu.point {
        out.push(Entry::new(command.clone(), point_label(command)).enabled(available(app, menu, command)));
        out.push(Entry::separator());
    }
    for row in rows(menu) {
        let Some((label, command)) = *row else {
            out.push(Entry::separator());
            continue;
        };
        let text = match row_label(app, menu, label, command) {
            // Spelled out so `cargo xtask i18n-coverage` lists the new label.
            "Free Transform Points" => tl!("Free Transform Points"),
            "Fill Subpath…" => tl!("Fill Subpath…"),
            "Fill Subpaths…" => tl!("Fill Subpaths…"),
            "Stroke Subpath…" => tl!("Stroke Subpath…"),
            "Stroke Subpaths…" => tl!("Stroke Subpaths…"),
            "Rasterize Vector Mask" => tl!("Rasterize Vector Mask"),
            key => tl!(key),
        };
        out.push(Entry::new(command.to_string(), text.to_string()).enabled(entry_enabled(app, menu, command)).checked(mode_checked(app, menu, command)));
    }
    out
}

/// Runs every pass of the canvas that owns the menu. Closes a menu whose reason is gone, opens the
/// one a request asks for (a secondary press the tool did not take, the Menu key, or a guide's
/// context request, or a secondary press on the Crop frame's overlay), and shows the open menu on the framework's menu layer at its press point, then
/// the Brush Preset picker. A pass applies only its own requests: `host.context`,
/// `guides.context` and `overlay_context` are this pass's, so a second pass in a frame opens nothing twice.
pub(crate) fn run(
    ui: &mut egui::Ui,
    app: &mut PhotocraftApp,
    canvas: &CanvasOutput,
    host: &ToolHostResponse,
    guides: Option<&GuidesOutput>,
    overlay_context: Option<(Pos2, Mods)>,
) {
    close_stale(app);
    if let Some((at, mods)) = overlay_context
        && let Some(view) = crate::canvas::document_display::active_view(app)
    {
        // The overlay takes the press the tool host would have reported, so the Space-held Hand's menu is picked here
        // the way the host picks it.
        let tool = if crate::hold_keys::reposition_held(app) { Tool::Hand } else { app.ui.tool };
        let doc = view.to_doc(at);
        open_at(app, tool, mods, at, [doc.x, doc.y], None);
    }
    if let Some(request) = host.context {
        // A Space-held Hand opens the Hand's menu over the current tool. A momentary Zoom opens no menu of its own, but
        // with ⌘ held the right-click is still the one that lists the layers under the pointer (#307).
        let tool = match request.momentary {
            Some(MomentaryTool::Hand) => Some(Tool::Hand),
            Some(_) => crate::layer_pick_ui::is_gesture(Tool::Zoom, request.mods).then_some(app.ui.tool),
            None => Some(app.ui.tool),
        };
        if let Some(tool) = tool
            && let Some(view) = crate::canvas::document_display::active_view(app)
        {
            let doc = view.to_doc(request.at);
            open_at(app, tool, request.mods, request.at, [doc.x, doc.y], None);
        }
    }
    if let Some((key, at)) = guides.and_then(|g| g.context)
        && let Some(view) = crate::canvas::document_display::active_view(app)
    {
        let (vertical, index) = crate::guide_select::unkey(key);
        crate::guide_select::select(app, vertical, index);
        let doc = view.to_doc(at);
        open(app, app.ui.tool, MenuKind::Guide, at, [doc.x, doc.y], None);
    }
    if let Some(menu) = app.ui.canvas_tool_menu.as_mut()
        && menu.canvas == egui::Id::NULL
    {
        menu.canvas = canvas.response.id;
    }
    if let Some(menu) = app.ui.canvas_tool_menu.clone()
        && menu.canvas == canvas.response.id
    {
        let list = entries(app, &menu);
        let mut open = true;
        let shown = Menu::new(tl!("Canvas menu").to_string())
            .opened_by(&canvas.response, Trigger::Manual)
            .anchor(Anchor::Point(Pos2::new(menu.pos[0], menu.pos[1])))
            .open(Bind::bound(&mut open))
            .pick(ui, &list);
        if let Some(command) = shown.picked {
            choose(app, &ui.ctx().clone(), &command);
        } else if !open || shown.closed.is_some() {
            app.ui.canvas_tool_menu = None;
        }
    }
    crate::brush_picker::show_at(ui, app);
}

#[cfg(test)]
mod cam_menu_tests;
#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
pub(crate) mod count_tests;

#[cfg(test)]
mod pen_free_transform_label_tests;

#[cfg(test)]
mod make_selection_dialog_tests;

#[cfg(test)]
mod type_layer_menu_tests;

#[cfg(test)]
mod add_anchor_menu_tests;
#[cfg(test)]
mod delete_anchor_menu_tests;

#[cfg(test)]
mod convert_point_menu_tests;

#[cfg(test)]
mod object_selection_menu_tests;

#[cfg(test)]
mod line_menu_tests;
