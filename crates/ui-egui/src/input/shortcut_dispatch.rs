//! Keyboard shortcut catalog and the one drain of fired commands (#127): every shortcut the menus
//! display fires.
//!
//! The menus show Photoshop's default shortcuts from [`crate::menu_catalog::CATALOG`], while
//! commands also declare their own (the engine's command table, [`crate::menus::UI_COMMANDS`]).
//! [`command_rows`] merges all of them into the app's command catalog; the runtime's keymap
//! binds the catalog's default keys and [`overrides`] carries the user's changes from Edit ›
//! Keyboard Shortcuts, so a shortcut printed next to a live menu item always reaches that item.
//! The runtime takes the keys and reports the commands they fired; [`run_fired`] runs them. A
//! shortcut whose command is disabled in the current state reports why on the status bar instead
//! of doing nothing silently, as the greyed menu item shows.

use photocraft_engine::actions_cmds::ActionsExt;
use std::collections::HashMap;

use rustc_hash::FxHashMap;

use craft::color_ui::ui::SwatchesCommand;
use craft::keymap::{Binding, Chord, CommandDef, CommandKind, Overrides, Scope};
use craft::layers::ui::panel::LayersPanelCommands;
use craft::ui::canvas::{MomentaryCommands, PickStep, ViewCommands, tool_pick};
use craft::ui::docking::WorkspaceCommands;
use craft::ui::rails::{LastUsed, RailTool};
use craft::ui::runtime::commands::{AppCatalog, ChromeCommand, CommandRef, CommandSet, EditCommands, FirePhase, FireSource, Fired};
use craft::ui::runtime::frame::ChromeToggle;
use craft::ui::shell::ScreenModeCommands;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// What a dispatched shortcut did (kept for tests and `ui.inspect`-style debugging).
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// The command ran.
    Ran,
    /// The command is disabled in the current state; the reason.
    Disabled(String),
    /// The command ran and failed; the message.
    Failed(String),
}

fn log_id() -> egui::Id {
    egui::Id::unique("pc-shortcut-log")
}

fn dry_run_id() -> egui::Id {
    egui::Id::unique("pc-shortcut-dry-run")
}

/// Tests: resolve and check shortcuts without running them (File › Exit, Open, browser links…).
pub fn set_dry_run(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(dry_run_id(), on));
}

/// The shortcuts dispatched since the last call (command id and outcome), oldest first.
pub fn take_log(ctx: &egui::Context) -> Vec<(String, Outcome)> {
    ctx.data_mut(|d| d.remove_temp::<Vec<(String, Outcome)>>(log_id())).unwrap_or_default()
}

/// The framework command a PhotoCraft command is: the runtime's own command carries out the row
/// (the chrome toggles, the canvas's view commands, the momentary tools), and the user's keys for the
/// PhotoCraft id bind it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Bridged {
    /// A command the document canvas runs on its view.
    View(ViewCommands),
    /// A window chrome toggle.
    Chrome(ChromeCommand),
    /// A tool in effect while its key is held.
    Momentary(MomentaryCommands),
    /// A framework edit command that owns the chord of a PhotoCraft action; PhotoCraft runs the action when it fires.
    Edit(EditCommands),
    /// A Layers panel stacking command that owns the chord of a `layer.arrange.*` action; PhotoCraft runs the
    /// action when it fires.
    Stack(LayersPanelCommands),
    /// A Swatches panel menu command under the id the engine's command of that name has; the panel runs it
    /// ([`crate::panels::swatches_ui::run_fired`]).
    Swatches(SwatchesCommand),
    /// A Window › Workspace command under its framework id; the shell runs it ([`crate::workspace_ui::run_fired`]).
    Workspace(WorkspaceCommands),
    /// A View › Screen Mode command: the window's shell takes it and writes `ui.view.screen_mode`, which it binds.
    ScreenMode(ScreenModeCommands),
}

impl Bridged {
    /// The framework command, as the runtime fires and reports it.
    pub(crate) fn command(self) -> CommandRef {
        match self {
            Bridged::View(command) => CommandRef::from(command),
            Bridged::Chrome(command) => CommandRef::from(command),
            Bridged::Momentary(command) => CommandRef::from(command),
            Bridged::Edit(command) => CommandRef::from(command),
            Bridged::Stack(command) => CommandRef::from(command),
            Bridged::Swatches(command) => CommandRef::from(command),
            Bridged::Workspace(command) => CommandRef::from(command),
            Bridged::ScreenMode(command) => CommandRef::from(command),
        }
    }
}

