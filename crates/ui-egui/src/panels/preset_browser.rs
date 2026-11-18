//! The preset panels (Gradients, Patterns, Styles, Shapes, Tool Presets), the shape and pattern pickers on the
//! framework's `PresetBrowser` and `PresetPicker`. Rows come from a panel's groups in the panel's folder order; the
//! browser's events run as the panel's engine commands, through one mapping for every preset panel (`events`).

use craft::color_ui::ui::gradient::GradientRamp;
use craft::keymap::CommandRef;
use craft::theme::Srgba;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, DropZone, Props as _, Show as _};
use craft::ui::list::ThumbAspect;
use craft::ui::menu::PanelRow;
use craft::ui::patterns::presets::{
    CellShape, DefaultOpen, PresetBrowser, PresetCommands, PresetEvent, PresetExport, PresetFileRequest, PresetFiles, PresetFooterEvent, PresetFooterOptions,
    PresetMenuAction, PresetPanelMenu, PresetPicker, PresetReorder, PresetRow, PresetState, PresetView,
};
use craft::ui::primitives::ContentPainterCx;
use craft::ui::primitives::textures::TextureCache;
use craft::ui::runtime::commands::{CommandSet as _, CommandState};
use craft::ui::tile_grid::TileZoom;
use egui::{Color32, ColorImage, Pos2, Rect, pos2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::preset_panels::{PresetUi, run, tool_presets};
use crate::PhotocraftApp;
use crate::state::Tool;

/// A row of a preset browser: a folder by name, or a preset by its key in the panel (a pattern id, a style name).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PresetKey {
    Group(String),
    Item(String),
}

/// What tells one preset panel from another: its key in `PresetUi`, its engine command prefix and its New button.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PresetPanel {
    /// The panel's key in `PresetUi` (`selected`, `group_order`, `browsers`).
    pub(crate) id: &'static str,
    /// The panel's name, in English.
    label: &'static str,
    /// The engine prefix of `{prefix}.edit` and `{prefix}.new`.
    pub(crate) prefix: &'static str,
    /// The params of `{prefix}.new`.
    new_params: fn() -> Value,
    /// A new preset opens its rename (patterns are named by their source and do not).
    rename_new: bool,
}

pub(crate) const PATTERNS: PresetPanel =
    PresetPanel { id: "patterns", label: "Patterns", prefix: "pattern.presets", new_params: || json!({}), rename_new: false };
pub(crate) const STYLES: PresetPanel =
    PresetPanel { id: "styles", label: "Styles", prefix: "style.presets", new_params: || json!({"name": "Style"}), rename_new: true };
pub(crate) const GRADIENTS: PresetPanel =
    PresetPanel { id: "gradients", label: "Gradients", prefix: "gradient.presets", new_params: || json!({"name": "Custom"}), rename_new: true };
pub(crate) const SHAPES: PresetPanel =
    PresetPanel { id: "shapes", label: "Shapes", prefix: "shape.presets", new_params: || json!({"name": "Shape"}), rename_new: true };
// A new tool preset stores the current tool's options, so its params come from the app (`panel_act`), not from here.
pub(crate) const TOOL_PRESETS: PresetPanel =
    PresetPanel { id: "toolPresets", label: "Tool Presets", prefix: "tool.presets", new_params: || json!({}), rename_new: true };

/// A folder of a panel: its name and its presets as (key, name).
pub(crate) struct Folder {
    pub(crate) name: String,
    pub(crate) items: Vec<(String, String)>,
}

/// The browser's rows: the folders in `order` (library indices), each followed by its presets.
fn rows(folders: &[Folder], order: &[usize]) -> Vec<PresetRow<PresetKey>> {
    let mut rows = Vec::new();
    for folder in order.iter().filter_map(|&at| folders.get(at)) {
        rows.push(PresetRow::group(PresetKey::Group(folder.name.clone()), folder.name.clone(), 0));
        rows.extend(folder.items.iter().map(|(key, name)| PresetRow::item(PresetKey::Item(key.clone()), name.clone(), 1)));
    }
    rows
}

/// The rows of `panel`'s `folders` in the panel's folder order.
fn panel_rows(presets: &PresetUi, panel: PresetPanel, folders: &[Folder]) -> Vec<PresetRow<PresetKey>> {
    rows(folders, &presets.display_order(panel.id, &names_of(folders)))
}

/// The thumbnail sizes of every preset panel, as factors of the grid's large thumbnail (48 pt): 20 to 96 pt in 6 pt
/// steps, 40 pt by default.
const THUMB_RANGE: TileZoom = TileZoom::bounded(20.0 / 48.0, 2.0, 6.0 / 48.0, 40.0 / 48.0);

/// Where preset drops are accepted: the active canvas's content rect, rulers excluded; nothing with no document.
pub(crate) fn canvas_drop_rect(app: &PhotocraftApp) -> Rect {
    crate::canvas::document_display::active_view(app).map_or(Rect::NOTHING, |view| view.rect)
}

