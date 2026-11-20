//! ⌘F command palette: the framework's `CommandPalette` over every menu command and tool.

use craft::ui::contract::Bind;
use craft::ui::menu::HelpEntry;
use craft::ui::patterns::panels::CommandPalette;
use craft::ui::runtime::commands::CommandRef;

use crate::PhotocraftApp;
use crate::state::Tool;

/// What the app keeps for the palette: whether it is open and the commands last run. Neither is saved: the palette
/// starts closed and the recents last one session.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PaletteState {
    /// The palette is open.
    #[serde(skip)]
    pub open: bool,
    /// The commands last run, newest first (per session).
    #[serde(skip)]
    pub recents: Vec<CommandRef>,
}

/// Every command the palette lists: the menu commands with their paths and English names, and one entry per tool's
/// select command.
pub(crate) fn entries(app: &mut PhotocraftApp, ctx: &egui::Context) -> Vec<HelpEntry> {
    let mut entries = crate::menus::help_entries(app, ctx);
    entries.extend(
        Tool::ALL.iter().map(|&tool| {
            HelpEntry::new(CommandRef::new(crate::tool_shortcuts::id(tool)), tl!(tool.label())).with_english(tool.label()).with_path([tl!("Tool")])
        }),
    );
    entries
}

/// Shows the palette while `app.ui.palette.open` is set. A pick fires through the runtime as a palette command and
/// runs where a menu's fired command runs, so nothing is invoked here.
pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if !app.ui.palette.open {
        return;
    }
    let ctx = ui.ctx().clone();
    let listed = entries(app, &ctx);
    let PaletteState { open, recents } = &mut app.ui.palette;
    CommandPalette::new(tl!("Command Palette"), &listed, Bind::bound(open)).recents(Bind::bound(recents)).show(ui);
}

/// Records `id` as the most recently run command. The palette records its own picks; this covers commands run from
/// a menu or a shortcut.
pub(crate) fn note_recent(app: &mut PhotocraftApp, id: &str) {
    CommandPalette::remember(&mut app.ui.palette.recents, CommandRef::new(id));
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::entries;
    use crate::PhotocraftApp;

    /// The entries carry the English name of each menu command, so a query in English finds a translated command.
    #[test]
    fn azerbaijani_palette_keeps_english_aliases() {
        if !crate::i18n::needs_catalogs() {
            return;
        }
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = app.ctx().clone();
        let az = crate::i18n::Lang::from_code("az").unwrap();
        let listed = crate::i18n::with_language(az, || entries(&mut app, &ctx));
        let menu = &listed[..listed.len() - crate::state::Tool::ALL.len()];
        assert!(!menu.is_empty() && menu.iter().all(|e| e.english().is_some()), "every menu command has an English name");
        assert!(menu.iter().any(|e| e.english().is_some_and(|english| english.as_str() != e.label().as_str())), "some label is translated");
    }

    /// A command run from a menu (not only from the palette) is first in the recents once; running it again does
    /// not duplicate it.
    #[test]
    fn a_command_run_from_a_menu_is_remembered_once() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = app.ctx().clone();
        let new = || json!({"width": 64, "height": 64});
        crate::menus::invoke(&mut app, &ctx, "file.new", new()).unwrap();
        let first = app.ui.palette.recents.first().cloned().expect("file.new was recorded");
        crate::menus::invoke(&mut app, &ctx, "file.new", new()).unwrap();
        assert_eq!(app.ui.palette.recents.first(), Some(&first));
        assert_eq!(app.ui.palette.recents.len(), 1, "no duplicate");
    }
}
