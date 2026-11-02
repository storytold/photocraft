use std::collections::HashSet;

use super::defaults::apply;
use super::{Arc, HashMap, PhotocraftApp, PoisonError, json};

/// Font families: bundled, system, and those the host serves (`photocraft_text::served`, fetched
/// when picked). Cached until the shared engine registers fonts or the served catalog changes.
pub fn families() -> Arc<Vec<String>> {
    type Cached = Option<((u64, u64), Arc<Vec<String>>)>;
    static FAMILIES: std::sync::Mutex<Cached> = std::sync::Mutex::new(None);
    let key = (photocraft_text::fonts::shared_generation(), photocraft_text::served::generation());
    let mut cached = FAMILIES.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((k, v)) = cached.as_ref()
        && *k == key
    {
        return v.clone();
    }
    // Never wait for the background font scan (#3021): until it finishes the menus draw their
    // loading box (`fonts_ready`), so the empty list is not shown. The fallback is not cached
    // either: `mark_shared` moves the generation before the scan thread publishes the engine, so
    // that frame's key would otherwise keep the empty list for good.
    let engine = menu_engine();
    let mut v = engine.as_ref().map(|e| e.lock().map(|mut e| e.fonts.families()).unwrap_or_default()).unwrap_or_default();
    for f in photocraft_text::served::families() {
        if !v.contains(&f) {
            v.push(f);
        }
    }
    v.sort_by_key(|s| s.to_lowercase());
    let v = Arc::new(v);
    if engine.is_some() {
        *cached = Some((key, v.clone()));
    }
    v
}

/// Where [`sync_font_names`] leaves the font menus' names and grouping for this frame.
pub(super) fn font_menu_id() -> egui::Id {
    egui::Id::unique("photocraft-font-menu")
}

/// The font menu's names and grouping for the interface language (see [`sync_font_names`]).
#[derive(Default)]
pub(crate) struct FontMenuModel {
    /// The label a family shows: its own name in the interface's writing system when the
    /// preference lists names in their own language, else the English family name.
    pub labels: HashMap<String, String>,
    /// The label a family with a single face shows, "family + style" ("Aa厚底黑 常规",
    /// "SimSun-ExtB Regular"), as Photoshop lists such a family.
    pub single: HashMap<String, String>,
    /// Every name a family gives itself, so a search finds "未来荧黑" by "未来" or "Glow".
    pub names: HashMap<String, Vec<String>>,
    /// The families of the interface's writing system, listed above the rest as in Photoshop.
    pub native: HashSet<String>,
    /// The writing system the menu groups and sorts by. `None` in an English (or other non-CJK)
    /// interface, which has no group of its own to head the list (its names still sort as shown).
    pub script: Option<photocraft_text::cjk::CjkScript>,
}

impl FontMenuModel {
    /// The label `family` shows.
    pub(super) fn label<'a>(&'a self, family: &'a str) -> &'a str {
        self.single.get(family).or_else(|| self.labels.get(family)).map_or(family, String::as_str)
    }

    /// Does the family match `query` (lowercase) by its English name, its label or any of its
    /// own names?
    fn matches(&self, family: &str, label: &str, query: &str) -> bool {
        family.to_lowercase().contains(query)
            || label.to_lowercase().contains(query)
            || self.names.get(family).is_some_and(|names| names.iter().any(|name| name.to_lowercase().contains(query)))
    }

    /// Is `family` one of the interface language's own fonts?
    fn is_native(&self, family: &str) -> bool {
        self.script.is_some() && self.native.contains(family)
    }
}

/// Preferences ▸ Type ▸ Show Font Names in English and the interface language: works out the font
/// menus' names and grouping once a frame, for the menus to read from the context. A Chinese,
/// Japanese or Korean interface lists its own fonts first, in the writing system's own order
/// (pinyin for Chinese, as Photoshop's menu does); an English interface with the preference on
/// lists English names alphabetically, as it always has.
pub fn sync_font_names(ctx: &egui::Context, show_in_english: bool) {
    let model = font_menu_model(show_in_english);
    ctx.data_mut(|d| match model {
        Some(model) => {
            d.insert_temp(font_menu_id(), model);
        }
        None => {
            d.remove::<Arc<FontMenuModel>>(font_menu_id());
        }
    });
}

/// The menu model for the interface language and preference, cached until the registered fonts
/// change. Every interface gets one, so a search finds a font by any name it gives itself in any
/// language; an interface that shows English names keeps its labels and order all the same (no
/// row is relabelled, so `font_menu_rows` neither sorts nor groups).
fn font_menu_model(show_in_english: bool) -> Option<Arc<FontMenuModel>> {
    let script = ui_script();
    type Cached = Option<((u64, u64, u8, bool), Arc<FontMenuModel>)>;
    static CACHE: std::sync::Mutex<Cached> = std::sync::Mutex::new(None);
    let key =
        (photocraft_text::fonts::shared_generation(), photocraft_text::served::generation(), script_code(script), !show_in_english);
    let mut cached = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((k, v)) = cached.as_ref()
        && *k == key
    {
        return Some(v.clone());
    }
    // The index is built on the font-scan thread (`photocraft_text::shared`): this only copies it
    // and never waits for the scan.
    let engine = menu_engine()?;
    let index = engine.lock().map(|mut e| e.fonts.menu_index()).unwrap_or_default();
    let mut model = FontMenuModel { script, ..FontMenuModel::default() };
    // Names in their own language are shown in a Chinese, Japanese or Korean interface (whatever
    // the preference) and wherever the preference turns English names off; an English interface
    // with English names on keeps the family names it always showed.
    let named = !show_in_english || script.is_some();
    for (family, info) in &index {
        if named
            && !show_in_english
            && let Some(label) = photocraft_text::font_menu::display_name(family, &info.names, script)
        {
            model.labels.insert(family.clone(), label);
        }
        if script.is_some_and(|script| info.scripts.has(script)) {
            model.native.insert(family.clone());
        }
        // A family of one face reads "family + style" ("Aa厚底黑 常规"), as Photoshop lists it;
        // several faces get a style menu under the family name instead.
        if named
            && info.faces == 1
            && let Some(label) = photocraft_text::font_menu::single_face_label(family, info, script, show_in_english)
        {
            model.single.insert(family.clone(), label);
        }
        if !info.search_names.is_empty() {
            model.names.insert(family.clone(), info.search_names.clone());
        }
    }
    // The web build's served fonts are listed before their files are fetched: the manifest's
    // scripts put them in the interface language's group already.
    if let Some(script) = script {
        for family in photocraft_text::served::families_for_script(script) {
            if !index.contains_key(&family) {
                model.native.insert(family);
            }
        }
    }
    let model = Arc::new(model);
    *cached = Some((key, model.clone()));
    Some(model)
}

