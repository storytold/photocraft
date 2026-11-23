//! The preset panels (Gradients, Patterns, Styles, Shapes, Tool Presets), the shape and pattern pickers on the
//! framework's `PresetBrowser` and `PresetPicker`. Rows come from a panel's groups in the panel's folder order; the
//! browser's events run as the panel's engine commands, through one mapping for every preset panel (`events`).

use craft::color_ui::ui::gradient::GradientRamp;
use craft::keymap::CommandRef;
use craft::theme::Srgba;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, DropZone, MemoProps, PlainData, Props as _, Revision, Show as _, Tracked};
use craft::ui::list::ThumbAspect;
use craft::ui::menu::PanelRow;
use craft::ui::patterns::presets::{
    CellShape, DefaultOpen, PresetBrowser, PresetCommands, PresetEvent, PresetExport, PresetFileRequest, PresetFiles, PresetFns, PresetFooterEvent,
    PresetFooterOptions, PresetMenuAction, PresetPanelMenu, PresetPicker, PresetReorder, PresetRow, PresetSource, PresetState, PresetView, ThumbFn, ThumbInfo,
};
use craft::ui::primitives::ContentPainterCx;
use craft::ui::primitives::textures::TextureCache;
use craft::ui::runtime::commands::{CommandSet as _, CommandState};
use craft::ui::runtime::env::Env;
use craft::ui::tile_grid::TileZoom;
use egui::{Color32, ColorImage, Pos2, Rect, TextureId, pos2};
use photocraft_engine::pattern_cmds::PatternLibrary;
use photocraft_engine::presets::PresetsExt;
use photocraft_engine::presets::shapes::ShapePreset;
use photocraft_engine::presets::styles::StylePreset;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::sync::Arc;

use super::preset_panels::{PresetUi, run, tool_presets};
use crate::PhotocraftApp;
use crate::state::Tool;

/// A row of a preset browser: a folder by name, or a preset by its key in the panel (a pattern id, a style name).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, MemoProps)]
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
#[derive(MemoProps)]
pub(crate) struct Folder {
    pub(crate) name: String,
    pub(crate) items: Vec<(String, String)>,
}

/// What a panel's presets are built from: its folders, their display order (library indices) and what its
/// thumbnails paint from.
#[derive(MemoProps)]
struct PanelInputs<T> {
    folders: Vec<Folder>,
    order: Vec<usize>,
    thumbs: T,
}

impl<T: MemoProps> PanelInputs<T> {
    /// `panel`'s `folders` in the panel's folder order, with `thumbs`.
    fn new(presets: &PresetUi, panel: PresetPanel, folders: Vec<Folder>, thumbs: T) -> Self {
        let order = presets.display_order(panel.id, &names_of(&folders));
        PanelInputs { folders, order, thumbs }
    }
}

/// The browser's rows: the folders in their order, each followed by its presets.
fn panel_rows<T>(inputs: &PanelInputs<T>, _: &Env) -> Vec<PresetRow<PresetKey>> {
    let mut rows = Vec::new();
    for folder in inputs.order.iter().filter_map(|&at| inputs.folders.get(at)) {
        rows.push(PresetRow::group(PresetKey::Group(folder.name.clone()), folder.name.clone(), 0));
        rows.extend(folder.items.iter().map(|(key, name)| PresetRow::item(PresetKey::Item(key.clone()), name.clone(), 1)));
    }
    rows
}

/// The source of a panel's presets over `inputs`, its thumbnails painted by `thumb`.
fn panel_source<T: MemoProps + Send + 'static>(inputs: PanelInputs<T>, thumb: ThumbFn<PanelInputs<T>, PresetKey>) -> PresetSource<'static, PresetKey> {
    PresetSource::new(inputs, PresetFns::new(panel_rows).thumb(thumb))
}

/// What a swatch's picture is drawn from.
#[derive(Clone)]
enum Art {
    Pattern(Arc<photocraft_doc::Pattern>),
    Shape(Arc<ShapePreset>),
}