/// PhotoCraft's commands the craft runtime carries out, with the framework command each one is. Their
/// keys reach the runtime as keymap overrides ([`overrides`]) and the menu bar's rows fire them. App code (the menus
/// PhotoCraft runs, action steps, automation) asks the active document's canvas for a view row's command
/// (`actions::runner::run_view`), which journals it; the other rows it fires through [`fire_bridged`]. The action
/// recorder records the view commands a canvas ran from the runtime ([`record_view_commands`]). They are not rows of
/// [`catalog`]: the runtime defines them.
pub(crate) const RUNTIME_COMMANDS: &[(&str, Bridged)] = &[
    ("window.togglePanels", Bridged::Chrome(ChromeCommand::TogglePanels)),
    ("window.toggle.dock", Bridged::Chrome(ChromeCommand::TogglePanelsKeepTools)),
    ("view.zoomIn", Bridged::View(ViewCommands::ZoomIn)),
    ("view.zoomOut", Bridged::View(ViewCommands::ZoomOut)),
    ("view.fitOnScreen", Bridged::View(ViewCommands::FitOnScreen)),
    ("view.actualPixels", Bridged::View(ViewCommands::ActualPixels)),
    ("view.twoHundredPercent", Bridged::View(ViewCommands::TwoHundredPercent)),
    ("view.printSize", Bridged::View(ViewCommands::PrintSize)),
    ("view.fitArtboardOnScreen", Bridged::View(ViewCommands::FitArtboard)),
    ("view.fitLayersOnScreen", Bridged::View(ViewCommands::FitLayers)),
    ("view.fillScreen", Bridged::View(ViewCommands::FillScreen)),
    ("view.rotateLeft", Bridged::View(ViewCommands::RotateCounterclockwise)),
    ("view.rotateRight", Bridged::View(ViewCommands::RotateClockwise)),
    ("view.resetView", Bridged::View(ViewCommands::ResetRotation)),
    ("tools.temporary.hand", Bridged::Momentary(MomentaryCommands::Hand)),
    ("tools.temporary.zoomIn", Bridged::Momentary(MomentaryCommands::ZoomIn)),
    ("tools.temporary.zoomOut", Bridged::Momentary(MomentaryCommands::ZoomOut)),
    ("select.all", Bridged::Edit(EditCommands::SelectAll)),
    // The ids the framework's sets give these commands are PhotoCraft's own ids, so the engine's rows of the same
    // names are not catalog rows either.
    ("edit.undo", Bridged::Edit(EditCommands::Undo)),
    ("edit.redo", Bridged::Edit(EditCommands::Redo)),
    ("edit.cut", Bridged::Edit(EditCommands::Cut)),
    ("edit.copy", Bridged::Edit(EditCommands::Copy)),
    ("edit.paste", Bridged::Edit(EditCommands::Paste)),
    ("swatches.reset", Bridged::Swatches(SwatchesCommand::Reset)),
    ("swatches.import", Bridged::Swatches(SwatchesCommand::Import)),
    ("swatches.export", Bridged::Swatches(SwatchesCommand::Export)),
    ("workspace.new", Bridged::Workspace(WorkspaceCommands::New)),
    ("workspace.delete", Bridged::Workspace(WorkspaceCommands::Delete)),
    ("workspace.lock", Bridged::Workspace(WorkspaceCommands::Lock)),
    ("workspace.reset", Bridged::Workspace(WorkspaceCommands::Reset)),
    ("layer.arrange.bringForward", Bridged::Stack(LayersPanelCommands::MoveUp)),
    ("layer.arrange.sendBackward", Bridged::Stack(LayersPanelCommands::MoveDown)),
    ("layer.arrange.bringToFront", Bridged::Stack(LayersPanelCommands::MoveToTop)),
    ("layer.arrange.sendToBack", Bridged::Stack(LayersPanelCommands::MoveToBottom)),
    ("view.screenMode.standard", Bridged::ScreenMode(ScreenModeCommands::Standard)),
    ("view.screenMode.fullScreenWithMenuBar", Bridged::ScreenMode(ScreenModeCommands::FullScreenWithMenuBar)),
    ("view.screenMode.fullScreen", Bridged::ScreenMode(ScreenModeCommands::FullScreen)),
    ("view.screenMode.cycle", Bridged::ScreenMode(ScreenModeCommands::Cycle)),
    ("view.screenMode.cycleBack", Bridged::ScreenMode(ScreenModeCommands::CycleBack)),
];

/// The framework command of every row of [`RUNTIME_COMMANDS`], built once. `Bridged::command` allocates the
/// command's id, and the menu bar asks [`bridge_of`] for each of its items on every frame (the command gate's
/// [`screen_mode_row`]): building every row's command there cost about a tenth of the UI pass (#15493).
fn bridged_commands() -> &'static [(CommandRef, &'static str, Bridged)] {
    static TABLE: std::sync::OnceLock<Vec<(CommandRef, &'static str, Bridged)>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| RUNTIME_COMMANDS.iter().map(|(id, bridged)| (bridged.command(), *id, *bridged)).collect())
}

/// The PhotoCraft id and bridge row of a framework command, if a row bridges it.
fn bridge_of(command: &CommandRef) -> Option<(&'static str, Bridged)> {
    bridged_commands().iter().find(|(bridged, _, _)| bridged == command).map(|(_, id, bridged)| (*id, *bridged))
}

/// Runtime rows indexed by id, keeping the first row if an id appears more than once.
fn runtime_by_id() -> &'static FxHashMap<&'static str, Bridged> {
    static TABLE: std::sync::OnceLock<FxHashMap<&'static str, Bridged>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = FxHashMap::default();
        for &(id, bridged) in RUNTIME_COMMANDS {
            table.entry(id).or_insert(bridged);
        }
        table
    })
}

/// Whether the runtime defines command `id` (a row of [`RUNTIME_COMMANDS`]).
pub(crate) fn is_bridged(id: &str) -> bool {
    runtime_by_id().contains_key(id)
}

/// Whether `id` is a view row of [`RUNTIME_COMMANDS`]: a command the document canvas runs on its view.
pub(crate) fn is_view_command(id: &str) -> bool {
    view_command(id).is_some()
}