/// Where preset drops land: the canvas proper, rulers excluded, on the canvas's layer.
fn canvas_zone(app: &PhotocraftApp) -> DropZone {
    DropZone::new(app.last_canvas_layer, canvas_drop_rect(app))
}

/// The browser footer's own controls.
type Footer<'a> = Box<dyn Fn(&mut egui::Ui) + 'a>;

/// How one panel's browser differs from the common one.
#[derive(Default)]
struct Look<'a> {
    /// The most height the presets take before they scroll; else the available height.
    room_height: Option<f32>,
    /// The shape of a grid tile.
    cell: Option<CellShape>,
    /// The browser shows a list only.
    list: bool,
    /// The notice of a browser with no presets.
    empty: Option<&'a str>,
    /// The footer's own controls.
    leading: Option<Footer<'a>>,
    /// The panel has no folders: no New Group button and no reordering.
    flat: bool,
}

/// Shows `panel`'s browser over `rows` with `thumb` painting a preset's thumbnail, and returns its events with the
/// keys of the rows it showed, in order. `presets` is the panel state, `zone` the canvas.
fn show(
    presets: &mut PresetUi,
    zone: DropZone,
    ui: &mut egui::Ui,
    panel: PresetPanel,
    rows: Vec<PresetRow<PresetKey>>,
    thumb: &dyn Fn(&ContentPainterCx, Rect, &str),
    look: Look<'_>,
) -> (Vec<PresetEvent<PresetKey>>, Vec<PresetKey>) {
    let selected = presets.selected.get(panel.id).cloned().map(PresetKey::Item);
    let mut state = presets.browsers.get(panel.id).cloned().unwrap_or_else(|| {
        let mut state = PresetState::new();
        state.zoom = THUMB_RANGE.default_value();
        state
    });
    let mut footer = PresetFooterOptions::new().new_group(!look.flat).can_delete(selected.is_some());
    if let Some(leading) = look.leading {
        footer = footer.leading(leading);
    }
    let mut browser = PresetBrowser::new(tl!(panel.label), Bind::bound(&mut state), &rows);
    if let Some(key) = selected.clone() {
        browser = browser.selected(key);
    }
    if let Some(height) = look.room_height {
        browser = browser.room_height(height);
    }
    if let Some(cell) = look.cell {
        browser = browser.cell_shape(cell);
    }
    if look.list {
        browser = browser.view_lock(PresetView::List);
    }
    if let Some(text) = look.empty {
        browser = browser.empty(text.to_owned());
    }
    if !look.flat {
        browser = browser.reorder(PresetReorder::items_and_groups());
    }
    let out = browser
        .browser_id(browser_id(panel))
        .default_open(DefaultOpen::First)
        .thumb_range(THUMB_RANGE)
        .drop_zone(zone)
        .footer(footer)
        .thumb(|cx, rect, info| {
            if let PresetKey::Item(key) = info.key {
                thumb(cx, rect, key);
            }
        })
        .show(ui);
    presets.browsers.insert(panel.id.to_string(), state);
    (out.events, rows.into_iter().map(|row| row.key).collect())
}

pub(crate) fn browser_id(panel: PresetPanel) -> egui::Id {
    egui::Id::unique(("preset-browser", panel.id))
}

/// Paints `image` (cached under `key` in the framework's texture store) into `rect`.
fn picture(cx: &ContentPainterCx, rect: Rect, key: impl std::hash::Hash + 'static, image: impl FnOnce() -> ColorImage) {
    let texture = TextureCache::texture(cx.painter.ctx(), key, image);
    cx.picture(texture, rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), 0.0);
}

fn c32(c: [f32; 4]) -> Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgba_unmultiplied(q(c[0]), q(c[1]), q(c[2]), q(c[3]))
}

/// A 64×64 swatch of `pattern`. Only the pixels the swatch shows are read: a pattern file can declare a padded source
/// of any size, and reading it whole to pick 4096 of its pixels is gigabytes of floats. Tiles smaller than the swatch
/// repeat; larger ones are shown shrunk.
fn pattern_image(pattern: &photocraft_doc::Pattern) -> ColorImage {
    const S: u32 = 64;
    let (w, h) = (pattern.width.max(1), pattern.height.max(1));
    let format = pattern.surface.format();
    let step = (w.max(h) as f32 / S as f32).max(1.0);
    let mut image = ColorImage::new([S as usize, S as usize], vec![Color32::TRANSPARENT; (S * S) as usize]);
    for y in 0..S {
        for x in 0..S {
            let sx = ((x as f32 * step) as u32) % w;
            let sy = ((y as f32 * step) as u32) % h;
            let px = pattern.surface.read_region(photocraft_geom::Rect::from_xywh(sx as i32, sy as i32, 1, 1));
            image.pixels[(y * S + x) as usize] = c32(photocraft_raster::to_rgba(&format, &px));
        }
    }
    image
}

/// The cached texture of `pattern`'s swatch.
pub(crate) fn pattern_texture(ctx: &egui::Context, pattern: &photocraft_doc::Pattern) -> egui::TextureId {
    TextureCache::texture(ctx, ("pattern", pattern.id.clone()), || pattern_image(pattern))
}

