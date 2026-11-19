//! The craft framework's runtime on PhotoCraft's context, hosted through `craft::ui::shell::embed`
//! (craft-libs docs/craft-framework.md 2.2, "Host the runtime from your own eframe `App`"):
//! installed once with PhotoCraft's fonts, then given PhotoCraft's preferences before each frame.
//!
//! eframe's `ui` would also call `embed::before_ui`, which needs craft-ui-shell's `app` feature;
//! that feature turns on egui's `default_fonts`, which PhotoCraft leaves out (it ships them
//! deflated). Its two jobs are PhotoCraft's own here: `work_area::fit_window` fits the window to
//! its monitor, and the Windows window comes from PhotoCraft's title bar code.
//!
//! The `craft` facade features PhotoCraft enables for this: `text`, `text-font-picker`,
//! `font-manager`, `menu`, `timeline` and `print`, plus `dialogs` on native targets.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use craft::host::target::{Backend, ShellInfo};
use craft::host::{AppInfo, Files, Host, SectionStore};
use craft::i18n::Direction;
use craft::keymap::Overrides;
use craft::theme::{AppDefaults, AppSizes, ModeChoice, Size, SuiteChoice, ThemeId, ThemePair, resolve_sized};
use craft::theme_store::{SelectionChange, SuiteClient};
use craft::ui::dialog::DialogHost;
use craft::ui::fonts::UiFonts;
use craft::ui::runtime::commands::{AppCatalog, CommandInput, CommandRef, CommandSet as _, CommandState, Fired, NavCommand};
use craft::ui::runtime::env::{PointSize, RootEnv, Scale, SystemReader};
use craft::ui::runtime::focus::TabReach;
use craft::ui::runtime::frame::{FrameInput, FrameOutput};
use craft::ui::runtime::install::WheelModifiers;
use craft::ui::runtime::{InstallError, InstallSpec};
use craft::ui::shell::embed;
use craft::window::policy::Decision;
use craft::window::system::SystemWindows;
use photocraft_engine::Session;
use photocraft_engine::prefs::{AppearanceMode, Preferences};
use serde_json::Value;

use super::PhotocraftApp;
use crate::i18n;
use crate::state::Tool;
use crate::theme::AppTheme;

/// Installs the runtime on `ctx` with `fonts` and the root window's decided `frame` (`None` where
/// the platform decides none), unless it is there already (`setup_window` again for the same
/// context). Only `theme::install_runtime` calls it.
pub(crate) fn install(ctx: &egui::Context, fonts: &UiFonts<'_>, frame: Option<Decision>) {
    // The wheel zooms with the canvas's own keys and scrolls sideways with Shift or Command, as
    // PhotoCraft's input always has.
    let wheel = WheelModifiers::new(egui::Modifiers::NONE, egui::Modifiers::SHIFT | egui::Modifiers::COMMAND);
    let wake = ctx.clone();
    let mut spec = InstallSpec::new(ShellInfo::default().backend(Backend::from_env()), Arc::new(SystemWindows))
        .wheel(wheel)
        .app_id(crate::os_window::APP_ID)
        .system(SystemReader::start(move || wake.request_repaint()));
    if let Some(decision) = frame {
        spec = spec.frame(decision);
    }
    // The platform's pen readings: the app writes them into this feed, the runtime pairs them with the pointer moves.
    let pen = crate::stylus::StylusFeed::default();
    spec = spec.pen(pen.source());
    // The icons the interface draws are registered before the runtime reads its first frame.
    crate::icons::register();
    match embed::install(ctx, spec, fonts) {
        Ok(_) => {
            pen.keep_on(ctx);
            #[cfg(test)]
            crate::testing::announce_modifiers(ctx);
        }
        Err(InstallError::AlreadyInstalled) => {}
        Err(error) => log::error!("the craft runtime could not be installed: {error}"),
    }
}

#[cfg(test)]
type TestBody = Box<dyn FnMut(&mut egui::Ui, &mut PhotocraftApp)>;