/// A swatch picture's key in the framework's texture store: its kind, the preset's id or name, and the pattern's pixel
/// generation or the shape's path size.
type PictureKey = (&'static str, String, u64);

/// A thumbnail in the framework's texture store: the preset's key and name, the picture's key in the store and what
/// the picture is drawn from. The texture is asked for when the swatch paints, so only the swatches a browser shows
/// load a picture, and one the store dropped (it holds `TextureCache::CAPACITY` textures) loads again.
#[derive(Clone, MemoProps)]
pub(crate) struct Swatch {
    key: String,
    name: String,
    #[memo_props(hash = "a texture store key: a kind, the preset's id or name and a number for what its picture shows")]
    picture: PictureKey,
    #[memo_props(covered_by = "picture", reason = "the picture is drawn from this art, and `picture` names what the drawing shows")]
    art: Art,
}

impl Swatch {
    /// The swatch's texture in `ctx`'s store, drawn on a miss.
    fn texture(&self, ctx: &egui::Context) -> TextureId {
        match &self.art {
            Art::Pattern(pattern) => TextureCache::texture(ctx, self.picture.clone(), || pattern_image(pattern)),
            Art::Shape(shape) => TextureCache::texture(ctx, self.picture.clone(), || shape_image(shape)),
        }
    }

    /// Paints the swatch's picture into `rect`.
    fn paint(&self, cx: &ContentPainterCx, rect: Rect) {
        paint_texture(cx, rect, self.texture(cx.painter.ctx()));
    }
}

/// The patterns and styles the preset views draw from, held while the library and the presets are unchanged, so a
/// view's inputs keep their memo keys between passes instead of being copied and keyed anew in each.
#[derive(Default)]
pub(crate) struct PresetArt {
    /// The library's patterns, with the [`patterns_stamp`] they were taken at.
    patterns: Option<(u64, HeldPatterns)>,
    /// The style library, with the presets' revision and the [`patterns_stamp`] it was taken at.
    styles: Option<(StylesStamp, Arc<Tracked<StyleLibrary>>)>,
}

/// The library's patterns as the preset views share them.
type HeldPatterns = Arc<[Arc<photocraft_doc::Pattern>]>;

/// What a held style library was taken at: the presets' revision and the [`patterns_stamp`].
type StylesStamp = (Revision, u64);

/// A number for the library's patterns and their pixels: equal stamps mean the same patterns with the same pixels
/// (`Surface::generation` names a pixel state, and is shared by clones). It also moves with the scratch read-error
/// generation: a pattern tile that fails to read draws zeros and its pixels again once it reads, under the same
/// generation (`crate::scratch_reads`), and the style thumbnails drawn over the patterns follow it through this.
fn patterns_stamp(library: &PatternLibrary) -> u64 {
    let mut hasher = DefaultHasher::new();
    crate::scratch_reads::generation().hash(&mut hasher);
    for pattern in &library.items {
        (&pattern.id, &pattern.name, pattern.surface.generation()).hash(&mut hasher);
    }
    hasher.finish()
}

/// The library's patterns as the preset views draw them: the held ones while the library is unchanged.
fn held_patterns(app: &mut PhotocraftApp) -> HeldPatterns {
    let stamp = patterns_stamp(&app.session.patterns);
    if let Some((held, patterns)) = &app.preset_art.patterns
        && *held == stamp
    {
        return Arc::clone(patterns);
    }
    let patterns: HeldPatterns = app.session.patterns.items.iter().cloned().map(Arc::new).collect();
    app.preset_art.patterns = Some((stamp, Arc::clone(&patterns)));
    patterns
}

/// The swatch of `key` among `swatches`.
fn swatch_of<'a>(swatches: &'a [Swatch], key: &str) -> Option<&'a Swatch> {
    swatches.iter().find(|swatch| swatch.key == key)
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

