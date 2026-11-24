use craft::i18n::Words;
use craft::keymap::{Chord, CommandRef, Mods};
use craft::menu::{
    Alternate, Customization, ItemKind, ItemRole, LanguageChoice, LanguageLabels, LanguageMenu, LayoutOptions, MenuBar, MenuRole, Node, Platforms, Row, Target,
    ThemeLabels, from_items, layout,
};
use std::sync::Arc;

use craft::ui::contract::Tracked;
use craft::ui::runtime::derived::derived;
use serde_json::json;

use super::facts::MenuFacts;
use super::invoke::{panel_alias, proof_preset, workspace_id};
use super::state::{checked, is_enabled, reveal_label, translated_menu_label};
use super::{DOCUMENT_ITEM, PhotocraftApp, TOP_MENUS, UI_COMMANDS};

/// Is `id` implemented by the engine or the shell (a live menu item)? Shared by the menus and
/// the parity report ([`crate::parity`]). The command gate asks this for every command a pass
/// resolves, framework commands included, so the hash lookups and prefix tests come first and
/// the scans of the bridged rows and the workspace commands last.
pub fn is_live(id: &str) -> bool {
    static UI_IDS: std::sync::OnceLock<std::collections::HashSet<&'static str>> = std::sync::OnceLock::new();
    photocraft_engine::commands::find(id).is_some()
        || UI_IDS.get_or_init(|| UI_COMMANDS.iter().map(|c| c.0).collect()).contains(id)
        || id.starts_with(super::DOCUMENT_ITEM)
        || id == "view.proofSetup.custom"
        || crate::tool_shortcuts::tool(id).is_some()
        || panel_alias(id).is_some()
        || crate::panel_docking::toggle_panel(id).is_some()
        || workspace_id(id).is_some()
        || proof_preset(id).is_some()
        || crate::analysis::handles(id)
        || crate::view_cmds::handles(id)
        || crate::shortcut_dispatch::is_bridged(id)
        || crate::workspace_ui::handles(id)
}

/// Commands outside the catalogue that belong right after a catalogue item: `(id, after)`.
/// The command takes that item's submenu path, so it joins the catalogue's submenu rather than
/// opening a second one spelled from its own id (Filter › Other).
const PLACE_AFTER: &[(&str, &str)] = &[
    ("file.newFromClipboard", "file.new"),
    ("filter.render.relight", "filter.render.lightingEffects"),
    ("view.rotateLeft", "view.flipHorizontal"),
    ("view.rotateRight", "view.rotateLeft"),
    ("view.resetView", "view.rotateRight"),
    ("filter.other.colorToAlpha", "filter.other.offset"),
];

/// The menu rows of one state of the app, built once and kept until a fact they show changes
/// ([`MenuFacts`]): building them asks every command whether it can run, which on a document with
/// thousands of layers costs far more than a frame.
pub struct MenuModel {
    built: Arc<Built>,
    /// The menus laid out for this platform, titles as shown, each translated title and label with its English name.
    /// A rebuild that lays out the same menus keeps the held value, so the bar's key stays.
    pub bar: Tracked<MenuBar>,
}

impl MenuModel {
    /// The tagged, translated menus before any platform layout: what the native bar and the Help search lay out
    /// themselves, so a bar is never laid out twice.
    pub fn source(&self) -> &MenuBar {
        &self.built.source
    }
}

impl MenuModel {
    /// The rows with labels as shown, after the user's Edit › Menus customization; each translated label carries its
    /// English name.
    pub fn rows(&self) -> &[Row] {
        &self.built.rows
    }
}

/// One build of the model: the rows and the bar laid out from them.
struct Built {
    rows: Vec<Row>,
    source: MenuBar,
    bar: MenuBar,
}

/// `shown` with the English name of each item and submenu whose label `english`, the same node built in English,
/// words differently.
fn with_english(mut shown: Node, english: &Node) -> Node {
    fn name(node: &mut Node, english: &Node) {
        match (node, english) {
            (Node::Item(item), Node::Item(en)) => {
                if item.label != en.label {
                    item.english = Some(en.label.clone());
                }
            }
            (Node::Submenu { label, english, children, .. }, Node::Submenu { label: en_label, children: en_children, .. }) => {
                if label != en_label {
                    *english = Some(en_label.clone());
                }
                children.iter_mut().zip(en_children).for_each(|(child, en)| name(child, en));
            }
            _ => {}
        }
    }
    name(&mut shown, english);
    shown
}

