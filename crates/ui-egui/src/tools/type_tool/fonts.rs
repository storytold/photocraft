use craft::font_manager::picker::{FontFilters, FontPickerSettings, FontPreviewSize};
use craft::text::ui::character::FontPickerSource;
use craft::ui::contract::Bind;
use photocraft_engine::prefs::FontPreview;

use super::{Arc, PhotocraftApp, PoisonError, json};

/// Font families: the catalog's (bundled and installed) and those the host serves
/// (`photocraft_text::served`, fetched when picked). Cached until the catalog or the served list changes.
pub fn families() -> Arc<Vec<String>> {
    type Cached = Option<((u64, u64), Arc<Vec<String>>)>;
    static FAMILIES: std::sync::Mutex<Cached> = std::sync::Mutex::new(None);
    let catalog = photocraft_text::fonts::catalog();
    let key = families_key();
    let mut cached = FAMILIES.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((k, v)) = cached.as_ref()
        && *k == key
    {
        return v.clone();
    }
    let mut v: Vec<String> = catalog.families().to_vec();
    for f in photocraft_text::served::families() {
        if !v.contains(&f) {
            v.push(f);
        }
    }
    v.sort_by_key(|s| s.to_lowercase());
    let v = Arc::new(v);
    *cached = Some((key, v.clone()));
    v
}

/// Changes whenever [`families`] does: a family installed or served while the app runs reaches the font menus with no
/// command or input, so the regions that show them hold this in their versions.
pub(crate) fn families_key() -> (u64, u64) {
    (photocraft_text::fonts::catalog().generation(), photocraft_text::served::generation())
}

/// Style names available for a family: the names of the catalog's faces of it (named instances included),
/// "Regular" when it has none. The picker's style rows list the same faces.
pub fn styles(family: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for face in photocraft_text::fonts::catalog().styles(family) {
        if !names.contains(&face.style) {
            names.push(face.style);
        }
    }
    if names.is_empty() { vec!["Regular".into()] } else { names }
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

/// Localized style label for the dropdown. The raw string stays the engine's `fontStyle` key;
/// the words (weights and "Italic") are translated in the `fontWeight` context, where "Light"
/// means a weight — unlike the Camera Raw "Light" section.
pub fn style_label(style: &str) -> String {
    style.split(' ').map(|w| crate::i18n::tr_ctx(crate::i18n::current(), "fontWeight", w)).collect::<Vec<_>>().join(" ")
}

/// Starts scanning the installed fonts on a background thread (desktop start-up), so choosing
/// the Type tool doesn't stall the UI while they are read (#3021), and follows fonts installed
/// while the app runs (`installed_fonts`). Does nothing on the web.
pub fn warm_fonts() {
    #[cfg(not(target_arch = "wasm32"))]
    crate::text::installed_fonts::start();
    photocraft_text::warm_shared();
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

/// The font picker's most recent families the settings may hold, whatever the preference says.
const MAX_RECENT_FONTS: usize = 50;

/// What the font pickers of one pass edit: the stars, recents and filters. The picker writes them through its
/// `Bind`s, so the values live here, owned by the caller, until [`finish_font_state`] stores what changed.
pub(crate) struct FontSourceState {
    /// The picker's settings as the pass edits them.
    pub(crate) settings: FontPickerSettings,
    /// The settings as the pass began, to tell what the pass changed.
    before: FontPickerSettings,
    filters: FontFilters,
    preview_text: Option<String>,
    served: Vec<String>,
}

/// The state for one pass's pickers: settings from the prefs and `UiState.recent_fonts`, the filters from
/// `UiState.font_filters`, the preview text and the served families the catalog lacks.
pub(crate) fn font_state(app: &PhotocraftApp) -> FontSourceState {
    let prefs = app.session.prefs();
    // `FontPickerSettings` is non-exhaustive: start from its default and set every field.
    let mut settings = FontPickerSettings::default();
    settings.favorites = photocraft_engine::font_library_cmds::favorite_fonts(&app.session);
    settings.recent = app.ui.recent_fonts.clone();
    settings.recent_limit = (prefs.type_.recent_fonts as usize).min(MAX_RECENT_FONTS);
    settings.preview = match prefs.type_.font_preview {
        FontPreview::Off => FontPreviewSize::None,
        FontPreview::Small => FontPreviewSize::Small,
        FontPreview::Medium => FontPreviewSize::Medium,
        FontPreview::Large => FontPreviewSize::Large,
        FontPreview::ExtraLarge => FontPreviewSize::ExtraLarge,
        FontPreview::Huge => FontPreviewSize::Huge,
    };
    settings.names_in_english = prefs.type_.show_font_names_in_english;
    let catalog = photocraft_text::fonts::catalog();
    let served = photocraft_text::served::families().into_iter().filter(|f| catalog.family(f).is_none()).collect();
    FontSourceState { before: settings.clone(), settings, filters: app.ui.font_filters.clone(), preview_text: preview_text(app), served }
}

impl FontSourceState {
    /// The picker's source over this state.
    pub(crate) fn source(&mut self) -> FontPickerSource<'_> {
        let catalog = photocraft_text::fonts::catalog();
        let mut source = FontPickerSource::new(catalog).settings(Bind::bound(&mut self.settings)).filters(Bind::bound(&mut self.filters)).served(&self.served);
        if let Some(text) = self.preview_text.as_deref() {
            source = source.preview_text(text);
        }
        source
    }
}

/// Stores what the pass's pickers changed: a changed star as `type.font.favorite {family, on}` (favourites stay
/// engine prefs), the recent list (capped by the preference at once) and the filters.
pub(crate) fn finish_font_state(app: &mut PhotocraftApp, state: FontSourceState) {
    let FontSourceState { settings, before, filters, .. } = state;
    let stars = before
        .favorites
        .iter()
        .filter(|f| !settings.favorites.contains(f))
        .map(|f| (f, false))
        .chain(settings.favorites.iter().filter(|f| !before.favorites.contains(f)).map(|f| (f, true)));
    for (family, on) in stars {
        if let Err(e) = app.run("type.font.favorite", json!({"family": family, "on": on})) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    // The cap applies on every pass, so lowering the preference trims the list without a pick.
    let cap = (app.session.prefs().type_.recent_fonts as usize).min(MAX_RECENT_FONTS);
    let mut recent = settings.recent;
    recent.truncate(cap);
    if recent != app.ui.recent_fonts {
        app.ui.recent_fonts = recent;
    }
    app.ui.font_filters = filters;
}