/// The shared font engine the menus read: [`photocraft_text::try_shared`], which never waits for
/// the background font scan (#3021). A test waits for the scan instead, so the list it draws does
/// not depend on its timing (`Harness` step counts included).
fn menu_engine() -> Option<&'static std::sync::Mutex<photocraft_text::TextEngine>> {
    if cfg!(test) { Some(photocraft_text::shared()) } else { photocraft_text::try_shared() }
}

/// A cache key for a writing system, and for none.
fn script_code(script: Option<photocraft_text::cjk::CjkScript>) -> u8 {
    use photocraft_text::cjk::CjkScript::*;
    match script {
        Some(SimplifiedChinese) => 0,
        Some(TraditionalChinese) => 1,
        Some(Japanese) => 2,
        Some(Korean) => 3,
        None => 4,
    }
}

/// The font menu's rows: each family with the label it shows, matching `query` (lowercase) by any
/// of its names. In a Chinese, Japanese or Korean interface the interface's own fonts head the
/// list, in the writing system's own order (Photoshop's order); the rest follow. As in Photoshop,
/// recently used families also head the menu in their own group, most recent first (Preferences ▸
/// Type ▸ Recent Fonts), and keep their place below it; a query shows each match once.
///
/// Returns the rows, how many of them form the recent group, and how many of the rows after it
/// form the interface language's group.
pub(super) fn font_menu_rows<'a>(
    families: &'a [String],
    model: Option<&'a FontMenuModel>,
    query: &str,
    recents: &[String],
) -> (Vec<(&'a String, &'a str)>, usize, usize) {
    let mut rows: Vec<(&String, &str)> = families
        .iter()
        .map(|f| (f, model.map_or(f.as_str(), |m| m.label(f))))
        .filter(|(f, label)| query.is_empty() || model.map_or_else(|| f.to_lowercase().contains(query), |m| m.matches(f, label, query)))
        .collect();
    match model {
        // The interface's own fonts first, each group in the writing system's order, with case
        // and full-width forms folded: pinyin for Chinese ("阿里巴巴" before "阿里健康"), stroke
        // order for Traditional Chinese, gojūon for Japanese.
        Some(m) if m.script.is_some() => {
            rows.sort_by_cached_key(|(f, label)| (!m.is_native(f), photocraft_text::collate::menu_sort_key(label, m.script), f.to_lowercase()));
        }
        // Names in their own language in an English interface: sorted as shown, as before.
        Some(m) if !m.labels.is_empty() || !m.single.is_empty() => rows.sort_by_cached_key(|(_, label)| label.to_lowercase()),
        _ => {}
    }
    let native = rows.iter().filter(|(f, _)| model.is_some_and(|m| m.is_native(f))).count();
    if !query.is_empty() {
        return (rows, 0, native);
    }
    let recent: Vec<(&String, &str)> = recents.iter().filter_map(|r| rows.iter().find(|(f, _)| *f == r).copied()).collect();
    let n = recent.len();
    (recent.into_iter().chain(rows).collect(), n, native)
}

fn weight_name(w: f32) -> &'static str {
    match w.round() as i32 {
        ..=150 => "Thin",
        151..=250 => "ExtraLight",
        251..=350 => "Light",
        351..=450 => "Regular",
        451..=550 => "Medium",
        551..=650 => "SemiBold",
        651..=750 => "Bold",
        751..=850 => "ExtraBold",
        _ => "Black",
    }
}

/// Style names available for a family: each face's actual OpenType subfamily (including numeric
/// and nonstandard ones), plus the standard weights a variable face's `wght` axis covers.
pub fn styles(family: &str) -> Vec<String> {
    let faces = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner()).fonts.faces(family);
    let v = style_names(&faces);
    if v.is_empty() { vec!["Regular".into()] } else { v }
}