/// The id the model is derived under.
pub(crate) fn model_id() -> egui::Id {
    egui::Id::unique("photocraft-menu-model")
}

/// The model for the app's current state: derived from its [`MenuFacts`], so it is built again only when one of them
/// changed. The held bar is replaced only when the new build lays out different menus.
pub fn model<'a>(app: &'a mut PhotocraftApp, ctx: &egui::Context) -> &'a MenuModel {
    let built = {
        let app = &*app;
        derived(ctx, model_id(), &MenuFacts::of(app), || build(app))
    };
    let held = app.menu_model.get_or_insert_with(|| MenuModel { bar: Tracked::new(built.bar.clone()), built: Arc::clone(&built) });
    if !Arc::ptr_eq(&held.built, &built) {
        if *held.bar != built.bar {
            *held.bar = built.bar.clone();
        }
        held.built = built;
    }
    held
}

/// The row that runs `id`.
pub fn row<'a>(rows: &'a [Row], id: &str) -> Option<&'a Row> {
    rows.iter().find(|r| r.command.id == id)
}

fn find<'a>(rows: &'a mut [Row], id: &str) -> Option<&'a mut Row> {
    rows.iter_mut().find(|r| r.command.id == id)
}

/// A live row shows the chord the runtime's keymap runs its command for (`commands::chord`: overrides included, a
/// key another command wins left out); a catalogue row shows its text.
fn shortcut_of(app: &PhotocraftApp, id: &str, catalogue: Option<&str>) -> Option<Chord> {
    if is_live(id) {
        return craft::ui::runtime::commands::chord(app.ctx(), &crate::shortcut_dispatch::keymap_command(id));
    }
    let text = catalogue?;
    match Chord::parse(text) {
        Ok(chord) => Some(chord),
        Err(e) => {
            log::warn!("menu shortcut {text:?} of {id} does not parse: {e:?}");
            None
        }
    }
}

fn with_state(app: &PhotocraftApp, id: &str, mut row: Row) -> Row {
    if let Some(on) = checked(app, id) {
        row = row.kind(ItemKind::Check(on));
    }
    row
}

/// The tooltip and the Alt alternate a row carries.
fn decorate(row: &mut Row) {
    let id = row.command.id.clone();
    match id.as_str() {
        "image.mode.bits8" | "image.mode.bits16" => row.tooltip = Some("Integer".into()),
        "image.mode.bits32" => row.tooltip = Some("Floating point".into()),
        _ => {}
    }
    // ⌥ while choosing an Image › Adjustments item opens it at its last settings (#8900).
    if crate::adjust_dialog::has_dialog(&id) {
        row.alternate = Some(Alternate::new(Mods::ALT, CommandRef::with_params(id, json!({"lastSettings": true}))));
    } else if let Some(stamp) = photocraft_engine::stamp_cmds::alt_variant(&id) {
        row.alternate = Some(Alternate::new(Mods::ALT, CommandRef::new(stamp)));
    }
}

