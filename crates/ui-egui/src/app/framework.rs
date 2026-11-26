//! The craft framework's runtime on PhotoCraft's context, hosted through `craft::ui::shell::embed`
//! (craft-libs docs/craft-framework.md 2.2, "Host the runtime from your own eframe `App`"):
//! installed once with PhotoCraft's fonts, then given PhotoCraft's preferences before each frame.
//!
//! eframe's `ui` calls `embed::before_ui` first, which hands the runtime the native window (on
//! Windows the runtime answers its hit test: resize edges, the title bar's move area, snap
//! layouts) and fits the window to its monitor once.
//!
//! The `craft` facade features PhotoCraft enables for this: `text`, `text-font-picker`,
//! `font-manager`, `menu`, `timeline` and `print`, plus `dialogs` on native targets.

use photocraft_engine::actions_cmds::ActionsExt;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use craft::host::target::{Backend, ShellInfo};
use craft::host::{AppInfo, Files, Host, SectionStore};
use craft::i18n::Direction;
use craft::keymap::Overrides;
use craft::theme::{AppDefaults, AppSizes, ComponentSet, ModeChoice, Size, SuiteChoice, ThemeId, ThemePair, resolve_sized};
use craft::theme_store::{SelectionChange, SuiteClient};
use craft::ui::contract::{MemoProps, Tracked};
use craft::ui::dialog::DialogHost;
use craft::ui::fonts::UiFonts;
use craft::ui::runtime::commands::{AppCatalog, CommandInput, CommandRef, CommandSet as _, CommandState, CommandStates, Fired, NavCommand};
use craft::ui::runtime::env::{PointSize, RootEnv, Scale, SystemReader};
use craft::ui::runtime::focus::TabReach;
use craft::ui::runtime::frame::{FrameInput, FrameOutput};
use craft::ui::runtime::install::WheelModifiers;
use craft::ui::runtime::{InstallError, InstallSpec};
use craft::ui::shell::embed;
use craft::window::policy::Decision;
// Tests run headless: the desktop pointer Windows tells is the mouse of the machine running them, not the test's
// pointer, so tests see no desktop services on any platform, as on Linux.
#[cfg(test)]
use craft::window::system::NoWindows as Desktop;
#[cfg(not(test))]
use craft::window::system::SystemWindows as Desktop;
use photocraft_engine::Session;
use photocraft_engine::prefs::{AppearanceMode, Preferences};
use serde_json::Value;

use super::PhotocraftApp;
use crate::i18n;
use crate::state::Tool;
use crate::theme::AppTheme;

/// The store the dialogs that open their own window (RAW Develop, Vanishing Point) keep their size and place in:
/// `window.json` in the settings folder `settings`. Without a settings folder (tests, the web) there is none, and those
/// windows open at their default size each time.
pub(crate) fn placement_store(settings: Option<&std::path::Path>) -> Option<Arc<Mutex<craft::window::restore::Store>>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        settings.map(|dir| Arc::new(Mutex::new(craft::window::restore::Store::at(&dir.join(craft::window::restore::FILE_NAME)))))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = settings;
        None
    }
}