/// What PhotoCraft keeps for the framework: the `Host` its components read services from, and
/// the host of the framework dialogs it opens.
pub struct Framework {
    /// A test harness's own pass body, run in place of the app's frame (see [`BuildApp`]).
    #[cfg(test)]
    pub(crate) test_body: Option<TestBody>,
    sections: Arc<PrefSections>,
    host: Arc<Host>,
    /// The UI language `host` speaks.
    lang: &'static str,
    pub dialogs: DialogHost,
    /// The per-open state each open parameter dialog's family needs when its answer comes back, by dialog key.
    pub captured: std::collections::BTreeMap<String, serde_json::Value>,
    /// The runtime's keymap overrides. The runtime rebuilds its keymap when the `Arc` changes, so
    /// it is replaced only when the bindings it carries change.
    overrides: Arc<Overrides>,
    /// The preferences revision `overrides` and `catalog` were last built at.
    overrides_rev: Option<u64>,
    /// The app's command catalog as of `overrides_rev`.
    catalog: Option<AppCatalog>,
    /// The rows `catalog` was built from, and its generation: the runtime rebuilds its keymap when the generation
    /// changes, so it advances only when the rows differ.
    catalog_rows: Vec<craft::keymap::CommandDef>,
    catalog_generation: u64,
    /// The file services the host gives components (pickers, reads and writes); `None` keeps the
    /// host's own default.
    files: Option<Arc<dyn Files>>,
    /// The commands the runtime's keys and surfaces fired since `shortcuts::handle` last ran.
    pub(crate) fired: Vec<Fired>,
    /// Whether a component refused its input in the frames since the last alert: the frame plays
    /// its alert once.
    pub(crate) refused: bool,
    /// The commands the runtime's components ran since the last frame, for the action recorder in `logic`.
    pub(crate) delivered: Vec<Fired>,
    /// The suite's theme selection, shared with the other Craft apps.
    suite: SuiteClient,
    /// The theme slots and mode as PhotoCraft's preferences and the suite last agreed on them.
    synced: Option<ThemePrefs>,
    /// The preferences' rev `synced` was taken at: with no suite move since, nothing to sync.
    synced_rev: Option<u64>,
    /// Whether the start's agreement ran: the suite's chosen fields over the preferences, once the
    /// suite's selection is read.
    started: bool,
    /// How many times `theme_for` ran, for tests that count resolves.
    #[cfg(test)]
    pub(crate) theme_resolves: std::sync::atomic::AtomicUsize,
}

/// Why a theme can't show now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ThemeUnavailable {
    /// A library theme picked for the suite that the suite hasn't loaded yet.
    Loading,
    /// A library theme in the suite's library that nothing picked, so the suite doesn't load it.
    NotPicked,
    /// No theme the suite knows.
    Unknown,
}

impl ThemeUnavailable {
    /// The message for theme `id`: a refusal for an unknown one, a note for one still loading.
    pub(crate) fn message(self, id: &ThemeId) -> String {
        match self {
            ThemeUnavailable::Loading | ThemeUnavailable::NotPicked => format!("theme `{id}` is still loading"),
            ThemeUnavailable::Unknown => format!("unknown theme `{id}`"),
        }
    }
}

/// The preferences that choose the theme: the appearance mode and the theme id of each slot.
#[derive(Clone, Debug, PartialEq)]
struct ThemePrefs {
    mode: AppearanceMode,
    dark: String,
    light: String,
}

impl ThemePrefs {
    fn of(prefs: &Preferences) -> Self {
        let i = &prefs.interface;
        ThemePrefs { mode: i.appearance_mode, dark: i.dark_theme.clone(), light: i.light_theme.clone() }
    }

    /// What the suite shows: its chosen fields, else PhotoCraft's defaults.
    fn of_suite(choice: &SuiteChoice) -> Self {
        let defaults = defaults();
        let shown = choice.with_defaults(&defaults);
        ThemePrefs { mode: app_mode(shown.mode), dark: shown.theme.dark.as_str().to_owned(), light: shown.theme.light.as_str().to_owned() }
    }

    /// The suite fields that differ between `self` and `before`.
    fn change_from(&self, before: &ThemePrefs) -> SelectionChange {
        let id = |value: &str| ThemeId::parse(value).ok();
        SelectionChange {
            theme_dark: (self.dark != before.dark).then(|| id(&self.dark)).flatten(),
            theme_light: (self.light != before.light).then(|| id(&self.light)).flatten(),
            mode: (self.mode != before.mode).then(|| mode_choice(self.mode)),
            ..SelectionChange::default()
        }
    }
}

fn app_mode(mode: ModeChoice) -> AppearanceMode {
    match mode {
        ModeChoice::System => AppearanceMode::Auto,
        ModeChoice::Dark => AppearanceMode::Dark,
        ModeChoice::Light => AppearanceMode::Light,
    }
}

