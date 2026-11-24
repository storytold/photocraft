//! The Layers panel's menu rows, Panel Options and Lock Layers dialogs, and the saved display options.
//! UI options do not edit document pixels or add history steps.
use craft::keymap::CommandRef;
use craft::layers::ui::options::{LayersOptionsDialog, LayersSettings};
use craft::layers::ui::panel::{LayersMenuAction, LayersPanelCommands, LayersPanelWords};
use craft::layers::ui::thumbs::ThumbContents;
use craft::ui::contract::Bind;
use craft::ui::dialog::{DialogId, DialogResult};
use craft::ui::list::ThumbSize;
use craft::ui::menu::PanelRow;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::commands::{CommandSet, CommandState};
use craft::ui::runtime::env::Env;
use photocraft_doc::Document;
use serde_json::{Value, json};

use crate::PhotocraftApp;

const OPTIONS_PREF: &str = "ui.layersPanelOptions";
const CLIP_PREF: &str = "interface.clipThumbnailsToLayerBounds";
const LOCK_LAYERS: &str = "layer.lockLayers";
const DUPLICATE: &str = "layer.duplicate";

/// The Layers panel menu's own commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft::ui::contract::CommandSet)]
#[command(prefix = "layers")]
pub(crate) enum LayersMenuCommand {
    /// Opens the Lock Layers dialog with All ticked, as production's panel and row menus do; Layer › Lock Layers… and
    /// Cmd+/ (`layer.lockLayers`) lock All directly.
    #[command(label = "Lock Layers…")]
    LockLayersDialog,
}

/// The saved name of a thumbnail size: the one rule both the saved entry and the panel's row menu use.
pub(crate) fn size_key(size: ThumbSize) -> &'static str {
    match size {
        ThumbSize::None => "none",
        ThumbSize::Small => "small",
        ThumbSize::Large => "large",
        _ => "medium",
    }
}

/// The size a saved name stands for; `None` for a name that is no size.
pub(crate) fn size_of_key(key: &str) -> Option<ThumbSize> {
    ThumbSize::ALL.into_iter().find(|size| size_key(*size) == key)
}

/// The saved entry. The engine reads these camelCase keys (`layer_panel_option`), so the entry is not the serialized
/// `LayersSettings`; Thumbnail Contents is not in it, the clip preference holds it.
fn options_value(settings: &LayersSettings) -> Value {
    let o = &settings.options;
    json!({
        "thumbnailSize": size_key(o.thumbnail_size),
        "showFilters": settings.show_filters,
        "useDefaultMasksOnFillLayers": o.use_default_masks_on_fill_layers,
        "expandNewEffects": o.expand_new_effects,
        "addCopyToCopiedLayersAndGroups": o.add_copy_to_copied_layers_and_groups,
        "previewOnCanvasWhenReorderingLayers": o.preview_on_canvas_when_reordering,
        "showLayerMaskBadges": o.show_layer_mask_badges,
    })
}

/// The settings a saved entry holds; a missing key takes its default.
fn settings_of(saved: &Value) -> LayersSettings {
    let mut settings = LayersSettings::default();
    let flag = |key: &str, into: &mut bool| {
        if let Some(on) = saved.get(key).and_then(Value::as_bool) {
            *into = on;
        }
    };
    if let Some(size) = saved.get("thumbnailSize").and_then(Value::as_str).and_then(size_of_key) {
        settings.options.thumbnail_size = size;
    }
    flag("showFilters", &mut settings.show_filters);
    flag("useDefaultMasksOnFillLayers", &mut settings.options.use_default_masks_on_fill_layers);
    flag("expandNewEffects", &mut settings.options.expand_new_effects);
    flag("addCopyToCopiedLayersAndGroups", &mut settings.options.add_copy_to_copied_layers_and_groups);
    flag("previewOnCanvasWhenReorderingLayers", &mut settings.options.preview_on_canvas_when_reordering);
    flag("showLayerMaskBadges", &mut settings.options.show_layer_mask_badges);
    settings
}

