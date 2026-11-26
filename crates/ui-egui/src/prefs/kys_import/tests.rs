//! Photoshop `.kys` sets import into Edit › Keyboard Shortcuts (#1163): rows match our commands
//! by label and path through craft-keymap's plan over the catalog the dialog edits, with the
//! command whose default key the row carries winning a shared label.

use craft::keymap::import::kys;
use craft::keymap::{ImportPlan, Scope, plan};
use craft::menu::shortcuts::{ImportOffer, OfferAnswer};
use craft::ui::dialog::DialogHost;
use photocraft_engine::Session;
use photocraft_engine::prefs::{Preferences, normalize_shortcut};
use serde_json::json;

use super::{draft_keymap, import_options};
use crate::PhotocraftApp;

/// A cut-down set in Photoshop's own layout: BOM, a dynamic (filter) row, static rows with
/// their ids, a row with two shortcuts, an entity, scripts, and the tool keys.
const SAMPLE: &str = "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<photoshop-keyboard-shortcuts version=\"4\" filename=\"$$$/FileName/Presets/KeyboardCustomization/texcuts=texcuts\" modified=\"0\" multi-undo=\"2\">
\t<command kind=\"dynamic\" name=\"Script_6F17BFA7-EFC8-40EA-B850-7B95ED8EA713\">
\t\t<shortcut>Control+Opt+Cmd+4</shortcut>
\t</command>
\t<command kind=\"dynamic\" name=\"Gaussian Blur...\">
\t\t<shortcut>Control+Opt+Cmd+G</shortcut>
\t</command>
\t<command kind=\"static\" name=\"Undo\" id=\"101\">
\t\t<shortcut>Cmd+Z</shortcut>
\t\t<shortcut>F1</shortcut>
\t</command>
\t<command kind=\"static\" name=\"Zoom In\" id=\"1004\">
\t\t<shortcut>Cmd++</shortcut>
\t\t<shortcut>Cmd+=</shortcut>
\t</command>
\t<command kind=\"static\" name=\"Levels...\" id=\"1801\">
\t\t<shortcut>Cmd+L</shortcut>
\t</command>
\t<command kind=\"static\" name=\"All\" id=\"1017\">
\t\t<shortcut>Cmd+A</shortcut>
\t</command>
\t<command kind=\"static\" name=\"Layer Via Copy\" id=\"2970\">
\t\t<shortcut>Cmd+J</shortcut>
\t</command>
\t<command kind=\"static\" name=\"Black &amp; White...\" id=\"1824\">
\t\t<shortcut>Opt+Shift+Cmd+B</shortcut>
\t</command>
\t<command kind=\"static\" name=\"General...\" id=\"2311\">
\t\t<shortcut>Cmd+K</shortcut>
\t</command>
\t<command kind=\"static\" name=\"Last Filter\" id=\"1019\">
\t\t<shortcut>Cmd+F</shortcut>
\t</command>
\t<command kind=\"static\" name=\"Hide Photoshop\" id=\"7001\">
\t\t<shortcut>Cmd+H</shortcut>
\t</command>
\t<tool name=\"Move Tool\" type=\"1\" key=\"1819113074\">V</tool>
\t<tool name=\"Default Foreground/Background Colors\" type=\"2\"></tool>
</photoshop-keyboard-shortcuts>
";

const PSP: &str = "/Users/me/Library/Preferences/Adobe Photoshop 2026 Settings/Keyboard Shortcuts.psp";

fn app() -> Box<PhotocraftApp> {
    PhotocraftApp::new(Session::new(), crate::Services::default())
}

/// One pass of the Keyboard Shortcuts dialog.
fn pass(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    crate::testing::run_ui(&ctx, Default::default(), |ui| crate::prefs_ui::show_shortcuts(app, ui));
}

/// One pass of the dialog host: a queued startup prompt opens in it.
fn host_pass(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    crate::testing::run_ui(&ctx, Default::default(), |ui| app.framework.dialogs.run(ui));
}

fn set_of(rows: &str) -> String {
    format!("<photoshop-keyboard-shortcuts version=\"4\">{rows}</photoshop-keyboard-shortcuts>")
}

/// The plan of `text` over the catalog the dialog edits.
fn plan_of(app: &PhotocraftApp, text: &str) -> ImportPlan {
    plan(&kys::read(text).unwrap(), &draft_keymap(app), &import_options(&Default::default()))
}

/// The keys the plan gives command `id`, as the preferences spell them.
fn keys(plan: &ImportPlan, id: &str) -> Vec<String> {
    let of =
        |p: &craft::keymap::import::Planned| -> Vec<String> { p.bindings.iter().filter_map(|b| normalize_shortcut(&b.binding.text(&Scope::app()))).collect() };
    plan.commands.iter().find(|p| p.command.id == id).map(of).unwrap_or_default()
}