/// The canvas's view command a view row `id` of [`RUNTIME_COMMANDS`] is.
pub(crate) fn view_command(id: &str) -> Option<ViewCommands> {
    RUNTIME_COMMANDS.iter().find_map(|(row, bridged)| match bridged {
        Bridged::View(command) if *row == id => Some(*command),
        _ => None,
    })
}

/// Whether `id` is a screen-mode row of [`RUNTIME_COMMANDS`]: a command the window's shell runs.
pub(crate) fn is_screen_mode(id: &str) -> bool {
    RUNTIME_COMMANDS.iter().any(|(row, bridged)| *row == id && matches!(bridged, Bridged::ScreenMode(_)))
}

/// The PhotoCraft row of a screen-mode command of the shell: the command gate judges the framework command as the
/// menus judge that row.
pub(crate) fn screen_mode_row(command: &CommandRef) -> Option<&'static str> {
    // The command gate asks this for every command a pass resolves: only the screen-mode rows build their command.
    RUNTIME_COMMANDS.iter().find(|(_, bridged)| matches!(bridged, Bridged::ScreenMode(_)) && bridged.command() == *command).map(|(id, _)| *id)
}

/// The id of the command that steps a tool group to its next tool: the group is named by its key.
pub(crate) fn next_in_group_id(letter: char) -> String {
    format!("tools.nextInGroup.{}", letter.to_ascii_lowercase())
}

/// The tool key a [`next_in_group_id`] names, as `Tool::key` gives it (upper case).
fn next_in_group_letter(id: &str) -> Option<char> {
    let mut chars = id.strip_prefix("tools.nextInGroup.")?.chars();
    let letter = chars.next()?;
    chars.next().is_none().then_some(letter.to_ascii_uppercase())
}

/// A default binding from the text the tables write it in.
fn binding(text: &str) -> Option<Binding> {
    Binding::parse(text).ok()
}

/// The menu path of command `id`: its menu, then its label.
fn menu_path(id: &str) -> Option<Vec<String>> {
    let (menu, label) = menu_paths().get(id)?;
    Some(menu.iter().chain([label]).map(|part| (*part).to_string()).collect())
}

/// Each command's menu and label: the catalogue's first row of the command, else the first shell command of that id
/// that has a menu.
fn menu_paths() -> &'static HashMap<&'static str, (&'static [&'static str], &'static str)> {
    static PATHS: std::sync::OnceLock<HashMap<&'static str, (&'static [&'static str], &'static str)>> = std::sync::OnceLock::new();
    PATHS.get_or_init(|| {
        let mut paths = HashMap::new();
        for &(menu, label, _, id) in crate::menu_catalog::CATALOG {
            paths.entry(id).or_insert((menu, label));
        }
        for &(id, label, menu, _) in crate::menus::UI_COMMANDS.iter().filter(|c| !c.2.is_empty()) {
            paths.entry(id).or_insert((menu, label));
        }
        paths
    })
}

/// Adds `def` unless an earlier source gave its id or the runtime defines it. `index` holds each row's place in
/// `rows` by its id.
fn push_row(rows: &mut Vec<CommandDef>, index: &mut HashMap<String, usize>, def: CommandDef) {
    if !is_bridged(&def.command.id) && !index.contains_key(&def.command.id) {
        index.insert(def.command.id.clone(), rows.len());
        rows.push(def);
    }
}