/// Save independently of Workspace > Remember Workspace Changes, as Photoshop does. A pick of the
/// current value writes nothing unless the saved entry is missing or stale.
pub fn persist_options(app: &mut PhotocraftApp) {
    let value = options_value(&app.ui.layers_settings);
    if app.session.prefs().dialogs.get(OPTIONS_PREF) == Some(&value) {
        return;
    }
    app.session.prefs.edit(|p| {
        p.dialogs.insert(OPTIONS_PREF.into(), value);
    });
}

/// Restore independently from a saved workspace. Older preferences retain the defaults.
pub fn restore_options(app: &mut PhotocraftApp) {
    let saved = app.session.prefs().dialogs.get(OPTIONS_PREF).map(settings_of);
    if let Some(settings) = saved {
        app.ui.layers_settings = settings;
    }
}

/// Thumbnail Contents as the shared preference has it: Layer Bounds is Clip Thumbnails to Layer
/// Bounds (#4281), which frames each layer's pixels instead of the canvas.
pub fn thumbnail_contents(app: &PhotocraftApp) -> ThumbContents {
    if app.session.prefs().interface.clip_thumbnails_to_layer_bounds { ThumbContents::LayerBounds } else { ThumbContents::EntireDocument }
}

/// Writes Thumbnail Contents into the clip preference when it differs.
fn set_contents(app: &mut PhotocraftApp, contents: ThumbContents) -> Result<(), String> {
    if contents == thumbnail_contents(app) {
        return Ok(());
    }
    app.run("prefs.set", json!({"path": CLIP_PREF, "value": contents == ThumbContents::LayerBounds})).map(|_| ())
}