fn mode_choice(mode: AppearanceMode) -> ModeChoice {
    match mode {
        AppearanceMode::Auto => ModeChoice::System,
        AppearanceMode::Dark => ModeChoice::Dark,
        AppearanceMode::Light => ModeChoice::Light,
    }
}

/// PhotoCraft's theme in the dark mode when nobody chose one: Pro (Medium Gray).
const DEFAULT_DARK: &str = "craft.artcraft";
/// PhotoCraft's theme in the light mode when nobody chose one: Pro (Light).
const DEFAULT_LIGHT: &str = "craft.pro";

/// What PhotoCraft shows for each suite field nobody has chosen: Pro (Medium Gray) in the dark
/// mode, Pro in the light, and the dark mode.
fn defaults() -> AppDefaults {
    let theme = |id: &str| ThemeId::parse(id).unwrap_or_else(|_| ThemeId::suite_default());
    AppDefaults {
        theme: ThemePair { light: theme(DEFAULT_LIGHT), dark: theme(DEFAULT_DARK) },
        mode: ModeChoice::Dark,
        size: Size::default(),
        sizes: (**app_sizes()).clone(),
    }
}

/// PhotoCraft's size overrides: Photoshop's panel dock width, and 12-point Label and Mono text in the compact density.
const SIZES: &[u8] = include_bytes!("sizes.json");

/// The size overrides, parsed once: the root keeps one `Arc` so the runtime's token and hit-size caches, which key on
/// its identity, resolve once.
fn app_sizes() -> &'static Arc<AppSizes> {
    static SIZES_ONCE: std::sync::LazyLock<Arc<AppSizes>> = std::sync::LazyLock::new(|| Arc::new(AppSizes::parse_or_default(SIZES)));
    &SIZES_ONCE
}

/// The root values PhotoCraft gives the runtime before the theme, mode, scale and direction: the
/// contrast, size and sizes a theme's tokens resolve under.
fn base_root() -> RootEnv {
    let mut root = RootEnv::new();
    root.sizes = Arc::clone(app_sizes());
    // Photoshop's Tab hides the panels, Shift+Tab all but the tools, over the canvas.
    root.tab_toggles_chrome = true;
    // Tab in a dialog moves between its fields, as in Photoshop.
    root.dialog_tab = TabReach::Fields;
    root
}

impl Default for Framework {
    fn default() -> Self {
        Framework::new(None)
    }
}

fn build_host(sections: &Arc<PrefSections>, lang: &str, files: Option<&Arc<dyn Files>>) -> Host {
    let i18n = i18n::framework_i18n();
    let mut builder = Host::builder(AppInfo { id: "photocraft", name: "PhotoCraft", version: env!("CARGO_PKG_VERSION") })
        .i18n(Arc::clone(i18n))
        .settings(Arc::clone(sections) as Arc<dyn SectionStore>)
        .jobs(craft::host::default_jobs());
    if let Some(files) = files {
        builder = builder.files(Arc::clone(files));
    }
    if let Some(lang) = i18n.from_code(lang) {
        builder = builder.lang(lang);
    }
    builder.build()
}

impl Framework {
    /// The host the framework's components read services from. A window that previews another language
    /// takes a copy with `Host::with_lang`.
    pub(crate) fn host(&self) -> &Arc<Host> {
        &self.host
    }

    /// The framework state of a new app, its host serving `files` where the app has file services.
    pub(crate) fn new(files: Option<Arc<dyn Files>>) -> Self {
        let sections = Arc::new(PrefSections::default());
        let lang = "en";
        Framework {
            #[cfg(test)]
            test_body: None,
            host: Arc::new(build_host(&sections, lang, files.as_ref())),
            sections,
            lang,
            dialogs: DialogHost::new(),
            captured: Default::default(),
            overrides: Arc::new(Overrides::new()),
            overrides_rev: None,
            catalog: None,
            catalog_rows: Vec::new(),
            catalog_generation: 0,
            files,
            fired: Vec::new(),
            refused: false,
            delivered: Vec::new(),
            suite: SuiteClient::memory(),
            synced: None,
            synced_rev: None,
            started: false,
            #[cfg(test)]
            theme_resolves: Default::default(),
        }
    }

