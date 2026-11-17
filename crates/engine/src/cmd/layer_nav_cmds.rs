//! Keyboard layer navigation, the shortcuts that have no menu item (#352): ⌥] / ⌥[ select the
//! layer above / below, ⌥. / ⌥, the top / bottom layer, and ⇧⌥] / ⇧⌥[ add the layer above /
//! below to the selection. They follow the Layers panel's rows, top to bottom: a group's row comes
//! before its children. Navigation skips invisible layers and collapsed children, and cycles
//! at the ends, as recorded Photoshop ordinal selections do.

use photocraft_doc::{Document, Layer, LayerContent, LayerId};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::layer_multi_cmds::set_selection;
use crate::{DocState, EngineError, Result, Session};

fn has_layers(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.doc.layers.is_empty() { Err("the document has no layers".into()) } else { Ok(()) }
}

/// The Layers panel's rows, top to bottom: each group before its children, collapsed groups
/// without them.
pub fn panel_rows(doc: &Document) -> Vec<LayerId> {
    panel_entries(doc).into_iter().map(|(id, _)| id).collect()
}

/// Keep hidden rows as navigation origins, but not as destinations. Descendants inherit their
/// parent's visibility; a collapsed group contributes only its header, not its stored children.
fn panel_entries(doc: &Document) -> Vec<(LayerId, bool)> {
    let mut out = Vec::new();
    let mut pending: Vec<(&Layer, bool)> = doc.layers.iter().map(|layer| (layer, true)).collect();
    while let Some((layer, parent_visible)) = pending.pop() {
        let visible = parent_visible && layer.visible;
        out.push((layer.id, visible));
        if let LayerContent::Group(group) = &layer.content
            && group.expanded
        {
            pending.extend(group.children.iter().map(|child| (child, visible)));
        }
    }
    out
}

/// Where `id` is in `rows`; a layer inside a collapsed group counts as that group's row (the
/// nearest enclosing one that has a row).
fn row_of(doc: &Document, rows: &[(LayerId, bool)], id: LayerId) -> Option<usize> {
    let path = doc.path_of(id)?;
    (1..=path.len()).rev().find_map(|n| path.get(..n).and_then(|path| doc.layer_at(path)).and_then(|l| rows.iter().position(|(row, _)| *row == l.id)))
}

/// Photoshop layer navigation ordinals, shared by keyboard commands and recorded references.
/// Explicit names and indexes use their own lookup and may still select hidden layers.
pub(crate) fn ordinal_target(st: &DocState, ordinal: &str) -> Result<LayerId> {
    if ordinal == "Trgt" {
        return st.active_layer.filter(|id| st.doc.layer(*id).is_some()).ok_or_else(|| EngineError::Other("no active layer".into()));
    }
    let rows = panel_entries(&st.doc);
    let current = st.active_layer.and_then(|id| row_of(&st.doc, &rows, id));
    let target = match ordinal {
        "Frnt" => rows.iter().find(|row| row.1),
        "Back" => rows.iter().rev().find(|row| row.1),
        "Frwr" => {
            let current = current.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            rows.iter().take(current).rev().find(|row| row.1).or_else(|| rows.iter().rev().find(|row| row.1))
        }
        "Bckw" => {
            let current = current.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            rows.iter().skip(current.saturating_add(1)).find(|row| row.1).or_else(|| rows.iter().find(|row| row.1))
        }
        _ => return Err(EngineError::Other(format!("unsupported layer ordinal {ordinal}"))),
    };
    target.map(|row| row.0).ok_or_else(|| EngineError::Other("no visible layer to select".into()))
}

#[derive(Clone, Copy)]
enum Step {
    Above,
    Below,
    Top,
    Bottom,
}