fn report(app: &mut PhotocraftApp, result: Result<(), String>) {
    if let Err(e) = result {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The rows of the Layers panel menu, above the framework's own rows (Filter Options, Panel Options…, Collapse All Groups,
/// Clip Thumbnails to Layer Bounds). Animation Options is not offered: PhotoCraft has no frame-unification buttons.
pub(crate) fn panel_rows(app: &PhotocraftApp) -> Vec<PanelRow> {
    let command = |id: &str| PanelRow::Command(CommandRef::new(id));
    let any_hidden = app.session.active().is_some_and(|st| st.selected_layers().iter().any(|id| st.doc.layer(*id).is_some_and(|l| !l.visible)));
    let mut rows = vec![
        command("layer.new.layer"),
        command("layer.new.group"),
        // Duplicate Layer… opens the Duplicate Layer dialog, like Layer › Duplicate Layer… (#14615).
        command(DUPLICATE),
        command("layer.new.groupFromLayers"),
        PanelRow::Separator,
        command("layer.flattenImage"),
        PanelRow::Separator,
        PanelRow::Command(CommandRef::from(LayersMenuCommand::LockLayersDialog)),
        command("layer.renameLayer"),
        command(if any_hidden { "layer.showLayers" } else { "layer.hideLayers" }),
        PanelRow::Separator,
        command("layer.new.artboard"),
        PanelRow::Separator,
    ];
    let words = Env::of_root(app.ctx()).words::<LayersPanelWords>();
    rows.extend(craft::layers::ui::panel::panel_rows(&words));
    rows
}

/// The states of the framework's settings rows: Show Filters and Clip Thumbnails are checked when on. Duplicate Layer…
/// reads Duplicate Layers… with several layers selected (#14615).
pub(crate) fn command_states(app: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    let row = |command: LayersPanelCommands, state: CommandState| (CommandRef::from(command), state);
    let mut states = vec![
        row(LayersPanelCommands::ShowFilters, CommandState::new().checked(app.ui.layers_settings.show_filters)),
        row(LayersPanelCommands::ClipThumbnails, CommandState::new().checked(thumbnail_contents(app) == ThumbContents::LayerBounds)),
        row(LayersPanelCommands::CollapseAll, CommandState::new().enabled(app.session.is_enabled("layer.setExpanded"))),
        (CommandRef::from(LayersMenuCommand::LockLayersDialog), CommandState::new().enabled(app.session.is_enabled(LOCK_LAYERS))),
    ];
    if app.session.active().is_some_and(|st| st.selected_layers().len() > 1) {
        let duplicate = CommandState::new().enabled(crate::menus::is_enabled(app, DUPLICATE)).title(tl!("Duplicate Layers…"));
        states.push((CommandRef::new(DUPLICATE), duplicate));
    }
    states
}

/// What [`command_states`] reads: the rules (the document's selection among them), the filter bar, the clip
/// preference's value and the UI language, which Duplicate Layers…' title is translated into.
#[derive(craft::ui::contract::MemoProps)]
struct StatesFacts<'a> {
    rules: crate::menus::facts::RuleFacts<'a>,
    language: &'static str,
    show_filters: bool,
    clip: bool,
}

/// [`command_states`], derived from [`StatesFacts`].
pub(crate) fn source_states(app: &PhotocraftApp, name: &'static str) -> std::sync::Arc<crate::app::framework::SourceStates> {
    let facts = StatesFacts {
        rules: crate::menus::facts::RuleFacts::of(app),
        language: crate::i18n::current().code(),
        show_filters: app.ui.layers_settings.show_filters,
        clip: app.session.prefs().interface.clip_thumbnails_to_layer_bounds,
    };
    crate::app::framework::derived_states(app, name, &facts, || command_states(app))
}

/// Runs a fired settings command of the panel menu. `false` for every other command: PhotoCraft's `layer.arrange.*`
/// bindings stay the one path of the stacking keys, and the catalogue commands run through the menus.
pub(crate) fn run_fired(app: &mut PhotocraftApp, _ctx: &egui::Context, command: &CommandRef) -> bool {
    if LayersMenuCommand::from_ref(command).is_some() {
        open_lock_layers(app);
        return true;
    }
    let Some(which) = LayersPanelCommands::from_ref(command) else { return false };
    // The field is set only in a panel pass, so it is read from the preference before a toggle.
    app.ui.layers_settings.options.thumbnail_contents = thumbnail_contents(app);
    let Some(action) = which.settings_action(&mut app.ui.layers_settings) else { return false };
    match action {
        LayersMenuAction::SettingsChanged => {
            persist_options(app);
            if which == LayersPanelCommands::ClipThumbnails {
                let contents = app.ui.layers_settings.options.thumbnail_contents;
                let result = set_contents(app, contents);
                report(app, result);
            }
        }
        LayersMenuAction::OpenOptions => {
            open_options(app);
        }
        LayersMenuAction::CollapseAll => {
            let result = app.run("layer.setExpanded", json!({"all": true, "expanded": false})).map(|_| ());
            report(app, result);
        }
        _ => {}
    }
    true
}

/// Opens the Panel Options dialog over the current options.
pub(crate) fn open_options(app: &mut PhotocraftApp) -> DialogId {
    let mut options = app.ui.layers_settings.options.clone();
    options.thumbnail_contents = thumbnail_contents(app);
    app.framework.dialogs.open(LayersOptionsDialog::new(options))
}

/// Takes the Panel Options dialog's answer: OK saves the options and sets Thumbnail Contents in the shared preference;
/// Cancel changes nothing.
pub(crate) fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    let Some(DialogResult::Ok(options)) = app.framework.dialogs.take::<LayersOptionsDialog>() else { return };
    let contents = options.thumbnail_contents;
    app.ui.layers_settings.options = options;
    persist_options(app);
    let result = set_contents(app, contents);
    report(app, result);
}

/// The Lock Layers dialog's check boxes: parameter key, label, opening value.
fn lock_params() -> Vec<Param> {
    vec![
        Param::new("transparency", tl!("Transparency"), ParamKind::bool(false)),
        Param::new("pixels", tl!("Image"), ParamKind::bool(false)),
        Param::new("position", tl!("Position"), ParamKind::bool(false)),
        Param::new("artboard", tl!("Prevent Auto-Nesting"), ParamKind::bool(false)),
        Param::new("all", tl!("All"), ParamKind::bool(true)),
    ]
}

