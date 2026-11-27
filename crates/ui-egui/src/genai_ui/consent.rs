//! PhotoCraft's consent prompt: before Generative Fill sends anything to a model off this
//! computer, the person sees where it goes and what is sent (each kind of data and its size), and
//! answers with one of the four choices every Craft app offers: only this time, for an hour, for a
//! day, or until they turn it off. Only the longer answers are remembered, in PhotoCraft's own
//! consent record kept by the shared AI layer. Don't Send discards the fills; nothing was sent.
//!
//! One prompt answers every fill waiting for the same destination (several areas filled at once):
//! fills that come to wait while it is open join it ([`feed`]).

use std::sync::{Arc, Mutex};

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::Label;
use photocraft_engine::genai::{ConsentTerm, Disclosure, SentInput};
use photocraft_engine::genai_cmds::{AGREE, DISCARD};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::dialogs::params;

/// The consent prompt's dialog key: confirming it is judged as `layer.generative.agree`, which
/// automation may not run; only the person agrees to sending their pixels.
pub const KEY: &str = AGREE;

/// What the open prompt asks about, shared with its rows' drawing.
#[derive(Clone, Default, PartialEq)]
pub struct Asked {
    /// The fills it answers.
    fills: Vec<u64>,
    destination: String,
    connection: String,
    model: String,
    /// Each kind of data sent, with every waiting fill's size of it.
    items: Vec<(SentInput, Vec<u64>)>,
}

/// The open prompt's [`Asked`], read by its rows each pass, and the prompt's dialog id.
#[derive(Default)]
pub struct Shared {
    asked: Arc<Mutex<Asked>>,
    dialog: Option<DialogId>,
}

/// The fills waiting for the same destination as the first one that waits, and what each of
/// them would send, if any wait.
fn waiting(app: &PhotocraftApp) -> Option<(Vec<u64>, Vec<Disclosure>)> {
    let fills = app.session.generative_fills();
    let first = fills.iter().find(|f| f.state == "consent")?;
    let disclosure = app.session.generative_fill_disclosure(first.id)?;
    let (ids, disclosures) = fills
        .iter()
        .filter(|f| f.state == "consent")
        .filter_map(|f| app.session.generative_fill_disclosure(f.id).map(|d| (f.id, d)))
        .filter(|(_, d)| d.destination == disclosure.destination && d.connection == disclosure.connection)
        .unzip();
    Some((ids, disclosures))
}

/// What the prompt lists for `ids` waiting with `disclosures`.
fn asked(ids: Vec<u64>, disclosures: &[Disclosure]) -> Option<Asked> {
    let first = disclosures.first()?;
    let items = first
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let sizes = disclosures.iter().filter_map(|d| d.items.get(i).filter(|x| x.input == item.input).and_then(|x| x.bytes)).collect();
            (item.input, sizes)
        })
        .collect();
    Some(Asked { fills: ids, destination: first.destination.clone(), connection: first.connection.clone(), model: first.model.clone(), items })
}

/// Is dialog `id` the consent prompt (open, or just closed)?
pub(super) fn is(app: &PhotocraftApp, id: DialogId) -> bool {
    app.genai.consent.dialog == Some(id)
}

/// The fills the open prompt asks about. For tests.
#[cfg(test)]
pub(super) fn asked_fills(app: &PhotocraftApp) -> Vec<u64> {
    app.genai.consent.asked.lock().map(|a| a.fills.clone()).unwrap_or_default()
}

/// Ask about fills waiting for consent: open the prompt, or add newly waiting fills for the same
/// destination to the one that is open.
pub fn ask_if_waiting(app: &mut PhotocraftApp) {
    let Some((ids, disclosures)) = waiting(app) else { return };
    let Some(next) = asked(ids, &disclosures) else { return };
    if super::open_dialog(app, KEY).is_some() {
        if let Ok(mut open) = app.genai.consent.asked.lock()
            && open.destination == next.destination
            && *open != next
        {
            *open = next;
        }
        return;
    }
    if let Ok(mut open) = app.genai.consent.asked.lock() {
        *open = next;
    }
    let shared = Arc::clone(&app.genai.consent.asked);
    let terms: Vec<(String, Label)> = ConsentTerm::ALL.iter().map(|t| (t.id().to_owned(), Label::from(term_label(*t)))).collect();
    let id = params::open_param_unsaved(app, KEY, DialogFields::new(), Value::Null, move |fields: &mut DialogFields| {
        let rows = vec![Param::new("term", tl!("Allow sending to it:"), ParamKind::radio(terms.clone(), Some(ConsentTerm::ThisTime.id().to_owned())))];
        let shared = Arc::clone(&shared);
        ParamDialog::command(KEY, tl!("Generative Fill"), rows)
            .values(Bind::bound(fields))
            .ok_label(Label::from(tl!("Send")))
            .cancel_label(Label::from(tl!("Don't Send")))
            .below(move |ui, _| {
                let Ok(a) = shared.lock() else { return };
                body(ui, &a);
            })
    });
    app.genai.consent.dialog = Some(id);
}

