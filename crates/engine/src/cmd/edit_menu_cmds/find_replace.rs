use super::{Document, EngineError, FindCursor, LayerContent, LayerId, Result, Session, Value, bad, bool_or, json, layer_param, str_or};

// ------------------------------------------------------------------ Find and Replace Text

/// Byte ranges of `find` in `text` (case-insensitive unless `case`; whole words only with `whole`).
pub fn find_matches(text: &str, find: &str, case: bool, whole: bool) -> Vec<(usize, usize)> {
    if find.is_empty() {
        return Vec::new();
    }
    let norm = |c: char| -> String { if case { c.to_string() } else { c.to_lowercase().collect() } };
    let needle: String = find.chars().map(norm).collect();
    let mut out = Vec::new();
    let mut at = 0;
    while at < text.len() {
        // Compare char by char so case folding that changes byte lengths stays aligned.
        let mut hay = String::new();
        let mut end = at;
        for (i, c) in text[at..].char_indices() {
            if hay.len() >= needle.len() {
                break;
            }
            hay.push_str(&norm(c));
            end = at + i + c.len_utf8();
        }
        if hay == needle {
            let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            let before = text[..at].chars().next_back();
            let after = text[end..].chars().next();
            if !whole || (!word(before) && !word(after)) {
                out.push((at, end));
                at = end;
                continue;
            }
        }
        at += text[at..].chars().next().map_or(1, char::len_utf8);
    }
    out
}

fn type_layers(doc: &Document) -> Vec<LayerId> {
    // Layers panel order: bottom to top (upward), with group contents in stack position.
    doc.walk().into_iter().filter(|(p, _, l)| matches!(l.content, LayerContent::Text(_)) && !doc.locks_at(p).all).map(|(_, _, l)| l.id).collect()
}

/// Index in `layers` to start a new round from `active` (going upward in `doc.walk()`).
fn start_index(doc: &Document, layers: &[LayerId], active: Option<LayerId>) -> usize {
    let Some(aid) = active else { return 0 };
    if let Some(pos) = layers.iter().position(|id| *id == aid) {
        return pos;
    }
    let walk = doc.walk();
    let Some(active_walk) = walk.iter().position(|(_, _, l)| l.id == aid) else {
        return 0;
    };
    walk.iter().skip(active_walk + 1).find_map(|(_, _, l)| layers.iter().position(|id| *id == l.id)).unwrap_or(0)
}

pub(super) fn can_find_replace(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    // A document without type keeps the menu available. Documents with only locked type
    // retain their existing refusal.
    if type_layers(&d.doc).is_empty() && d.doc.walk().iter().any(|(_, _, layer)| matches!(layer.content, LayerContent::Text(_))) {
        Err("the document has no type layers".into())
    } else {
        Ok(())
    }
}