/// Opens the Lock Layers dialog with All ticked ([`LayersMenuCommand::LockLayersDialog`]).
pub(crate) fn open_lock_layers(app: &mut PhotocraftApp) -> DialogId {
    let dialog = HostedParamDialog::new(app.ctx(), LOCK_LAYERS, DialogFields::new(), |fields| {
        ParamDialog::new(LOCK_LAYERS, tl!("Lock Layers"), lock_params()).values(Bind::bound(fields))
    });
    app.framework.dialogs.open(dialog)
}

/// Whether the parameter dialog `key` is the Lock Layers dialog (its `ParamFamily` row).
pub(crate) fn owns(key: &str) -> bool {
    key == LOCK_LAYERS
}

/// Runs the Lock Layers dialog's OK.
pub(crate) fn run(app: &mut PhotocraftApp, _ctx: &egui::Context, result: ParamResult) {
    let on = |key: &str| result.fields.bool(key).unwrap_or(false);
    let params = json!({"transparency": on("transparency"), "pixels": on("pixels"), "position": on("position"), "artboard": on("artboard"), "all": on("all")});
    let outcome = app.run(LOCK_LAYERS, params).map(|_| ());
    report(app, outcome);
}

/// Preview cache recomputes only when hovering a new target, not each pointer frame.
/// Compositor sees it as a temporary document and never commits it to history.
pub(crate) struct ReorderPreview {
    pub source: photocraft_doc::DocId,
    pub revision: u64,
    pub signature: u64,
    pub key: u64,
    pub document: std::sync::Arc<Document>,
}

/// Cache the on-canvas preview of the layer drop in `payload` (a `layer.moveTo` request), unless Preview on Canvas is off.
/// Marks the destination as hovered every frame; the preview itself is rebuilt only when the target changes.
pub(crate) fn preview_reorder(app: &mut PhotocraftApp, payload: &serde_json::Value) {
    if !app.ui.layers_settings.options.preview_on_canvas_when_reordering {
        app.reorder_preview = None;
        return;
    }
    // Mark the destination as hovered even if the cached preview already matches.
    app.reorder_preview_hit = true;
    let Some(st) = app.session.active() else { return };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    payload.to_string().hash(&mut hasher);
    st.doc.id.0.hash(&mut hasher);
    st.revision.hash(&mut hasher);
    let signature = hasher.finish();
    if app.reorder_preview.as_ref().is_some_and(|cache| cache.signature == signature) {
        return;
    }
    let (source, revision) = (st.doc.id, st.revision);
    let add_copy = app.ui.layers_settings.options.add_copy_to_copied_layers_and_groups;
    let preview = photocraft_engine::layer_multi_cmds::preview_move(&st.doc, payload, add_copy);
    app.reorder_preview = preview.map(|document| ReorderPreview {
        source,
        revision,
        signature,
        key: (1u64 << 60) | (signature & ((1u64 << 60) - 1)),
        document: std::sync::Arc::new(document),
    });
}

/// The cached reorder preview and its compositor cache key for document `index`, if it was built for that document's current revision.
pub(crate) fn display_reorder(app: &PhotocraftApp, index: usize) -> Option<(std::sync::Arc<Document>, u64)> {
    if !app.ui.layers_settings.options.preview_on_canvas_when_reordering {
        return None;
    }
    let st = app.session.documents().get(index)?;
    let cached = app.reorder_preview.as_ref()?;
    (cached.source == st.doc.id && cached.revision == st.revision).then(|| (cached.document.clone(), cached.key))
}

