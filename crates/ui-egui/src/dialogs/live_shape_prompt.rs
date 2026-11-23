//! The question the Delete Anchor Point tool asks before it turns a live shape into a regular path
//! (#12768): "This operation will turn a live shape into a regular path. Continue?", Yes (the
//! default) and No, and a "Don't show again" box kept with the other dialog preferences. Yes runs
//! the `path.deleteAnchor` the click would have run; No changes nothing.

use craft::ui::dialog::{PromptAnswer, PromptChoice, StandardPrompt};
use serde_json::Value;

use super::alerts;
use crate::PhotocraftApp;
use crate::direct_select::{DirectSelection, PathRef};
use photocraft_doc::{LayerContent, LayerId};

/// Its "Don't show again", under `Preferences::dialogs`.
pub const PREF: &str = "path.deleteAnchor.liveShape";

const QUESTION: &str = "This operation will turn a live shape into a regular path. Continue?";

/// What the question asked about: the anchor deletion of `params` on `layer`. The layer id means
/// something only in `document` (another may become active meanwhile), and the knot index only in
/// this `revision` of it (an edit meanwhile can shift the anchors).
pub(crate) struct LiveShapeAsk {
    layer: u64,
    document: u64,
    revision: u64,
    params: Value,
}

/// Whether `r` is a live shape layer, such as a rectangle from the Rectangle tool in Shape mode.
fn is_live(app: &PhotocraftApp, r: PathRef) -> bool {
    let PathRef::Layer(id) = r else { return false };
    app.session.active().and_then(|st| st.doc.layer(LayerId(id))).is_some_and(|layer| matches!(&layer.content, LayerContent::Shape(sh) if sh.live.is_some()))
}

/// Ask before `path.deleteAnchor` with `params` runs on `r`. True when the question is open (or
/// already was), so the caller stops; false when the deletion goes ahead: `r` is not a live shape
/// or "Don't show again" was ticked.
pub fn ask(app: &mut PhotocraftApp, r: PathRef, params: &Value) -> bool {
    let PathRef::Layer(layer) = r else { return false };
    let Some((document, revision)) = app.session.active().map(|st| (st.doc.id.0, st.revision)) else { return false };
    if !is_live(app, r) || alerts::dismissed(app, PREF) {
        return false;
    }
    let prompt = StandardPrompt::custom(tl!(QUESTION).to_owned())
        .title(tl!("Delete Anchor Point").to_owned())
        .button(PromptChoice::Proceed, tl!("Yes").to_owned())
        .button(PromptChoice::Cancel, tl!("No").to_owned())
        .dont_show_again()
        .tag(LiveShapeAsk { layer, document, revision, params: params.clone() });
    // An `Err` is a question already open (this one or another family's): the click waits for it.
    let _ = alerts::open_prompt(app, prompt);
    true
}

/// The `PROMPTS` entry: Yes deletes the anchor in one "Delete Anchor Point" step and keeps a ticked
/// "Don't show again"; No changes nothing. `false` for another family's answer.
pub(crate) fn answer(app: &mut PhotocraftApp, _ctx: &egui::Context, answer: &mut PromptAnswer) -> bool {
    let Some(ask) = answer.take_tag::<LiveShapeAsk>() else { return false };
    if answer.choice != PromptChoice::Proceed {
        return true;
    }
    match confirm(app, ask) {
        Ok(()) if answer.skip => alerts::remember_skip(app, PREF),
        Ok(()) => {}
        Err(error) => alerts::report_error(app, error),
    }
    true
}

fn confirm(app: &mut PhotocraftApp, ask: LiveShapeAsk) -> Result<(), String> {
    if app.session.active().map(|st| (st.doc.id.0, st.revision)) != Some((ask.document, ask.revision)) {
        return Err(tl!("No anchor to delete").into());
    }
    app.run("path.deleteAnchor", ask.params)?;
    // The anchors shift: a Direct Selection on this path would point at the wrong ones.
    let r = PathRef::Layer(ask.layer);
    if app.ui.direct_selection.target.as_ref() == Some(&r) {
        app.ui.direct_selection = DirectSelection { target: Some(r), ..Default::default() };
    }
    Ok(())
}