/// Selects (or with `add`, adds to the selection) the row `step` away from the active layer.
/// At either end, relative navigation cycles to the other end.
fn navigate(s: &mut Session, step: Step, add: bool) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let ordinal = match (step, st.active_layer) {
        (Step::Top, _) | (Step::Above, None) => "Frnt",
        (Step::Bottom, _) | (Step::Below, None) => "Back",
        (Step::Above, _) => "Frwr",
        (Step::Below, _) => "Bckw",
    };
    let id = ordinal_target(st, ordinal)?;
    if add {
        let mut sel = st.selected_layers();
        if !sel.contains(&id) {
            sel.push(id);
        }
        let anchor = st.layer_anchor;
        set_selection(s, sel, Some(id), anchor)?;
    } else {
        s.select_layer(id)?;
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    Ok(json!({"layer": id.0, "selected": st.selected_layers().iter().map(|l| l.0).collect::<Vec<_>>()}))
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $sc:expr, $step:expr, $add:expr) => {
            CommandSpec {
                id: $id,
                label: $label,
                menu: &[],
                shortcut: Some($sc),
                params: "{} (follows visible Layers panel rows; collapsed groups are one row; relative navigation cycles at the ends)",
                enabled: has_layers,
                journal: true,
                run: |s, _| navigate(s, $step, $add),
            }
        };
    }
    vec![
        spec!("layer.selectAbove", "Select Layer Above", "Alt+]", Step::Above, false),
        spec!("layer.selectBelow", "Select Layer Below", "Alt+[", Step::Below, false),
        spec!("layer.selectTop", "Select Top Layer", "Alt+.", Step::Top, false),
        spec!("layer.selectBottom", "Select Bottom Layer", "Alt+,", Step::Bottom, false),
        spec!("layer.addAboveToSelection", "Add Layer Above to Selection", "Alt+Shift+]", Step::Above, true),
        spec!("layer.addBelowToSelection", "Add Layer Below to Selection", "Alt+Shift+[", Step::Below, true),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bottom to top: Background, A, Group { B, C }, D. Panel rows: D, Group, C, B, A, Background.
    fn session() -> (Session, [LayerId; 6]) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        let bg = s.active().unwrap().doc.layers[0].id;
        let id = |s: &mut Session, cmd: &str, p: Value| LayerId(s.execute(cmd, p).unwrap()["layer"].as_u64().unwrap());
        let a = id(&mut s, "layer.new.layer", json!({"name": "A"}));
        let b = id(&mut s, "layer.new.layer", json!({"name": "B"}));
        let c = id(&mut s, "layer.new.layer", json!({"name": "C"}));
        s.execute("layer.select", json!({"layer": b.0})).unwrap();
        s.execute("layer.select", json!({"layer": c.0, "mode": "add"})).unwrap();
        s.execute("layer.groupLayers", json!({})).unwrap();
        let group = s.active().unwrap().doc.layers.iter().find(|l| l.is_group()).unwrap().id;
        // D goes above the closed group (an open one would take it inside, as in Photoshop).
        let d = id(&mut s, "layer.new.layer", json!({"name": "D"}));
        // Group Layers makes a collapsed group; open it so the panel shows B and C.
        s.execute("layer.setExpanded", json!({"layer": group.0, "expanded": true})).unwrap();
        (s, [bg, a, group, b, c, d])
    }

    fn active(s: &Session) -> LayerId {
        s.active().unwrap().active_layer.unwrap()
    }

    #[test]
    fn steps_follow_the_panel_rows_and_cycle_at_the_ends() {
        let (mut s, [bg, a, group, b, c, d]) = session();
        assert_eq!(panel_rows(&s.active().unwrap().doc), vec![d, group, c, b, a, bg]);
        s.execute("layer.select", json!({"layer": d.0})).unwrap();
        for want in [group, c, b, a, bg, d] {
            s.execute("layer.selectBelow", json!({})).unwrap();
            assert_eq!(active(&s), want);
        }
        s.select_layer(bg).unwrap();
        for want in [a, b, c, group, d, bg] {
            s.execute("layer.selectAbove", json!({})).unwrap();
            assert_eq!(active(&s), want);
        }
        s.execute("layer.selectBottom", json!({})).unwrap();
        assert_eq!((active(&s), s.active().unwrap().selected_layers()), (bg, vec![bg]));
        s.execute("layer.selectTop", json!({})).unwrap();
        assert_eq!(active(&s), d);
    }

    #[test]
    fn collapsed_groups_are_one_row_and_adding_extends_the_selection() {
        let (mut s, [bg, a, group, _b, c, d]) = session();
        s.edit("collapse", |doc, _| {
            if let Some(LayerContent::Group(g)) = doc.layer_mut(group).map(|l| &mut l.content) {
                g.expanded = false;
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(panel_rows(&s.active().unwrap().doc), vec![d, group, a, bg]);
        // A layer inside the collapsed group steps from the group's row.
        s.execute("layer.select", json!({"layer": c.0})).unwrap();
        s.execute("layer.selectBelow", json!({})).unwrap();
        assert_eq!(active(&s), a);
        s.execute("layer.addAboveToSelection", json!({})).unwrap();
        s.execute("layer.addAboveToSelection", json!({})).unwrap();
        let mut sel = s.active().unwrap().selected_layers();
        sel.sort();
        let mut want = vec![a, group, d];
        want.sort();
        assert_eq!((active(&s), sel), (d, want));
        s.execute("layer.addBelowToSelection", json!({})).unwrap();
        assert_eq!(active(&s), group, "the active layer moves; the selection keeps every layer");
        assert_eq!(s.active().unwrap().selected_layers().len(), 3);
    }

    #[test]
    fn selecting_never_dirties_and_bad_states_are_errors() {
        let (mut s, _) = session();
        let st = s.active_mut().unwrap();
        st.saved_revision = st.revision;
        s.execute("layer.selectTop", json!({"junk": [1, 2]})).unwrap();
        assert!(!s.active().unwrap().is_dirty(), "selecting a layer is not an edit");
        let mut empty = Session::new();
        for c in specs() {
            assert!(!empty.is_enabled(c.id));
            assert!(empty.execute(c.id, json!({})).is_err());
        }
        empty.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
        empty
            .edit("clear", |doc, active| {
                doc.layers.clear();
                *active = None;
                Ok(())
            })
            .unwrap();
        for c in specs() {
            assert!(empty.execute(c.id, json!({})).is_err(), "{}: no layers", c.id);
        }
    }
}