/// Installs the runtime on `ctx` with `fonts`, the root window's decided `frame` (`None` where
/// the platform decides none) and the window placement store of the settings folder `settings`
/// ([`placement_store`]), unless it is there already (`setup_window` again for the same context).
/// Only `theme::install_runtime` calls it.
pub(crate) fn install(ctx: &egui::Context, fonts: &UiFonts<'_>, frame: Option<Decision>, settings: Option<&std::path::Path>) {
    // The wheel zooms with the canvas's own keys and scrolls sideways with Shift or Command, as
    // PhotoCraft's input always has.
    let wheel = WheelModifiers::new(egui::Modifiers::NONE, egui::Modifiers::SHIFT | egui::Modifiers::COMMAND);
    let wake = ctx.clone();
    let mut spec = InstallSpec::new(ShellInfo::default().backend(Backend::from_env()), Arc::new(Desktop))
        .wheel(wheel)
        .app_id(crate::os_window::app_id())
        .system(SystemReader::start(move || wake.request_repaint()));
    if let Some(decision) = frame {
        spec = spec.frame(decision);
    }
    // Without it a dialog's own window saves its size and place nowhere and opens at its default each time (#17012).
    if let Some(store) = placement_store(settings) {
        spec = spec.placement(store);
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
    /// What `overrides` was built from: the preferences' catalog revision and the catalog's generation.
    overrides_key: Option<(u64, u64)>,
    /// Advances each time `overrides` is replaced.
    overrides_generation: u64,
    /// The app's command catalog, built from `catalog_rows`.
    catalog: Option<AppCatalog>,
    /// The rows `catalog` was built from ([`command_rows`](crate::shortcut_dispatch::command_rows)), the action
    /// names they were built from (their one input that changes at run time), the actions list's revision those names
    /// were last read at, and the catalog's generation: the runtime rebuilds its keymap when the generation changes,
    /// so it advances only when the names change. The revision is the fast check; it assumes the list is edited in
    /// place or through `ActionList::replace`, which keep it moving forward.
    catalog_rows: Vec<craft::keymap::CommandDef>,
    catalog_actions: Vec<String>,
    catalog_rows_rev: Option<u64>,
    catalog_generation: u64,
    /// How many times the rows and the overrides were built, for the count trials.
    #[cfg(test)]
    pub(crate) catalog_builds: usize,
    #[cfg(test)]
    pub(crate) overrides_builds: usize,
    /// The command sources' states the runtime reads ([`command_states`]).
    pub(crate) held_states: HeldStates,
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
/// mode, Pro in the light, and the dark mode, with the Creative component set.
fn defaults() -> AppDefaults {
    let theme = |id: &str| ThemeId::parse(id).unwrap_or_else(|_| ThemeId::suite_default());
    AppDefaults {
        theme: ThemePair { light: theme(DEFAULT_LIGHT), dark: theme(DEFAULT_DARK) },
        mode: ModeChoice::Dark,
        size: Size::default(),
        set: ComponentSet::Creative,
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
    // The title bar keeps the arrow, also over the menu titles on it, as an OS title bar does: the default open hand
    // shows as Windows' four-way Move cursor (#16993).
    root.values.set(craft::ui::contract::MoveBandCursor::Arrow);
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
            overrides_key: None,
            overrides_generation: 0,
            catalog: None,
            catalog_rows: Vec::new(),
            catalog_actions: Vec::new(),
            catalog_rows_rev: None,
            catalog_generation: 0,
            #[cfg(test)]
            catalog_builds: 0,
            #[cfg(test)]
            overrides_builds: 0,
            held_states: HeldStates::default(),
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

    /// The input for the coming frame, from PhotoCraft's preferences: the held states of every command
    /// source, the held catalog and overrides, and `gate` over the commands.
    fn frame_input(
        &mut self,
        session: &mut Session,
        ctx: &egui::Context,
        theme: &AppTheme,
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
        let mut commands = CommandInput::new().states(&self.held_states.states);
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
        // A nightly keeps its in-memory suite; never open the stable shared theme store.
        if !crate::os_window::stable_services_allowed() {
            return;
        }
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

    /// What the runtime's keymap was last built from: the generations of the overrides and of the catalog. They
    /// change at the start of the frame whose keymap follows them, so a surface that shows the keymap's chords (the
    /// menu model) hashes them to rebuild in that frame.
    pub(crate) fn keymap_inputs(&self) -> (u64, u64) {
        (self.overrides_generation, self.catalog_generation)
    }

    /// The catalog's rows when they were built at actions list revision `actions_rev`, else `None`.
    pub(crate) fn catalog_rows_at(&self, actions_rev: u64) -> Option<&[craft::keymap::CommandDef]> {
        (self.catalog_rows_rev == Some(actions_rev)).then_some(self.catalog_rows.as_slice())
    }

    /// Takes `rows`, built from the action names `actions` at actions list revision `actions_rev`, as the catalog's
    /// rows: a new catalog generation.
    fn set_catalog(&mut self, actions_rev: u64, actions: Vec<String>, rows: Vec<craft::keymap::CommandDef>) {
        self.catalog_generation += 1;
        self.catalog = crate::shortcut_dispatch::catalog(rows.clone(), self.catalog_generation);
        self.catalog_rows = rows;
        self.catalog_actions = actions;
        self.catalog_rows_rev = Some(actions_rev);
        #[cfg(test)]
        {
            self.catalog_builds += 1;
        }
    }

    /// Takes the runtime's keymap overrides built from `key` ([`Self::overrides_key`]). The pane
    /// moves are unbound in the overrides, because PhotoCraft's own table binds F6 and Shift+F6
    /// (Window › Color, Select › Modify › Feather) until those are a framework `CommandSet`. The
    /// held `Arc` is replaced only when the bindings differ, so the runtime rebuilds its keymap
    /// only on a change.
    fn set_overrides(&mut self, key: (u64, u64), mut overrides: Overrides) {
        self.overrides_key = Some(key);
        for command in [NavCommand::NextPane, NavCommand::PreviousPane] {
            overrides.insert(CommandRef::from(command).key(), Vec::new());
        }
        if *self.overrides != overrides {
            self.overrides = Arc::new(overrides);
            self.overrides_generation += 1;
        }
        #[cfg(test)]
        {
            self.overrides_builds += 1;
        }
    }
}

/// One source's command states.
pub(crate) type SourceStates = Vec<(CommandRef, CommandState)>;

/// The id a command source's states are derived under.
pub(crate) fn states_id(source: &'static str) -> egui::Id {
    egui::Id::unique(("photocraft-states", source))
}

/// `build`'s states for `source`, derived from `facts`: the kept `Arc` while the facts are unchanged, else a new build.
/// The facts can hold only checked inputs, but nothing checks that they hold everything `build` reads: a read they
/// leave out keeps stale states, which the held-equals-fresh trial catches.
pub(crate) fn derived_states<K: MemoProps>(app: &PhotocraftApp, source: &'static str, facts: &K, build: impl FnOnce() -> SourceStates) -> Arc<SourceStates> {
    craft::ui::runtime::derived::derived(app.ctx(), states_id(source), facts, build)
}

/// The states of a source that has no command states: one shared empty list, whatever the source's name.
pub(crate) fn no_states(app: &PhotocraftApp, _name: &'static str) -> Arc<SourceStates> {
    derived_states(app, "none", &(), Vec::new)
}

/// A source's states for the coming frame, derived under the source's name ([`states_id`]) from what they read.
pub(crate) type StatesOf = fn(&PhotocraftApp, &'static str) -> Arc<SourceStates>;

/// One user of the framework's command sets: the states its commands show and the runner of the
/// ones it fires.
pub(crate) struct CommandSource {
    /// The name the source's states are derived under.
    pub(crate) name: &'static str,
    /// The states of the source's commands.
    pub(crate) states: StatesOf,
    /// Runs `command` when it is this source's; whether it was.
    run: fn(&mut PhotocraftApp, &egui::Context, &CommandRef) -> bool,
}

/// Every command source besides the panels' ([`crate::panels::registry::PANELS`], one source each), in the order
/// [`run_command`] tries them after the panels. The theme has no entry: it has no command state, and its picks run
/// through `menus::invoke`.
pub(crate) const COMMAND_SOURCES: &[CommandSource] = &[
    CommandSource { name: "tool", states: tool_states, run: run_tool },
    CommandSource { name: "workspace", states: crate::workspace_ui::source_states, run: crate::workspace_ui::run_fired },
    CommandSource { name: "slice", states: crate::slice_ui::source_states, run: crate::slice_ui::run_fired },
    CommandSource { name: "align", states: crate::align_bar::source_states, run: crate::align_bar::run_fired },
    CommandSource { name: "artboard", states: crate::artboard_tool::source_states, run: crate::artboard_tool::run_fired },
    // Space+F: the pasteboard colour keys of the held Hand.
    CommandSource { name: "canvasColor", states: no_states, run: crate::canvas_tool_menu::run_canvas_color },
    // The options bar's engine-command buttons: their commands run through `shortcuts::handle`.
    CommandSource { name: "selection", states: crate::panels::options_bar::selection::source_states, run: |_, _, _| false },
];

/// Every source by its name, with its states: the command sources, then the panels, each under its panel id. A later
/// source's state wins for a command two sources name.
pub(crate) fn sources() -> impl Iterator<Item = (&'static str, StatesOf)> {
    let panels = crate::panels::registry::PANELS.iter().map(|panel| (panel.id, panel.states));
    COMMAND_SOURCES.iter().map(|source| (source.name, source.states)).chain(panels)
}

/// Every source's states, in [`sources`]' order.
fn source_states(app: &PhotocraftApp) -> impl Iterator<Item = Arc<SourceStates>> + '_ {
    sources().map(move |(name, states)| states(app, name))
}

/// The states each source last gave, in [`source_states`]' order, and the one list of all of them the runtime reads.
#[derive(Default)]
pub(crate) struct HeldStates {
    kept: Vec<Arc<SourceStates>>,
    /// Every source's states, written only when one source's list changes: the runtime reads them again only when
    /// the revision moves.
    pub(crate) states: Tracked<CommandStates>,
}

/// The tool commands: the current tool's row is checked.
fn build_tool_states(app: &PhotocraftApp) -> SourceStates {
    Tool::ALL.iter().map(|&tool| (CommandRef::from(crate::panels::tool_commands::command_of(tool)), CommandState::new().checked(app.ui.tool == tool))).collect()
}

/// [`build_tool_states`], derived from the tool.
fn tool_states(app: &PhotocraftApp, name: &'static str) -> Arc<SourceStates> {
    derived_states(app, name, &(app.ui.tool as u64), || build_tool_states(app))
}

/// Brings the held states up to date for the coming frame. A source whose states are the `Arc` it last gave costs a
/// pointer compare; the held list is written only when a source's states differ by value from the ones it last gave.
fn command_states(app: &PhotocraftApp, held: &mut HeldStates) {
    let mut changed = false;
    for (at, states) in source_states(app).enumerate() {
        match held.kept.get_mut(at) {
            Some(kept) if Arc::ptr_eq(kept, &states) => {}
            Some(kept) => {
                changed |= **kept != *states;
                *kept = states;
            }
            None => {
                held.kept.push(states);
                changed = true;
            }
        }
    }
    if changed {
        let mut states = CommandStates::new();
        for (command, state) in held.kept.iter().flat_map(|kept| kept.iter()) {
            states.set(command.clone(), state.clone());
        }
        *held.states = states;
    }
}

#[cfg(test)]
mod catalog_tests;

/// Selects the tool a tool command names.
fn run_tool(app: &mut PhotocraftApp, _ctx: &egui::Context, command: &CommandRef) -> bool {
    let Some(command) = crate::panels::tool_commands::ToolCommand::from_ref(command) else {
        return false;
    };
    crate::tool_memory::select(app, crate::panels::tool_commands::tool_of(command));
    true
}

/// Runs `command` through the first source that owns it, the panels first; whether one did.
pub(crate) fn run_command(app: &mut PhotocraftApp, ctx: &egui::Context, command: &CommandRef) -> bool {
    crate::panels::registry::run_fired(app, ctx, command) || COMMAND_SOURCES.iter().any(|source| (source.run)(app, ctx, command))
}

impl PhotocraftApp {
    /// Before each frame (`raw_input_hook`): the runtime's output of the last frame, and its
    /// input for this one. Of the output, PhotoCraft keeps the fired commands (`shortcuts::handle`
    /// runs them), the commands the canvas ran (`record_view_commands`, in `logic`) and whether a
    /// component refused its input.
    pub(crate) fn framework_before_input(&mut self, ctx: &egui::Context) {
        // The catalog's rows change only with the action names; the bindings with the rows and the
        // shortcut preferences.
        let list = &self.session.actions().list;
        let actions_rev = list.revision();
        if self.framework.catalog_rows_rev != Some(actions_rev) {
            let same = self.framework.catalog_rows_rev.is_some()
                && self.framework.catalog_actions.iter().map(String::as_str).eq(list.actions().iter().map(|action| action.name.as_str()));
            if same {
                self.framework.catalog_rows_rev = Some(actions_rev);
            } else {
                let actions = list.actions().iter().map(|action| action.name.clone()).collect();
                let rows = crate::shortcut_dispatch::command_rows(self);
                self.framework.set_catalog(actions_rev, actions, rows);
            }
        }
        let overrides_key = (self.session.prefs.catalog_rev(), self.framework.catalog_generation);
        if self.framework.overrides_key != Some(overrides_key) {
            let overrides = crate::shortcut_dispatch::overrides(self, &self.framework.catalog_rows);
            self.framework.set_overrides(overrides_key, overrides);
        }
        if self.framework.sync_theme(&mut self.session) {
            crate::prefs_ui::show_selected_theme(self, ctx);
        }
        let mut held = std::mem::take(&mut self.framework.held_states);
        command_states(self, &mut held);
        self.framework.held_states = held;
        let gate = crate::menus::command_gate(self);
        let latched = self.ui.shell.latched;
        embed::before_input(ctx, |output: FrameOutput| {
            crate::shortcut_dispatch::record_chrome_toggles(ctx, &output.chrome_toggles);
            self.framework.fired.extend(output.fired);
            self.framework.delivered.extend(output.delivered);
            self.framework.refused |= output.refused;
            self.framework.frame_input(&mut self.session, ctx, &self.ui.theme, gate, latched)
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
    install(&ctx, &UiFonts::new(&craft::ui::fonts::I18nScripts), None, None);
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

    /// The dialogs' own windows keep their place in the settings folder's `window.json` and find it there on the next
    /// launch; without a settings folder nothing is kept (#17012).
    #[test]
    fn dialog_windows_keep_their_placement_in_the_settings_folder() {
        use craft::window::restore::FILE_NAME;
        let dir = tempfile::tempdir().unwrap();
        let key = "craft_imaging.raw_develop.window";
        let placement = craft::behavior::placement::Placement::new([1400.0, 900.0], 1.25);
        let store = placement_store(Some(dir.path())).expect("a settings folder has a placement store");
        {
            let mut store = store.lock().unwrap();
            store.set(key, &placement);
            store.save().unwrap();
        }
        assert!(dir.path().join(FILE_NAME).is_file());
        let reopened = placement_store(Some(dir.path())).unwrap();
        assert_eq!(reopened.lock().unwrap().get(key).map(|p| p.normal.size), Some([1400.0, 900.0]), "the next launch reads it");
        assert!(placement_store(None).is_none());
    }

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