/// Shows `panel`'s browser over the presets of `source`, and returns its events with the keys of the rows it showed,
/// in order. `presets` is the panel state, `zone` the canvas.
fn show(
    presets: &mut PresetUi,
    zone: DropZone,
    ui: &mut egui::Ui,
    panel: PresetPanel,
    source: &PresetSource<'_, PresetKey>,
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
    let mut browser = PresetBrowser::new(tl!(panel.label), Bind::bound(&mut state), source);
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
    let out = browser.browser_id(browser_id(panel)).default_open(DefaultOpen::First).thumb_range(THUMB_RANGE).drop_zone(zone).footer(footer).show(ui);
    presets.browsers.insert(panel.id.to_string(), state);
    let keys = if out.events.is_empty() { Vec::new() } else { source.rows(ui.ctx(), &Env::of(ui)).iter().map(|row| row.key.clone()).collect() };
    (out.events, keys)
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

/// The texture store key of `pattern`'s swatch: its id and its pixels, so a pattern replaced under the same id draws
/// anew, and a swatch drawn while a tile failed to read draws again once it reads (`crate::scratch_reads`).
fn pattern_key(pattern: &photocraft_doc::Pattern) -> PictureKey {
    ("pattern", pattern.id.clone(), crate::scratch_reads::with(pattern.surface.generation()))
}

/// A texture of `pattern`'s swatch that its holder owns: it stays while the handle lives, where a cached texture
/// lasts only while something asks for it every frame.
pub(crate) fn pattern_texture_handle(ctx: &egui::Context, pattern: &photocraft_doc::Pattern) -> egui::TextureHandle {
    ctx.load_texture(format!("photocraft/pattern-swatch/{}", pattern.id), pattern_image(pattern), egui::TextureOptions::LINEAR)
}

/// The swatch of `pattern`, keyed by its id and named by its display name.
fn pattern_swatch(pattern: &Arc<photocraft_doc::Pattern>) -> Swatch {
    Swatch { key: pattern.id.clone(), name: pattern.display_name().to_string(), picture: pattern_key(pattern), art: Art::Pattern(Arc::clone(pattern)) }
}

/// Paints the whole of `texture` into `rect`.
fn paint_texture(cx: &ContentPainterCx, rect: Rect, texture: TextureId) {
    cx.picture(texture, rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), 0.0);
}

/// A pattern panel thumbnail: the preset's swatch.
fn swatch_thumb(inputs: &PanelInputs<Vec<Swatch>>, cx: &ContentPainterCx, rect: Rect, info: ThumbInfo<'_, PresetKey>) {
    if let PresetKey::Item(key) = info.key
        && let Some(swatch) = swatch_of(&inputs.thumbs, key)
    {
        swatch.paint(cx, rect);
    }
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
    let groups = &app.session.presets().gradients;
    // PLAN: the current-gradient strip above the browser needs a painter component; the browser's selected row shows it.
    let folders: Vec<Folder> = groups
        .iter()
        .map(|group| Folder { name: group.name.clone(), items: group.items.iter().map(|preset| (preset.name.clone(), preset.name.clone())).collect() })
        .collect();
    let names = names_of(&folders);
    let stops: Vec<ResolvedGradient> = groups.iter().flat_map(|group| &group.items).map(|preset| (preset.name.clone(), preset.resolve(fg, bg))).collect();
    let source = panel_source(PanelInputs::new(&app.ui.presets_ui, GRADIENTS, folders, stops), gradient_thumb);
    let zone = canvas_zone(app);
    let look = Look { cell: Some(CellShape::ratio(32, 18)), ..Look::default() };
    let (events, keys) = show(&mut app.ui.presets_ui, zone, ui, GRADIENTS, &source, look);
    apply(app, ui.ctx(), GRADIENTS, &names, &keys, events);
}

/// A gradient preset's name and its stops (offset, RGBA), resolved against the current colours.
type ResolvedGradient = (String, Vec<(f32, [f32; 4])>);