fn paint_pattern(cx: &ContentPainterCx, rect: Rect, pattern: &photocraft_doc::Pattern) {
    cx.picture(pattern_texture(cx.painter.ctx(), pattern), rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), 0.0);
}

// ------------------------------------------------------------------ Gradients

/// A gradient chip over `stops`, resolved against the current colours.
fn paint_gradient(cx: &ContentPainterCx, rect: Rect, stops: &[(f32, [f32; 4])]) {
    let colors: Vec<(f32, Srgba)> = stops.iter().map(|&(at, c)| (at, Srgba::new(c[0], c[1], c[2], c[3]))).collect();
    let (radius, edge) = GradientRamp::chip_look(&cx.style_cx());
    GradientRamp::paint_chip(cx, rect, radius, &colors, edge);
}

/// Window › Gradients: a click selects the gradient for the Gradient tool, a double click or a drop on the canvas
/// applies it.
pub fn gradients_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
    let groups = app.session.presets.gradients.clone();
    // PLAN: the current-gradient strip above the browser needs a painter component; the browser's selected row shows it.
    let folders: Vec<Folder> = groups
        .iter()
        .map(|group| Folder { name: group.name.clone(), items: group.items.iter().map(|preset| (preset.name.clone(), preset.name.clone())).collect() })
        .collect();
    let rows = panel_rows(&app.ui.presets_ui, GRADIENTS, &folders);
    let thumb = |cx: &ContentPainterCx, rect: Rect, name: &str| {
        if let Some(preset) = groups.iter().flat_map(|group| &group.items).find(|preset| preset.name == name) {
            paint_gradient(cx, rect, &preset.resolve(fg, bg));
        }
    };
    let zone = canvas_zone(app);
    let look = Look { cell: Some(CellShape::ratio(32, 18)), ..Look::default() };
    let (events, keys) = show(&mut app.ui.presets_ui, zone, ui, GRADIENTS, rows, &thumb, look);
    apply(app, ui.ctx(), GRADIENTS, &names_of(&folders), &keys, events);
}

// ------------------------------------------------------------------ Patterns

/// Window › Patterns: the library's patterns in the panel's folders. A click selects, a double click or a drop on
/// the canvas fills with the pattern.
pub fn patterns_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    // Until the preset store has loaded the saved patterns, their folder entries would look deleted: don't prune
    // them yet.
    if app.services.preset_store.is_none() {
        photocraft_engine::presets::patterns::sync(&mut app.session);
    }
    let library = app.session.patterns.items.clone();
    let folders: Vec<Folder> = app
        .session
        .presets
        .pattern_groups
        .iter()
        .map(|group| Folder {
            name: group.name.clone(),
            items: group
                .items
                .iter()
                .filter_map(|id| library.iter().find(|pattern| &pattern.id == id))
                .map(|pattern| (pattern.id.clone(), pattern.display_name().to_string()))
                .collect(),
        })
        .collect();
    let rows = panel_rows(&app.ui.presets_ui, PATTERNS, &folders);
    let thumb = |cx: &ContentPainterCx, rect: Rect, id: &str| {
        if let Some(pattern) = library.iter().find(|pattern| pattern.id == id) {
            paint_pattern(cx, rect, pattern);
        }
    };
    let zone = canvas_zone(app);
    let (events, keys) = show(&mut app.ui.presets_ui, zone, ui, PATTERNS, rows, &thumb, Look::default());
    apply(app, ui.ctx(), PATTERNS, &names_of(&folders), &keys, events);
}

// ------------------------------------------------------------------ Styles

/// The most height the Styles presets take before they scroll.
const STYLES_HEIGHT: f32 = 300.0;

/// The 64×64 thumbnail of `preset`, drawn over `session`'s patterns.
pub(crate) fn style_image(session: &photocraft_engine::Session, preset: &photocraft_engine::presets::styles::StylePreset) -> ColorImage {
    const S: u32 = 64;
    let px = photocraft_engine::presets::styles::thumbnail(session, preset, S);
    ColorImage::from_rgba_unmultiplied([S as usize, S as usize], &px)
}

/// Window › Styles: a click applies the style to the selected layers (Shift adds its effects), a drop on the canvas
/// applies it to the active layer.
pub fn styles_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let groups = app.session.presets.styles.clone();
    let folders = style_folders(app);
    let rows = panel_rows(&app.ui.presets_ui, STYLES, &folders);
    let zone = canvas_zone(app);
    let session = &app.session;
    let thumb = |cx: &ContentPainterCx, rect: Rect, name: &str| {
        let Some(style) = groups.iter().flat_map(|group| &group.items).find(|style| style.name == name) else { return };
        let key = ("style", style.name.clone(), style.effects.len(), format!("{:?}{:?}", style.blend, style.fill_opacity));
        picture(cx, rect, key, || style_image(session, style));
    };
    let look = Look { room_height: Some(STYLES_HEIGHT), ..Look::default() };
    let (events, keys) = show(&mut app.ui.presets_ui, zone, ui, STYLES, rows, &thumb, look);
    apply(app, ui.ctx(), STYLES, &names_of(&folders), &keys, events);
}