#[cfg(test)]
#[path = "layers_menu_duplicate_tests.rs"]
mod layers_menu_duplicate_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_engine::Session;

    fn app_with_document() -> Box<PhotocraftApp> {
        let mut session = Session::new();
        session.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        PhotocraftApp::new(session, crate::Services::default())
    }

    fn fired(command: LayersPanelCommands) -> CommandRef {
        CommandRef::from(command)
    }

    /// #4281: Thumbnail Contents and Clip Thumbnails to Layer Bounds are one setting: the menu row and the dialog's OK
    /// both set the preference, and the saved entry never holds it.
    #[test]
    fn thumbnail_contents_is_clip_thumbnails_to_layer_bounds() {
        let mut app = app_with_document();
        let ctx = app.ctx().clone();
        assert_eq!(thumbnail_contents(&app), ThumbContents::EntireDocument);
        assert!(run_fired(&mut app, &ctx, &fired(LayersPanelCommands::ClipThumbnails)));
        assert!(app.session.prefs().interface.clip_thumbnails_to_layer_bounds);
        assert_eq!(thumbnail_contents(&app), ThumbContents::LayerBounds);
        assert!(app.session.prefs().dialogs[OPTIONS_PREF].get("thumbnailContents").is_none(), "saved once, in the shared preference");
        // The dialog opens on Layer Bounds; the menu row then clears the preference; the dialog's OK sets it again.
        open_options(&mut app);
        assert!(run_fired(&mut app, &ctx, &fired(LayersPanelCommands::ClipThumbnails)));
        assert!(!app.session.prefs().interface.clip_thumbnails_to_layer_bounds);
        assert!(app.framework.dialogs.confirm::<LayersOptionsDialog>(&ctx));
        take(&mut app, &ctx);
        assert!(app.session.prefs().interface.clip_thumbnails_to_layer_bounds);
        assert!(app.session.prefs().dialogs[OPTIONS_PREF].get("thumbnailContents").is_none());
    }

    /// With the option off a new fill layer has no mask and the mask is not targeted; with it on the new layer's mask is targeted.
    #[test]
    fn disabling_default_fill_masks_never_targets_an_absent_mask() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        app.ui.layers_settings.options.use_default_masks_on_fill_layers = false;
        persist_options(&mut app);
        app.run("layer.newFillLayer.solidColor", json!({"color": "#778899"})).unwrap();
        let layer = app.session.active().and_then(|st| st.active_layer.and_then(|id| st.doc.layer(id))).unwrap();
        assert!(layer.mask.is_none());
        assert!(!app.ui.mask_target && !app.ui.vector_mask_target);
        app.ui.layers_settings.options.use_default_masks_on_fill_layers = true;
        persist_options(&mut app);
        app.run("layer.newFillLayer.gradient", json!({})).unwrap();
        let layer = app.session.active().and_then(|st| st.active_layer.and_then(|id| st.doc.layer(id))).unwrap();
        assert!(layer.mask.is_some());
        assert!(app.ui.mask_target);
    }

    /// Every switch goes to the preferences under the camelCase keys the engine reads, and comes back from them; a
    /// missing entry is written again.
    #[test]
    fn prefs_options_save_and_restore_all_switches() {
        let mut app = app_with_document();
        let mut saved = LayersSettings::default();
        saved.options.thumbnail_size = ThumbSize::Large;
        saved.show_filters = false;
        saved.options.use_default_masks_on_fill_layers = false;
        saved.options.expand_new_effects = false;
        saved.options.add_copy_to_copied_layers_and_groups = false;
        saved.options.preview_on_canvas_when_reordering = false;
        saved.options.show_layer_mask_badges = true;
        app.ui.layers_settings = saved.clone();
        persist_options(&mut app);
        let entry = app.session.prefs().dialogs[OPTIONS_PREF].clone();
        assert_eq!(entry["showLayerMaskBadges"], true);
        assert_eq!(entry["thumbnailSize"], "large");
        assert_eq!(entry["previewOnCanvasWhenReorderingLayers"], false);
        assert_eq!(entry["useDefaultMasksOnFillLayers"], false);
        app.ui.layers_settings = LayersSettings::default();
        restore_options(&mut app);
        assert_eq!(app.ui.layers_settings, saved);
        app.session.prefs.edit(|p| {
            p.dialogs.remove(OPTIONS_PREF);
        });
        persist_options(&mut app);
        assert_eq!(app.session.prefs().dialogs.get(OPTIONS_PREF), Some(&entry), "a missing entry is written again");
    }

    /// Preferences saved before an option existed still load, with the default for the missing ones.
    #[test]
    fn panel_options_missing_fields_deserialize_with_defaults() {
        let mut app = app_with_document();
        for (entry, size) in [(json!({}), ThumbSize::Medium), (json!({"thumbnailSize": "large"}), ThumbSize::Large)] {
            app.session.prefs.edit(|p| {
                p.dialogs.insert(OPTIONS_PREF.into(), entry.clone());
            });
            app.ui.layers_settings = LayersSettings::default();
            app.ui.layers_settings.show_filters = false;
            restore_options(&mut app);
            let mut want = LayersSettings::default();
            want.options.thumbnail_size = size;
            assert_eq!(app.ui.layers_settings, want, "{entry}");
        }
    }

    fn groups_open(s: &Session) -> Vec<bool> {
        s.active()
            .unwrap()
            .doc
            .walk()
            .into_iter()
            .filter_map(|(_, _, l)| if let photocraft_doc::LayerContent::Group(g) = &l.content { Some(g.expanded) } else { None })
            .collect()
    }

    /// Two open groups, one inside the other, over "Loose".
    fn nested() -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Inner"})).unwrap();
        s.execute("layer.groupLayers", json!({"name": "In"})).unwrap();
        s.execute("layer.groupLayers", json!({"name": "Out"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Loose"})).unwrap();
        s.execute("layer.setExpanded", json!({"all": true, "expanded": true})).unwrap();
        s
    }

    /// #143: Collapse All Groups closes every group, and the closed state survives a PSD and a .pcraft round trip.
    #[test]
    fn collapse_all_groups_closes_every_group_and_it_is_saved() {
        let mut app = PhotocraftApp::new(nested(), crate::Services::default());
        let ctx = app.ctx().clone();
        assert!(groups_open(&app.session).iter().all(|o| *o));
        assert!(run_fired(&mut app, &ctx, &fired(LayersPanelCommands::CollapseAll)));
        let open = groups_open(&app.session);
        assert_eq!(open.len(), 2);
        assert!(open.iter().all(|o| !o), "every group closed: {open:?}");
        let doc = (*app.session.active().unwrap().doc).clone();
        for name in ["c.psd", "c.pcraft"] {
            let bytes = photocraft_io::export(&doc, name, &Default::default()).unwrap().bytes;
            let back = photocraft_io::import(name, &bytes).unwrap().document;
            let mut s = Session::new();
            s.add_document(back, None);
            assert!(groups_open(&s).iter().all(|o| !o), "{name}: groups stay closed");
        }
    }

    /// Photoshop's panel menu has New Group… and New Group from Layers… just above Collapse All Groups; both open the
    /// one New Group dialog.
    #[test]
    fn the_new_group_rows_open_the_one_new_group_dialog() {
        let app = PhotocraftApp::new(nested(), crate::Services::default());
        let rows = panel_rows(&app);
        let at = |id: &str| rows.iter().position(|r| matches!(r, PanelRow::Command(c) if c.id == id)).unwrap_or_else(|| panic!("no row {id}"));
        let collapse = at(fired(LayersPanelCommands::CollapseAll).id.as_str());
        for command in [crate::new_group_ui::NEW_GROUP, crate::new_group_ui::FROM_LAYERS] {
            assert!(at(command) < collapse, "{command} comes before Collapse All Groups");
            let mut app = PhotocraftApp::new(nested(), crate::Services::default());
            let app_ctx = app.ctx().clone();
            crate::menus::invoke(&mut app, &app_ctx, command, json!({})).unwrap();
            assert!(app.dialog_open(), "{command} opens the New Group dialog");
        }
    }
}