/// Every row of the app's command catalog, one per command id, first source first: the tools and
/// the held keys, the engine's and shell's commands with their menu paths, the live menu items
/// with no command of their own, the recorded actions, and the alternate keys. Ids the runtime defines
/// ([`RUNTIME_COMMANDS`]) are not rows. The Keyboard Shortcuts dialog lists exactly these rows.
pub(crate) fn command_rows(app: &PhotocraftApp) -> Vec<CommandDef> {
    let mut rows: Vec<CommandDef> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let app_row = |id: &str, label: &str| CommandDef::new(CommandRef::new(id), label, Scope::app());

    // The tool letters, and the Shift+letter step of each group.
    for def in crate::tool_shortcuts::defs(app) {
        push_row(&mut rows, &mut index, def);
    }
    for (letter, first) in crate::tool_shortcuts::group_letters() {
        let label = format!("Next {}", first.label());
        let mut def = app_row(&next_in_group_id(letter), &label).with_path(vec!["Tools".into(), label.clone()]);
        def.defaults.extend(binding(&format!("Shift+{}", letter.to_ascii_uppercase())));
        push_row(&mut rows, &mut index, def);
    }
    // The held erase key is momentary; the hand and zoom keys are the runtime's.
    for &(id, label, key) in photocraft_engine::prefs::TEMPORARY_TOOLS {
        if is_bridged(id) {
            continue;
        }
        let mut def = app_row(id, label).with_kind(CommandKind::Momentary).with_path(vec!["Tools".into(), "Temporary".into(), label.into()]);
        def.defaults.extend(binding(key));
        push_row(&mut rows, &mut index, def);
    }
    // Alt+1 … Alt+0 set the brush smoothing (press only). The opacity and blend digits are command sets of the
    // framework (`OpacityDigit`, `BlendStep`, `BlendShortcut`), so they are no rows here.
    for digit in 1..=10u32 {
        let key = digit % 10;
        let label = format!("Brush Smoothing {}%", digit * 10);
        let mut def = app_row(&format!("tools.smoothing.{key}"), &label).with_path(vec!["Tools".into(), "Smoothing".into(), label.clone()]);
        def.defaults.extend(binding(&format!("Alt+{key}")));
        push_row(&mut rows, &mut index, def);
    }

    // The engine's and the shell's commands, with the menu paths the shell knows.
    // PLAN: `keymap_defs::command_defs` is top-w20's tp.PT25 row, assumed as `command_defs() -> Vec<CommandDef>`.
    for mut def in photocraft_engine::keymap_defs::command_defs() {
        if def.paths.is_empty() {
            def.paths.extend(menu_path(&def.command.id));
        }
        push_row(&mut rows, &mut index, def);
    }

    // Live menu items whose command has no row yet carry the catalogue's key.
    for &(_, label, shortcut, id) in crate::menu_catalog::CATALOG {
        // A Window panel item runs its shell command (`menus::panel_alias`), which has the row.
        let owner = crate::menus::panel_alias(id).filter(|alias| index.contains_key(*alias)).unwrap_or(id);
        if label == "---" || !crate::menus::is_live(id) {
            continue;
        }
        if let Some(&at) = index.get(owner) {
            // An engine row that names no key takes the catalogue's.
            if let Some(key) = shortcut.and_then(binding)
                && rows[at].defaults.is_empty()
            {
                rows[at].defaults.push(key);
            }
            continue;
        }
        let mut def = app_row(id, label.trim_end_matches('…'));
        def.paths.extend(menu_path(id));
        def.defaults.extend(shortcut.and_then(binding));
        push_row(&mut rows, &mut index, def);
    }
    // Recorded actions run on their own key (the user's assignment is an override).
    for action in app.session.actions().list.actions() {
        let id = crate::actions::shortcut_id(&action.name);
        push_row(&mut rows, &mut index, app_row(&id, &action.name).with_path(vec!["Actions".into(), action.name.clone()]));
    }

    // A command's alternate keys (#3665: F3 is Copy as well as ⌘C), after every main key.
    for &(id, key) in photocraft_engine::prefs::ALTERNATE_SHORTCUTS {
        let Some(alternate) = binding(key) else { continue };
        if let Some(&at) = index.get(id)
            && !rows[at].defaults.contains(&alternate)
        {
            rows[at].defaults.push(alternate);
        }
    }
    // Photoshop: ⌥ added to an adjustment's own key opens it at its last settings (⌥⌘L, ⌥⌘M, #8900); Layer › New › Layer's
    // key with ⌥ makes the layer at once (#4794).
    for def in &mut rows {
        if crate::adjust_dialog::has_dialog(&def.command.id) || def.command.id == crate::new_layer_ui::COMMAND {
            def.has_alt_variant = true;
        }
    }
    // Edit in Quick Mask Mode is a button in the tool rail's footer, which shows its command's icon.
    if let Some(&at) = index.get(crate::panels::QUICK_MASK) {
        rows[at].icon = Some("square-dashed".into());
    }
    // Smart Objects › Reveal in Finder names this platform's file manager, in the list as in the menu.
    if let Some(&at) = index.get("layer.smartObjects.revealInFinder") {
        rows[at].label = crate::menus::reveal_label().into();
    }
    rows
}

/// The command of the runtime's keymap that menu row `id` runs: the framework command a row of [`RUNTIME_COMMANDS`]
/// bridges, the shell command a Window panel item runs (`menus::panel_alias`), else the row's own.
pub(crate) fn keymap_command(id: &str) -> CommandRef {
    if let Some(bridged) = runtime_by_id().get(id) {
        return bridged.command();
    }
    CommandRef::new(crate::menus::panel_alias(id).unwrap_or(id))
}

/// The rows of the commands the runtime defines ([`RUNTIME_COMMANDS`]) under their PhotoCraft ids, as the engine's
/// catalog gives them with their default keys. The Keyboard Shortcuts dialog and a `.kys` import edit them with the
/// rows of [`command_rows`]; the runtime's own catalog holds them under the framework commands.
pub(crate) fn bridged_rows() -> Vec<CommandDef> {
    photocraft_engine::keymap_defs::command_defs()
        .into_iter()
        .map(|mut def| {
            if def.paths.is_empty() {
                def.paths.extend(menu_path(&def.command.id));
            }
            def
        })
        .filter(|def| is_bridged(&def.command.id))
        .collect()
}

/// The app's command catalog over `rows` ([`command_rows`]). The runtime rebuilds its keymap when `generation`
/// changes, so the holder (`framework_before_input`) takes a new one only when it rebuilds the rows: when the actions
/// list's revision moved.
pub(crate) fn catalog(rows: Vec<CommandDef>, generation: u64) -> Option<AppCatalog> {
    // `command_rows` gives each id once and every row the app scope, the two things `new` refuses; a refusal leaves the
    // runtime with no app catalog.
    AppCatalog::new(rows, generation).ok()
}