/// Each frame: fills that came to wait for the open prompt's destination join it, and a prompt is
/// opened for any left waiting once it closed.
pub(super) fn feed(app: &mut PhotocraftApp) {
    if waiting(app).is_some() {
        ask_if_waiting(app);
    }
}

/// A size as the prompt shows it.
fn size(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let b = bytes as f64;
    if b >= K * K { format!("{:.1} MB", b / (K * K)) } else { format!("{:.0} KB", (b / K).ceil().max(1.0)) }
}

/// What a disclosed item is, in words, with its size in each fill that waits (in order).
pub(super) fn item_text(input: SentInput, sizes: &[u64]) -> String {
    let what = match input {
        SentInput::Source => tl!("The picture around the selection"),
        SentInput::Mask => tl!("The selection's shape"),
        SentInput::Prompt => return tl!("Your prompt").to_owned(),
        SentInput::Other => tl!("Other data"),
    };
    if sizes.is_empty() {
        return what.to_owned();
    }
    format!("{what} · {}", sizes.iter().map(|b| size(*b)).collect::<Vec<_>>().join(", "))
}

/// A consent term as the prompt offers it.
fn term_label(t: ConsentTerm) -> &'static str {
    match t {
        ConsentTerm::ThisTime => tl!("Only this time"),
        ConsentTerm::OneHour => tl!("For an hour"),
        ConsentTerm::OneDay => tl!("For a day"),
        ConsentTerm::UntilTurnedOff => tl!("Until I turn it off"),
    }
}

/// Where it goes and what is sent, with each fill's sizes.
fn body(ui: &mut egui::Ui, a: &Asked) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Generative Fill will send this to {destination}:"), &[("destination", &a.destination)])).color(t.text));
    super::faint(ui, &format!("{} · {}", a.connection, a.model));
    ui.add_space(6.0);
    for (input, sizes) in &a.items {
        ui.add(egui::Label::new(format!("•  {}", item_text(*input, sizes))).wrap());
    }
    let n = a.fills.len() as u64;
    if n > 1 {
        ui.add_space(4.0);
        super::faint(
            ui,
            &crate::i18n::trn(crate::i18n::current(), n, "{n} Generative Fill waits for this answer.", "{n} Generative Fills wait for this answer."),
        );
    }
}

/// The fills the closed prompt answered, taken from it.
fn take_fills(app: &mut PhotocraftApp) -> Vec<u64> {
    app.genai.consent.dialog = None;
    app.genai.consent.asked.lock().map(|mut a| std::mem::take(&mut *a).fills).unwrap_or_default()
}

/// Send: agree to each waiting fill for the chosen term (`term`, the radio row's; `None` is only
/// this time); they start.
pub(super) fn confirm(app: &mut PhotocraftApp, term: Option<&str>) -> Result<Value, String> {
    let term = term.unwrap_or(ConsentTerm::ThisTime.id()).to_owned();
    let mut last = Ok(Value::Null);
    let mut first_error = None;
    for id in take_fills(app) {
        let still = app.session.generative_fills().iter().any(|f| f.id == id && f.state == "consent");
        if still {
            last = app.run(AGREE, json!({"fill": id, "term": term}));
            // A later fill starting doesn't hide that an earlier one couldn't.
            if let Err(e) = &last {
                first_error.get_or_insert_with(|| e.clone());
            }
        }
    }
    match first_error {
        Some(e) => Err(e),
        None => last,
    }
}

/// Don't Send: the fills it asked about are discarded; nothing of them was sent.
pub(super) fn cancelled(app: &mut PhotocraftApp) {
    let ids = take_fills(app);
    let waiting: Vec<u64> = ids.into_iter().filter(|id| app.session.generative_fills().iter().any(|f| f.id == *id && f.state == "consent")).collect();
    for id in waiting {
        let _ = app.run(DISCARD, json!({"fill": id}));
    }
}