/// [`styles`] of these faces, lightest first. A variable face reports only its default instance
/// (Montserrat's is Thin), yet the layout draws any weight its `wght` axis covers, so the standard
/// weights in that range are listed too.
pub(super) fn style_names(faces: &[photocraft_text::FaceInfo]) -> Vec<String> {
    let label = |weight: f32, italic: bool| {
        let name = match (weight_name(weight), italic) {
            ("Regular", true) => "Italic".to_string(),
            (w, true) => format!("{w} Italic"),
            (w, false) => w.to_string(),
        };
        (weight.round() as i32, italic, name)
    };
    let mut v: Vec<(i32, bool, String)> = faces.iter().map(|f| (f.weight.round() as i32, f.italic, f.style.clone())).collect();
    for f in faces {
        if let Some((_, min, _, max)) = f.axes.iter().find(|a| a.0 == "wght") {
            v.extend((100..=900).step_by(100).map(|w| w as f32).filter(|w| (*min..=*max).contains(w)).map(|w| label(w, f.italic)));
        }
    }
    v.sort();
    let mut names: Vec<String> = Vec::new();
    for (_, _, name) in v {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// A variable-font axis: (tag, min, default, max), as [`photocraft_text::FaceInfo::axes`].
pub(crate) type Axis = (String, f32, f32, f32);

/// The axes the Character section shows as sliders (#2985): those of the faces matching the
/// style's italic, else of any face. Optical size has none: it follows the type size.
pub(crate) fn slider_axes(faces: &[photocraft_text::FaceInfo], italic: bool) -> Vec<Axis> {
    let pick = |want: Option<bool>| {
        let mut v: Vec<Axis> = Vec::new();
        for f in faces.iter().filter(|f| want.is_none_or(|w| f.italic == w)) {
            for a in &f.axes {
                let usable = a.0 != "opsz" && a.1.is_finite() && a.3.is_finite() && a.1 < a.3;
                if usable && !v.iter().any(|b| b.0 == a.0) {
                    v.push(a.clone());
                }
            }
        }
        v
    };
    let v = pick(Some(italic));
    if v.is_empty() { pick(None) } else { v }
}

/// The value an axis slider shows: the style's own value for the axis, else what the layout
/// draws (the style's weight on `wght`, the axis default otherwise), within the axis range.
pub(crate) fn axis_value(c: &photocraft_doc::text::CharStyle, (tag, min, default, max): &Axis) -> f32 {
    let v = c.variations.iter().find(|v| &v.axis == tag).map(|v| v.value).unwrap_or_else(|| match tag.as_str() {
        "wght" => f32::from(c.weight),
        "ital" if c.italic => *max,
        _ => *default,
    });
    let d = default.clamp(*min, *max);
    if v.is_finite() { v.clamp(*min, *max) } else { d }
}

/// Slider label for an axis: the registered axes by name, others by their tag (as the font
/// names them in its `fvar` table, e.g. `GRAD`).
fn axis_label(tag: &str) -> String {
    match tag {
        "wght" => tl!("Weight").to_string(),
        "wdth" => tl!("Width").to_string(),
        "slnt" => tl!("Slant").to_string(),
        "ital" => tl!("Italic").to_string(),
        _ => tag.to_string(),
    }
}

/// One slider row per variable-font axis of the style's font, below the font style (#2985).
pub(super) fn axis_sliders(app: &mut PhotocraftApp, ui: &mut egui::Ui, c: &photocraft_doc::text::CharStyle) {
    // Drawn every frame: no sliders until the background font scan is done (#3021).
    let Some(engine) = photocraft_text::try_shared() else { return };
    let axes = {
        let faces = engine.lock().unwrap_or_else(PoisonError::into_inner).fonts.faces(&c.font_family);
        slider_axes(&faces, c.italic)
    };
    let t = crate::theme::Tokens::get(ui.ctx());
    for axis in &axes {
        let (min, max) = (axis.1, axis.3);
        let mut v = axis_value(c, axis);
        let mut changed = false;
        ui.push_id(("type-axis", &axis.0), |ui| {
            ui.horizontal(|ui| {
                let axis_name = axis_label(&axis.0);
                ui.label(egui::RichText::new(&axis_name).color(t.text_dim));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    changed |= crate::widgets::value_field(ui, &mut v, min..=max, "", 74.0).accessible_name(axis_name).changed();
                });
            });
            changed |= crate::widgets::slider(ui, &mut v, min..=max, None).changed();
        });
        ui.add_space(crate::theme::ROW_GAP);
        if changed {
            // Whole units on wide axes (weight 100–900), hundredths on narrow ones (ital 0–1).
            let v = if max - min >= 10.0 { v.round() } else { (v * 100.0).round() / 100.0 };
            let mut m = serde_json::Map::new();
            m.insert(axis.0.clone(), json!(v));
            apply(app, ui.ctx(), json!({ "variations": m }));
        }
    }
}

/// PSD text can identify a face only by its PostScript name. Show its real subfamily too.
pub(crate) fn selected_style(style: &photocraft_doc::text::CharStyle) -> String {
    // Drawn every frame: while the font scan runs, the stored style name stands in.
    if let Some(ps) = &style.postscript_name
        && let Some(engine) = photocraft_text::try_shared()
    {
        let faces = engine.lock().unwrap_or_else(|e| e.into_inner()).fonts.faces(&style.font_family);
        if let Some(face) = faces.into_iter().find(|f| f.postscript_name.as_ref() == Some(ps)) {
            return face.style;
        }
    }
    if style.font_style.is_empty() { if style.italic { "Italic".into() } else { "Regular".into() } } else { style.font_style.clone() }
}

/// The styles of a family with the label each shows in the menus: the subfamily the font writes
/// in the interface's language where it has one ("常规"), else its own name as written ("Wide
/// ExtraLight"). Never a word-by-word translation, which mixed languages ("Wide 极细"). The
/// first of each pair is the style name `fontStyle` keeps.
pub fn style_options(family: &str) -> Vec<(String, String)> {
    let faces = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner()).fonts.faces(family);
    let mut options = style_labels(style_names(&faces), &faces, ui_script());
    if options.is_empty() {
        options.push(("Regular".into(), "Regular".into()));
    }
    options
}

/// Each style's label: the subfamily the font writes in `prefer` ("常规"), else the style as it
/// is written ("Wide ExtraLight"). Never a word-by-word translation, which mixed languages
/// ("Wide 极细"); the style name itself is the key `fontStyle` keeps.
pub(super) fn style_labels(
    styles: Vec<String>,
    faces: &[photocraft_text::FaceInfo],
    prefer: Option<photocraft_text::cjk::CjkScript>,
) -> Vec<(String, String)> {
    styles
        .into_iter()
        .map(|style| {
            // Only a Chinese, Japanese or Korean interface localizes a style; the others show the
            // name as the font writes it, whatever language that is in.
            let label = prefer
                .and_then(|script| {
                    faces
                        .iter()
                        .find(|face| face.style == style)
                        .and_then(|face| photocraft_text::font_menu::name_for_script(&face.localized_styles, script))
                })
                .unwrap_or_else(|| style.clone());
            (style, label)
        })
        .collect()
}

