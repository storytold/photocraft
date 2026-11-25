use craft::menu::MenuBarUi;

use super::PhotocraftApp;
use super::items;
use super::menu_gate;

/// Mnemonic letters of the top-level menus (File F, Edit E, …). A title takes its letter only when
/// no earlier title took it.
pub(super) const TOP_PREFERRED: [(&str, char); 10] = [
    ("File", 'F'),
    ("Edit", 'E'),
    ("Image", 'I'),
    ("Layer", 'L'),
    ("Type", 'Y'),
    ("Select", 'S'),
    ("Filter", 'T'),
    ("View", 'V'),
    ("Window", 'W'),
    ("Help", 'H'),
];

/// Draws the in-window menu bar over the one menu model. Picks are fired commands from
/// `FireSource::Menu`, run by the frame's command drain; the bar's keys wait while a dialog or the
/// shortcut recorder has the keyboard.
pub fn menu_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let (gate, _) = menu_gate(app);
    let ctx = ui.ctx().clone();
    let model = items::model(app, &ctx);
    MenuBarUi::new(&model.bar).mnemonics(&TOP_PREFERRED).help_search(true).keyboard(!(gate.modal || gate.recording_shortcut)).show(ui);
}

/// The commands of the menu model's bar as the command palette lists them, with each path and the English names the model carries. The model's
/// source is laid out with the options the model was built with, so the palette and the bars agree.
pub(crate) fn help_entries(app: &mut PhotocraftApp, ctx: &egui::Context) -> Vec<craft::ui::menu::HelpEntry> {
    use craft::menu::LayoutOptions;
    let lang = crate::i18n::current();
    let model = items::model(app, ctx);
    let label = |s: &'static str| crate::i18n::tr(lang, s).to_string();
    let keys = |chord: &craft::keymap::Chord| craft::ui::runtime::commands::holder(ctx, chord);
    let options = LayoutOptions::new(crate::native_menu::APP_NAME).label(&label).keys(&keys);
    model.source().help_entries(items::target(ctx), &options)
}