/// A gradient panel thumbnail: the preset's stops, resolved against the current colours.
fn gradient_thumb(inputs: &PanelInputs<Vec<ResolvedGradient>>, cx: &ContentPainterCx, rect: Rect, info: ThumbInfo<'_, PresetKey>) {
    if let PresetKey::Item(name) = info.key
        && let Some((_, stops)) = inputs.thumbs.iter().find(|(preset, _)| preset == name)
    {
        paint_gradient(cx, rect, stops);
    }
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
    let library = held_patterns(app);
    let folders: Vec<Folder> = app
        .session
        .presets()
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
    let names = names_of(&folders);
    let swatches: Vec<Swatch> =
        folders.iter().flat_map(|folder| &folder.items).filter_map(|(id, _)| library.iter().find(|pattern| &pattern.id == id)).map(pattern_swatch).collect();
    let source = panel_source(PanelInputs::new(&app.ui.presets_ui, PATTERNS, folders, swatches), swatch_thumb);
    let zone = canvas_zone(app);
    let (events, keys) = show(&mut app.ui.presets_ui, zone, ui, PATTERNS, &source, Look::default());
    apply(app, ui.ctx(), PATTERNS, &names, &keys, events);
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

/// What the style thumbnails draw from: every style with its picture's key, and the patterns their effects use, as the
/// app holds them when the thumbnails' presets are built.
#[derive(PlainData)]
pub(crate) struct StyleLibrary {
    /// Each style and the number its picture is keyed by in the texture store ([`style_stamp`]).
    #[plain_data(assume = "engine presets: owned values with no interior mutability")]
    styles: Vec<(u64, StylePreset)>,
    #[plain_data(assume = "the engine's pattern library: owned patterns with no interior mutability")]
    patterns: PatternLibrary,
}

/// The texture store key of `style`'s picture: everything the style holds and the pixels of the patterns its effects
/// may draw (`patterns`, a [`patterns_stamp`]), so a style or pattern edit under the same name draws anew.
fn style_stamp(style: &StylePreset, patterns: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{style:?}").hash(&mut hasher);
    patterns.hash(&mut hasher);
    hasher.finish()
}

impl StyleLibrary {
    /// The app's styles and patterns now: the held library while the presets and the patterns are unchanged, so the
    /// views keyed by it settle.
    pub(crate) fn held(app: &mut PhotocraftApp) -> Arc<Tracked<Self>> {
        let patterns = patterns_stamp(&app.session.patterns);
        let stamp = (app.session.presets().revision(), patterns);
        if let Some((held, library)) = &app.preset_art.styles
            && *held == stamp
        {
            return Arc::clone(library);
        }
        let styles =
            app.session.presets().styles.iter().flat_map(|group| group.items.iter()).map(|style| (style_stamp(style, patterns), style.clone())).collect();
        let library = Arc::new(Tracked::new(StyleLibrary { styles, patterns: app.session.patterns.clone() }));
        app.preset_art.styles = Some((stamp, Arc::clone(&library)));
        library
    }

    /// Paints the thumbnail of the style `name`. A picture is rendered on a texture miss only, under one key for
    /// every place that shows the style, so they share the textures.
    pub(crate) fn paint(&self, cx: &ContentPainterCx, rect: Rect, name: &str) {
        let Some((stamp, style)) = self.styles.iter().find(|(_, style)| style.name == name) else { return };
        picture(cx, rect, ("style", *stamp), || {
            let mut scratch = photocraft_engine::Session::new();
            scratch.patterns = self.patterns.clone();
            style_image(&scratch, style)
        });
    }
}

/// A Styles panel thumbnail.
fn style_thumb(inputs: &PanelInputs<Arc<Tracked<StyleLibrary>>>, cx: &ContentPainterCx, rect: Rect, info: ThumbInfo<'_, PresetKey>) {
    if let PresetKey::Item(name) = info.key {
        inputs.thumbs.paint(cx, rect, name);
    }
}

/// Window › Styles: a click applies the style to the selected layers (Shift adds its effects), a drop on the canvas
/// applies it to the active layer.
pub fn styles_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let folders = style_folders(app);
    let names = names_of(&folders);
    let library = StyleLibrary::held(app);
    let source = panel_source(PanelInputs::new(&app.ui.presets_ui, STYLES, folders, library), style_thumb);
    let zone = canvas_zone(app);
    let look = Look { room_height: Some(STYLES_HEIGHT), ..Look::default() };
    let (events, keys) = show(&mut app.ui.presets_ui, zone, ui, STYLES, &source, look);
    apply(app, ui.ctx(), STYLES, &names, &keys, events);
}

