//! Layers panel: showing or hiding a layer's effects and Smart Filters under its row (the
//! shared disclosure triangle). View state ([`crate::DocState::fx_collapsed`]): no history step, and a
//! clean document stays clean.

use photocraft_doc::{Layer, LayerContent, LayerId};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const ID: &str = "layer.setEffectsExpanded";

/// A `layer.setEffectsExpanded` parameter error.
fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: msg.into() }
}

/// The command is available while some layer has effects or Smart Filters to disclose.
fn has_effects(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if d.doc.walk().iter().any(|(_, _, l)| has_children(l)) { Ok(()) } else { Err("no layer has effects or smart filters".into()) }
}

/// Whether the layer has a list to disclose: effects, or Smart Filters on a Smart Object.
fn has_children(l: &Layer) -> bool {
    !l.effects.items.is_empty() || matches!(&l.content, LayerContent::Smart(sm) if !sm.smart_filters.is_empty())
}

/// An optional boolean param: `None` when absent or null, an error when it is anything but true or false.
fn opt_bool(p: &Value, key: &str) -> Result<Option<bool>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_bool().map(Some).ok_or_else(|| bad(format!("`{key}` must be true or false"))),
    }
}

/// `{"layer":id?,"expanded":bool?,"all":bool?}`: show or hide a layer's effects list (no
/// `expanded`: toggle). With `all`, every layer with effects takes the new state (⌥-click).
fn set_expanded(s: &mut Session, p: &Value) -> Result<Value> {
    let want = opt_bool(p, "expanded")?;
    let all = opt_bool(p, "all")?.unwrap_or(false);
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let layer = match p.get("layer") {
        None | Some(Value::Null) if all => None,
        None | Some(Value::Null) => Some(st.active_layer.ok_or_else(|| bad("no layer given and no active layer"))?),
        Some(v) => Some(LayerId(v.as_u64().ok_or_else(|| bad("`layer` must be a layer id"))?)),
    };
    let with_fx: Vec<LayerId> = st.doc.walk().iter().filter(|(_, _, l)| has_children(l)).map(|(_, _, l)| l.id).collect();
    let current = match layer {
        Some(id) => {
            let l = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            if !has_children(l) {
                return Err(bad(format!("layer {} has no effects or smart filters", id.0)));
            }
            !st.fx_collapsed.contains(&id)
        }
        None => with_fx.iter().any(|id| !st.fx_collapsed.contains(id)),
    };
    let expanded = want.unwrap_or(!current);
    let targets = if all { with_fx } else { layer.into_iter().collect() };
    // Forget ids no longer in the document so the list can't grow without bound.
    let doc = st.doc.clone();
    st.fx_collapsed.retain(|id| doc.layer(*id).is_some() && !targets.contains(id));
    if !expanded {
        st.fx_collapsed.extend(targets.iter().copied());
    }
    Ok(json!({"expanded": expanded, "layers": targets.len()}))
}

/// Register `layer.setEffectsExpanded`: view state, so it is not journaled and adds no undo step.
pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Expand/Collapse Effects",
        menu: &[],
        shortcut: None,
        params: r##"{"layer":id?,"expanded":bool?,"all":bool?} (no expanded: toggle; all: every layer with effects or smart filters; view state, not an undo step)"##,
        enabled: has_effects,
        run: set_expanded,
        journal: false,
    }]
}
