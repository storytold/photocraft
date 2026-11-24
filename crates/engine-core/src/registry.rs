//! The command registry: the command tables a process installs, in their order, looked up by id.

use std::collections::HashMap;
use std::sync::{LazyLock, OnceLock};

use serde_json::{Value, json};

use crate::hooks::Hooks;
use crate::slots::SlotInit;
use crate::{EngineError, Result, Session};

type Run = fn(&mut Session, &Value) -> Result<Value>;
type Enabled = fn(&Session) -> std::result::Result<(), String>;

/// Metadata + implementation for one command.
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Menu placement, e.g. `["Layer", "New"]`. Empty = not in menus.
    pub menu: &'static [&'static str],
    /// Default shortcut in egui-style notation (`Cmd+Shift+N`), mapped per platform by the UI.
    pub shortcut: Option<&'static str>,
    /// Human/agent-readable parameter description (JSON-ish).
    pub params: &'static str,
    pub enabled: Enabled,
    pub run: Run,
    /// Record in the session journal (false for queries).
    pub journal: bool,
}

/// One command table: a module's or a table fragment's specs, under the name that identifies it
/// when two installs are compared and in the order test's messages.
pub type Table = (&'static str, fn() -> Vec<CommandSpec>);

/// What a process installs before its first session.
#[derive(Default)]
pub struct Install {
    /// The commands: these tables in turn, each in its own order, which is the registry order
    /// unless [`Self::order`] gives one. Menus, the Command Palette and `command.list` list
    /// commands in registry order.
    pub commands: Vec<Table>,
    /// The registry order across the tables of several crates: command ids, one per line. Commands
    /// it lists come in its order, then any others in table order. The app's comes from
    /// `crates/engine/src/install/command_ids.txt`.
    pub order: Option<&'static str>,
    /// The pipeline hooks of the crates whose tables these are.
    pub hooks: Hooks,
    /// Each crate's commands whose docs do not yet name every key they read: their params are not
    /// checked for unknown keys ([`is_incomplete`]). A crate lists only its own commands, sorted.
    pub incomplete: Vec<&'static [&'static str]>,
    /// The session slots of the crates whose tables these are, one per type.
    pub(crate) slots: Vec<SlotInit>,
}

impl Install {
    /// Give every session a slot of `T` ([`Session::slot`]); a type already registered keeps its one.
    pub(crate) fn slot<T: Default + Send + 'static>(&mut self) {
        let init = SlotInit::of::<T>();
        if !self.slots.iter().any(|s| s.id() == init.id()) {
            self.slots.push(init);
        }
    }
}

/// Install `install` for this process. Installing the same tables again (the same names in the
/// same order) does nothing. Different tables are refused with [`EngineError::InstallConflict`],
/// and from then on every command fails with it and the registry lists nothing.
/// Only the tables are compared: each crate's `register` adds its hooks and slots with its tables,
/// so the same tables come with the same hooks and slots.
pub fn install(install: Install) -> Result<()> {
    INSTALLED.install(install)
}

/// Every installed command, in registry order (none before an install).
pub fn command_specs() -> &'static [CommandSpec] {
    INSTALLED.registry().map_or(&[][..], |r| r.specs.as_slice())
}

/// The installed command `id`, if any.
pub fn find(id: &str) -> Option<&'static CommandSpec> {
    INSTALLED.registry().ok()?.get(id)
}

/// Whether the installed command `id` is on a crate's incomplete-docs list: unknown keys are not
/// rejected yet.
pub fn is_incomplete(id: &str) -> bool {
    INSTALLED.installed().is_ok_and(|install| install.incomplete.iter().any(|list| list.contains(&id)))
}

/// The installed pipeline hooks (none before an install, or after a conflicting one).
pub(crate) fn hooks() -> &'static Hooks {
    static NO_HOOKS: LazyLock<Hooks> = LazyLock::new(Hooks::default);
    INSTALLED.installed().map_or(&*NO_HOOKS, |install| &install.hooks)
}

/// The installed session slots; none before an install or after a conflicting one (a session made
/// then reports the install at its first command, and at each slot it reads).
pub(crate) fn slot_inits() -> &'static [SlotInit] {
    INSTALLED.installed().map_or(&[], |install| install.slots.as_slice())
}

/// Why a session has no slot of a type: the install's error, or [`EngineError::NotInstalled`] when
/// the crate that registers it was not installed before the session was made.
pub(crate) fn slot_error() -> EngineError {
    INSTALLED.installed().err().unwrap_or(EngineError::NotInstalled)
}