/// The Styles folders: each style group with its styles, keyed and named by the style's name.
fn style_folders(app: &PhotocraftApp) -> Vec<Folder> {
    let groups = &app.session.presets().styles;
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
    let any = app.session.presets().styles.iter().any(|group| !group.items.is_empty());
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

/// [`styles_states`], derived from what it reads: the presets the styles live in, the selected style and the browser's
/// view.
pub(crate) fn styles_source_states(app: &PhotocraftApp, name: &'static str) -> std::sync::Arc<crate::app::framework::SourceStates> {
    let view = app.ui.presets_ui.browsers.get(STYLES.id).map_or(PresetView::Grid, |state| state.view);
    let facts = (app.session.presets().revision(), app.ui.presets_ui.selected.get(STYLES.id), view);
    crate::app::framework::derived_states(app, name, &facts, || styles_states(app))
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
fn shape_key(shape: &ShapePreset) -> PictureKey {
    let knots: usize = shape.path.subpaths.iter().map(|s| s.knots.len()).sum();
    ("shape", shape.name.clone(), u64::try_from(knots).unwrap_or(u64::MAX))
}

/// The swatch of `shape`, keyed and named by its name.
fn shape_swatch(shape: ShapePreset) -> Swatch {
    Swatch { key: shape.name.clone(), name: shape.name.clone(), picture: shape_key(&shape), art: Art::Shape(Arc::new(shape)) }
}

/// A Shapes panel thumbnail: the shape's raster, inset from the tile's edge.
fn shape_thumb(inputs: &PanelInputs<Vec<Swatch>>, cx: &ContentPainterCx, rect: Rect, info: ThumbInfo<'_, PresetKey>) {
    if let PresetKey::Item(key) = info.key
        && let Some(swatch) = swatch_of(&inputs.thumbs, key)
    {
        swatch.paint(cx, rect.shrink(2.0));
    }
}

/// Window › Shapes: a click picks the shape for the Custom Shape tool, a double click places it, and a drop on the
/// canvas places it where it lands.
pub fn shapes_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let groups = photocraft_engine::presets::shapes::all_groups(&app.session);
    let folders: Vec<Folder> = groups
        .iter()
        .map(|group| Folder { name: group.name.clone(), items: group.items.iter().map(|shape| (shape.name.clone(), shape.name.clone())).collect() })
        .collect();
    let names = names_of(&folders);
    let swatches: Vec<Swatch> = groups.into_iter().flat_map(|group| group.items).map(shape_swatch).collect();
    let source = panel_source(PanelInputs::new(&app.ui.presets_ui, SHAPES, folders, swatches), shape_thumb);
    let zone = canvas_zone(app);
    let (events, keys) = show(&mut app.ui.presets_ui, zone, ui, SHAPES, &source, Look::default());
    apply(app, ui.ctx(), SHAPES, &names, &keys, events);
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
    const ID: &str = "shapePicker";
    let groups: Vec<(String, Vec<Swatch>)> = photocraft_engine::presets::shapes::all_groups(&app.session)
        .into_iter()
        .map(|group| (group.name, group.items.into_iter().map(shape_swatch).collect()))
        .collect();
    let source = PresetSource::new(groups, PresetFns::new(shape_rows));
    let mut value = Some(app.ui.presets_ui.shape().to_string());
    let before = value.clone();
    let mut state = picker_state(&app.ui.presets_ui, ID);
    let _ = PresetPicker::new(tl!("Shape"), Bind::bound(&mut value), &source, Bind::bound(&mut state)).columns(7).show(ui); // the picked value comes back through `value`
    keep_picker_state(&mut app.ui.presets_ui, ID, state);
    if let Some(picked) = value.filter(|picked| before.as_ref() != Some(picked) && !picked.starts_with(SHAPE_GROUP)) {
        app.ui.presets_ui.custom_shape = picked;
    }
}

/// The key prefix of a shape group's row in the shape picker.
const SHAPE_GROUP: &str = "group:";

/// The shape picker's rows: each group, then its shapes with their rasters, named in their tooltips.
fn shape_rows(groups: &Vec<(String, Vec<Swatch>)>, _: &Env) -> Vec<PresetRow<String>> {
    let mut rows = Vec::new();
    for (group, shapes) in groups {
        rows.push(PresetRow::group(format!("{SHAPE_GROUP}{group}"), group.clone(), 0));
        for shape in shapes {
            let swatch = shape.clone();
            rows.push(PresetRow::item(shape.key.clone(), shape.name.clone(), 1).tooltip(shape.name.clone()).preview(ThumbAspect::SQUARE, move |cx, rect| {
                swatch.paint(cx, rect.shrink(2.0));
            }));
        }
    }
    rows
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
    let presets: Vec<(String, String)> = app
        .session
        .presets()
        .tool_presets
        .iter()
        .filter(|preset| !only || Tool::from_name(&preset.tool) == Some(current))
        .map(|preset| (preset.name.clone(), preset.tool.clone()))
        .collect();
    let source = PresetSource::new(presets.as_slice(), PresetFns::new(tool_preset_rows));
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
    let (events, keys) = show(&mut app.ui.presets_ui, zone, ui, TOOL_PRESETS, &source, look);
    app.ui.presets_ui.current_tool_only = flag.get();
    apply(app, ui.ctx(), TOOL_PRESETS, &[], &keys, events);
}

/// The Tool Presets rows: each preset, as (name, tool name), with its tool's icon.
fn tool_preset_rows(presets: &&[(String, String)], _: &Env) -> Vec<PresetRow<PresetKey>> {
    presets
        .iter()
        .map(|(name, tool)| {
            let icon = Tool::from_name(tool).map_or_else(|| craft::behavior::IconId::new("settings"), crate::icons::tool_icon);
            PresetRow::item(PresetKey::Item(name.clone()), name.clone(), 0).icon(icon)
        })
        .collect()
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

/// The library's patterns as swatches, in library order. The pattern picker and the dialogs' pattern lists show these
/// through [`pattern_source`].
pub(crate) fn pattern_swatches(app: &mut PhotocraftApp) -> Vec<Swatch> {
    held_patterns(app).iter().map(pattern_swatch).collect()
}

/// The presets of `swatches`: one row per pattern, keyed by its id, with its swatch and its name as the tooltip.
pub(crate) fn pattern_source(swatches: Vec<Swatch>) -> PresetSource<'static, String> {
    PresetSource::new(swatches, PresetFns::new(|swatches, env| swatch_rows(swatches, env)))
}

/// The rows of [`pattern_source`].
fn swatch_rows(swatches: &[Swatch], _: &Env) -> Vec<PresetRow<String>> {
    swatches
        .iter()
        .map(|swatch| {
            let picture = swatch.clone();
            PresetRow::item(swatch.key.clone(), swatch.name.clone(), 0)
                .tooltip(swatch.name.clone())
                .preview(ThumbAspect::SQUARE, move |cx, rect| picture.paint(cx, rect))
        })
        .collect()
}

/// Photoshop's pattern picker: a swatch of the library pattern `selected` (an id) that opens the library's swatches,
/// each named in its tooltip. Returns the id of a newly picked pattern.
pub(crate) fn pattern_picker(app: &mut PhotocraftApp, ui: &mut egui::Ui, selected: &str) -> Option<String> {
    const ID: &str = "patternPicker";
    let source = pattern_source(pattern_swatches(app));
    let mut value = Some(selected.to_string()).filter(|id| app.session.patterns.items.iter().any(|pattern| &pattern.id == id));
    let before = value.clone();
    let mut state = picker_state(&app.ui.presets_ui, ID);
    let _ = PresetPicker::new(tl!("Patterns"), Bind::bound(&mut value), &source, Bind::bound(&mut state)).show(ui);
    keep_picker_state(&mut app.ui.presets_ui, ID, state);
    value.filter(|picked| before.as_ref() != Some(picked))
}

#[cfg(test)]
mod tests;