/// The Styles folders: each style group with its styles, keyed and named by the style's name.
fn style_folders(app: &PhotocraftApp) -> Vec<Folder> {
    let groups = &app.session.presets.styles;
    groups
        .iter()
        .map(|group| Folder { name: group.name.clone(), items: group.items.iter().map(|style| (style.name.clone(), style.name.clone())).collect() })
        .collect()
}

/// The pick purpose of the style library files, which keeps their last folder.
const STYLE_FILES: &str = "styles";

/// The Styles files: Load… reads a style library (`.asl`); Save… writes `export`, offered while there are styles.
fn style_files(any: bool, export: Option<PresetExport>) -> PresetFiles<'static, PresetKey> {
    PresetFiles::new(STYLE_FILES, tl!("Style Library"), &["asl"]).export(move |_| export.clone()).can_export(move |_| any)
}

/// What the Styles panel menu is built from: whether any style exists, the selected style and the browser's view.
fn styles_menu_facts(app: &PhotocraftApp) -> (bool, Option<PresetKey>, PresetView) {
    let any = app.session.presets.styles.iter().any(|group| !group.items.is_empty());
    let selected = app.ui.presets_ui.selected.get(STYLES.id).cloned().map(PresetKey::Item);
    let view = app.ui.presets_ui.browsers.get(STYLES.id).map_or(PresetView::Grid, |state| state.view);
    (any, selected, view)
}

/// The rows of the Styles panel menu: the framework's `PresetPanelMenu` rows. Save… writes the selected style, else
/// every style.
pub(crate) fn styles_rows(app: &PhotocraftApp) -> Vec<PanelRow> {
    let (any, selected, view) = styles_menu_facts(app);
    let files = style_files(any, None);
    PresetPanelMenu::new(view).new_group(true).files(&files).selected(selected.as_ref()).rows()
}

/// The state of each command `styles_rows` offers, read from the same rows so the two lists cannot drift.
pub(crate) fn styles_states(app: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    let (any, selected, view) = styles_menu_facts(app);
    let files = style_files(any, None);
    let menu = PresetPanelMenu::new(view).new_group(true).files(&files).selected(selected.as_ref());
    menu.rows()
        .into_iter()
        .filter_map(|row| {
            let PanelRow::Command(command) = row else { return None };
            let id = PresetCommands::from_ref(&command)?;
            let mut state = CommandState::new().enabled(menu.enabled(id));
            if let Some(on) = menu.checked(id) {
                state = state.checked(on);
            }
            Some((command, state))
        })
        .collect()
}

/// Runs a Styles panel menu command; `false` when `cmd` is not one of its commands.
pub(crate) fn styles_run(app: &mut PhotocraftApp, ctx: &egui::Context, cmd: &CommandRef) -> bool {
    let Some(command) = PresetCommands::from_ref(cmd) else { return false };
    let (any, selected, view) = styles_menu_facts(app);
    let files = style_files(any, None);
    let menu = PresetPanelMenu::new(view).new_group(true).files(&files).selected(selected.as_ref());
    let action = match command {
        PresetCommands::Save => export_styles(app, selected.as_ref()).and_then(|(export, count)| {
            style_files(any, Some(export)).request(PresetCommands::Save, selected.as_ref()).map(|request| (PresetMenuAction::File(request), count))
        }),
        command => menu.run_command(command).map(|action| (action, 0)),
    };
    let Some((action, count)) = action else { return true };
    let result = match action {
        PresetMenuAction::View(view) => {
            app.ui.presets_ui.browsers.entry(STYLES.id.to_string()).or_default().view = view;
            Ok(Value::Null)
        }
        PresetMenuAction::NewGroup => {
            let names = names_of(&style_folders(app));
            apply(app, ctx, STYLES, &names, &[], vec![PresetEvent::Footer(PresetFooterEvent::NewGroup)]);
            Ok(Value::Null)
        }
        PresetMenuAction::File(request) => style_file(app, request, count),
        _ => Ok(Value::Null),
    };
    if let Err(error) = result
        && error != "cancelled"
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
    true
}

/// The `.asl` of the `selected` style, else of every style, through `style.presets.exportAsl`, suggested as the
/// style's name (or "Styles") without path-illegal characters; with the number of styles it holds.
fn export_styles(app: &mut PhotocraftApp, selected: Option<&PresetKey>) -> Option<(PresetExport, u64)> {
    let name = match selected {
        Some(PresetKey::Item(name)) => Some(name.as_str()),
        _ => None,
    };
    let reply = run(app, "style.presets.exportAsl", name.map_or_else(|| json!({}), |name| json!({"styles": [name]})))?;
    let bytes = reply["data"].as_str().and_then(photocraft_engine::paint::tile::b64_decode)?;
    let stem: String = name.unwrap_or("Styles").chars().filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')).collect();
    let stem = if stem.trim().is_empty() { "Styles" } else { stem.trim() };
    Some((PresetExport::new(format!("{stem}.asl"), bytes), reply["count"].as_u64().unwrap_or(0)))
}