/// The user's bindings for the runtime's keymap: Edit › Keyboard Shortcuts changes (`prefs.shortcuts`,
/// `prefs.alternate_shortcuts`), the panel alias rule (#1272), and each row of [`RUNTIME_COMMANDS`] with its
/// effective keys, so the framework command carries PhotoCraft's keys. The pane moves stay unbound
/// (`app::framework`: PhotoCraft binds F6 and Shift+F6). `rows` are the app's catalog rows ([`command_rows`]).
pub(crate) fn overrides(app: &PhotocraftApp, rows: &[CommandDef]) -> Overrides {
    let prefs = app.session.prefs();
    // PLAN: `keymap_defs::prefs_overrides` is top-w20's tp.PT25 row, assumed as `prefs_overrides(&Preferences) -> Overrides`.
    let mut overrides = photocraft_engine::keymap_defs::prefs_overrides(prefs);
    // The Keyboard Shortcuts dialog lists Photoshop's Window › <panel> item (`window.panel.*`) in place of the
    // shell command it runs (`window.toggle.*`, see `menus::panel_alias`), so an edit to that row reassigns or
    // removes the shell command's default too (#1272).
    for key in prefs.shortcuts.keys() {
        let Some(id) = crate::menus::panel_alias(key) else { continue };
        if prefs.shortcuts.contains_key(id) {
            continue;
        }
        let default = crate::shortcuts::default_shortcut(id);
        if crate::menu_catalog::CATALOG.iter().any(|c| c.3 == key.as_str() && c.2.is_some() && c.2 == default.as_deref()) {
            overrides.insert(id, Vec::new());
        }
    }
    // A tool's key is bound on the rail's tool command (`panels::tool_commands`), which answers to the saved
    // `tools.select.<tool>` id: the saved entry is that command's too.
    for tool in Tool::ALL {
        let saved = crate::tool_shortcuts::id(tool);
        if let Some(keys) = overrides.get(&saved).map(<[String]>::to_vec) {
            overrides.insert(CommandRef::from(crate::panels::tool_commands::command_of(tool)).key(), keys);
        }
    }
    let taken: Vec<Chord> = prefs
        .shortcuts
        .iter()
        .filter(|(id, key)| id.starts_with(crate::actions::SHORTCUT_PREFIX) && !key.is_empty())
        .filter_map(|(_, key)| Chord::parse(key).ok())
        .collect();
    for (id, bridged) in RUNTIME_COMMANDS {
        let main = crate::shortcuts::effective_shortcut(app, id, crate::shortcuts::default_shortcut(id).as_deref());
        let mut keys: Vec<String> = main.iter().cloned().collect();
        keys.extend(prefs.alternates(id, main.as_deref()));
        keys.retain(|key| !held_by_action(&taken, key));
        overrides.insert(bridged.command().key(), keys);
    }
    // A key an action holds is the action's alone (it is checked before every command's key, as in Photoshop): the
    // command that has it by default keeps its other keys.
    if !taken.is_empty() {
        for row in rows {
            let id = row.command.id.as_str();
            if id.starts_with(crate::actions::SHORTCUT_PREFIX) || is_bridged(id) {
                continue;
            }
            let main = crate::shortcuts::effective_shortcut(app, id, crate::shortcuts::default_shortcut(id).as_deref());
            let mut keys: Vec<String> = main.iter().cloned().collect();
            keys.extend(prefs.alternates(id, main.as_deref()));
            if keys.iter().any(|key| held_by_action(&taken, key)) {
                keys.retain(|key| !held_by_action(&taken, key));
                overrides.insert(id, keys);
            }
        }
    }
    overrides
}

/// Whether `key` is one of the chords the actions hold.
fn held_by_action(taken: &[Chord], key: &str) -> bool {
    Chord::parse(key).is_ok_and(|chord| taken.contains(&chord))
}

/// Delete's key (Delete / Backspace) with nothing selected deletes the selected layers, of any
/// kind, as in Photoshop (#1077): Edit › Clear only clears pixel layers, so on an adjustment, fill,
/// type or shape layer the key did nothing. A selection, a single channel or Quick Mask keeps Clear.
/// With the active layer's mask targeted the key edits the mask, never the layer's pixels: a
/// selection is filled with the background colour in the mask (Photoshop's Delete on a mask), and
/// with no selection the mask itself is deleted, as Layer › Layer Mask › Delete, unless several
/// layers are selected: then they are deleted.
pub(crate) fn delete_key_command(app: &PhotocraftApp, id: String) -> String {
    if id != "edit.clear" {
        return id;
    }
    let Some(st) = app.session.active() else { return id };
    // A targeted filter mask (FILE-215-3) takes a selection's Delete as a layer mask does: the
    // selection fills with the background colour there. With no selection Photoshop deletes the
    // layer, as below.
    if app.filter_mask_targeted() && st.doc.selection.is_some() {
        return "edit.fillBackground".into();
    }
    let composite = st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::Composite && st.doc.quick_mask.is_none();
    let has_mask = st.active_layer.and_then(|l| st.doc.layer(l)).is_some_and(|l| l.mask.is_some());
    let colors_off = st.channel_view.visible_colors(photocraft_engine::channel_cmds::color_count(&st.doc)) == 0;
    let alpha_on = (0..st.doc.channels.len()).any(|i| st.channel_view.alpha_shown(i));
    let solo_mask = colors_off && has_mask && !alpha_on && st.doc.quick_mask.is_none();
    let layer_mask = st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::LayerMask;
    let mask = (app.ui.mask_target || solo_mask || layer_mask) && has_mask;
    // Several selected layers outrank the active one's targeted mask (#4025): a new adjustment
    // layer targets its mask, so the first press used to delete only that mask.
    let several = st.selected_layers().len() > 1;
    if mask && (composite || layer_mask) && !(several && composite && st.doc.selection.is_none()) {
        let cmd = if st.doc.selection.is_some() { "edit.fillBackground" } else { "layer.layerMask.delete" };
        return cmd.into();
    }
    if st.doc.selection.is_some() || !composite {
        return id;
    }
    "layer.delete".into()
}