/// The rows before Edit › Menus customization: every command with its label, shortcut and state.
/// The Keyboard Shortcuts Menus tab reads these, so hidden rows can be unhidden.
pub fn base_rows(app: &PhotocraftApp) -> Vec<Row> {
    // 1) Photoshop's full menu tree, in Photoshop order; live where we implement the command.
    let mut rows: Vec<Row> = crate::menu_catalog::CATALOG
        .iter()
        .map(|&(path, label, sc, id)| {
            if label == "---" {
                return Row::separator(path);
            }
            let live = is_live(id);
            let mut row = Row::new(path, id, label).enabled(live && is_enabled(app, id));
            if let Some(chord) = shortcut_of(app, id, sc) {
                row = row.shortcut(chord);
            }
            with_state(app, id, row)
        })
        .collect();
    // 2) Our commands that Photoshop's tree doesn't list (or lists under another id).
    let mut extra: Vec<Row> = Vec::new();
    for &(id, label, path, sc) in UI_COMMANDS {
        let mut row = Row::new(path, id, label).enabled(is_enabled(app, id));
        if let Some(chord) = shortcut_of(app, id, sc) {
            row = row.shortcut(chord);
        }
        extra.push(with_state(app, id, row));
    }
    for c in photocraft_engine::command_specs().iter().filter(|c| !c.menu.is_empty()) {
        let mut row = Row::new(c.menu, c.id, c.label).enabled(is_enabled(app, c.id));
        if let Some(chord) = shortcut_of(app, c.id, c.shortcut) {
            row = row.shortcut(chord);
        }
        extra.push(row);
    }
    for mut e in extra {
        let dup = rows.iter().any(|i| i.command.id == e.command.id || (i.path == e.path && i.label.trim_end_matches('…') == e.label.trim_end_matches('…')));
        if !dup {
            // Insert after the item it belongs next to, else after the last item of the same
            // top-level menu, keeping menus contiguous.
            let top = e.path.first().cloned();
            let after = PLACE_AFTER.iter().find(|(id, _)| *id == e.command.id).and_then(|(_, a)| rows.iter().position(|i| i.command.id == *a));
            if let Some(path) = after.and_then(|a| rows.get(a)).map(|i| i.path.clone()) {
                e.path = path;
            }
            let at = after.or_else(|| rows.iter().rposition(|i| i.path.first() == top.as_ref())).map_or(rows.len(), |p| p + 1);
            rows.insert(at, e);
        }
    }
    crate::plugin_ui::rows(app, &mut rows);
    if let Some(mask) = app.session.active().and_then(|s| s.active_layer.and_then(|id| s.doc.layer(id))).and_then(|l| l.mask.as_ref())
        && let Some(item) = find(&mut rows, "layer.layerMask.enabled")
    {
        item.label = crate::layer_menu_ui::mask_toggle_label(mask.enabled).into();
    }
    // Layer › Merge Layers reads Merge Group with just a group selected, as in Photoshop (#3832).
    if let Some(st) = app.session.active()
        && st.selected_layers().len() <= 1
        && st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| matches!(l.content, photocraft_doc::LayerContent::Group(_)))
        && let Some(item) = find(&mut rows, "layer.mergeLayers")
    {
        item.label = "Merge Group".into();
    }
    // The File quick-export command uses the format selected in Export Preferences.
    // On the web it intentionally downloads PNG until the quick-export service supports
    // the other formats; the Layer quick-export command also always produces PNG.
    if let Some(item) = find(&mut rows, "file.export.quickExportAsPng") {
        use photocraft_engine::prefs::QuickExportFormat;
        let format = if cfg!(target_arch = "wasm32") {
            "PNG"
        } else {
            match app.session.prefs().export.quick_export_format {
                QuickExportFormat::Png => "PNG",
                QuickExportFormat::Jpg => "JPG",
                QuickExportFormat::Gif => "GIF",
                QuickExportFormat::Webp => "WebP",
                QuickExportFormat::Exr => "EXR",
            }
        };
        item.label = format!("Quick Export as {format}");
    }
    // Smart Objects › Reveal in Finder names this platform's file manager (UI-217-19).
    if let Some(item) = find(&mut rows, "layer.smartObjects.revealInFinder") {
        item.label = reveal_label().into();
    }
    // File › Open Recent: a dynamic submenu of recently opened files, after Open as Smart Object… (#14053).
    if let Some(after) = rows.iter().position(|i| i.command.id == "file.openAsSmartObject").or_else(|| rows.iter().position(|i| i.command.id == "file.openAs"))
    {
        let rp = ["File", "Open Recent"];
        let mut recent: Vec<Row> = app
            .ui
            .recent_files
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let label = std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.clone());
                Row::new(&rp, format!("file.openRecent.{i}").as_str(), label)
            })
            .collect();
        if !recent.is_empty() {
            recent.push(Row::separator(&rp));
        }
        recent.push(Row::new(&rp, "file.clearRecent", "Clear Recent File List").enabled(!app.ui.recent_files.is_empty()));
        for (k, it) in recent.into_iter().enumerate() {
            rows.insert(after + 1 + k, it);
        }
    }
    // Window › Workspace: saved workspaces follow the presets, checked while current.
    let set = &app.workspaces.set;
    if !set.saved.is_empty()
        && let Some(after) = rows.iter().position(|i| i.command.id == "window.workspace.motion")
    {
        let wp = ["Window", "Workspace"];
        let entries = set.saved.iter().map(|w| {
            let id = format!("window.workspace.{}", w.id);
            Row::new(&wp, id.as_str(), w.name.clone()).kind(ItemKind::Check(set.current == w.id))
        });
        for (k, it) in std::iter::once(Row::separator(&wp)).chain(entries).enumerate() {
            rows.insert(after + 1 + k, it);
        }
    }
    // Keep the update command with the links, ahead of System Info and About.
    if let Some(at) = rows.iter().position(|i| i.command.id == "help.checkForUpdates") {
        let update = rows.remove(at);
        let before = rows.iter().position(|i| i.command.id == "help.systemInfo" || i.command.id == "help.about").unwrap_or(rows.len());
        rows.insert(before, update);
    }
    // Help: the link items and update check, a separator, then System Info and About.
    if let Some(at) = rows.iter().position(|i| i.command.id == "help.systemInfo" || i.command.id == "help.about") {
        rows.insert(at, Row::separator(&["Help"]));
    }
    // Window: the open documents in tab order, after a separator, the active one checked.
    if let Some(last) = rows.iter().rposition(|i| i.path.first().map(String::as_str) == Some("Window")) {
        let active = app.session.active_index();
        let docs = app
            .session
            .documents()
            .iter()
            .enumerate()
            .map(|(i, st)| Row::new(&["Window"], format!("{DOCUMENT_ITEM}{i}").as_str(), st.doc.name.clone()).kind(ItemKind::Check(active == Some(i))));
        let listed: Vec<Row> = std::iter::once(Row::separator(&["Window"])).chain(docs).collect();
        if listed.len() > 1 {
            rows.splice(last + 1..last + 1, listed);
        }
    }
    // Layer › Create Clipping Mask is Photoshop's one Create / Release item: when its shortcut
    // would release ([`crate::shortcut_dispatch::clip_key_releases`]) it reads Release Clipping
    // Mask and runs it, keeping that shortcut. The engine's own Release item lives in that slot.
    rows.retain(|i| i.command.id != "layer.releaseClippingMask");
    if crate::shortcut_dispatch::clip_key_releases(app)
        && let Some(item) = find(&mut rows, "layer.createClippingMask")
    {
        item.command = CommandRef::new("layer.releaseClippingMask");
        item.label = "Release Clipping Mask".into();
        item.enabled = is_enabled(app, "layer.releaseClippingMask");
    }
    // Edit › Undo names the most recent change, a document's or the Actions library's.
    if let Some(label) = crate::actions::library::edit_undo_label(app)
        && let Some(row) = rows.iter_mut().find(|row| row.command.id == "edit.undo")
    {
        row.label = label.into();
    }
    for row in &mut rows {
        decorate(row);
    }
    rows
}