/// Runs a Styles file request: Load… imports the picked style library, Save… writes the export (`count` styles) where
/// the user picks.
fn style_file(app: &mut PhotocraftApp, request: PresetFileRequest, count: u64) -> Result<Value, String> {
    match request {
        PresetFileRequest::Open { extensions, .. } => {
            let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
            app.pick_file_bytes_filtered(&extensions, |app, name, bytes| match crate::preset_files_ui::open(app, &name, &bytes) {
                Some(result) => result.map(|()| Value::Null),
                None => Err(crate::i18n::fmt(tl!("{file} is not a style library (.asl)"), &[("file", &crate::file_open::display_name(&name))])),
            })
        }
        PresetFileRequest::Save { file_name, bytes, .. } => app.pick_save(&file_name, move |app, path| {
            let write = app.services.write.as_mut().ok_or("no writer configured")?;
            write(&path, &bytes)?;
            app.ui.status_error = false;
            app.ui.status = crate::i18n::fmt(
                &crate::i18n::trn(crate::i18n::current(), count, "Exported {n} style to {file}", "Exported {n} styles to {file}"),
                &[("file", &crate::file_open::display_name(&path))],
            );
            Ok(json!({"path": path, "count": count}))
        }),
        _ => Ok(Value::Null),
    }
}

fn names_of(folders: &[Folder]) -> Vec<String> {
    folders.iter().map(|folder| folder.name.clone()).collect()
}

// ------------------------------------------------------------------ Shapes

/// The 64×64 raster of a shape: its coverage as the alpha of a mid grey, which reads on a light and a dark panel
/// (`ContentPainterCx::picture` takes no tint).
pub(crate) fn shape_image(shape: &photocraft_engine::presets::shapes::ShapePreset) -> ColorImage {
    const S: u32 = 64;
    let coverage = photocraft_engine::presets::shapes::thumbnail(&shape.path, S);
    ColorImage::new(
        [S as usize, S as usize],
        coverage.iter().map(|a| Color32::from_rgba_unmultiplied(150, 150, 150, (a.clamp(0.0, 1.0) * 255.0).round() as u8)).collect(),
    )
}

/// The texture cache key of a shape's raster: its name and size of path.
fn shape_key(shape: &photocraft_engine::presets::shapes::ShapePreset) -> (&'static str, String, usize) {
    ("shape", shape.name.clone(), shape.path.subpaths.iter().map(|s| s.knots.len()).sum())
}

fn shape_texture(ctx: &egui::Context, shape: &photocraft_engine::presets::shapes::ShapePreset) -> egui::TextureId {
    TextureCache::texture(ctx, shape_key(shape), || shape_image(shape))
}

/// Window › Shapes: a click picks the shape for the Custom Shape tool, a double click places it, and a drop on the
/// canvas places it where it lands.
pub fn shapes_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let groups = photocraft_engine::presets::shapes::all_groups(&app.session);
    let folders: Vec<Folder> = groups
        .iter()
        .map(|group| Folder { name: group.name.clone(), items: group.items.iter().map(|shape| (shape.name.clone(), shape.name.clone())).collect() })
        .collect();
    let rows = panel_rows(&app.ui.presets_ui, SHAPES, &folders);
    let thumb = |cx: &ContentPainterCx, rect: Rect, name: &str| {
        if let Some(shape) = groups.iter().flat_map(|group| &group.items).find(|shape| shape.name == name) {
            picture(cx, rect.shrink(2.0), shape_key(shape), || shape_image(shape));
        }
    };
    let zone = canvas_zone(app);
    let (events, keys) = show(&mut app.ui.presets_ui, zone, ui, SHAPES, rows, &thumb, Look::default());
    apply(app, ui.ctx(), SHAPES, &names_of(&folders), &keys, events);
}

fn shape_fill(app: &PhotocraftApp) -> Value {
    let f = app.session.tools.foreground;
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if app.ui.tool_options.shape_fill { json!(format!("#{:02x}{:02x}{:02x}", q(f[0]), q(f[1]), q(f[2]))) } else { Value::Null }
}

/// The `shape.presets.place` params of `key`: at its default place, or in a square centred on the document point under
/// the drop at `at`.
fn shape_place(app: &PhotocraftApp, key: &str, at: Option<Pos2>) -> Value {
    let mut params = json!({"preset": key, "fill": shape_fill(app)});
    if let Some(pos) = at
        && let (Some(c), Some(d)) = (crate::canvas::document_display::doc_point_at(app, pos), app.session.active())
    {
        let side = (d.doc.size.width.min(d.doc.size.height) as f64 * 0.3).max(8.0);
        params["rect"] = json!([c.x - side / 2.0, c.y - side / 2.0, side, side]);
    }
    params
}