    /// The host's file service. The web has no host `Files` until FW-264 and reads its own dialog path.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn files(&self) -> &dyn Files {
        self.host.files()
    }

    /// Moves the dialogs' last-used values the preferences still hold into the framework's saved
    /// memory, once, before the first frame. Nothing is left to move after the first run.
    pub(crate) fn move_memories(&mut self, session: &mut Session) {
        self.sections.sync(session);
        session.edit_prefs(|prefs| crate::dialogs::move_memories(prefs, &self.host));
        self.sections.sync(session);
    }

    /// The input for the coming frame, from PhotoCraft's preferences: `states` of every command
    /// source, the held catalog and overrides, and `gate` over the commands.
    fn frame_input(
        &mut self,
        session: &mut Session,
        ctx: &egui::Context,
        theme: &AppTheme,
        states: Vec<(CommandRef, CommandState)>,
        gate: craft::ui::runtime::commands::CommandGate,
        latched: craft::keymap::Mods,
    ) -> FrameInput {
        self.sections.sync(session);
        let lang = i18n::current().code();
        if lang != self.lang {
            self.lang = lang;
            self.host = Arc::new(build_host(&self.sections, lang, self.files.as_ref()));
        }
        let rtl = i18n::framework_i18n().from_code(lang).is_some_and(|l| i18n::framework_i18n().rtl(l));
        let root = match theme.library() {
            Some(def) => base_root().library_theme(Arc::clone(def)),
            None => base_root().theme(theme.id().clone()),
        };
        let mut root = root
            .mode(if theme.is_dark() { ModeChoice::Dark } else { ModeChoice::Light })
            .scale(Scale::new(crate::prefs_ui::interface_zoom(session.prefs().interface.ui_scale, ctx)))
            .direction(if rtl { Direction::Rtl } else { Direction::Ltr });
        // Read each frame, so a preference change shows at once.
        root.canvas_pattern = session.prefs().interface.show_canvas_dots;
        root.values.set(craft::ui::canvas::ShowReadouts(session.prefs().tools.show_transformation_values));
        // Length fields read the Point/Pica size from the root; each passes its document's pixels
        // per inch in its own `LengthUnits`.
        root.values.set(match session.prefs().units_and_rulers.point_size {
            photocraft_engine::prefs::PointSize::PostScript => PointSize::PostScript,
            photocraft_engine::prefs::PointSize::Traditional => PointSize::Traditional,
        });
        let mut commands = states.into_iter().fold(CommandInput::new(), |input, (command, state)| input.state(command, state));
        if let Some(catalog) = &self.catalog {
            commands = commands.catalog(catalog.clone());
        }
        let commands = commands.overrides(Arc::clone(&self.overrides)).gate(gate);
        FrameInput::new(root).host(Arc::clone(&self.host)).commands(commands).latched(latched)
    }

    /// Opens the suite folder this machine shares with the other Craft apps, in place of the
    /// in-memory suite the app starts with.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn open_suite(&mut self, ctx: &egui::Context) {
        use craft::theme_store::{StoreOptions, SuiteDir, SuiteWatch};
        let ctx = ctx.clone();
        let options = StoreOptions::new(concat!("photocraft ", env!("CARGO_PKG_VERSION")));
        self.suite = SuiteClient::new(SuiteWatch::open(&SuiteDir::resolve(false), options, move || ctx.request_repaint()));
        self.synced = None;
        self.synced_rev = None;
        self.started = false;
    }

    /// Brings the preferences and the suite's selection together: a change made in the
    /// preferences goes to the suite, a change another app made to the suite goes to the
    /// preferences. At the start, once the suite's selection is read, the suite's chosen fields
    /// win and the preferences' own choices fill the fields it has not chosen; until then the
    /// preferences show. Runs only when the suite moved or the preferences changed. Returns
    /// whether the theme on screen may need to change.
    pub(crate) fn sync_theme(&mut self, session: &mut Session) -> bool {
        let moved = self.suite.sync();
        if !moved && self.synced_rev == Some(session.prefs.rev()) {
            return false;
        }
        let mine = ThemePrefs::of(session.prefs());
        let suite = moved.then(|| ThemePrefs::of_suite(&self.suite.shown()));
        let previous = self.synced.take();
        let agreed = match &previous {
            _ if !self.suite.selection_read() => mine.clone(),
            _ if !self.started => {
                self.started = true;
                let chosen = self.suite.accepted().suite.clone();
                let blank = ThemePrefs::of_suite(&SuiteChoice::default());
                let mut start = mine.clone();
                let mut change = SelectionChange::default();
                match &chosen.theme_dark {
                    Some(id) => start.dark = id.as_str().to_owned(),
                    None if mine.dark != blank.dark => change.theme_dark = ThemeId::parse(&mine.dark).ok(),
                    None => {}
                }
                match &chosen.theme_light {
                    Some(id) => start.light = id.as_str().to_owned(),
                    None if mine.light != blank.light => change.theme_light = ThemeId::parse(&mine.light).ok(),
                    None => {}
                }
                match chosen.mode {
                    Some(mode) => start.mode = app_mode(mode),
                    None if mine.mode != blank.mode => change.mode = Some(mode_choice(mine.mode)),
                    None => {}
                }
                self.suite.change(change);
                start
            }
            Some(agreed) if mine != *agreed => {
                self.suite.change(mine.change_from(agreed));
                mine.clone()
            }
            Some(agreed) => match suite {
                Some(suite) if suite != *agreed => suite,
                _ => agreed.clone(),
            },
            None => mine.clone(),
        };
        if agreed != mine {
            session.edit_prefs(|p| {
                p.interface.appearance_mode = agreed.mode;
                p.interface.dark_theme = agreed.dark.clone();
                p.interface.light_theme = agreed.light.clone();
            });
        }
        let changed = moved || previous.as_ref() != Some(&agreed);
        self.synced = Some(agreed);
        self.synced_rev = Some(session.prefs.rev());
        changed
    }

    /// The theme `id` to show in the dark or light mode: PhotoCraft's palette for it, else its
    /// tokens resolved from the suite's definition under the root's contrast, size and sizes, as
    /// the runtime resolves its own. A theme the suite can't resolve shows as PhotoCraft's default
    /// for the mode.
    pub(crate) fn theme_for(&self, id: &ThemeId, dark: bool, ctx: &egui::Context) -> AppTheme {
        #[cfg(test)]
        self.theme_resolves.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Some(theme) = AppTheme::palette(id, dark) {
            return theme;
        }
        let Some(def) = self.suite.theme(id) else {
            let defaults = defaults().theme;
            return AppTheme::palette(if dark { &defaults.dark } else { &defaults.light }, dark).unwrap_or_default();
        };
        let root = base_root().mode(if dark { ModeChoice::Dark } else { ModeChoice::Light });
        let resolved = resolve_sized(&def, root.appearance(ctx), None, &root.sizes);
        AppTheme::resolved(Arc::new(def), dark, &resolved)
    }

    /// The suite: the Preferences window's theme rows read it, and tests put themes in its library.
    pub(crate) fn suite(&self) -> &SuiteClient {
        &self.suite
    }

    /// Whether `shown` is what [`theme_for`](Self::theme_for) gives for the slot id `slot` in
    /// `shown`'s mode because the suite can't resolve it: the mode's default palette, and `slot`
    /// a theme the suite doesn't know. Checked without resolving or copying a theme.
    pub(crate) fn shows_fallback_for(&self, slot: &str, shown: &AppTheme) -> bool {
        let default = if shown.is_dark() { DEFAULT_DARK } else { DEFAULT_LIGHT };
        shown.id().as_str() == default
            && shown.library().is_none()
            && ThemeId::parse(slot).is_ok_and(|id| self.theme_availability(&id) == Err(ThemeUnavailable::Unknown))
    }

    /// Whether theme `id` can show now: the suite has it as a built-in or a loaded library theme.
    pub(crate) fn theme_availability(&self, id: &ThemeId) -> Result<(), ThemeUnavailable> {
        if self.suite.has_theme(id) {
            Ok(())
        } else if self.suite.is_loading(id) {
            Err(ThemeUnavailable::Loading)
        } else if self.suite.in_library(id) {
            Err(ThemeUnavailable::NotPicked)
        } else {
            Err(ThemeUnavailable::Unknown)
        }
    }

    /// The name of theme `id` for a menu: PhotoCraft's own name for its palettes, else the suite's.
    pub(crate) fn theme_name(&self, id: &ThemeId, dark: bool) -> String {
        AppTheme::palette_label(id.as_str(), dark)
            .map(str::to_owned)
            .or_else(|| self.suite.theme(id).map(|def| def.name))
            .unwrap_or_else(|| id.as_str().to_owned())
    }

    /// What the runtime's keymap was last built from: the preferences revision of the overrides and the catalog's
    /// generation. They change at the start of the frame whose keymap follows them, so a surface that shows the
    /// keymap's chords (the menu model) hashes them to rebuild in that frame.
    pub(crate) fn keymap_inputs(&self) -> (Option<u64>, u64) {
        (self.overrides_rev, self.catalog_generation)
    }

    /// Rebuilds the catalog and the runtime's keymap overrides at preferences revision `rev`. The
    /// pane moves are unbound in the overrides, because PhotoCraft's own table binds F6 and
    /// Shift+F6 (Window › Color, Select › Modify › Feather) until those are a framework
    /// `CommandSet`. The held `Arc` is replaced only when the bindings differ, so the runtime
    /// rebuilds its keymap only on a change.
    fn set_overrides(&mut self, rev: u64, catalog: Option<AppCatalog>, mut overrides: Overrides) {
        self.overrides_rev = Some(rev);
        self.catalog = catalog;
        for command in [NavCommand::NextPane, NavCommand::PreviousPane] {
            overrides.insert(CommandRef::from(command).key(), Vec::new());
        }
        if *self.overrides != overrides {
            self.overrides = Arc::new(overrides);
        }
    }
}

