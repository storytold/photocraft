//! What the menu model shows, as the plain values and revisions it is built from: [`super::items::model`] builds the
//! model again only when these change. A rule the rows read is listed here through the signal of the state it reads:
//! a `Tracked` value's or a framework value's revision, an engine marker, or the plain value itself when it is small.
//! Nothing here grows with the documents, presets, recent files or layers, so an idle frame keys the same few bytes
//! whatever the model's size.

use std::sync::Arc;

use craft::ui::contract::{MemoProps, Revision, Tracked};
use photocraft_doc::DocId;
use photocraft_engine::DocState;

use crate::PhotocraftApp;

/// The facts of one state of the app ([`MenuFacts::of`]).
#[derive(MemoProps)]
pub(crate) struct MenuFacts<'a> {
    /// The UI language: labels and titles are translated when the model is built.
    language: &'static str,
    /// The keymap's overrides and catalog generations: the rows show its chords.
    keymap: (u64, u64),
    rules: RuleFacts<'a>,
    prefs: PrefFacts<'a>,
    shown: ShownFacts<'a>,
}

/// What the enable rules read ([`super::is_enabled`], `Session::is_enabled`): the documents, the session and the shell
/// state they ask about. A command source whose states follow the rules holds these; the menu model holds them with
/// what its rows show.
#[derive(MemoProps)]
pub(crate) struct RuleFacts<'a> {
    documents: Documents<'a>,
    session: SessionFacts,
    ui: UiRuleFacts<'a>,
}

/// The open documents and the active one.
#[derive(MemoProps)]
struct Documents<'a> {
    /// Moves when a document opens, closes or is renamed (the Window menu lists them by name).
    generation: u64,
    active_generation: u64,
    active_index: Option<usize>,
    /// The active document, by its `Tracked` revision, which moves with every change to it: its content, layers,
    /// selection, history, mode, mask and view state, which most rules read.
    active: Option<&'a Tracked<DocState>>,
    /// Every open document's revision in tab order, folded into one value: rules that read other documents than the
    /// active one (Merge Channels needs grayscale documents of the active one's size) follow their changes (#16252).
    all: u64,
    /// The scratch read-error generation: a tile that fails to read shows zeros through `&self`, and the real pixels
    /// again once it reads, without moving its document's revision. Rules that read pixels (the align commands read
    /// content bounds) follow both (#16252).
    scratch: u64,
    /// A running job greys the commands that would change its document.
    jobs: (bool, Option<u64>),
    /// Smart Objects › Edit Contents' parent: whether the active document is a smart object's contents.
    smart_child: bool,
    /// Edit › Fade: the document and revision of the step it fades.
    #[memo_props(hash = "a plain model id")]
    fade: Option<(DocId, u64)>,
    /// The proof setups (View › Proof Colors, Gamut Warning, the simulations).
    proofs: Revision,
    /// File › Generate › Image Assets and View › Lock Slices.
    image_assets: &'a Tracked<Vec<DocId>>,
    slices_locked: bool,
}

/// Session state outside the documents that rules read.
#[derive(MemoProps)]
struct SessionFacts {
    clipboard: Revision,
    style_clipboard: Revision,
    /// The path fill and stroke clipboards: Paste Shape Fill and Stroke.
    path_clipboards: (bool, bool),
    presets: Revision,
    no_patterns: bool,
    type_defaults: bool,
    measurement_log: bool,
    mask_targeted: bool,
    /// The Actions library: the list, recording and playing (library edits are undone from Edit › Undo).
    actions: (u64, Option<(usize, usize)>, u8),
    /// Edit › Undo's label when it undoes an Actions library change.
    undo_label: Option<&'static str>,
    /// The installed plug-ins' revision: a plug-in's menu item needs it installed.
    plugins: u64,
}

/// The preferences the rows show.
#[derive(MemoProps)]
struct PrefFacts<'a> {
    language: &'a str,
    hidden: &'a Vec<String>,
    colors: &'a std::collections::BTreeMap<String, String>,
    show_menu_colors: bool,
    quick_export: &'static str,
    font_preview: &'static str,
    generator: bool,
    appearance: &'static str,
    favorite_themes: &'a Vec<String>,
}