/// The pickers' view state, from the panel state's `browsers` (keyed by preset kind there, by `String` here).
fn picker_state(presets: &PresetUi, id: &str) -> PresetState<String> {
    let mut state = PresetState::new();
    if let Some(saved) = presets.browsers.get(id) {
        state.view = saved.view;
        state.zoom = saved.zoom;
        state.filter = saved.filter.clone();
    }
    state
}

/// Keeps the view state of a picker's popover in `browsers[id]`.
// PLAN: `browsers` holds `PresetState<PresetKey>`; a picker keys its rows by `String`, so its view, zoom and filter are copied.
fn keep_picker_state(presets: &mut PresetUi, id: &str, state: PresetState<String>) {
    let saved = presets.browsers.entry(id.to_string()).or_default();
    saved.view = state.view;
    saved.zoom = state.zoom;
    saved.filter = state.filter;
}

/// Options-bar shape picker for the Custom Shape tool.
pub fn shape_picker(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    const GROUP: &str = "group:";
    const ID: &str = "shapePicker";
    let ctx = ui.ctx().clone();
    let groups = photocraft_engine::presets::shapes::all_groups(&app.session);
    let mut rows: Vec<PresetRow<String>> = Vec::new();
    for group in &groups {
        rows.push(PresetRow::group(format!("{GROUP}{}", group.name), group.name.clone(), 0));
        for shape in &group.items {
            let texture = shape_texture(&ctx, shape);
            rows.push(PresetRow::item(shape.name.clone(), shape.name.clone(), 1).tooltip(shape.name.clone()).preview(ThumbAspect::SQUARE, move |cx, rect| {
                cx.picture(texture, rect.shrink(2.0), Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), 0.0);
            }));
        }
    }
    let mut value = Some(app.ui.presets_ui.shape().to_string());
    let before = value.clone();
    let mut state = picker_state(&app.ui.presets_ui, ID);
    let _ = PresetPicker::new(tl!("Shape"), Bind::bound(&mut value), &rows, Bind::bound(&mut state)).columns(7).show(ui); // the picked value comes back through `value`
    keep_picker_state(&mut app.ui.presets_ui, ID, state);
    if let Some(picked) = value.filter(|picked| before.as_ref() != Some(picked) && !picked.starts_with(GROUP)) {
        app.ui.presets_ui.custom_shape = picked;
    }
}

/// Custom Shape tool drag: places the selected shape in the dragged rect (⇧ keeps proportions).
pub fn finish_custom_shape(app: &mut PhotocraftApp, rect: [f64; 4], keep: bool, fill: Value, stroke: Value) {
    let name = app.ui.presets_ui.shape().to_string();
    let r = run(app, "shape.presets.place", json!({"preset": name, "rect": rect, "keepAspect": keep, "fill": fill, "stroke": stroke}));
    if r.is_none() && !photocraft_engine::presets::shapes::all_groups(&app.session).iter().any(|g| g.items.iter().any(|s| s.name == name)) {
        let template = "No custom shape \"{name}\": pick one in the options bar or Window › Shapes";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("name", &name)]);
    }
}

// ------------------------------------------------------------------ Tool Presets

/// Window › Tool Presets: a click applies the preset's tool and options. Current Tool Only lists the current tool's
/// presets.
pub fn tool_presets_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let only = app.ui.presets_ui.current_tool_only;
    let current = app.ui.tool;
    let rows: Vec<PresetRow<PresetKey>> = app
        .session
        .presets
        .tool_presets
        .iter()
        .filter(|preset| !only || Tool::from_name(&preset.tool) == Some(current))
        .map(|preset| {
            let icon = Tool::from_name(&preset.tool).map_or_else(|| craft::behavior::IconId::new("settings"), crate::icons::tool_icon);
            PresetRow::item(PresetKey::Item(preset.name.clone()), preset.name.clone(), 0).icon(icon)
        })
        .collect();
    // The footer's closure is `Fn`, so the check box's value goes through a cell.
    let flag = std::cell::Cell::new(only);
    let leading = |ui: &mut egui::Ui| {
        let mut on = flag.get();
        Checkbox::new(Bind::bound(&mut on)).label(tl!("Current Tool Only")).show(ui);
        flag.set(on);
    };
    let zone = canvas_zone(app);
    let look = Look {
        list: true,
        flat: true,
        empty: Some(if only { tl!("No presets for the current tool.") } else { tl!("No tool presets.") }),
        leading: Some(Box::new(leading)),
        ..Look::default()
    };
    let (events, keys) = show(&mut app.ui.presets_ui, zone, ui, TOOL_PRESETS, rows, &|_, _, _| {}, look);
    app.ui.presets_ui.current_tool_only = flag.get();
    apply(app, ui.ctx(), TOOL_PRESETS, &[], &keys, events);
}

// ------------------------------------------------------------------ events