/// Create Clipping Mask's key (⌥⌘G) releases when there is nothing left to clip, as in Photoshop,
/// whose Layer menu has one item that reads Create or Release Clipping Mask with the same
/// shortcut: pressing it again takes the clip off. That is when the layers Create Clipping Mask
/// would clip are all clipped already: the selected layer, or with several selected, every one
/// above the lowest.
pub(crate) fn clip_key_releases(app: &PhotocraftApp) -> bool {
    let Some(st) = app.session.active() else { return false };
    let selected = st.selected_layers();
    let clips = match selected {
        [] => return false,
        [one] => std::slice::from_ref(one),
        [_lowest, above @ ..] => above,
    };
    clips.iter().all(|id| st.doc.layer(*id).is_some_and(|l| l.clipped))
}

/// `id`, or Release Clipping Mask when Create Clipping Mask's key finds nothing left to clip.
pub(crate) fn clip_key_command(app: &PhotocraftApp, id: String) -> String {
    if id == "layer.createClippingMask" && clip_key_releases(app) { "layer.releaseClippingMask".into() } else { id }
}

/// Why `id` can't run now (the engine's reason when it has one).
pub fn disabled_reason(app: &PhotocraftApp, id: &str) -> String {
    photocraft_engine::commands::find(id).and_then(|_| app.session.disabled_reason_with(id, &app.with_mask_target(id, serde_json::Value::Null))).unwrap_or_else(
        || match id {
            "tools.decreaseBrushHardness" | "tools.increaseBrushHardness" => "the current tool has no brush tip".into(),
            _ if app.session.active().is_none() => "no document open".into(),
            _ => "not available in the current state".into(),
        },
    )
}

fn label(id: &str) -> String {
    crate::menu_catalog::CATALOG
        .iter()
        .find(|c| c.3 == id)
        .map(|c| c.1)
        .or_else(|| crate::menus::UI_COMMANDS.iter().find(|c| c.0 == id).map(|c| c.1))
        .or_else(|| photocraft_engine::commands::find(id).map(|c| c.label))
        .or_else(|| crate::tool_shortcuts::tool(id).map(|tool| tool.label()))
        .unwrap_or(id)
        .trim_end_matches('…')
        .to_string()
}

/// The runtime's command for the row `id` of [`RUNTIME_COMMANDS`], fired as its chord fires it: `None` for any
/// other id, else whether the runtime took it (a canvas will run a view command).
pub(crate) fn fire_bridged(ctx: &egui::Context, id: &str) -> Option<bool> {
    let (_, bridged) = RUNTIME_COMMANDS.iter().find(|(row, _)| *row == id)?;
    Some(craft::ui::runtime::commands::fire(ctx, bridged.command(), FireSource::Menu))
}

/// Records each view row of [`RUNTIME_COMMANDS`] a canvas ran from the runtime since the last frame, by its chord or its
/// menu-bar row, as PhotoCraft's view command: an action being recorded gets `view.fitOnScreen` for ⌘0 and for the menu
/// item alike. The ones app code asked for (`run_view`) never reach the runtime and are journaled when asked.
/// The screen-mode rows the shell ran go to the shortcut log only.
pub(crate) fn record_view_commands(app: &mut PhotocraftApp) {
    for fired in std::mem::take(&mut app.framework.delivered) {
        match bridge_of(&fired.command) {
            Some((id, Bridged::View(_))) => {
                crate::panels::actions::journal_view(app, id, json!({}));
                record(app.ctx(), id, Outcome::Ran);
            }
            Some((id, Bridged::ScreenMode(_))) if fired.phase == FirePhase::Press => record(app.ctx(), id, Outcome::Ran),
            _ => {}
        }
    }
}

/// Records each chrome toggle a key ran since the last frame as its row of [`RUNTIME_COMMANDS`]: the runtime runs a key's
/// toggle in its key stage and fires no command for it, and the window's shell hides its panels.
pub(crate) fn record_chrome_toggles(ctx: &egui::Context, toggles: &[(egui::ViewportId, ChromeToggle)]) {
    for (_, toggle) in toggles {
        let row = RUNTIME_COMMANDS.iter().find(|(_, bridged)| matches!(bridged, Bridged::Chrome(command) if command.toggle() == *toggle));
        if let Some((id, _)) = row {
            record(ctx, id, Outcome::Ran);
        }
    }
}