/// The shell's state the rules read.
#[derive(MemoProps)]
struct UiRuleFacts<'a> {
    tool: u64,
    /// The open Free Transform: its undo steps' and phase's revision, Content-Aware Scale, its layer count and a
    /// running commit.
    transform: Option<(u64, bool, usize, bool)>,
    /// The typed text being edited: Cut and Copy follow its selection, the Type menu its style.
    text_edit: Option<&'a crate::state::TextEdit>,
    /// The Pen draft's and the Crop tool's own Undo and Redo, and whether a crop is pending.
    pen: (Option<bool>, Option<bool>),
    crop: (Option<bool>, Option<bool>, bool),
    /// Mask targets the rules fill into a command's target, and a multi-channel colour target.
    targets: (bool, bool, bool, bool),
    /// Symmetry › Transform.
    symmetry: bool,
    /// The modal state the rows read: open dialogs, the discard prompt, a distort workspace, the shortcut recorder,
    /// a brush resize.
    modal: (usize, bool, Option<&'static str>, bool, bool),
    selected_guide: bool,
    /// Save, Paste and Place Embedded ask which services this platform has: export, the clipboard image and files.
    services: (bool, bool, bool),
    /// The locked workspace greys the panel commands that move, float or close panels.
    workspaces_locked: bool,
    /// Window › Arrange: the docked and floating document windows.
    document_windows: (usize, usize),
    /// File › Automate › Batch: an action is selected in the Actions panel.
    batch_action: bool,
}

/// The shell's state the rows show beside the rules: ticks, labels and the lists they name.
#[derive(MemoProps)]
struct ShownFacts<'a> {
    extras: crate::state::Extras,
    view: &'a crate::view_cmds::ViewOptions,
    /// The options, status, menu and contextual task bars.
    bars: (bool, bool, bool, bool),
    palette: bool,
    theme: (&'a str, bool),
    #[memo_props(hash = "craft-behavior's document arrangement: a fieldless enum")]
    arrangement: Option<craft::behavior::dock::Arrangement>,
    /// The Window menu's ticks: which panels are shown ([`window_ticks`]).
    ticks: Arc<Vec<bool>>,
    recent_files: &'a Tracked<Vec<String>>,
    workspaces: &'a Tracked<crate::workspace_ui::Workspaces>,
}