/// What an event does: an engine command, a change of the panel's folder order (app UI state, no command), or a
/// switch of the app's tool.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Act {
    Run(String, Value),
    MoveFolder {
        from: String,
        before: Option<String>,
    },
    /// Picking a shape switches to the Custom Shape tool.
    CustomShapeTool,
    /// Apply the tool preset of this name.
    ToolPreset(String),
}

/// The command `panel` runs for `event`, `None` for the events the browser keeps (open state) or that change nothing.
/// `rows` is the browser's rows in display order, `selected` the panel's selected preset. Shared by every preset panel;
/// a panel's own Select, Activate and DropAt come first in `panel_act`.
pub(crate) fn act(panel: PresetPanel, event: &PresetEvent<PresetKey>, rows: &[PresetKey], selected: Option<&str>) -> Option<Act> {
    let edit = format!("{}.edit", panel.prefix);
    let run = |params: Value| Some(Act::Run(edit.clone(), params));
    match event {
        PresetEvent::Rename { key: PresetKey::Item(key), text, .. } => run(json!({"action": "rename", "preset": key, "name": text})),
        PresetEvent::Rename { key: PresetKey::Group(group), text, .. } => run(json!({"action": "renameGroup", "group": group, "name": text})),
        PresetEvent::Delete(PresetKey::Item(key)) => run(json!({"action": "delete", "preset": key})),
        PresetEvent::Delete(PresetKey::Group(group)) => run(json!({"action": "deleteGroup", "group": group})),
        PresetEvent::MoveTo { key: PresetKey::Item(key), group: PresetKey::Group(group), .. } => run(json!({"action": "move", "preset": key, "to": group})),
        PresetEvent::Footer(PresetFooterEvent::NewGroup) => run(json!({"action": "newGroup", "name": "Group"})),
        PresetEvent::Footer(PresetFooterEvent::New) => Some(Act::Run(format!("{}.new", panel.prefix), (panel.new_params)())),
        PresetEvent::Footer(PresetFooterEvent::Delete) => selected.and_then(|key| run(json!({"action": "delete", "preset": key}))),
        PresetEvent::Reorder(edit) if matches!(edit.phase(), Some(craft::behavior::GesturePhase::Commit) | None) => {
            let mv = edit.value()?;
            // `to` counts the rows left once the moved ones, with a moved folder's presets, are taken out.
            let mut moving_folder = false;
            let remaining: Vec<&PresetKey> = rows
                .iter()
                .filter(|key| {
                    let moved = mv.keys.contains(key);
                    if matches!(key, PresetKey::Group(_)) {
                        moving_folder = moved;
                    }
                    !moved && !(moving_folder && matches!(key, PresetKey::Item(_)))
                })
                .collect();
            match &mv.key {
                PresetKey::Group(from) => {
                    // The next folder after the landing place, in the order without the moved one.
                    let before = remaining[mv.to.min(remaining.len())..].iter().find_map(|key| match key {
                        PresetKey::Group(name) => Some(name.clone()),
                        PresetKey::Item(_) => None,
                    });
                    Some(Act::MoveFolder { from: from.clone(), before })
                }
                PresetKey::Item(key) => {
                    let head = &remaining[..mv.to.min(remaining.len())];
                    let folder = head.iter().rposition(|key| matches!(key, PresetKey::Group(_)))?;
                    let PresetKey::Group(group) = head[folder] else { return None };
                    run(json!({"action": "move", "preset": key, "to": group, "index": head.len() - folder - 1}))
                }
            }
        }
        _ => None,
    }
}

/// `panel`'s own handling of the events that use a preset (and, for Tool Presets, of New): Patterns select and fill,
/// Styles apply, Gradients select and apply, Shapes pick and place, Tool Presets apply.
fn panel_act(app: &PhotocraftApp, panel: PresetPanel, event: &PresetEvent<PresetKey>) -> Option<Act> {
    if panel.id == TOOL_PRESETS.id && matches!(event, PresetEvent::Footer(PresetFooterEvent::New)) {
        return Some(Act::Run(format!("{}.new", panel.prefix), tool_presets::new_params(app, app.ui.tool)));
    }
    let (key, add, at) = match event {
        PresetEvent::Select { key: PresetKey::Item(key), mods, .. } | PresetEvent::Activate { key: PresetKey::Item(key), mods, .. } => (key, mods.shift, None),
        PresetEvent::DropAt { key: PresetKey::Item(key), at, .. } => (key, false, Some(*at)),
        _ => return None,
    };
    let select = matches!(event, PresetEvent::Select { .. });
    Some(match panel.id {
        "patterns" if select => Act::Run("pattern.presets.select".into(), json!({"pattern": key})),
        "patterns" => Act::Run("pattern.presets.apply".into(), json!({"pattern": key})),
        "gradients" if select => Act::Run("gradient.presets.select".into(), json!({"preset": key})),
        "gradients" => Act::Run("gradient.presets.apply".into(), json!({"preset": key})),
        "shapes" if select => Act::CustomShapeTool,
        "shapes" => Act::Run("shape.presets.place".into(), shape_place(app, key, at)),
        "toolPresets" if at.is_some() => return None,
        "toolPresets" => Act::ToolPreset(key.clone()),
        // A drop styles the active layer; a click the selected layers, Shift adding the effects.
        _ if at.is_some() => Act::Run("style.presets.apply".into(), json!({"preset": key})),
        _ => Act::Run("style.presets.apply".into(), json!({"preset": key, "add": add})),
    })
}