/// One user of the framework's command sets: the states its commands show and the runner of the
/// ones it fires.
pub(crate) struct CommandSource {
    /// The states of the source's commands for the coming frame.
    states: fn(&PhotocraftApp) -> Vec<(CommandRef, CommandState)>,
    /// Runs `command` when it is this source's; whether it was.
    run: fn(&mut PhotocraftApp, &egui::Context, &CommandRef) -> bool,
}

/// Every command source, in the order [`run_command`] tries them. The theme has no entry: it has
/// no command state, and its picks run through `menus::invoke`.
pub(crate) const COMMAND_SOURCES: &[CommandSource] = &[
    CommandSource { states: crate::panels::registry::states, run: crate::panels::registry::run_fired },
    CommandSource { states: tool_states, run: run_tool },
    CommandSource { states: crate::workspace_ui::states, run: crate::workspace_ui::run_fired },
    CommandSource { states: crate::slice_ui::command_states, run: crate::slice_ui::run_fired },
    CommandSource { states: crate::align_bar::command_states, run: crate::align_bar::run_fired },
    CommandSource { states: crate::artboard_tool::command_states, run: crate::artboard_tool::run_fired },
    // Space+F: the pasteboard colour keys of the held Hand.
    CommandSource { states: |_| Vec::new(), run: crate::canvas_tool_menu::run_canvas_color },
    // The options bar's engine-command buttons: their commands run through `shortcuts::handle`.
    CommandSource { states: crate::panels::options_bar::selection::command_states, run: |_, _, _| false },
];