impl<'a> MenuFacts<'a> {
    /// The facts of `app`'s current state.
    pub(crate) fn of(app: &'a PhotocraftApp) -> MenuFacts<'a> {
        let prefs = app.session.prefs();
        let ui = &app.ui;
        MenuFacts {
            language: crate::i18n::current().code(),
            keymap: app.framework.keymap_inputs(),
            rules: RuleFacts::of(app),
            prefs: PrefFacts {
                language: &prefs.interface.language,
                hidden: &prefs.menus.hidden,
                colors: &prefs.menus.colors,
                show_menu_colors: prefs.interface.show_menu_colors,
                quick_export: prefs.export.quick_export_format.name(),
                font_preview: prefs.type_.font_preview.name(),
                generator: prefs.plug_ins.generator_enabled,
                appearance: prefs.interface.appearance_mode.name(),
                favorite_themes: &prefs.interface.favorite_themes,
            },
            shown: ShownFacts {
                extras: ui.extras,
                view: &ui.view,
                bars: (ui.panels.options_bar, ui.panels.status_bar, ui.panels.menu_bar, ui.panels.context_task_bar),
                palette: ui.palette.open,
                theme: (ui.theme.id().as_str(), ui.theme.is_dark()),
                arrangement: ui.document_windows.arrangement,
                ticks: window_ticks(app),
                recent_files: &ui.recent_files,
                workspaces: &app.workspaces,
            },
        }
    }
}

/// The revisions of `documents` in their order, folded into one value that moves when any of them changes or they
/// are reordered. A fixed size, so the key is the same few bytes however many documents are open.
fn all_revisions(documents: &[Tracked<DocState>]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    for document in documents {
        document.revision().hash(&mut hasher);
    }
    hasher.finish()
}

impl<'a> RuleFacts<'a> {
    /// What the rules read in `app`'s current state.
    pub(crate) fn of(app: &'a PhotocraftApp) -> RuleFacts<'a> {
        let s = &app.session;
        let active = s.active();
        let ui = &app.ui;
        RuleFacts {
            documents: Documents {
                generation: s.documents_generation(),
                active_generation: s.active_generation(),
                active_index: s.active_index(),
                active: s.active_tracked(),
                all: all_revisions(s.documents()),
                scratch: crate::scratch_reads::generation(),
                jobs: (s.has_jobs(), s.active_job().map(|job| job.id.0)),
                smart_child: active.is_some_and(|d| s.smart_links.iter().any(|link| link.child == d.doc.id)),
                fade: s.edit_state.fade.as_ref().map(|fade| (fade.doc, fade.revision)),
                proofs: s.color.proofs_revision(),
                image_assets: &s.file_menu.image_assets,
                slices_locked: s.file_menu.slices_locked,
            },
            session: SessionFacts {
                clipboard: s.clipboard.revision(),
                style_clipboard: s.style_clipboard.revision(),
                path_clipboards: (s.path_fill_clipboard.is_some(), s.path_stroke_clipboard.is_some()),
                presets: s.presets.revision(),
                no_patterns: s.patterns.items.is_empty(),
                type_defaults: s.type_defaults.is_some(),
                measurement_log: s.analysis.log.is_empty(),
                mask_targeted: s.tools.mask_targeted,
                actions: (s.actions.list.revision(), s.actions.recording, s.actions.playing),
                undo_label: crate::actions::library::edit_undo_label(app),
                plugins: photocraft_plugins::registry::revision(),
            },
            ui: UiRuleFacts {
                tool: ui.tool as u64,
                transform: ui
                    .transform
                    .as_ref()
                    .map(|t| (t.session.revision(), t.content_aware.is_some(), t.layers.len(), crate::transform_tool::committing(app).is_some())),
                text_edit: ui.text_edit.as_ref(),
                pen: (crate::vector_ui::pen_history_enabled(app, "edit.undo"), crate::vector_ui::pen_history_enabled(app, "edit.redo")),
                crop: (crate::crop_ui::is_enabled(app, "edit.undo"), crate::crop_ui::is_enabled(app, "edit.redo"), crate::crop_ui::pending(app)),
                targets: (app.filter_mask_targeted(), app.vector_mask_targeted(), app.layer_mask_targeted(), crate::canvas::multiple_color_target(app)),
                symmetry: crate::symmetry_ui::can_transform(app),
                modal: (
                    app.dialog_count(),
                    app.discard.is_some(),
                    crate::distort_ui::modal_workspace(app),
                    crate::prefs_ui::recording(app),
                    app.brush_resize.active(),
                ),
                selected_guide: crate::guide_select::selected(app).is_some(),
                services: (app.services.export.is_some(), app.services.clipboard_get_image.is_some(), app.services.files.is_some()),
                workspaces_locked: app.workspaces.set.locked,
                document_windows: (ui.document_windows.docked.len(), ui.document_windows.floating.len()),
                batch_action: crate::actions::selected_action(app).is_some(),
            },
        }
    }
}

/// Panels whose ticks the rows read besides the registry's: the dock, the Tools and Options bars, and panels the
/// analysis and workspace checks name.
const OTHER_PANELS: [&str; 6] = ["dock", "tools", "options", "modifierKeys", crate::analysis::MEASUREMENT_LOG, crate::analysis::NOTES];

/// Which panels the Window menu ticks, derived from the dock: its revision moves with every write (a column resize
/// included), so the ticks are worked out again then, and the menu model only when one of them changed.
fn window_ticks(app: &PhotocraftApp) -> Arc<Vec<bool>> {
    let key = (app.ui.docking.revision(), app.ui.flyout.as_deref());
    craft::ui::runtime::derived::derived(app.ctx(), egui::Id::unique("photocraft-window-ticks"), &key, || {
        let registry = crate::panels::registry::PANELS.iter().map(|panel| crate::panel_docking::is_checked(app, panel.id));
        let others = OTHER_PANELS.iter().flat_map(|panel| [crate::panel_docking::is_checked(app, panel), crate::panel_docking::is_shown(app, panel)]);
        registry.chain(others).collect()
    })
}

/// The key the model is derived on, for tests that check a change reaches the menus.
#[cfg(test)]
pub(crate) fn key(app: &PhotocraftApp) -> u128 {
    craft::ui::contract::MemoKey::of(&MenuFacts::of(app))
}