/// [`base_rows`] with Edit › Menus applied: hidden rows dropped and the chosen colours set.
pub fn menu_rows(app: &PhotocraftApp) -> Vec<Row> {
    let mut rows = base_rows(app);
    let prefs = app.session.prefs();
    let hidden = prefs.menus.hidden.iter().filter(|id| photocraft_engine::prefs::MenuCustomization::hideable(id)).cloned();
    let colors = if prefs.interface.show_menu_colors {
        prefs.menus.colors.iter().filter_map(|(id, name)| menu_tint(name).map(|c| (id.clone(), c.into()))).collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    Customization::new(hidden, colors).apply(&mut rows);
    rows
}

/// Background tint of an Edit › Menus colour name.
pub(super) fn menu_tint(name: &str) -> Option<egui::Color32> {
    Some(match name {
        "red" => egui::Color32::from_rgb(190, 60, 60),
        "orange" => egui::Color32::from_rgb(200, 120, 40),
        "yellow" => egui::Color32::from_rgb(190, 170, 40),
        "green" => egui::Color32::from_rgb(60, 150, 70),
        "blue" => egui::Color32::from_rgb(50, 110, 200),
        "violet" => egui::Color32::from_rgb(130, 80, 190),
        "gray" => egui::Color32::from_rgb(120, 120, 120),
        _ => return None,
    })
}

/// Translates the titles of menus and submenus, each translated one keeping its English title.
fn translate_titles(bar: &mut MenuBar, lang: crate::i18n::Lang) {
    /// `title` as shown in `lang`, and its English name when the two differ.
    fn translate(title: &mut String, english: &mut Option<String>, lang: crate::i18n::Lang) {
        let shown = crate::i18n::tr(lang, title).to_string();
        if shown != *title {
            *english = Some(std::mem::replace(title, shown));
        }
    }
    fn level(nodes: &mut [Node], lang: crate::i18n::Lang) {
        for node in nodes {
            if let Node::Submenu { label, english, children, .. } = node {
                translate(label, english, lang);
                level(children, lang);
            }
        }
    }
    for menu in &mut bar.menus {
        if menu.role != MenuRole::App {
            translate(&mut menu.title, &mut menu.english, lang);
        }
        level(&mut menu.children, lang);
    }
}

/// Window › Workspace's neighbour: the Appearance and Language submenus go after it. The layouts
/// move them to the Mac app menu by their roles.
fn put_in_window(bar: &mut MenuBar, node: Node) {
    let Some(menu) = bar.menus.iter_mut().find(|m| m.role == MenuRole::Window) else { return };
    let at = menu.children.iter().position(|n| matches!(n, Node::Submenu { label, .. } if label == "Workspace")).map_or(menu.children.len(), |p| p + 1);
    menu.children.insert(at, node);
}

/// The bar a build from `app`'s state lays out now, for trials that check the held model is never stale.
#[cfg(test)]
pub(crate) fn fresh_bar(app: &PhotocraftApp) -> MenuBar {
    build(app).bar
}

/// The platform the menus are laid out for: the shell's, so a test can run the macOS layout on any build.
pub(crate) fn target(ctx: &egui::Context) -> Target {
    Target::of_shell(&craft::ui::runtime::windows::shell_of(ctx, egui::ViewportId::ROOT))
}

fn build(app: &PhotocraftApp) -> Built {
    let lang = crate::i18n::current();
    let mut rows = menu_rows(app);
    for row in &mut rows {
        if row.label == "---" {
            continue;
        }
        let shown = translated_menu_label(lang, &row.command.id, &row.label);
        if shown != row.label {
            row.english = Some(std::mem::replace(&mut row.label, shown));
        }
        if let Some(tip) = row.tooltip.take() {
            row.tooltip = Some(crate::i18n::tr(lang, &tip).to_string());
        }
    }
    let mut bar = from_items(&TOP_MENUS, &rows);
    bar.tag_item(&CommandRef::new("help.about"), ItemRole::About);
    bar.tag_item(&CommandRef::new("file.exit"), ItemRole::Quit);
    for (title, role) in [("File", MenuRole::File), ("Edit", MenuRole::Edit), ("View", MenuRole::View), ("Window", MenuRole::Window), ("Help", MenuRole::Help)]
    {
        bar.tag_menu(title, role);
    }
    bar.tag_submenu(MenuRole::Edit, "Preferences", ItemRole::Settings);
    // The labels of the two menus come from the language the rest of the model uses.
    let i18n = crate::i18n::framework_i18n();
    let words = Words::from_i18n(i18n.clone(), i18n.from_code(lang.code()).unwrap_or_else(|| i18n.system_lang()));
    let themes = crate::prefs_ui::theme_menu(app);
    put_in_window(&mut bar, with_english(themes.submenu(&words.set::<ThemeLabels>()), &themes.submenu(&Words::english().set::<ThemeLabels>())));
    // Production shows Language only in the macOS app menu.
    let pref = &app.session.prefs().interface.language;
    let choice = if pref == "auto" { LanguageChoice::Automatic } else { LanguageChoice::Code(pref.clone()) };
    let language = LanguageMenu::from_i18n(i18n, choice).submenu(&words.set::<LanguageLabels>()).platforms(Platforms::MAC);
    put_in_window(&mut bar, language);
    let label = |s: &'static str| crate::i18n::tr(lang, s).to_string();
    let keys = |chord: &Chord| craft::ui::runtime::commands::holder(app.ctx(), chord);
    let options = LayoutOptions::new(crate::native_menu::APP_NAME).label(&label).keys(&keys);
    translate_titles(&mut bar, lang);
    let laid = layout(&bar, target(app.ctx()), &options).bar;
    Built { rows, source: bar, bar: laid }
}