/// The tool commands: the current tool's row is checked.
fn tool_states(app: &PhotocraftApp) -> Vec<(CommandRef, CommandState)> {
    Tool::ALL.iter().map(|&tool| (CommandRef::from(crate::panels::tool_commands::command_of(tool)), CommandState::new().checked(app.ui.tool == tool))).collect()
}

/// Selects the tool a tool command names.
fn run_tool(app: &mut PhotocraftApp, _ctx: &egui::Context, command: &CommandRef) -> bool {
    let Some(command) = crate::panels::tool_commands::ToolCommand::from_ref(command) else {
        return false;
    };
    crate::tool_memory::select(app, crate::panels::tool_commands::tool_of(command));
    true
}

/// Runs `command` through the first source that owns it; whether one did.
pub(crate) fn run_command(app: &mut PhotocraftApp, ctx: &egui::Context, command: &CommandRef) -> bool {
    COMMAND_SOURCES.iter().any(|source| (source.run)(app, ctx, command))
}

impl PhotocraftApp {
    /// Before each frame (`raw_input_hook`): the runtime's output of the last frame, and its
    /// input for this one. Of the output, PhotoCraft keeps the fired commands (`shortcuts::handle`
    /// runs them), the commands the canvas ran (`record_view_commands`, in `logic`) and whether a
    /// component refused its input.
    pub(crate) fn framework_before_input(&mut self, ctx: &egui::Context) {
        // The catalog and the bindings come from walking the whole command table: only after a
        // preferences change.
        let rev = self.session.prefs.rev();
        if self.framework.overrides_rev != Some(rev) {
            let rows = crate::shortcut_dispatch::command_rows(self);
            if !crate::shortcut_dispatch::same_rows(&self.framework.catalog_rows, &rows) {
                self.framework.catalog_generation += 1;
                self.framework.catalog_rows = rows.clone();
            }
            let catalog = crate::shortcut_dispatch::catalog(rows, self.framework.catalog_generation);
            let overrides = crate::shortcut_dispatch::overrides(self);
            self.framework.set_overrides(rev, catalog, overrides);
        }
        if self.framework.sync_theme(&mut self.session) {
            crate::prefs_ui::show_selected_theme(self, ctx);
        }
        let states: Vec<(CommandRef, CommandState)> = COMMAND_SOURCES.iter().flat_map(|source| (source.states)(self)).collect();
        let gate = crate::menus::command_gate(self);
        let latched = self.ui.shell.latched;
        embed::before_input(ctx, |output: FrameOutput| {
            crate::shortcut_dispatch::record_chrome_toggles(ctx, &output.chrome_toggles);
            self.framework.fired.extend(output.fired);
            self.framework.delivered.extend(output.delivered);
            self.framework.refused |= output.refused;
            self.framework.frame_input(&mut self.session, ctx, &self.ui.theme, states, gate, latched)
        });
    }

