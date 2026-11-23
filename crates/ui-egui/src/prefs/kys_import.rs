//! Photoshop keyboard shortcut sets (`.kys`, Edit › Keyboard Shortcuts › Save Set) imported
//! into Edit › Keyboard Shortcuts (#1163): the dialog's Import button (the dialog's own, in
//! `prefs_ui::shortcuts`), File › Open and a drop on the window. Reading a set and matching its
//! rows to commands are craft-keymap's (`craft::keymap::import::kys::read`, `craft::keymap::plan`);
//! the headless `edit.keyboardShortcuts {"importKys": …}` plans the same way over the same
//! catalog.
//!
//! File › Open opens the dialog and queues the set for review, so OK applies it like hand-made
//! changes (a shortcut moved to a command is taken from its old owner) and Cancel discards it.
//!
//! At first launch beside Photoshop, its live set is offered, never applied unasked: a hosted
//! prompt names the set, and Import applies it ([`offer`], [`take`]).

use craft::keymap::import::kys;
use craft::menu::shortcuts::{ImportOffer, OfferAnswer, OfferRecord};
use craft::ui::dialog::{DialogResult, StartupStage};
use serde_json::Value;

use crate::PhotocraftApp;
use crate::prefs_ui::shortcuts::{draft_keymap, import_options, kys_text, profiles_of, queue_import, save_keys};

/// Preferences › `dialogs` key that records the first-launch offer (an [`OfferRecord`]: the source
/// and whether it was imported), so it is made once.
pub const IMPORTED_PREF: &str = "photoshopShortcuts";

/// A `.kys` opened like a document (File › Open, a drop): its bytes are text. The dialog opens on
/// Keyboard Shortcuts and reviews the set on its next pass.
pub fn open_bytes(app: &mut PhotocraftApp, name: &str, bytes: &[u8]) -> Result<Value, String> {
    let text = kys_text(name, bytes)?;
    queue_import(app, text);
    Ok(crate::prefs_ui::shortcuts::control_reply())
}

fn remember(app: &mut PhotocraftApp, record: &OfferRecord) {
    let record = serde_json::to_value(record).unwrap_or_default();
    app.session.edit_prefs(|p| {
        p.dialogs.insert(IMPORTED_PREF.into(), record);
    });
}

fn not_imported(app: &mut PhotocraftApp, error: &str) {
    app.ui.status = crate::i18n::fmt(tl!("The keyboard shortcut set was not imported: {error}"), &[("error", error)]);
    app.ui.status_error = true;
}

/// Photoshop's live set ([`crate::Services::photoshop_shortcuts`]) as its source path and text.
fn live_set(app: &mut PhotocraftApp) -> Option<(String, String)> {
    app.services.photoshop_shortcuts.as_mut().and_then(|find| find())
}

/// First launch on a machine with Photoshop: its live set is offered once, as a prompt hosted at
/// the startup stage for offers. Someone who already changed a shortcut keeps their own set and is
/// not asked; a set that changes nothing (Photoshop's defaults are ours too) is not offered
/// either. The preferences remember each outcome, so a later launch never asks again.
pub fn offer(app: &mut PhotocraftApp) {
    if app.session.prefs().dialogs.contains_key(IMPORTED_PREF) {
        return;
    }
    let Some((source, text)) = live_set(app) else { return };
    let set = match kys::read(&text) {
        Ok(set) => set,
        Err(e) => {
            remember(app, &OfferRecord::new(&source, false));
            return not_imported(app, &e.to_string());
        }
    };
    let keymap = draft_keymap(app);
    match ImportOffer::prepare(&source, &set, &keymap, &import_options(&Default::default()), None) {
        Ok(offer) => app.framework.dialogs.enqueue(StartupStage::Offer, offer),
        Err(no) => {
            if let Some(record) = no.record(&source) {
                remember(app, &record);
            }
        }
    }
}

/// The offer's answer. The record is stored first, as the offer is answered, whatever follows;
/// Import then applies the set, saves it as OK would and says so. A prompt closed from outside
/// records nothing, so the next launch asks again.
pub fn take(app: &mut PhotocraftApp, _ctx: &egui::Context) {
    let Some(DialogResult::Ok(answer)) = app.framework.dialogs.take::<ImportOffer>() else { return };
    answered(app, answer);
}

/// What an answer of the prompt does ([`take`]).
fn answered(app: &mut PhotocraftApp, answer: OfferAnswer) {
    let Some((source, text)) = live_set(app) else { return };
    remember(app, &OfferRecord::new(&source, answer == OfferAnswer::Import));
    if answer != OfferAnswer::Import {
        return;
    }
    // The plan is made again from the file and the preferences: the hosted prompt keeps no plan after it answers, and
    // nothing has changed the bindings since it was offered.
    let applied = kys::read(&text).map_err(|e| e.to_string()).and_then(|set| {
        let mut keymap = draft_keymap(app);
        let offer = ImportOffer::prepare(&source, &set, &keymap, &import_options(&Default::default()), None).map_err(|_| "nothing to import".to_owned())?;
        offer.plan.apply(&mut keymap);
        let profiles = profiles_of(app.session.prefs());
        save_keys(app, &keymap, &profiles)?;
        Ok((offer.set_name, keymap.overrides().len()))
    });
    let (name, changed) = match applied {
        Ok(done) => done,
        Err(e) => return not_imported(app, &e),
    };
    let id = crate::notices::post(
        app,
        "Keyboard shortcuts imported",
        vec!["Your keyboard shortcut set {set} is in use here: {n} shortcuts differ from the defaults. Edit › Keyboard Shortcuts shows them; Reset All to Defaults undoes this.".to_owned()],
        craft::theme::variant::Tone::Neutral,
        None,
    );
    if let Some(n) = app.ui.notices.iter_mut().find(|n| n.id == id) {
        n.args = vec![("set".to_owned(), name), ("n".to_owned(), changed.to_string())];
    }
}

#[cfg(test)]
mod tests;