/// Runs the commands the runtime's keys and surfaces fired, in the order they fired. The caller passes the
/// commands of its own pass only (`shortcuts::handle` takes them from the framework state), so a second pass in
/// one frame applies no command twice.
///
/// Per command: the held erase key; the opacity and blend keys (their own rules); a tool letter picks its
/// group's tool on the press and releases the spring on the key-up; every other command runs on its press only.
/// A framework command goes to [`crate::app::framework::run_command`], a PhotoCraft id to [`dispatch_as`].
pub(crate) fn run_fired(app: &mut PhotocraftApp, ctx: &egui::Context, fired: &[Fired]) {
    // A fired command is a command run, as `menus::invoke` counts one: many of these (the opacity and blend keys, the
    // Artboard, Slice and panel commands) change what a settled region shows without going through `invoke`. A key's
    // repeats change only what has a revision or a tool option (brush size, a nudge, a mode step), so a held key
    // doesn't run every region at the repeat rate.
    // A view command (the canvas runs it) changes only what the view's regions hold in their versions.
    let counts = |f: &&Fired| bridge_of(&f.command).is_none_or(|(id, _)| !crate::panels::region_versions::changes_only_the_view(app, id));
    if fired.iter().filter(|f| f.target.is_none() && f.phase != FirePhase::Repeat).any(|f| counts(&f)) {
        app.commands_run += 1;
        ctx.request_repaint();
    }
    for f in fired {
        if f.target.is_some() {
            continue;
        }
        // The chrome toggles come back as the framework's command; the canvas runs its view commands itself.
        // The Swatches and Workspace commands run through their sets' handlers (`run_pressed`).
        if let Some((id, bridged)) = bridge_of(&f.command).filter(|(_, bridged)| !matches!(bridged, Bridged::Swatches(_) | Bridged::Workspace(_))) {
            if matches!(bridged, Bridged::Chrome(_) | Bridged::Edit(_) | Bridged::Stack(_)) && f.phase == FirePhase::Press {
                dispatch(app, ctx, id);
            }
            continue;
        }
        let id = f.command.id.as_str();
        if id == "tools.temporary.erase" {
            // PLAN: `hold_keys::erase_key(app, phase)` is top-w26's tp.PT21, assumed.
            crate::hold_keys::erase_key(app, f.phase);
            continue;
        }
        // PLAN: `opacity_keys::fired` and `blend_keys::fired` are top-w25's tp.PT14 rows, assumed.
        if crate::opacity_keys::fired(app, f) || crate::blend_keys::fired(app, f) {
            continue;
        }
        // A tool command a key fired: its letter picks within the group; a click or a menu row selects the tool itself.
        let keyed_tool = || (f.source == FireSource::Key).then(|| crate::panels::tool_commands::ToolCommand::from_ref(&f.command)).flatten();
        if let Some(tool) = crate::tool_shortcuts::tool(id).or_else(|| keyed_tool().map(crate::panels::tool_commands::tool_of)) {
            match f.phase {
                // A key the user gave this tool selects exactly it (#4086); its letter belongs to the group.
                FirePhase::Press if !crate::tool_shortcuts::uses_default(app, tool) => crate::tool_memory::select(app, tool),
                FirePhase::Press => {
                    let step = tool_step(app, f);
                    pick_tool(app, tool.key(), step, true, f.time);
                }
                FirePhase::Release => crate::spring_tools::released(app, tool.key(), f.time),
                FirePhase::Repeat => {}
            }
            continue;
        }
        if let Some(letter) = next_in_group_letter(id) {
            if f.phase == FirePhase::Press {
                pick_tool(app, letter, PickStep::Next, false, f.time);
            }
            continue;
        }
        if f.phase != FirePhase::Press {
            continue;
        }
        run_pressed(app, ctx, f);
    }
}

/// The step a tool letter takes: with Preferences › Tools › Use Shift Key for Tool Switch the plain letter keeps the
/// group's current tool (Shift+letter is [`next_in_group_id`]'s row); with it off the plain letter cycles a group
/// whose tools share the letter.
fn tool_step(app: &PhotocraftApp, fired: &Fired) -> PickStep {
    if app.session.prefs().tools.use_shift_key_for_tool_switch { PickStep::Last } else { PickStep::of(fired) }
}

/// Selects the tool the key `letter` picks. The group is the tools of that letter the user has not remapped, in the
/// toolbar layout's order across slots (Edit › Toolbar); the rail's last-used memory is `app.ui.group_tools`, with the group's index among the letters as
/// the slot. `plain` is a letter press with no Shift step: only those spring (#2672). `at` is the press's input time.
fn pick_tool(app: &mut PhotocraftApp, letter: char, step: PickStep, plain: bool, at: f64) {
    let mut group: Vec<Tool> = Tool::ALL.iter().copied().filter(|t| t.key() == letter && crate::tool_shortcuts::uses_default(app, *t)).collect();
    let order = crate::panels::toolbar::layout_order(&app.session.prefs().toolbar);
    group.sort_by_key(|t| order.iter().position(|x| x == t).unwrap_or(usize::MAX));
    let tools: Vec<RailTool<Tool>> = group.iter().map(|t| RailTool::new(*t, crate::panels::tool_commands::command_of(*t))).collect();
    let refs: Vec<&RailTool<Tool>> = tools.iter().collect();
    let slot = 0;
    let mut last = LastUsed::default();
    if let Some(remembered) = app.ui.group_tools.iter().copied().find(|t| group.contains(t)) {
        last.set(slot, remembered);
    }
    let before = app.ui.tool;
    let Some(picked) = tool_pick(&refs, slot, &before, &last, step, |_| true).map(|t| t.id) else { return };
    app.ui.tool = picked;
    crate::tool_memory::ended_by_tool(app);
    crate::spring_tools::pressed(app, letter, before, plain, at);
    if app.ui.spring.is_none() {
        // Permanent key choices own the tool; held-key switches keep the pen marker.
        app.stylus.tool_before_eraser = None;
    }
}