/// The installed command `id`, for running it: [`EngineError::NotInstalled`] before an install,
/// [`EngineError::UnknownCommand`] for an id no table has.
pub fn spec(id: &str) -> Result<&'static CommandSpec> {
    INSTALLED.registry()?.get(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))
}

/// One `command.list` / `engine.commands` row for `c`, given why it can't run now (`None` = it can).
/// `disabledReason` is that message; `needsDocument` is true when the reason is that no document is
/// open (#3664), so an agent can tell "open or create a document first" from a precondition that a
/// document alone won't meet (a selection, an empty clipboard).
pub fn list_entry(s: &Session, c: &CommandSpec, reason: Option<String>) -> Value {
    let needs_document =
        s.active().is_none() && reason.as_deref().is_some_and(|r| ["no document", "no active document", "no open document"].iter().any(|p| r.starts_with(p)));
    let mut entry = json!({
        "id": c.id, "label": c.label, "menu": c.menu, "shortcut": c.shortcut, "params": c.params,
        "enabled": reason.is_none(), "disabledReason": reason, "needsDocument": needs_document,
    });
    if let (Some(k), Some(o)) = (search_keywords(c.id), entry.as_object_mut()) {
        o.insert("keywords".into(), json!(k));
    }
    entry
}

/// Other names a command is looked for by, beyond its id and label: the Photoshop tools and
/// options it stands for (#5060). `select.rect` is also the Elliptical and Single Row/Column
/// Marquee, which an agent searches for as "ellipse" or "marquee". The RAW Develop filter and
/// RAW Defaults keep Photoshop's "Camera Raw" name as a search alias only: the menus and windows
/// say "RAW Develop" (#12403).
pub(crate) const SEARCH_KEYWORDS: &[(&str, &str)] = &[
    ("select.rect", "Rectangular Marquee, Elliptical Marquee (\"ellipse\":true), ellipse, oval, circle, Single Row/Column Marquee (\"line\")"),
    ("filter.cameraRaw", "Camera Raw Filter"),
    ("edit.preferences.rawDefaults", "Camera Raw Defaults"),
];

/// [`SEARCH_KEYWORDS`] of command `id`, if it has any.
pub fn search_keywords(id: &str) -> Option<&'static str> {
    SEARCH_KEYWORDS.iter().find(|(k, _)| *k == id).map(|(_, words)| *words)
}

/// Does a [`list_entry`] row match a command-list filter (`commands --filter`, `engine.commands`,
/// MCP `command_list`)? Its id, label or keywords contain `needle`, ignoring case.
pub fn list_entry_matches(entry: &Value, needle: &str) -> bool {
    let needle = needle.to_lowercase();
    ["id", "label", "keywords"].iter().any(|k| entry.get(*k).and_then(Value::as_str).is_some_and(|v| v.to_lowercase().contains(&needle)))
}

static INSTALLED: InstallCell = InstallCell::new();

/// A process's install: the first install, the registry built from its tables on first use,
/// and the conflict a later install with other tables raised.
struct InstallCell {
    installed: OnceLock<Install>,
    registry: OnceLock<Registry>,
    conflict: OnceLock<EngineError>,
}

impl InstallCell {
    const fn new() -> Self {
        Self { installed: OnceLock::new(), registry: OnceLock::new(), conflict: OnceLock::new() }
    }

    fn install(&self, install: Install) -> Result<()> {
        let mut requested = Some(install);
        let installed = &self.installed.get_or_init(|| requested.take().unwrap_or_default()).commands;
        let Some(Install { commands: requested, .. }) = requested else { return Ok(()) };
        let name = |tables: &[Table], at: usize| tables.get(at).map_or_else(|| "(none)".to_string(), |t| t.0.to_string());
        let Some(at) = (0..installed.len().max(requested.len())).find(|&at| installed.get(at).map(|t| t.0) != requested.get(at).map(|t| t.0)) else {
            return Ok(());
        };
        let conflict = EngineError::InstallConflict { at, installed: name(installed.as_slice(), at), requested: name(requested.as_slice(), at) };
        let _ = self.conflict.set(conflict.clone());
        Err(conflict)
    }

    fn installed(&self) -> Result<&Install> {
        if let Some(conflict) = self.conflict.get() {
            return Err(conflict.clone());
        }
        self.installed.get().ok_or(EngineError::NotInstalled)
    }