/// The interface language's writing system, which picks how a font or a style is named (`None`
/// outside a Chinese, Japanese or Korean interface).
pub(crate) fn ui_script() -> Option<photocraft_text::cjk::CjkScript> {
    photocraft_text::cjk::locale_script(Some(crate::i18n::current().code()))
}

/// Starts scanning the installed fonts on a background thread (desktop start-up), so choosing
/// the Type tool doesn't stall the UI while they are read (#3021). Does nothing on the web.
pub fn warm_fonts() {
    photocraft_text::warm_shared();
}

/// Whether the font database is ready. The installed fonts are scanned on a background thread
/// from launch (`photocraft_text::warm_shared`); until that finishes, font menus draw this
/// disabled "Loading fonts…" box `width` wide instead of blocking the UI, and a repaint picks the
/// menus up once the scan is done.
pub(crate) fn fonts_ready(ui: &mut egui::Ui, width: f32) -> bool {
    if photocraft_text::try_shared().is_some() {
        return true;
    }
    let label = tl!("Loading fonts…").to_string();
    ui.add_enabled_ui(false, |ui| crate::widgets::dropdown(ui, "fonts-loading", tl!("Font"), &mut label.clone(), &[(label.clone(), label.as_str())], width));
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    false
}

/// The font menu's local library (`photocraft_engine::font_library_cmds`): the recent fonts it
/// lists first and the favourites it stars, read before the menu is drawn, plus the font whose
/// star was clicked, starred or un-starred by [`FontLibrary::finish`] afterwards, and the style
/// picked from a family's expanded styles.
#[derive(Clone, Debug, Default)]
pub(crate) struct FontLibrary {
    pub recent: Vec<String>,
    pub favorites: Vec<String>,
    pub toggled: Option<String>,
    /// The style picked under a family's ">" (with the family), for the caller to apply.
    pub picked_style: Option<String>,
    /// Family traits to filter by instead of the installed fonts' (tests).
    pub traits: Option<Arc<crate::font_filters::TraitsMap>>,
    /// Each family's style names instead of the installed fonts' (tests).
    pub styles: Option<Arc<HashMap<String, Vec<String>>>>,
    /// The user's own text for the preview at the top of the menu (else it shows a sample).
    pub preview_text: Option<String>,
}

/// The text the font menu's preview sets: the selected characters while typing, else the type
/// target's whole text, on one line and at most 60 characters. `None` without a type target or
/// with blank text (the preview then shows its sample).
pub(super) fn preview_text(app: &PhotocraftApp) -> Option<String> {
    let (layer, range) = defaults::target(app)?;
    let text = &crate::type_tool::text_layer(&app.session.active()?.doc, crate::type_tool::LayerId(layer))?.text;
    use crate::type_tool::defaults;

    let chosen: String = match range {
        Some([a, b]) => text.chars().skip(a).take(b.saturating_sub(a)).collect(),
        None => text.clone(),
    };
    let line = chosen.split(['\n', '\r', '\u{2028}', '\u{2029}']).map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    let line: String = line.chars().take(60).collect();
    (!line.is_empty()).then_some(line)
}

impl FontLibrary {
    pub(crate) fn of(app: &PhotocraftApp) -> Self {
        use photocraft_engine::font_library_cmds as lib;
        // The recent fonts are the shell's (`PhotocraftApp::note_recent_font`), the favourites the engine's.
        Self { recent: app.ui.recent_fonts.clone(), favorites: lib::favorite_fonts(&app.session), ..Default::default() }
    }

    /// The styles a family's ">" expands to, with the label each shows.
    fn style_options_of(&self, family: &str) -> Vec<(String, String)> {
        match &self.styles {
            // Tests hand the plain names: they label themselves.
            Some(map) => map.get(family).cloned().unwrap_or_else(|| vec!["Regular".into()]).into_iter().map(|s| (s.clone(), s)).collect(),
            None => style_options(family),
        }
    }