    /// Whether a dialog is open, PhotoCraft's own or a framework one: the canvas, shortcuts, menus and tools under it
    /// take no input.
    pub fn dialog_open(&self) -> bool {
        !self.framework.dialogs.stack().is_empty()
    }

    /// How many dialogs are open, PhotoCraft's own and framework ones.
    pub fn dialog_count(&self) -> usize {
        self.framework.dialogs.stack().ids().count()
    }

    /// The id of the top open dialog.
    pub fn top_dialog(&self) -> Option<craft::ui::dialog::DialogId> {
        self.framework.dialogs.stack().top()
    }
}

/// The framework's settings sections, kept in PhotoCraft's preferences (`Preferences::sections`).
/// Components write from inside a pass, where the session isn't reachable: the tables wait here
/// and reach the preferences before the next frame.
#[derive(Default)]
struct PrefSections(Mutex<Tables>);

#[derive(Default)]
struct Tables {
    tables: BTreeMap<String, toml::Table>,
    /// The sections a component wrote since the last sync.
    written: BTreeSet<String>,
    /// Each section's preference revision as last synced, to notice a change from elsewhere (a
    /// reset, the control channel) without comparing the sections.
    seen: BTreeMap<String, u64>,
    /// How many sections a sync has read from the preferences.
    #[cfg(test)]
    loaded: usize,
}

impl PrefSections {
    /// Writes the sections the components wrote into the preferences, and takes the sections whose
    /// preference revision moved.
    fn sync(&self, session: &mut Session) {
        let mut t = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let written = std::mem::take(&mut t.written);
        if !written.is_empty() {
            let changed: Vec<(String, Value)> = written
                .iter()
                .filter_map(|key| Some((key.clone(), serde_json::to_value(t.tables.get(key)?).ok()?)))
                .filter(|(key, value)| session.prefs().sections.get(key) != Some(value))
                .collect();
            if !changed.is_empty() {
                session.edit_prefs(|p| p.sections.extend(changed));
            }
            for key in written {
                let rev = session.section_rev(&key);
                t.seen.insert(key, rev);
            }
        }
        let sections = &session.prefs().sections;
        let moved: Vec<&String> = sections.keys().filter(|key| t.seen.get(*key) != Some(&session.section_rev(key))).collect();
        let gone: Vec<String> = t.seen.keys().filter(|key| !sections.contains_key(*key)).cloned().collect();
        for key in moved {
            match serde_json::from_value(sections[key].clone()) {
                Ok(table) => t.tables.insert(key.clone(), table),
                Err(_) => t.tables.remove(key),
            };
            t.seen.insert(key.clone(), session.section_rev(key));
            #[cfg(test)]
            {
                t.loaded += 1;
            }
        }
        for key in gone {
            t.tables.remove(&key);
            t.seen.remove(&key);
        }
    }
}

impl SectionStore for PrefSections {
    fn read(&self, key: &str) -> Option<toml::Table> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).tables.get(key).cloned()
    }

    fn write(&self, key: &str, table: toml::Table) {
        let mut t = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        t.tables.insert(key.to_owned(), table);
        t.written.insert(key.to_owned());
    }
}

/// One frame with the runtime installed on a fresh context, running only the root window's close
/// path with PhotoCraft's decision, as `logic` does.
#[cfg(test)]
pub(crate) fn close_path_frame(app: &mut PhotocraftApp, input: egui::RawInput) -> egui::FullOutput {
    let ctx = app.ctx().clone();
    install(&ctx, &UiFonts::new(&craft::ui::fonts::I18nScripts), None);
    ctx.run_ui(input, |ui| embed::logic(ui.ctx(), || crate::discard_ui::close_decision(app)))
}

