//! PhotoCraft's consent prompt: before Generative Fill sends anything to a model off this
//! computer, the person sees where it goes and what is sent (each kind of data and its size), and
//! answers with one of the four choices every Craft app offers: only this time, for an hour, for a
//! day, or until they turn it off. Only the longer answers are remembered, in PhotoCraft's own
//! consent record kept by the shared AI layer. Don't Send discards the fills; nothing was sent.
//!
//! One prompt answers every fill waiting for the same destination (several areas filled at once).

use photocraft_engine::genai::{ConsentTerm, Disclosure};
use photocraft_engine::genai_cmds::{AGREE, DISCARD};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;

pub const CONSENT_MARK: &str = "__genaiConsent";

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

/// What the prompt lists: each kind of data sent, with every waiting fill's size of it (one
/// answer covers them all, so each one's sizes are shown, not just the first's).
fn items(disclosures: &[Disclosure]) -> Value {
    let Some(first) = disclosures.first() else { return json!([]) };
    let items: Vec<Value> = first
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let sizes: Vec<Option<u64>> = disclosures.iter().map(|d| d.items.get(i).filter(|x| x.input == item.input).and_then(|x| x.bytes)).collect();
            json!({"input": item.input, "kind": item.kind, "bytes": item.bytes, "sizes": sizes})
        })
        .collect();
    json!(items)
}

/// Ask about fills waiting for consent: open the prompt, or add newly waiting fills for the same
/// destination to the one that is open.
pub fn ask_if_waiting(app: &mut PhotocraftApp) {
    let Some((ids, disclosures)) = waiting(app) else { return };
    let Some(disclosure) = disclosures.first().cloned() else { return };
    if let Some(d) = app.ui.dialogs.iter_mut().find(|d| d.fields.contains_key(CONSENT_MARK)) {
        if d.fields.get("destination").and_then(Value::as_str) == Some(disclosure.destination.as_str()) {
            d.fields.insert("fills".into(), json!(ids));
            d.fields.insert("items".into(), items(&disclosures));
        }
        return;
    }
    let mut f = Map::new();
    f.insert(CONSENT_MARK.into(), json!(true));
    // Automation confirming it is judged as `layer.generative.agree`, which it may not run: only
    // the person agrees to sending their pixels.
    f.insert("__command".into(), json!(AGREE));
    f.insert("__label".into(), json!("Generative Fill"));
    f.insert("fills".into(), json!(ids));
    f.insert("destination".into(), json!(disclosure.destination));
    f.insert("connection".into(), json!(disclosure.connection));
    f.insert("model".into(), json!(disclosure.model));
    f.insert("items".into(), items(&disclosures));
    f.insert("term".into(), json!(ConsentTerm::ThisTime.id()));
    app.ui.open_dialog(DialogKind::Command, f);
}

/// A size as the prompt shows it.
fn size(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let b = bytes as f64;
    if b >= K * K { format!("{:.1} MB", b / (K * K)) } else { format!("{:.0} KB", (b / K).ceil().max(1.0)) }
}

/// What a disclosed item is, in words, with its size in each fill that waits (in order).
pub(super) fn item_text(input: &str, sizes: &[u64]) -> String {
    let what = match input {
        "source" => tl!("The picture around the selection"),
        "mask" => tl!("The selection's shape"),
        "prompt" => return tl!("Your prompt").to_owned(),
        _ => tl!("Other data"),
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

/// The consent prompt: where it goes, what is sent with each fill's sizes, and how long the yes lasts.
pub fn body(_app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let get = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    ui.set_max_width(420.0);
    ui.label(
        egui::RichText::new(crate::i18n::fmt(tl!("Generative Fill will send this to {destination}:"), &[("destination", &get("destination"))])).color(t.text),
    );
    ui.add(egui::Label::new(egui::RichText::new(format!("{} · {}", get("connection"), get("model"))).color(t.text_faint).size(12.0)).wrap());
    ui.add_space(6.0);
    for item in f.get("items").and_then(Value::as_array).cloned().unwrap_or_default() {
        let input = item.get("input").and_then(Value::as_str).unwrap_or("");
        let sizes: Vec<u64> = match item.get("sizes").and_then(Value::as_array) {
            Some(each) => each.iter().filter_map(Value::as_u64).collect(),
            None => item.get("bytes").and_then(Value::as_u64).into_iter().collect(),
        };
        ui.add(egui::Label::new(format!("•  {}", item_text(input, &sizes))).wrap());
    }
    let n = f.get("fills").and_then(Value::as_array).map_or(0, Vec::len) as u64;
    if n > 1 {
        ui.add_space(4.0);
        let line = crate::i18n::trn(crate::i18n::current(), n, "{n} Generative Fill waits for this answer.", "{n} Generative Fills wait for this answer.");
        ui.add(egui::Label::new(egui::RichText::new(line).color(t.text_faint).size(12.0)).wrap());
    }
    ui.add_space(10.0);
    ui.label(egui::RichText::new(tl!("Allow sending to it:")).font(crate::theme::semibold(12.5)).color(t.text));
    let mut term = get("term");
    for choice in ConsentTerm::ALL {
        if ui.radio_value(&mut term, choice.id().to_owned(), term_label(choice)).changed() {
            f.insert("term".into(), json!(term));
        }
    }
}

/// Send: agree to each waiting fill for the chosen term; they start.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let term = f.get("term").and_then(Value::as_str).unwrap_or(ConsentTerm::ThisTime.id()).to_owned();
    let ids: Vec<u64> = f.get("fills").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    let mut last = Ok(Value::Null);
    let mut first_error = None;
    for id in ids {
        let still = app.session.generative_fills().iter().any(|f| f.id == id && f.state == "consent");
        if still {
            last = app.run(AGREE, json!({"fill": id, "term": term}));
            // A later fill starting doesn't hide that an earlier one couldn't.
            if let Err(e) = &last {
                first_error.get_or_insert_with(|| e.clone());
            }
        }
    }
    // Others may have come to wait meanwhile (another destination): ask about them next.
    ask_if_waiting(app);
    match first_error {
        Some(e) => Err(e),
        None => last,
    }
}

/// Don't Send: the fills it asked about are discarded; nothing of them was sent.
pub fn cancelled(app: &mut PhotocraftApp, f: &Map<String, Value>) {
    let ids: Vec<u64> = f.get("fills").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    let waiting: Vec<u64> = ids.into_iter().filter(|id| app.session.generative_fills().iter().any(|f| f.id == *id && f.state == "consent")).collect();
    if !waiting.is_empty() {
        let _ = app.run(DISCARD, json!({"fills": waiting}));
    }
    // Fills for another destination may have come to wait behind this prompt: ask about them now.
    ask_if_waiting(app);
}