/// Runs the press of a command that is not a tool key: the framework's command sources first, else PhotoCraft's own
/// through [`dispatch_as`].
fn run_pressed(app: &mut PhotocraftApp, ctx: &egui::Context, f: &Fired) {
    let id = f.command.id.as_str();
    // The Magnetic Lasso's bracket keys step its detection width, not the brush (#11688), until an outline is open.
    if app.ui.tool == Tool::MagneticLasso && !app.canvas_tools.selection.outline_open() {
        let step = match id {
            "tools.decreaseBrushSize" => Some((-1, false)),
            "tools.increaseBrushSize" => Some((1, false)),
            "tools.decreaseBrushHardness" => Some((-1, true)),
            "tools.increaseBrushHardness" => Some((1, true)),
            _ => None,
        };
        if let Some((by, shift)) = step {
            app.ui.tool_options.magnetic.step_width(by, shift);
            return;
        }
    }
    if id == EditCommands::Delete.id() {
        // A finished work path shown by a Pen or Path Selection, or the path or segment Direct Selection targets:
        // ⌫/Delete edits the path, as in Photoshop (#5702, #12695, #13203), never Edit › Clear on the pixels under it.
        if let Some((command, params)) = crate::vector_ui::delete_key_edits_path(app) {
            if let Err(e) = app.run(command, params) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
            return;
        }
        // A selected smart filter row: ⌫/Delete removes the filter, not its smart object (#4751).
        if crate::smart_ui::keys(app) {
            return;
        }
    }
    if crate::app::framework::run_command(app, ctx, &f.command) {
        record(ctx, id, Outcome::Ran);
        return;
    }
    // The Delete key is Edit › Clear, or deletes the layers when nothing is selected (`delete_key_command`).
    let id = if id == EditCommands::Delete.id() { "edit.clear" } else { id };
    let id = clip_key_command(app, delete_key_command(app, id.to_string()));
    // A key that runs another command (Delete, the clip keys) runs it without the fired command's parameters.
    let command = if id == f.command.id { f.command.clone() } else { CommandRef::new(id) };
    dispatch_as(app, ctx, &command, f.alt_variant);
}

/// Run a shortcut's command like its menu item (dialogs included), or say why it can't run.
pub fn dispatch(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str) {
    dispatch_as(app, ctx, &CommandRef::new(id), false);
}

/// [`dispatch`] of a fired command, its parameters included (an Appearance or Language row's choice); with `last`, an
/// adjustment dialog opens at its last confirmed settings and New Layer makes the layer without its dialog.
fn dispatch_as(app: &mut PhotocraftApp, ctx: &egui::Context, command: &CommandRef, last: bool) {
    let id = command.id.as_str();
    let action = id.strip_prefix(crate::actions::SHORTCUT_PREFIX);
    let command_id = if action.is_some() { "actions.play" } else { id };
    // A key saved for an action that was since deleted or renamed names no action.
    let missing = action.is_some_and(|name| !app.session.actions().list.actions().iter().any(|a| a.name == name));
    let outcome = if missing || !crate::menus::is_enabled(app, command_id) {
        let why = if missing { "The Action no longer exists".into() } else { disabled_reason(app, command_id) };
        let template = "{label} is not available: {reason}";
        app.ui.status = crate::i18n::fmt(tl!(template), &[("label", crate::i18n::tr_id(crate::i18n::current(), id, &label(id))), ("reason", tl!(&why))]);
        app.ui.status_error = true;
        Outcome::Disabled(why)
    } else if ctx.data(|d| d.get_temp::<bool>(dry_run_id())).unwrap_or(false) {
        Outcome::Ran
    } else {
        let r = if let Some(name) = action {
            // Use the normal command path: synthetic input and every nested action
            // step must still pass the automation authorizer.
            crate::menus::invoke(app, ctx, "actions.play", json!({"action": name})).and_then(|v| {
                crate::actions::report_play(app, &v);
                if v.get("failed").is_some_and(|f| f.is_object()) { Err(app.ui.status.clone()) } else { Ok(v) }
            })
        } else if last && id == crate::new_layer_ui::COMMAND {
            // The plain shortcut asks with the New Layer dialog first, like the menu item; ⌥ makes the layer at once
            // (#4794).
            app.run(id, json!({}))
        } else if crate::adjust_dialog::has_dialog(id) {
            let opened = if last { crate::adjust_dialog::open_last(app, id) } else { crate::adjust_dialog::open(app, id) };
            opened.map(|_| serde_json::Value::Null)
        } else {
            let params = if command.params.is_null() { json!({}) } else { command.params.clone() };
            crate::menus::invoke(app, ctx, id, params)
        };
        match r {
            Ok(_) => Outcome::Ran,
            Err(e) => {
                app.ui.status = e.clone();
                app.ui.status_error = true;
                Outcome::Failed(e)
            }
        }
    };
    record(ctx, id, outcome);
}

/// Notes in the run log that command `id` ended as `outcome`.
fn record(ctx: &egui::Context, id: &str, outcome: Outcome) {
    ctx.data_mut(|d| {
        let log = d.get_temp_mut_or_default::<Vec<(String, Outcome)>>(log_id());
        if log.len() >= 64 {
            log.remove(0);
        }
        log.push((id.to_string(), outcome));
    });
}

/// The whole-app key harness the key tests of this module share (`app_harness`, `tap`).
#[cfg(test)]
pub(crate) mod tests;