/// Builds a kittest harness over the app whose every pass runs the app's frame hooks (`raw_input_hook` before it,
/// `logic` ahead of the UI) around `body`, the pass's UI, as eframe does. The harness steps one pass per queued
/// event, so the hooks belong to the harness's passes, not to the code that steps it.
#[cfg(test)]
pub(crate) trait BuildApp {
    fn build_app(
        self,
        body: impl FnMut(&mut egui::Ui, &mut PhotocraftApp) + 'static,
        app: Box<PhotocraftApp>,
    ) -> egui_kittest::Harness<'static, Box<PhotocraftApp>>;
}

#[cfg(test)]
impl BuildApp for egui_kittest::HarnessBuilder<Box<PhotocraftApp>> {
    fn build_app(
        self,
        body: impl FnMut(&mut egui::Ui, &mut PhotocraftApp) + 'static,
        mut app: Box<PhotocraftApp>,
    ) -> egui_kittest::Harness<'static, Box<PhotocraftApp>> {
        app.framework.test_body = Some(Box::new(body));
        let ctx = app.ctx().clone();
        self.with_context(ctx).build_eframe(move |_| app)
    }
}

/// Runs one frame of `input` on `ctx` as the app's runtime key pass: the runtime takes the input
/// with the app's command states, catalog and overrides, `body` runs in the pass, and the commands
/// the pass's keys fired go to `shortcuts::handle` in the same frame, as `raw_input_hook` and
/// `logic` hand them over in the next one.
#[cfg(test)]
pub(crate) fn keys_pass(app: &mut PhotocraftApp, ctx: &egui::Context, input: egui::RawInput, body: impl FnMut(&mut egui::Ui)) {
    app.framework_before_input(ctx);
    crate::testing::run_ui(ctx, input, body);
    // Collects what this pass fired.
    app.framework_before_input(ctx);
    crate::shortcuts::handle(app, ctx);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(n: i64) -> toml::Table {
        let mut t = toml::Table::new();
        t.insert("n".into(), toml::Value::Integer(n));
        t
    }

    /// A sync reads and writes only the sections that changed, one revision per section.
    #[test]
    fn section_sync_touches_only_changed_sections() {
        let store = PrefSections::default();
        let mut session = Session::new();
        store.write("one", table(1));
        store.write("two", table(2));
        store.sync(&mut session);
        assert_eq!(session.prefs().sections.len(), 2);
        let (one, two) = (session.section_rev("one"), session.section_rev("two"));
        let loaded = store.0.lock().unwrap().loaded;

        store.sync(&mut session);
        assert_eq!((session.section_rev("one"), session.section_rev("two")), (one, two));
        assert_eq!(store.0.lock().unwrap().loaded, loaded, "an idle sync reads nothing");

        store.write("two", table(3));
        store.sync(&mut session);
        assert_eq!(session.section_rev("one"), one, "a section nobody wrote keeps its revision");
        assert_eq!(session.section_rev("two"), two.wrapping_add(1));
        assert_eq!(store.0.lock().unwrap().loaded, loaded, "a component's own write is not read back");

        session.edit_prefs(|p| {
            p.sections.insert("one".into(), serde_json::json!({ "n": 9 }));
        });
        store.sync(&mut session);
        assert_eq!(store.0.lock().unwrap().loaded, loaded + 1, "only the changed section is read");
        assert_eq!(store.read("one"), Some(table(9)));
        assert_eq!(store.read("two"), Some(table(3)));

        session.edit_prefs(|p| {
            p.sections.remove("one");
        });
        store.sync(&mut session);
        assert_eq!(store.read("one"), None);
    }

    /// Every theme PhotoCraft has a palette for names a framework built-in.
    #[test]
    fn every_palette_theme_is_a_builtin() {
        let builtins = craft::theme::builtins();
        for theme in crate::theme::AppTheme::all() {
            assert!(builtins.iter().any(|t| t.id == *theme.id()), "{} is no built-in theme", theme.id().as_str());
        }
        assert_eq!(crate::theme::AppTheme::pro_medium().id().as_str(), "craft.artcraft");
    }

    /// The size overrides PhotoCraft ships read without an error, so none is dropped at startup.
    #[test]
    fn the_size_overrides_parse() {
        if let Err(errors) = craft::theme::AppSizes::parse(super::SIZES) {
            panic!("sizes.json: {errors:?}");
        }
    }

    /// The overrides reach the root every frame's tokens resolve under, not only the suite's defaults.
    #[test]
    fn the_root_carries_the_size_overrides() {
        assert_eq!(*super::base_root().sizes, craft::theme::AppSizes::parse(super::SIZES).unwrap());
    }
}