pub(super) fn find_replace(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "edit.findAndReplaceText";
    let find = p.get("find").and_then(Value::as_str).filter(|f| !f.is_empty()).ok_or_else(|| bad(cmd, "missing `find`"))?.to_string();
    let replace = str_or(p, "replace", "").to_string();
    let (case, whole, forward, all_layers) =
        (bool_or(p, "caseSensitive", false), bool_or(p, "wholeWord", false), bool_or(p, "forward", true), bool_or(p, "allLayers", true));
    let action = str_or(p, "action", "changeAll").to_string();
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let active_layer = st.active_layer;
    let mut layers = type_layers(&st.doc);
    if !all_layers {
        let active = layer_param(s, p).ok();
        layers.retain(|id| Some(*id) == active);
    }
    let text_of = |doc: &Document, id: LayerId| -> Option<String> {
        match &doc.layer(id)?.content {
            LayerContent::Text(t) => Some(t.text.clone()),
            _ => None,
        }
    };
    match action.as_str() {
        "changeAll" => {
            let doc = st.doc.clone();
            let plan: Vec<(LayerId, Vec<(usize, usize)>)> =
                layers.iter().filter_map(|id| Some((*id, find_matches(&text_of(&doc, *id)?, &find, case, whole)))).filter(|(_, m)| !m.is_empty()).collect();
            let count: usize = plan.iter().map(|(_, m)| m.len()).sum();
            if count == 0 {
                s.edit_state.find_cursor = None;
                s.edit_state.find_start = None;
                return Ok(json!({"count": 0, "layers": 0}));
            }
            s.edit("Replace All Text", |doc, _| {
                let (dpi, format) = (doc.resolution_dpi, doc.pixel_format());
                for (id, matches) in &plan {
                    if let Some(LayerContent::Text(t)) = doc.layer_mut(*id).map(|l| &mut l.content) {
                        for (a, b) in matches.iter().rev() {
                            crate::type_cmds::replace_text(t, *a, *b, &replace);
                        }
                        crate::type_cmds::refresh_at(dpi, format, t);
                    }
                }
                Ok(())
            })?;
            s.edit_state.find_cursor = None;
            s.edit_state.find_start = None;
            Ok(json!({"count": count, "layers": plan.len()}))
        }
        "find" | "change" | "changeFind" => {
            let doc = st.doc.clone();
            let same_query = s
                .edit_state
                .find_cursor
                .as_ref()
                .is_some_and(|c| c.find == find && c.case_sensitive == case && c.whole_word == whole && c.forward == forward && c.all_layers == all_layers);
            if !same_query {
                // A new query starts a new round: forget both the cursor and the round origin.
                s.edit_state.find_cursor = None;
                s.edit_state.find_start = None;
            }
            let cursor = s.edit_state.find_cursor.as_ref().map(|c| (c.layer, c.start, c.end));
            let mut changed = None;
            if action != "find"
                && let Some((id, a, b)) = cursor
                && text_of(&doc, id).is_some_and(|t| find_matches(&t, &find, case, whole).contains(&(a, b)))
            {
                crate::type_cmds::check_text_unlocked(&doc, id)?;
                s.edit("Replace Text", |doc, _| {
                    let (dpi, format) = (doc.resolution_dpi, doc.pixel_format());
                    if let Some(LayerContent::Text(t)) = doc.layer_mut(id).map(|l| &mut l.content) {
                        crate::type_cmds::replace_text(t, a, b, &replace);
                        crate::type_cmds::refresh_at(dpi, format, t);
                    }
                    Ok(())
                })?;
                let end = a + replace.len();
                // Collapse the cursor on the side the search moves away from, so the next search
                // skips the replaced text even when the replacement still matches.
                let c = if forward { end } else { a };
                s.edit_state.find_cursor =
                    Some(FindCursor { layer: id, start: c, end: c, find: find.clone(), case_sensitive: case, whole_word: whole, forward, all_layers });
                changed = Some((id, a, end));
                if action == "change" {
                    return Ok(json!({"changed": {"layer": id.0, "start": a, "end": end}}));
                }
            }
            if action == "change" {
                return Ok(json!({"changed": null}));
            }
            // Find the next match after the cursor (upward, wrapping once, stopping before the start layer).
            let doc = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
            let cursor = s.edit_state.find_cursor.as_ref().map(|c| (c.layer, c.start, c.end));
            if !layers.is_empty() {
                let start = cursor.and_then(|(id, _, _)| layers.iter().position(|l| *l == id)).unwrap_or_else(|| start_index(&doc, &layers, active_layer));
                let origin = match cursor {
                    Some(_) => s.edit_state.find_start.filter(|o| layers.contains(o)).unwrap_or(layers[start]),
                    None => layers[start],
                };
                for k in 0..layers.len() {
                    let Some(&id) = layers.get((start + k) % layers.len()) else { break };
                    if k > 0 && id == origin {
                        break;
                    }
                    let Some(text) = text_of(&doc, id) else { continue };
                    let ms = find_matches(&text, &find, case, whole);
                    let hit = match (k, cursor) {
                        (0, Some((cid, a, b))) if cid == id => {
                            if forward {
                                ms.into_iter().find(|m| m.0 >= b)
                            } else {
                                ms.into_iter().rev().find(|m| m.1 <= a)
                            }
                        }
                        _ => {
                            if forward {
                                ms.into_iter().next()
                            } else {
                                ms.into_iter().next_back()
                            }
                        }
                    };
                    if let Some((a, b)) = hit {
                        s.edit_state.find_cursor =
                            Some(FindCursor { layer: id, start: a, end: b, find, case_sensitive: case, whole_word: whole, forward, all_layers });
                        s.edit_state.find_start = Some(origin);
                        let _ = s.select_layer(id);
                        return Ok(
                            json!({"found": {"layer": id.0, "start": a, "end": b, "text": &text[a..b]}, "changed": changed.map(|(l, a, b)| json!({"layer": l.0, "start": a, "end": b}))}),
                        );
                    }
                }
            }
            s.edit_state.find_cursor = None;
            s.edit_state.find_start = None;
            Ok(json!({"found": null, "changed": changed.map(|(l, a, b)| json!({"layer": l.0, "start": a, "end": b}))}))
        }
        other => Err(bad(cmd, format!("unknown action `{other}` (find|change|changeFind|changeAll)"))),
    }
}

#[cfg(test)]
#[path = "find_replace_enabled_tests.rs"]
mod enabled_tests;