/// The set the dialog is asked to review: queued once, read by the dialog's next pass alone.
fn review_pending(app: &PhotocraftApp) -> bool {
    app.prefs_rt.shortcuts.as_ref().is_some_and(|s| s.pending_import.is_some())
}

/// Opening or dropping a `.kys` opens the dialog with the set queued for review; a second file
/// goes to the same dialog.
#[test]
fn an_opened_kys_file_opens_the_dialog_filled() {
    let mut app = app();
    assert!(!crate::prefs_ui::shortcuts_open(&app));
    let reply = super::open_bytes(&mut app, "texcuts.kys", SAMPLE.as_bytes()).unwrap();
    assert_eq!(reply, json!({"dialog": "shortcuts"}));
    assert!(crate::prefs_ui::shortcuts_open(&app));
    assert!(review_pending(&app));
    // A second pass of the same frame finds nothing left to review.
    pass(&mut app);
    assert!(!review_pending(&app));
    pass(&mut app);
    let one = "<photoshop-keyboard-shortcuts><command kind=\"static\" name=\"Copy\"><shortcut>F3</shortcut></command></photoshop-keyboard-shortcuts>";
    super::open_bytes(&mut app, "more.kys", one.as_bytes()).unwrap();
    assert!(review_pending(&app), "the second file is queued in the same dialog");
    assert!(crate::prefs_ui::shortcuts_open(&app));
    // Bytes that are not text are refused at once.
    let bad = super::open_bytes(&mut app, "notes.kys", &[0xff, 0xfe, 0x00]).unwrap_err();
    assert!(bad.contains("not UTF-8"), "{bad}");
}

/// An app beside a Photoshop whose live set is `set`, with the preferences `prefs` made first, and
/// the offer asked as it is at startup.
fn with_photoshop(set: &'static str, prefs: impl FnOnce(&mut Preferences)) -> Box<PhotocraftApp> {
    let mut app = app();
    app.session.edit_prefs(prefs);
    app.services.photoshop_shortcuts = Some(Box::new(move || Some((PSP.to_owned(), set.to_owned()))));
    super::offer(&mut app);
    app
}

/// First launch beside Photoshop: its live set is offered as a hosted prompt, not applied, and
/// nothing is remembered until it is answered.
#[test]
fn first_launch_offers_photoshops_live_set_and_import_applies_it() {
    let mut app = with_photoshop(SAMPLE, |_| {});
    assert!(app.session.prefs().shortcuts.is_empty(), "nothing applied unasked");
    assert!(!app.session.prefs().dialogs.contains_key(super::IMPORTED_PREF), "not answered yet");
    host_pass(&mut app);
    let offer = app.framework.dialogs.get::<ImportOffer>().expect("the hosted prompt");
    assert_eq!(offer.set_name, "texcuts");
    assert!(offer.changes > 0);
    // Import: the record is stored first, then the set is applied and a notice says so.
    super::answered(&mut app, OfferAnswer::Import);
    assert_eq!(crate::shortcuts::effective_shortcut(&app, "filter.blur.gaussianBlur", None).as_deref(), Some("Cmd+Ctrl+Alt+G"));
    assert_eq!(crate::shortcuts::effective_shortcut(&app, "filter.lastFilter", Some("Cmd+Alt+F")).as_deref(), Some("Cmd+F"));
    assert_eq!(crate::shortcuts::effective_shortcut(&app, "edit.search", Some("Cmd+F")), None, "taken by Last Filter");
    let answer = &app.session.prefs().dialogs[super::IMPORTED_PREF];
    assert_eq!(answer["imported"], json!(true));
    assert_eq!(answer["source"].as_str().map(|s| s.ends_with("Keyboard Shortcuts.psp")), Some(true));
    let notice = app.ui.notices.last().unwrap();
    assert_eq!(notice.title, "Keyboard shortcuts imported");
    let line = notice.text(&notice.lines[0]);
    assert!(line.starts_with("Your keyboard shortcut set texcuts is in use here: "), "{line}");
    assert!(!app.ui.status_error, "{}", app.ui.status);
    // Reset, then a second launch: the preference says it was answered, so nothing is queued.
    app.run("edit.keyboardShortcuts", json!({"reset": true})).unwrap();
    app.framework.dialogs = DialogHost::new();
    super::offer(&mut app);
    assert!(app.framework.dialogs.is_empty());
}

/// Don't Import changes nothing and is remembered too.
#[test]
fn dont_import_keeps_the_defaults_and_is_not_asked_again() {
    let mut app = with_photoshop(SAMPLE, |_| {});
    super::answered(&mut app, OfferAnswer::DontImport);
    assert!(app.session.prefs().shortcuts.is_empty());
    assert!(app.ui.notices.is_empty());
    assert_eq!(app.session.prefs().dialogs[super::IMPORTED_PREF]["imported"], json!(false));
    app.framework.dialogs = DialogHost::new();
    super::offer(&mut app);
    assert!(app.framework.dialogs.is_empty());
}

