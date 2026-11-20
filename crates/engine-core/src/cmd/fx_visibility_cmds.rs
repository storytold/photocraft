//! Layers panel: the eyes on a layer's effects rows (#1622). The eye on the "Effects" row shows or
//! hides every effect of that layer; the eye on one effect's row shows or hides that effect. The
//! effects keep their settings either way, and each toggle is one history step.

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const ID: &str = "layer.setEffectsVisible";

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: ID.into(), msg: msg.into() }
}

fn has_effects(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if d.doc.walk().iter().any(|(_, _, l)| !l.effects.items.is_empty()) { Ok(()) } else { Err("no layer has effects".into()) }
}

/// `{"layer":id?,"index":n?,"visible":bool?}`: show or hide a layer's effects (no `visible`:
/// toggle). Without `index` it is the whole list (the "Effects" row); with it, the effect at that
/// position in the layer's list.
fn set_visible(s: &mut Session, p: &Value) -> Result<Value> {
    let want = match p.get("visible") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_bool().ok_or_else(|| bad("`visible` must be true or false"))?),
    };
    let index = match p.get("index") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad("`index` must be an effect position (0, 1, …)"))?),
    };
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = crate::commands::explicit_layer(s, p)?.or(st.active_layer).ok_or_else(|| bad("no layer given and no active layer"))?;
    let l = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    if l.effects.items.is_empty() {
        return Err(bad(format!("layer {} has no effects", id.0)));
    }
    let (current, name) = match index {
        Some(i) => {
            let e = l.effects.items.get(i).ok_or_else(|| bad(format!("layer {} has no effect at index {i}", id.0)))?;
            (e.enabled(), e.label())
        }
        None => (l.effects.enabled, "Layer Effects"),
    };
    let visible = want.unwrap_or(!current);
    if visible != current {
        let label = format!("{} {name}", if visible { "Show" } else { "Hide" });
        s.edit(&label, |doc, _| {
            let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
            match index {
                Some(i) => l.effects.items.get_mut(i).ok_or_else(|| bad(format!("layer {} has no effect at index {i}", id.0)))?.set_enabled(visible),
                None => l.effects.enabled = visible,
            }
            Ok(())
        })?;
    }
    Ok(json!({"layer": id.0, "index": index, "visible": visible}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "Show/Hide Effects",
        menu: &[],
        shortcut: None,
        params: r##"{"layer":id?,"index":n?,"visible":bool?} (no index: all of the layer's effects, the "Effects" row; index: the effect at that position in the layer's list; no visible: toggle)"##,
        enabled: has_effects,
        run: set_visible,
        journal: true,
    }]
}
