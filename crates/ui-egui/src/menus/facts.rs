//! What the menu model shows, as the plain values and revisions it is built from: [`super::items::model`] builds the
//! model again only when these change. A rule the rows read is listed here through the signal of the state it reads:
//! a `Tracked` value's or a framework value's revision, an engine marker, or the plain value itself when it is small.
//! Nothing here grows with the documents, presets, recent files or layers, so an idle frame keys the same few bytes
//! whatever the model's size.

use std::sync::Arc;

use craft::ui::contract::{MemoProps, Revision, Tracked};
use photocraft_doc::DocId;

use crate::PhotocraftApp;

/// The facts of one state of the app ([`MenuFacts::of`]).
#[derive(MemoProps)]
pub(crate) struct MenuFacts<'a> {
    /// The UI language: labels and titles are translated when the model is built.
    language: &'static str,
    /// The keymap's overrides and catalog generations: the rows show its chords.
    keymap: (u64, u64),
    documents: Documents<'a>,
    session: SessionFacts,
    prefs: PrefFacts<'a>,
    ui: UiFacts<'a>,
}

/// The open documents and the active one.
#[derive(MemoProps)]
struct Documents<'a> {
    /// Moves when a document opens, closes or is renamed (the Window menu lists them by name).
    generation: u64,
    active_generation: u64,
    active_index: Option<usize>,
    /// The active document and its revision, which moves with every change to it: its content, layers, selection,
    /// history, mode and mask state, which most rules read.
    #[memo_props(hash = "a plain model id")]
    active: Option<(DocId, u64)>,
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

/// The shell's state the rows show.
#[derive(MemoProps)]
struct UiFacts<'a> {
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
    /// Save and Paste rules ask which services this platform has.
    services: (bool, bool),
}

impl<'a> MenuFacts<'a> {
    /// The facts of `app`'s current state.
    pub(crate) fn of(app: &'a PhotocraftApp) -> MenuFacts<'a> {
        let s = &app.session;
        let active = s.active();
        let prefs = s.prefs();
        let ui = &app.ui;
        MenuFacts {
            language: crate::i18n::current().code(),
            keymap: app.framework.keymap_inputs(),
            documents: Documents {
                generation: s.documents_generation(),
                active_generation: s.active_generation(),
                active_index: s.active_index(),
                active: active.map(|d| (d.doc.id, d.revision)),
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
            },
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
            ui: UiFacts {
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
                extras: ui.extras,
                view: &ui.view,
                bars: (ui.panels.options_bar, ui.panels.status_bar, ui.panels.menu_bar, ui.panels.context_task_bar),
                palette: ui.palette.open,
                theme: (ui.theme.id().as_str(), ui.theme.is_dark()),
                arrangement: ui.document_windows.arrangement,
                ticks: window_ticks(app),
                recent_files: &ui.recent_files,
                workspaces: &app.workspaces,
                services: (app.services.export.is_some(), app.services.clipboard_get_image.is_some()),
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