    fn registry(&self) -> Result<&Registry> {
        let install = self.installed()?;
        Ok(self.registry.get_or_init(|| Registry::new(build(&install.commands, install.order))))
    }
}

struct Registry {
    specs: Vec<CommandSpec>,
    by_id: HashMap<&'static str, usize>,
}

impl Registry {
    fn new(specs: Vec<CommandSpec>) -> Self {
        let mut by_id = HashMap::with_capacity(specs.len());
        for (index, spec) in specs.iter().enumerate() {
            // Preserve the linear lookup's first-match behavior if an id is duplicated.
            by_id.entry(spec.id).or_insert(index);
        }
        Self { specs, by_id }
    }

    fn get(&self, id: &str) -> Option<&CommandSpec> {
        self.by_id.get(id).and_then(|&index| self.specs.get(index))
    }
}

/// Every command of `tables`, in registry order: by `order` where it lists them, else in table order.
fn build(tables: &[Table], order: Option<&str>) -> Vec<CommandSpec> {
    let specs = tables.iter().flat_map(|(_, specs)| specs());
    // The dispatch guard's unit test needs a command that panics on purpose (REL-3).
    #[cfg(test)]
    let specs = specs.chain(std::iter::once(test_panic_command()));
    let mut specs: Vec<CommandSpec> = specs.collect();
    if let Some(order) = order {
        let rank: HashMap<&str, usize> = order.lines().enumerate().map(|(i, id)| (id, i)).collect();
        specs.sort_by_key(|c| rank.get(c.id).copied().unwrap_or(usize::MAX));
    }
    specs
}

/// A command whose `run` panics on purpose, so the last-resort guard in `jobs::dispatch`
/// can be tested (REL-3). Test builds only: [`build`] adds it under `cfg(test)`.
#[cfg(test)]
fn test_panic_command() -> CommandSpec {
    #[allow(clippy::panic)]
    fn run(_: &mut Session, _: &Value) -> Result<Value> {
        panic!("test: this command panicked on purpose")
    }
    CommandSpec { id: "test.panic", label: "Test Panic", menu: &[], shortcut: None, params: r##"{}"##, enabled: |_: &Session| Ok(()), run, journal: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &'static str) -> CommandSpec {
        CommandSpec { id, label: id, menu: &[], shortcut: None, params: "{}", enabled: |_| Ok(()), run: |_, _| Ok(Value::Null), journal: true }
    }

    #[test]
    fn duplicate_ids_keep_the_first_spec_and_listing_order() {
        let mut second = spec("a.one");
        second.label = "second";
        let registry = Registry::new(vec![spec("a.one"), second]);
        assert_eq!(registry.get("a.one").map(|c| c.label), Some("a.one"));
        assert_eq!(registry.specs.len(), 2);
    }

    #[test]
    fn lookups_before_an_install_fail_clearly_and_installs_must_agree() {
        let a: Table = ("a", || vec![spec("a.one")]);
        let b: Table = ("b", || vec![spec("b.one")]);
        let cell = InstallCell::new();
        assert!(matches!(cell.registry(), Err(EngineError::NotInstalled)));

        cell.install(Install { commands: vec![a], ..Install::default() }).unwrap();
        assert!(cell.registry().unwrap().get("a.one").is_some());
        cell.install(Install { commands: vec![a], ..Install::default() }).unwrap();
        assert!(cell.registry().unwrap().get("a.one").is_some(), "the same tables again change nothing");

        let conflict = cell.install(Install { commands: vec![a, b], ..Install::default() }).unwrap_err();
        assert_eq!(conflict, EngineError::InstallConflict { at: 1, installed: "(none)".into(), requested: "b".into() });
        assert_eq!(cell.registry().err(), Some(conflict), "every later lookup reports the conflict");
    }

    #[test]
    fn an_order_sorts_the_commands_of_several_tables_and_keeps_the_rest_after_them() {
        let a: Table = ("a", || vec![spec("a.one"), spec("a.two")]);
        let b: Table = ("b", || vec![spec("b.one"), spec("b.two")]);
        let ids = |specs: Vec<CommandSpec>| specs.into_iter().map(|c| c.id).filter(|id| *id != "test.panic").collect::<Vec<_>>();
        assert_eq!(ids(build(&[a, b], None)), ["a.one", "a.two", "b.one", "b.two"]);
        assert_eq!(ids(build(&[a, b], Some("b.one\na.two\na.one\n"))), ["b.one", "a.two", "a.one", "b.two"]);
    }
}