    /// Star or un-star the clicked font, through the engine command.
    pub(crate) fn finish(self, app: &mut PhotocraftApp) {
        if let Some(family) = self.toggled
            && let Err(e) = app.run("type.font.favorite", json!({"family": family}))
        {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// Searchable font-family combo box, with the recent and favourite fonts of `app`. `Some` when a
/// family was picked, holding the style when it was picked from the family's expanded styles.
pub(super) fn font_picker(app: &mut PhotocraftApp, ui: &mut egui::Ui, current: &mut String, width: f32) -> Option<Option<String>> {
    let preview = app.session.prefs().type_.font_preview;
    let mut library = FontLibrary { preview_text: preview_text(app), ..FontLibrary::of(app) };
    let changed = family_picker_in(ui, "type-font", current, width, &families(), preview, &mut library);
    let style = library.picked_style.take();
    library.finish(app);
    if !changed {
        return None;
    }
    // Each family applied is recorded, an arrow step too, as `PhotocraftApp::note_recent_font` says.
    app.note_recent_font(current);
    Some(style)
}

/// The font list's maximum height: it scrolls inside the menu, under the fixed preview and filters.
pub(super) const FONT_LIST_HEIGHT: f32 = 440.0;
/// The font list's width: star, style expander, name, format badge and sample.
const FONT_LIST_WIDTH: f32 = 420.0;
/// Room right of the font list's viewport for its scroll bar.
const FONT_SCROLL_BAR_WIDTH: f32 = 14.0;
/// Room for the format badge (O, TT, VAR) left of the sample.
const FONT_BADGE_WIDTH: f32 = 28.0;

/// [`font_picker`] over a given family list (tests pass their own).
#[cfg(test)]
pub(super) fn font_picker_in(
    ui: &mut egui::Ui,
    current: &mut String,
    width: f32,
    families: &[String],
    preview: photocraft_engine::prefs::FontPreview,
    recents: &[String],
) -> bool {
    family_picker_in(ui, "type-font", current, width, families, preview, &mut FontLibrary { recent: recents.to_vec(), ..FontLibrary::default() })
}

/// The row Enter and ⌘/Ctrl+Enter apply to a pending family query: a full name wins over a
/// substring match, whether it is the family's English name, the label the row shows, or another
/// name its font gives itself ("未来荧黑" finds Glow Sans SC in an English menu); then the
/// highlighted row, then the first.
pub(super) fn accept_row(rows: &[(&String, &str)], text: &str, selected: Option<usize>, model: Option<&FontMenuModel>) -> Option<usize> {
    let exact = text.trim().to_lowercase();
    let is_exact = |(family, shown): &(&String, &str)| {
        family.to_lowercase() == exact
            || shown.to_lowercase() == exact
            || model.is_some_and(|m| m.names.get(family.as_str()).is_some_and(|names| names.iter().any(|name| name.to_lowercase() == exact)))
    };
    rows.iter().position(is_exact).or(selected).or_else(|| (!rows.is_empty()).then_some(0))
}

/// Transient input belongs to one field, independently of the applied family and its peers.
#[derive(Clone)]
pub(super) struct FontFieldEdit {
    pub(super) text: String,
    pub(super) query: String,
    pub(super) current: String,
    pub(super) recents: Vec<String>,
    pub(super) row: Option<usize>,
    pub(super) opening: bool,
    pub(super) accepted: bool,
    pub(super) composing: bool,
    pub(super) menu_height: Option<f32>,
    /// The current family still has to be scrolled into view after the menu opened.
    pub(super) reveal: bool,
}

/// Shared by the Type options, Character, style and Glyphs panels. Selection changes still
/// flow through each caller's existing engine command.
pub(crate) fn family_picker_in(
    ui: &mut egui::Ui,
    salt: &str,
    current: &mut String,
    width: f32,
    families: &[String],
    preview: photocraft_engine::prefs::FontPreview,
    library: &mut FontLibrary,
) -> bool {
    let field = ui.make_persistent_id(egui::IdSalt::new(salt));
    let popup = field.with("popup");
    let edit_id = field.with("edit");
    let was_open = egui::Popup::is_id_open(ui.ctx(), popup);
    let had_focus = ui.memory(|m| m.has_focus(field));
    let model: Option<Arc<FontMenuModel>> = ui.ctx().data(|d| d.get_temp(font_menu_id()));
    let model = model.as_deref();
    let label = |f: &str| model.map_or_else(|| f.to_string(), |m| m.label(f).to_string());
    let mut edit =
        ui.data(|d| d.get_temp::<FontFieldEdit>(edit_id)).filter(|e| e.current == *current && (was_open || had_focus)).unwrap_or_else(|| FontFieldEdit {
            text: label(current),
            query: String::new(),
            current: current.clone(),
            recents: library.recent.clone(),
            row: None,
            opening: false,
            accepted: false,
            composing: false,
            menu_height: None,
            reveal: false,
        });
    // Take navigation before TextEdit interprets it, and before the canvas can see it.
    let owns_keys = ui.is_enabled() && (had_focus || was_open);
    if had_focus {
        ui.memory_mut(|m| m.set_focus_lock_filter(field, egui::EventFilter { vertical_arrows: true, ..Default::default() }));
    }
    // IME confirmation/navigation belongs to composition, even when its current spelling
    // happens to be an exact font name. A separate Enter accepts the completed name.
    let composing = edit.composing || ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Ime(_))));
    if owns_keys {
        ui.input(|i| {
            for event in &i.events {
                match event {
                    egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => edit.composing = !text.is_empty(),
                    egui::Event::Ime(egui::ImeEvent::Commit(_)) => edit.composing = false,
                    _ => {}
                }
            }
        });
    }
    let key = |k| owns_keys && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, k));
    let navigation_down = key(egui::Key::ArrowDown);
    let navigation_up = key(egui::Key::ArrowUp);
    let down = navigation_down && !composing;
    let up = navigation_up && !composing;
    let enter = key(egui::Key::Enter) && !composing;
    let escape = key(egui::Key::Escape) && !composing;
    let tab = owns_keys && ui.input(|i| i.key_pressed(egui::Key::Tab));
    if navigation_down || navigation_up {
        ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
    }
    if edit.opening {
        ui.memory_mut(|m| m.request_focus(field));
    }
    let (input, rect) = crate::widgets::text_field_with_trailing(ui, field, &mut edit.text, tl!("Font"), width, crate::widgets::POPUP_ARROW_W);
    let arrow_rect =
        egui::Rect::from_min_max(egui::pos2(rect.right() - crate::widgets::POPUP_ARROW_W - 2.0, rect.top() + 2.0), rect.max - egui::vec2(2.0, 2.0));
    let arrow = ui.interact(arrow_rect, field.with("arrow"), egui::Sense::click());
    arrow.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tl!("Font")));
    crate::widgets::chevron_icon(ui, arrow_rect, ui.style().interact(&arrow), was_open);
    // The arrow owns press-drag-release; dragging in the name remains normal text selection.
    // Its menu is anchored to the full control and keeps the existing stable popup id.
    let mut anchor = arrow.clone();
    anchor.id = field;
    anchor.rect = rect;
    anchor.interact_rect = rect;
    if arrow.clicked() && !ui.input(|i| i.pointer.any_click()) {
        egui::Popup::toggle_id(ui.ctx(), popup);
    }
    let mut select_all = input.gained_focus() || (!was_open && input.clicked());
    if (input.gained_focus() && !edit.accepted) || (!was_open && input.clicked()) || input.changed() {
        egui::Popup::open_id(ui.ctx(), popup);
    }
    if input.changed() {
        edit.query = edit.text.to_lowercase();
        edit.row = None;
        edit.accepted = false;
        select_all = false;
    }
    if escape || tab {
        egui::Popup::close_id(ui.ctx(), popup);
        if escape {
            ui.memory_mut(|m| m.surrender_focus(field));
        }
        edit.opening = false;
    }
    let mut changed = false;
    let mut picked = false;
    // The name is outside the popup: clicking it to move the caret keeps the list open.
    let close = if input.contains_pointer() { egui::PopupCloseBehavior::IgnoreClicks } else { egui::PopupCloseBehavior::CloseOnClickOutside };
    let response = crate::press_menu::show_with_close_behavior(&anchor, close, |ui, press| {
        // Positions kept between frames are relative to the menu's left edge: the menu moves
        // (the window's edge pushes it after a resize), and screen positions from an earlier
        // frame would widen the menu, an empty strip beside the list.
        let origin = ui.cursor().min.x;
        // Each opening reveals the current family once, on its first laid-out pass.
        if !was_open {
            edit.reveal = true;
        }
        let revealing = edit.reveal && !ui.is_sizing_pass();
        if revealing {
            edit.reveal = false;
        }
        // The filter bar, as in Photoshop: the class list toggle, Show Variable / Favorite /
        // Similar Fonts and Clear All (`font_filters`). The name field itself is the search.
        let filters_id = field.with("filters");
        let mut filters: crate::font_filters::FontFilters = ui.data(|d| d.get_temp(filters_id)).unwrap_or_default();
        // The preview on top: the font last under the pointer, else the current one.
        let preview_id = field.with("preview-font");
        if !was_open {
            ui.data_mut(|d| d.remove::<String>(preview_id));
        }
        let previewed: String = ui.data(|d| d.get_temp(preview_id)).unwrap_or_else(|| current.clone());
        let previewed_label = label(&previewed);
        // Everything above the lists is as wide as the class and font lists below, measured last
        // frame: never wider, so the menu has no empty strip beside the list.
        let body_id = field.with("body");
        let class_w = crate::font_filters::CLASS_COLUMN_WIDTH;
        let estimate = FONT_LIST_WIDTH + 16.0 + if filters.hide_classes { 0.0 } else { class_w + 14.0 };
        let body: Option<egui::Rect> = ui.data(|d| d.get_temp(body_id));
        let menu_width = body.map_or(estimate, |b| b.width());
        // Divider lines span the space they are given: cap it, or they stretch the menu.
        if body.is_some() {
            ui.set_max_width(menu_width);
        }
        crate::font_filters::preview_banner(ui, &previewed, &previewed_label, library.preview_text.as_deref(), menu_width);
        ui.horizontal(|ui| {
            crate::font_filters::filter_buttons(ui, &mut filters);
            crate::font_filters::clear_all(ui, &mut filters);
        });
        let lookup = crate::font_filters::lookup_in(library.traits.clone());
        // No recent group while a filter is on: the filters narrow the alphabetical list alone.
        let (mut rows, n_recent, _) = font_menu_rows(families, model, &edit.query, if filters.any() { &[] } else { &edit.recents });
        // Show Similar Fonts compares with the font current when it was turned on (or the menu
        // opened), as the recent group stays put: each arrow step applies a font, and sorting
        // around that one sent the next step back to the font before.
        let similar_id = field.with("similar-to");
        let reference: String = if filters.similar {
            ui.data_mut(|d| d.get_temp_mut_or_insert_with(similar_id, || current.clone()).clone())
        } else {
            ui.data_mut(|d| d.remove::<String>(similar_id));
            current.clone()
        };
        let comparable = crate::font_filters::apply(&mut rows, &filters, &library.favorites, &reference, &lookup);
        // The filters can drop rows (the interface language's group stays first) or, for similar
        // fonts, sort by likeness (which leaves no group boundary): count the group's rows again.
        // The recent group at the front is not the alphabetical list: skip it, so a recent row
        // (native or not) can't cut the group short or stretch it past its end.
        let n_native =
            if filters.similar { 0 } else { rows.iter().skip(n_recent).take_while(|(f, _)| model.is_some_and(|m| m.is_native(f))).count() };
        let mut selected = edit.row.filter(|&i| rows.get(i).is_some_and(|(f, _)| *f == current)).or_else(|| rows.iter().position(|(f, _)| *f == current));
        // The menu opens on the current family, as Photoshop's does: with the recent group at the
        // top of the view when the marked row fits below it, else the marked row itself, at the
        // top of the view when it is not a recent one. Applied on the first laid-out pass (the
        // popup's first pass only measures), whether or not a row matches, so a missing font or
        // a filtered list never scrolls the menu later.
        let reveal = if revealing { selected } else { None };
        let reveal_from = reveal.map(|i| if i < n_recent { 0 } else { i });
        let mut stepped = None;
        if down || up {
            let next = match selected {
                Some(i) if down => i.saturating_add(1).min(rows.len().saturating_sub(1)),
                Some(i) => i.saturating_sub(1),
                None if down => 0,
                None => rows.len().saturating_sub(1),
            };
            if let Some((f, shown)) = rows.get(next) {
                changed = **f != *current;
                current.clone_from(f);
                photocraft_text::served::request(f);
                edit.text = (*shown).to_string();
                edit.row = Some(next);
                selected = Some(next);
                stepped = Some(next);
                select_all = true;
            }
        }
        if enter && !edit.text.trim().is_empty() {
            // A full name wins over a substring match (including localized family names).
            let accept = accept_row(&rows, &edit.text, selected, model);
            if let Some((f, _)) = accept.and_then(|i| rows.get(i)) {
                // Acceptance is an application to the selected characters, even when a
                // mixed selection's representative already has this family (as row clicks do).
                changed = true;
                current.clone_from(f);
                photocraft_text::served::request(f);
                picked = true;
                ui.close();
            }
        }
        let preview = crate::font_preview::sample(preview);
        let row_h = preview.map_or(28.0, |(_, [_, h])| (h + 4.0).max(28.0));
        let expanded_id = field.with("expanded");
        let mut expanded: Vec<String> = ui.data(|d| d.get_temp(expanded_id)).unwrap_or_default();
        let mut hovered: Option<String> = None;
        let reveal = if stepped.is_some() { None } else { reveal };
        let mut reveal_top: Option<egui::Rect> = None;
        ui.separator();
        // The line between the class list and the font list. A separator in the row would take
        // the row's whole available height and hold the menu at its full height under a short
        // search result, so the row keeps its room and the line is drawn once the row is laid out.
        let mut divider: Option<f32> = None;
        let body = ui.horizontal_top(|ui| {
            if !filters.hide_classes {
                crate::font_filters::class_list(ui, &mut filters);
                divider = Some(ui.cursor().min.x + 3.0);
                ui.add_space(6.0 + ui.spacing().item_spacing.x);
            }
            // The list scrolls under the fixed preview and filters, at its full height or the room
            // the window leaves below it. That height is also its minimum, so the menu grows back
            // after a search that matched nothing (a popup never grows by itself, egui#5225); the
            // row beside the class list is no measure of it. With no rows it shrinks to its message.
            let room = ui.ctx().content_rect().bottom() - ui.cursor().top() - 16.0;
            let height = FONT_LIST_HEIGHT.min(room).max(row_h);
            let mut list = egui::ScrollArea::vertical().id_salt(field.with("list")).auto_shrink([true, true]);
            if !rows.is_empty() {
                list = list.max_height(height).min_scrolled_height(height).auto_shrink([true, false]);
            }
            // Top to bottom, though the list sits beside the class list.
            let shown = list.show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(FONT_LIST_WIDTH);
                    let weak = |ui: &mut egui::Ui, text: &str| {
                        ui.label(egui::RichText::new(text).weak());
                    };
                    if filters.similar && !comparable {
                        weak(ui, tl!("No similar fonts: the current font doesn't say what it looks like."));
                    }
                    if rows.is_empty() {
                        weak(
                            ui,
                            if filters.favorites && library.favorites.is_empty() {
                                tl!("No favorite fonts yet: click the star beside a font to add it.")
                            } else if filters.any() {
                                tl!("No fonts match these filters.")
                            } else {
                                tl!("No matching fonts")
                            },
                        );
                    }
                    // The first `n_recent` rows are the recent group, above a line (`font_menu_rows`).
                    for (i, &(f, shown)) in rows.iter().enumerate() {
                        let recent_row = i < n_recent;
                        // A line above the interface language's group and above the rest, whichever
                        // of the two groups comes first.
                        if (n_recent > 0 && i == n_recent) || (n_native > 0 && i == n_recent + n_native) {
                            ui.separator();
                        }
                        // Only rows on screen read their font's format and styles, and an expanded
                        // family: its style rows stay listed when its own row scrolls out of view.
                        let visible = ui.is_rect_visible(egui::Rect::from_min_size(ui.cursor().min, egui::vec2(FONT_LIST_WIDTH, row_h)));
                        let format = if visible { lookup(f).map(|t| t.format) } else { None };
                        let open = !recent_row && expanded.contains(f);
                        let styles = if visible || open { library.style_options_of(f) } else { Vec::new() };
                        let favorite = library.favorites.contains(f);
                        // Highlighted by its fill alone, as Photoshop's font menu is: a hovered or selected
                        // row's stroke grew it and pushed the rows below down (#7406).
                        let slot = preview.map_or(0.0, |(size, _)| crate::font_preview::slot_width(ui.ctx(), f, size));
                        // One row lit, though a recent family is listed twice.
                        let mut button = egui::Button::selectable(selected == Some(i), shown).truncate().stroke(egui::Stroke::NONE);
                        let [_, h] = preview.map_or([0.0, 16.0], |(_, b)| b);
                        button =
                            button.right_text(egui::Atom::custom(egui::IdSalt::new(("font-sample", f, recent_row)), egui::vec2(slot + FONT_BADGE_WIDTH, h)));
                        let response = ui
                            .allocate_ui_with_layout(egui::vec2(FONT_LIST_WIDTH, row_h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                // The star adds the font to (or removes it from) the favourites; it never picks it.
                                let tip = if favorite { tl!("Remove from Favorites") } else { tl!("Add to Favorites") };
                                if crate::icons::button(ui, if favorite { "star-filled" } else { "star" }, 22.0, false, tip).clicked() {
                                    library.toggled = Some(f.clone());
                                }
                                // A family with several styles expands to list them (">").
                                if styles.len() > 1 && !recent_row {
                                    let (icon, tip) = if open { ("chevron-down", tl!("Hide Styles")) } else { ("chevron-right", tl!("Show Styles")) };
                                    if crate::icons::button(ui, icon, 16.0, false, tip).clicked() {
                                        if open {
                                            expanded.retain(|e| e != f);
                                        } else {
                                            expanded.push(f.clone());
                                        }
                                    }
                                } else {
                                    ui.add_space(16.0);
                                }
                                ui.add_sized([ui.available_width(), row_h], button)
                            })
                            .inner;
                        if stepped == Some(i) {
                            response.scroll_to_me(Some(egui::Align::Center));
                        } else if reveal.is_some() {
                            if reveal_from == Some(i) {
                                reveal_top = Some(response.rect);
                            }
                            if reveal == Some(i) {
                                // Jump, not glide: the menu opens with its current family in view. Arrow
                                // steps above keep the animation. A recent row too far below the top of
                                // the group (large previews, many recents) is brought up to the bottom.
                                let top = reveal_top.unwrap_or(response.rect);
                                let (rect, align) = if response.rect.bottom() - top.top() <= ui.clip_rect().height() {
                                    (top, egui::Align::Min)
                                } else {
                                    (response.rect, egui::Align::Max)
                                };
                                ui.scroll_to_rect_animation(rect, Some(align), egui::style::ScrollAnimation::none());
                            }
                        }
                        if let Some((size, _)) = preview
                            && ui.is_rect_visible(response.rect)
                        {
                            crate::font_preview::paint(ui, f, response.rect, size);
                        }
                        // The format badge (O, TT, VAR) just left of the sample.
                        if let Some(format) = format {
                            let center = egui::pos2(response.rect.right() - 8.0 - slot - FONT_BADGE_WIDTH / 2.0, response.rect.center().y);
                            let t = crate::theme::Tokens::get(ui.ctx());
                            ui.painter().text(center, egui::Align2::CENTER_CENTER, format.badge(), egui::FontId::proportional(10.0), t.text_dim);
                            let tip = egui::Rect::from_center_size(center, egui::vec2(FONT_BADGE_WIDTH, 14.0));
                            ui.interact(tip, ui.scope_id().with(("font-format", f, recent_row)), egui::Sense::hover())
                                .on_hover_text(crate::font_filters::format_tip(format));
                        }
                        if response.hovered() {
                            hovered = Some(f.clone());
                        }
                        if press.chosen(ui, &response) {
                            changed = true;
                            current.clone_from(f);
                            // A served family not fetched yet: start now, before any text needs it.
                            photocraft_text::served::request(f);
                            picked = true;
                            hovered = None;
                            ui.close();
                        }
                        if !open {
                            continue;
                        }
                        // The family's styles, each picking the family in that style.
                        for (style, style_name) in &styles {
                            let style_row = ui
                                .horizontal(|ui| {
                                    ui.add_space(46.0);
                                    ui.add_sized(
                                        [ui.available_width(), 22.0],
                                        egui::Button::selectable(false, style_name.clone()).truncate().stroke(egui::Stroke::NONE),
                                    )
                                })
                                .inner;
                            if style_row.clicked() {
                                changed = true;
                                current.clone_from(f);
                                photocraft_text::served::request(f);
                                library.picked_style = Some(style.clone());
                                picked = true;
                                hovered = None;
                                ui.close();
                            }
                        }
                    }
                })
            });
            (shown.inner_rect.right(), (!rows.is_empty()).then_some(shown.inner_rect.height()))
        });
        if let Some(x) = divider {
            ui.painter().vline(x, body.response.rect.y_range(), ui.visuals().widgets.noninteractive.bg_stroke);
        }
        let (list_right, list_height) = body.inner;
        // The list's viewport, while it lists fonts.
        if list_height.is_some() {
            edit.menu_height = list_height;
        }
        // The lists' own extent, from the class list to the font list's scroll bar: the row
        // itself stretches with the popup, so it can't tell when the popup is too wide.
        let list_right = list_right + FONT_SCROLL_BAR_WIDTH;
        let body = egui::Rect::from_min_max(body.response.rect.min, egui::pos2(list_right.min(body.response.rect.right()), body.response.rect.bottom()));
        ui.data_mut(|d| {
            d.insert_temp(filters_id, filters);
            d.insert_temp(expanded_id, expanded);
            d.insert_temp(body_id, body.translate(egui::vec2(-origin, 0.0)));
            if let Some(f) = &hovered {
                d.insert_temp(preview_id, f.clone());
            }
        });
        if picked { None } else { hovered }
    });
    let open = egui::Popup::is_id_open(ui.ctx(), popup);
    if !was_open && open {
        edit.recents = library.recent.clone();
        edit.opening = arrow.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_down());
        ui.memory_mut(|m| m.request_focus(field));
        select_all = !input.changed();
    }
    // A held opening arrow can regain focus on release; keep the name selected through it.
    if edit.opening {
        ui.memory_mut(|m| m.request_focus(field));
        select_all |= !input.changed();
        edit.opening = ui.input(|i| i.pointer.primary_down());
    }
    if !open && !escape && ui.is_enabled() {
        let notches = if had_focus { f32::from(u8::from(up)) - f32::from(u8::from(down)) } else { 0.0 };
        let hovered = input.hovered() || arrow.hovered();
        if hovered {
            ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
        }
        let notches = notches + crate::widgets::wheel_notches_of(ui, field.with("family-wheel"), hovered, 1.0);
        // Closed-field navigation stays alphabetical, independent of recent picks (#3350, #7407).
        let (rows, _, _) = font_menu_rows(families, model, "", &[]);
        if let Some(i) = rows.iter().position(|(f, _)| *f == current)
            && let Some(n) = wheel_family_index(i, rows.len(), notches)
            && let Some((f, _)) = rows.get(n)
            && **f != *current
        {
            current.clone_from(f);
            photocraft_text::served::request(f);
            changed = true;
            select_all |= had_focus;
        }
    }
    if picked || (!open && (was_open || changed || escape)) {
        edit.text = label(current);
        edit.query.clear();
        edit.row = None;
        edit.accepted = true;
        edit.opening = false;
        edit.composing = false;
    }
    if picked {
        ui.memory_mut(|m| m.request_focus(field));
        select_all = true;
    }
    if select_all && !escape {
        crate::field_tab::select_all(ui.ctx(), field, &edit.text);
    }
    if !open {
        ui.data_mut(|d| d.remove::<String>(field.with("similar-to")));
    }
    edit.current.clone_from(current);
    if open || ui.memory(|m| m.has_focus(field)) {
        ui.data_mut(|d| d.insert_temp(edit_id, edit));
    } else {
        ui.data_mut(|d| d.remove::<FontFieldEdit>(edit_id));
    }
    if salt == "type-font" && (was_open || open || response.is_some()) {
        // Only this popup's owner clears its preview; a closed peer must leave it alone (#5152).
        let hover_id = egui::Id::unique("type-font-hover");
        if let Some(f) = response.flatten().filter(|_| open) {
            ui.ctx().data_mut(|d| d.insert_temp(hover_id, f));
        } else {
            ui.ctx().data_mut(|d| d.remove::<String>(hover_id));
        }
    }
    changed
}

/// The row `notches` wheel notches from row `index` of `len` (down, negative, is the next one),
/// stopping at the ends; none when the wheel did not move.
pub(super) fn wheel_family_index(index: usize, len: usize, notches: f32) -> Option<usize> {
    let steps = notches.abs().min(len as f32) as usize;
    if steps == 0 || len == 0 {
        return None;
    }
    let last = len - 1;
    Some(if notches < 0.0 { index.saturating_add(steps).min(last) } else { index.saturating_sub(steps) })
}
