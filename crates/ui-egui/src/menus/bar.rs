use std::collections::HashMap;
use std::sync::{PoisonError, RwLock};

use craft::menu::{Labeled, MenuBarUi};

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
    publish_english(model);
    MenuBarUi::new(&model.bar).english(english_name).mnemonics(&TOP_PREFERRED).help_search(true).keyboard(!(gate.modal || gate.recording_shortcut)).show(ui);
}

/// The English names of the menu model the bar last drew, by its state: the bar's English hook is a capture-free
/// function, so it reads them here.
struct EnglishNames {
    state: u64,
    labels: HashMap<(String, String), String>,
    titles: HashMap<String, String>,
}

static ENGLISH: RwLock<Option<EnglishNames>> = RwLock::new(None);

/// Keeps [`ENGLISH`] at `model`'s names; copies them only when the model's state changed.
fn publish_english(model: &items::MenuModel) {
    let mut names = ENGLISH.write().unwrap_or_else(PoisonError::into_inner);
    if names.as_ref().map(|n| n.state) != Some(model.state()) {
        let (labels, titles) = model.english_names();
        *names = Some(EnglishNames { state: model.state(), labels, titles });
    }
}

/// The English name of a command or menu title of the bar's model, for its mnemonics and Help search.
fn english_name(labeled: Labeled<'_>) -> Option<String> {
    let names = ENGLISH.read().unwrap_or_else(PoisonError::into_inner);
    let names = names.as_ref()?;
    match labeled {
        Labeled::Command(command) => names.labels.get(&items::command_key(command)).cloned(),
        Labeled::Title(title) => names.titles.get(title).cloned(),
        _ => None,
    }
}

/// The English name of a command or menu title of `model`, for the bar's and the palette's search.
fn english_of(model: &items::MenuModel) -> impl Fn(Labeled<'_>) -> Option<String> + '_ {
    move |labeled| match labeled {
        Labeled::Command(command) => model.english_label(command),
        Labeled::Title(title) => model.english_title(title),
        _ => None,
    }
}

/// The commands of the menu model's bar as the command palette lists them, with each path and English name. The bar
/// is laid out with the options the model was built with, so the palette and the bars agree.
pub(crate) fn help_entries(app: &mut PhotocraftApp, ctx: &egui::Context) -> Vec<craft::ui::menu::HelpEntry> {
    use craft::menu::{LayoutOptions, Target};
    let lang = crate::i18n::current();
    let model = items::model(app, ctx);
    let label = |s: &'static str| crate::i18n::tr(lang, s).to_string();
    let keys = |chord: &craft::keymap::Chord| craft::ui::runtime::commands::holder(ctx, chord);
    let options = LayoutOptions::new(crate::native_menu::APP_NAME).label(&label).keys(&keys);
    model.bar.help_entries(Target::current(), &options, Some(&english_of(model)))
}