/// The prompt the host shows names the set and the number of shortcuts that differ; its answer
/// applies the set.
#[test]
fn the_prompt_shows_the_set_and_its_import_button_applies_it() {
    let mut app = with_photoshop(SAMPLE, |_| {});
    host_pass(&mut app);
    assert!(app.framework.dialogs.is_open::<ImportOffer>());
    let (name, changes) = app.framework.dialogs.get::<ImportOffer>().map(|o| (o.set_name.clone(), o.changes)).unwrap();
    assert_eq!(name, "texcuts");
    assert!(changes > 0, "{changes}");
    super::answered(&mut app, OfferAnswer::Import);
    assert_eq!(crate::shortcuts::effective_shortcut(&app, "filter.blur.gaussianBlur", None).as_deref(), Some("Cmd+Ctrl+Alt+G"));
}

/// Shortcuts the user already changed are theirs: nothing is offered, and the launch is
/// remembered, so the set is never offered over them later either.
#[test]
fn a_user_with_their_own_shortcuts_is_not_asked() {
    let own = with_photoshop(SAMPLE, |p| {
        p.shortcuts.insert("edit.undo".into(), "F1".into());
    });
    assert_eq!(crate::shortcuts::effective_shortcut(&own, "edit.undo", Some("Cmd+Z")).as_deref(), Some("F1"));
    assert!(own.framework.dialogs.is_empty());
    assert!(own.session.prefs().dialogs.contains_key(super::IMPORTED_PREF));
    // A set that is just Photoshop's defaults changes nothing, so it is not offered.
    let defaults = "<photoshop-keyboard-shortcuts><command kind=\"static\" name=\"Undo\"><shortcut>Cmd+Z</shortcut></command></photoshop-keyboard-shortcuts>";
    let quiet = with_photoshop(defaults, |_| {});
    assert!(quiet.framework.dialogs.is_empty());
    assert!(quiet.session.prefs().dialogs.contains_key(super::IMPORTED_PREF));
    // A file that is not a set is reported, not offered, and not retried.
    let bad = with_photoshop("binary junk", |_| {});
    assert!(bad.framework.dialogs.is_empty());
    assert!(bad.ui.status_error, "{}", bad.ui.status);
    assert!(bad.ui.status.starts_with("The keyboard shortcut set was not imported: "), "{}", bad.ui.status);
    assert!(bad.ui.status.contains("not a Photoshop keyboard shortcut set"), "{}", bad.ui.status);
    assert!(bad.session.prefs().dialogs.contains_key(super::IMPORTED_PREF));
}

/// Photoshop on Windows writes Ctrl for the primary modifier and its tool rows give letters:
/// the keys import as Photoshop meant them, tool rows through the catalog's tools.
#[test]
fn a_windows_set_with_tool_keys_imports_as_photoshop_meant_it() {
    let set = set_of(
        "<command kind=\"static\" name=\"Copy\" id=\"103\"><shortcut>Ctrl+C</shortcut><shortcut>F3</shortcut></command>\
         <command kind=\"static\" name=\"Levels...\" id=\"1801\"><shortcut>Ctrl+Shift+Alt+L</shortcut></command>\
         <tool name=\"Move Tool\" type=\"1\">V</tool>\
         <tool name=\"Pencil Tool\" type=\"1\">N</tool>\
         <tool name=\"Imaginary Tool\" type=\"1\">Q</tool>",
    );
    let mut app = app();
    let ctx = app.ctx().clone();
    crate::prefs_ui::open_shortcuts(&mut app, 0);
    let p = plan_of(&app, &set);
    assert_eq!(keys(&p, "tools.select.pencil"), ["N"]);
    assert_eq!(keys(&p, "tools.select.move"), ["V"]);
    assert_eq!(p.report.unmatched.iter().map(|u| u.item.as_str()).collect::<Vec<_>>(), ["Imaginary Tool"]);
    let levels = p.commands.iter().find(|c| c.command.id.contains("levels")).expect("a Levels row");
    let text = levels.bindings.first().and_then(|b| normalize_shortcut(&b.binding.text(&Scope::app())));
    assert_eq!(text, normalize_shortcut("Cmd+Alt+Shift+L"));
    let keymap = &mut app.prefs_rt.shortcuts.as_mut().unwrap().keymap;
    p.apply(keymap);
    assert!(keymap.overrides().get("edit.copy").is_none(), "Ctrl+C on Windows is the default Cmd+C");
    assert!(keymap.overrides().get("tools.select.move").is_none(), "Move's own V keeps it cycling");
    crate::prefs_ui::shortcuts::press_ok(&mut app, &ctx);
    assert_eq!(crate::tool_shortcuts::shortcut(&app, crate::state::Tool::Pencil).as_deref(), Some("N"));
}