/// Runs `events` for `panel`, in order, and keeps the panel's selection and folder order. `rows` are the keys of the
/// rows the browser showed, which the events' positions count in.
fn apply(app: &mut PhotocraftApp, ctx: &egui::Context, panel: PresetPanel, names: &[String], rows: &[PresetKey], events: Vec<PresetEvent<PresetKey>>) {
    for event in events {
        let selected = app.ui.presets_ui.selected.get(panel.id).cloned();
        let Some(act) = panel_act(app, panel, &event).or_else(|| act(panel, &event, rows, selected.as_deref())) else { continue };
        if let PresetEvent::Select { key: PresetKey::Item(key), .. }
        | PresetEvent::Activate { key: PresetKey::Item(key), .. }
        | PresetEvent::DropAt { key: PresetKey::Item(key), .. } = &event
        {
            app.ui.presets_ui.selected.insert(panel.id.into(), key.clone());
            // The Custom Shape tool draws the shape the panel last used.
            if panel.id == SHAPES.id {
                app.ui.presets_ui.custom_shape = key.clone();
            }
        }
        match act {
            Act::MoveFolder { from, before } => app.ui.presets_ui.move_group(panel.id, names, &from, before.as_deref()),
            Act::CustomShapeTool => app.ui.tool = Tool::CustomShape,
            Act::ToolPreset(name) => tool_presets::select(app, &name),
            Act::Run(command, params) => {
                let Some(reply) = run(app, &command, params) else { continue };
                after(app, ctx, panel, &event, &reply);
            }
        }
    }
}

/// What a command's reply changes in the panel: the selection follows a rename, a delete clears it, a new group and a
/// new preset open their rename.
fn after(app: &mut PhotocraftApp, ctx: &egui::Context, panel: PresetPanel, event: &PresetEvent<PresetKey>, reply: &Value) {
    let ui = &mut app.ui.presets_ui;
    match event {
        PresetEvent::Rename { key: PresetKey::Item(key), text, .. } if ui.selected.get(panel.id) == Some(key) => {
            ui.selected.insert(panel.id.into(), text.clone());
        }
        PresetEvent::Delete(PresetKey::Item(_)) | PresetEvent::Footer(PresetFooterEvent::Delete) => {
            ui.selected.remove(panel.id);
        }
        PresetEvent::Footer(PresetFooterEvent::NewGroup) => {
            let group = reply["group"].as_str().unwrap_or_default().to_string();
            PresetBrowser::request_rename(ctx, browser_id(panel), &PresetKey::Group(group));
        }
        PresetEvent::Footer(PresetFooterEvent::New) => {
            let name = reply.get("name").or(reply.get("pattern")).and_then(Value::as_str).unwrap_or_default().to_string();
            ui.selected.insert(panel.id.into(), name.clone());
            if panel.rename_new {
                PresetBrowser::request_rename(ctx, browser_id(panel), &PresetKey::Item(name));
            }
        }
        _ => {}
    }
}

// ------------------------------------------------------------------ pattern picker

/// The library's patterns as picker rows, each with its swatch and its name as the tooltip. The pattern picker and the
/// dialogs' pattern lists show these.
pub(crate) fn pattern_rows(app: &PhotocraftApp, ctx: &egui::Context) -> Vec<PresetRow<String>> {
    app.session
        .patterns
        .items
        .iter()
        .map(|pattern| {
            let texture = pattern_texture(ctx, pattern);
            let name = pattern.display_name().to_string();
            PresetRow::item(pattern.id.clone(), name.clone(), 0).tooltip(name).preview(ThumbAspect::SQUARE, move |cx, rect| {
                cx.picture(texture, rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), 0.0);
            })
        })
        .collect()
}

/// Photoshop's pattern picker: a swatch of the library pattern `selected` (an id) that opens the library's swatches,
/// each named in its tooltip. Returns the id of a newly picked pattern.
pub(crate) fn pattern_picker(app: &mut PhotocraftApp, ui: &mut egui::Ui, selected: &str) -> Option<String> {
    const ID: &str = "patternPicker";
    let rows = pattern_rows(app, ui.ctx());
    let mut value = Some(selected.to_string()).filter(|id| app.session.patterns.items.iter().any(|pattern| &pattern.id == id));
    let before = value.clone();
    let mut state = picker_state(&app.ui.presets_ui, ID);
    let _ = PresetPicker::new(tl!("Patterns"), Bind::bound(&mut value), &rows, Bind::bound(&mut state)).show(ui);
    keep_picker_state(&mut app.ui.presets_ui, ID, state);
    value.filter(|picked| before.as_ref() != Some(picked))
}

#[cfg(test)]
mod tests;
