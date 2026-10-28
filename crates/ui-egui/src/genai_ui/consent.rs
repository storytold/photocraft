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

/// The waiting fills and what the first of them would send, if any wait.
fn waiting(app: &PhotocraftApp) -> Option<(Vec<u64>, Disclosure)> {
    let fills = app.session.generative_fills();
    let first = fills.iter().find(|f| f.state == "consent")?;
    let disclosure = app.session.generative_fill_disclosure(first.id)?;
    let ids = fills
        .iter()
        .filter(|f| f.state == "consent")
        .filter(|f| {
            app.session.generative_fill_disclosure(f.id).is_some_and(|d| d.destination == disclosure.destination && d.connection == disclosure.connection)
        })
        .map(|f| f.id)
        .collect();
    Some((ids, disclosure))
}

/// Ask about fills waiting for consent: open the prompt, or add newly waiting fills for the same
/// destination to the one that is open.
pub fn ask_if_waiting(app: &mut PhotocraftApp) {
    let Some((ids, disclosure)) = waiting(app) else { return };
    if let Some(d) = app.ui.dialogs.iter_mut().find(|d| d.fields.contains_key(CONSENT_MARK)) {
        if d.fields.get("destination").and_then(Value::as_str) == Some(disclosure.destination.as_str()) {
            d.fields.insert("fills".into(), json!(ids));
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
    f.insert("items".into(), serde_json::to_value(&disclosure.items).unwrap_or(Value::Null));
    f.insert("term".into(), json!(ConsentTerm::ThisTime.id()));
    app.ui.open_dialog(DialogKind::Command, f);
}

/// A size as the prompt shows it.
fn size(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let b = bytes as f64;
    if b >= K * K { format!("{:.1} MB", b / (K * K)) } else { format!("{:.0} KB", (b / K).ceil().max(1.0)) }
}

/// What a disclosed item is, in words.
fn item_text(input: &str, bytes: Option<u64>) -> String {
    let what = match input {
        "source" => tl!("The picture around the selection"),
        "mask" => tl!("The selection's shape"),
        "prompt" => return tl!("Your prompt").to_owned(),
        _ => tl!("Other data"),
    };
    match bytes {
        Some(b) => format!("{what} · {}", size(b)),
        None => what.to_owned(),
    }
}

fn term_label(t: ConsentTerm) -> &'static str {
    match t {
        ConsentTerm::ThisTime => tl!("Only this time"),
        ConsentTerm::OneHour => tl!("For an hour"),
        ConsentTerm::OneDay => tl!("For a day"),
        ConsentTerm::UntilTurnedOff => tl!("Until I turn it off"),
    }
}

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
        ui.label(format!("•  {}", item_text(input, item.get("bytes").and_then(Value::as_u64))));
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
    for id in ids {
        let still = app.session.generative_fills().iter().any(|f| f.id == id && f.state == "consent");
        if still {
            last = app.run(AGREE, json!({"fill": id, "term": term}));
        }
    }
    // Others may have come to wait meanwhile (another destination): ask about them next.
    ask_if_waiting(app);
    last
}

/// Don't Send: the fills it asked about are discarded; nothing of them was sent.
pub fn cancelled(app: &mut PhotocraftApp, f: &Map<String, Value>) {
    let ids: Vec<u64> = f.get("fills").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    let waiting: Vec<u64> = ids.into_iter().filter(|id| app.session.generative_fills().iter().any(|f| f.id == *id && f.state == "consent")).collect();
    if !waiting.is_empty() {
        let _ = app.run(DISCARD, json!({"fills": waiting}));
    }
}
