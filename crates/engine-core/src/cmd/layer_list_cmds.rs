//! `layer.list`: the layer tree alone, for agents and scripts (#5059). `document.inspect` also has
//! it, buried among history, channels and much more; an agent looking for the layers' ids looks
//! for a `layer.*` command first.

use photocraft_doc::{Layer, LayerContent, LayerId};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{DocState, EngineError, Result, Session};

const ID: &str = "layer.list";

/// Groups nest at most this deep in the listing; deeper layers are left out (documents can be
/// deep, and the tree is built recursively).
const MAX_DEPTH: usize = 256;

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// One layer's row: what an agent needs to pick a layer to work on.
fn row(l: &Layer, selected: &[LayerId], active: Option<LayerId>) -> Value {
    let bounds = crate::layer_multi_cmds::layer_bounds(l).map(|r| [r.x0, r.y0, r.width() as i32, r.height() as i32]);
    json!({
        "id": l.id.0,
        "name": l.name,
        "kind": l.content.kind_name(),
        "visible": l.visible,
        "bounds": bounds,
        "selected": selected.contains(&l.id),
        "active": active == Some(l.id),
    })
}

/// The layers top-down (the Layers panel's order), groups with their `children`.
fn tree(layers: &[Layer], d: &DocState, selected: &[LayerId], depth: usize) -> Vec<Value> {
    if depth >= MAX_DEPTH {
        return Vec::new();
    }
    layers
        .iter()
        .rev()
        .map(|l| {
            let mut v = row(l, selected, d.active_layer);
            if let LayerContent::Group(g) = &l.content {
                v["children"] = Value::Array(tree(&g.children, d, selected, depth + 1));
            }
            v
        })
        .collect()
}

/// The layers top-down as one list, each with its `depth` (0 = top level) and `parent` group id.
fn flat(layers: &[Layer], d: &DocState, selected: &[LayerId], parent: Option<LayerId>, depth: usize, out: &mut Vec<Value>) {
    if depth >= MAX_DEPTH {
        return;
    }
    for l in layers.iter().rev() {
        let mut v = row(l, selected, d.active_layer);
        v["depth"] = json!(depth);
        v["parent"] = json!(parent.map(|p| p.0));
        out.push(v);
        if let LayerContent::Group(g) = &l.content {
            flat(&g.children, d, selected, Some(l.id), depth + 1, out);
        }
    }
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let bad = |msg: &str| EngineError::BadParams { cmd: ID.into(), msg: msg.into() };
    let d = match p.get("document") {
        None | Some(Value::Null) => s.active().ok_or(EngineError::NoDocument)?,
        Some(v) => {
            let i = v.as_u64().and_then(|v| usize::try_from(v).ok()).ok_or_else(|| bad("`document` must be an index"))?;
            s.documents().get(i).ok_or_else(|| EngineError::Other(format!("no document at index {i}")))?
        }
    };
    let as_flat = match p.get("flat") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool().ok_or_else(|| bad("`flat` must be true or false"))?,
    };
    let selected = d.selected_layers();
    let layers = if as_flat {
        let mut out = Vec::new();
        flat(&d.doc.layers, d, selected, None, 0, &mut out);
        out
    } else {
        tree(&d.doc.layers, d, selected, 0)
    };
    Ok(json!({"layers": layers, "activeLayer": d.active_layer.map(|l| l.0)}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: ID,
        label: "List Layers",
        menu: &[],
        shortcut: None,
        params: r##"{"document":index?,"flat":bool=false} → {"layers":[{"id","name","kind","visible","bounds":[x,y,w,h]|null,"selected","active","children"?}],"activeLayer":id?} (top-down; groups list their "children", or with "flat" every layer is one row with its "depth" and "parent" group id)"##,
        enabled: has_doc,
        run,
        journal: false,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_with_doc() -> Session {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        s
    }

    #[test]
    fn is_a_query_and_leaves_the_document_alone() {
        let mut s = session_with_doc();
        let revision = s.active().unwrap().revision;
        let journal = s.journal.len();
        s.execute(ID, json!({})).unwrap();
        assert_eq!((s.active().unwrap().revision, s.journal.len()), (revision, journal));
    }

    #[test]
    fn bad_params_are_errors_not_panics() {
        let mut s = crate::session();
        assert!(s.execute(ID, json!({})).is_err(), "no document");
        let mut s = session_with_doc();
        for p in [json!({"flat": "yes"}), json!({"document": -1}), json!({"document": 99}), json!({"document": u64::MAX}), json!({"document": "a"})] {
            assert!(s.execute(ID, p.clone()).is_err(), "{p}");
        }
        assert!(s.execute(ID, json!({"document": 0, "flat": false})).is_ok());
    }
}
