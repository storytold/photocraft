use super::{Document, EngineError, FindCursor, LayerContent, LayerId, Result, Session, Value, bad, bool_or, json, layer_param, str_or};
use photocraft_doc::LayerPath;

#[cfg(test)]
#[path = "find_replace_hidden_tests.rs"]
mod hidden_tests;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockedTextLayer {
    Hidden,
    Locked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindReplaceWarning {
    pub layer: LayerId,
    pub name: String,
    pub reason: BlockedTextLayer,
}

fn type_layers(doc: &Document) -> impl Iterator<Item = (LayerPath, LayerId)> + '_ {
    doc.walk().into_iter().filter(|(_, _, l)| matches!(l.content, LayerContent::Text(_))).map(|(p, _, l)| (p, l.id))
}

fn blocked(doc: &Document, path: &[usize]) -> Option<BlockedTextLayer> {
    if !doc.shown_at(path) {
        Some(BlockedTextLayer::Hidden)
    } else if doc.locks_at(path).all {
        Some(BlockedTextLayer::Locked)
    } else {
        None
    }
}

pub(super) fn has_type_layer(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if type_layers(&d.doc).next().is_none() { Err("the document has no type layers".into()) } else { Ok(()) }
}

struct Query<'a> {
    find: &'a str,
    case: bool,
    whole: bool,
    forward: bool,
    all_layers: bool,
}

impl<'a> Query<'a> {
    fn parse(params: &'a Value) -> Result<Self> {
        let find = params.get("find").and_then(Value::as_str).filter(|f| !f.is_empty()).ok_or_else(|| bad("edit.findAndReplaceText", "missing `find`"))?;
        Ok(Self {
            find,
            case: bool_or(params, "caseSensitive", false),
            whole: bool_or(params, "wholeWord", false),
            forward: bool_or(params, "forward", true),
            all_layers: bool_or(params, "allLayers", true),
        })
    }

    fn includes(&self, id: LayerId, layer: Option<LayerId>) -> bool {
        self.all_layers || layer == Some(id)
    }

    fn accepts_cursor(&self, doc: &Document, c: &FindCursor) -> bool {
        c.find == self.find
            && c.case_sensitive == self.case
            && c.whole_word == self.whole
            && c.forward == self.forward
            && c.all_layers == self.all_layers
            && doc.path_of(c.layer).is_some_and(|p| doc.shown_at(&p))
    }

    fn next_match(&self, text: &str, cursor: Option<(usize, usize)>) -> Option<(usize, usize)> {
        let matches = find_matches(text, self.find, self.case, self.whole);
        if self.forward {
            matches.into_iter().find(|m| cursor.is_none_or(|(_, end)| m.0 >= end))
        } else {
            matches.into_iter().rev().find(|m| cursor.is_none_or(|(start, _)| m.1 <= start))
        }
    }
}

fn text_of(doc: &Document, id: LayerId) -> Option<&str> {
    match &doc.layer(id)?.content {
        LayerContent::Text(text) => Some(&text.text),
        _ => None,
    }
}

/// The blocked type layer a window would reach next. The check does not change the search cursor
/// or the document. Once the user acknowledges the warnings, the ordinary command does the work.
pub fn find_replace_warning(
    s: &Session,
    params: &Value,
    cursor: Option<&FindCursor>,
    skip: impl Fn(LayerId, BlockedTextLayer) -> bool,
) -> Result<Option<FindReplaceWarning>> {
    let query = Query::parse(params)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let window_layer = if query.all_layers { None } else { layer_param(s, params).ok() };
    let mut layers: Vec<_> = type_layers(doc).filter(|(_, id)| query.includes(*id, window_layer)).map(|(p, id)| (id, blocked(doc, &p))).collect();
    if query.forward {
        layers.reverse();
    }

    let find = str_or(params, "action", "changeAll") == "find";
    let cursor = cursor.filter(|c| find && query.accepts_cursor(doc, c));
    let start = cursor.and_then(|c| layers.iter().position(|(id, reason)| *id == c.layer && reason.is_none())).unwrap_or(0);
    let limit = layers.len() + usize::from(find && cursor.is_some());
    for k in 0..limit {
        let Some(&(id, reason)) = layers.get((start + k) % layers.len().max(1)) else { break };
        if let Some(reason) = reason {
            if !skip(id, reason)
                && let Some(layer) = doc.layer(id)
            {
                return Ok(Some(FindReplaceWarning { layer: id, name: layer.name.clone(), reason }));
            }
            continue;
        }
        if find {
            let range = cursor.filter(|c| k == 0 && c.layer == id).map(|c| (c.start, c.end));
            if text_of(doc, id).is_some_and(|text| query.next_match(text, range).is_some()) {
                break;
            }
        }
    }
    Ok(None)
}

pub(super) fn find_replace(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "edit.findAndReplaceText";
    let query = Query::parse(p)?;
    let Query { find, case, whole, forward, all_layers } = query;
    let replace = str_or(p, "replace", "");
    let action = str_or(p, "action", "changeAll");
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let window_layer = if all_layers { None } else { layer_param(s, p).ok() };
    let mut layers: Vec<_> =
        type_layers(&st.doc).filter(|(path, id)| query.includes(*id, window_layer) && blocked(&st.doc, path).is_none()).map(|(_, id)| id).collect();
    if forward {
        // Layers panel order: top first.
        layers.reverse();
    }
    match action {
        "changeAll" => {
            let doc = st.doc.clone();
            let plan: Vec<(LayerId, Vec<(usize, usize)>)> =
                layers.iter().filter_map(|id| Some((*id, find_matches(text_of(&doc, *id)?, find, case, whole)))).filter(|(_, m)| !m.is_empty()).collect();
            let count: usize = plan.iter().map(|(_, m)| m.len()).sum();
            if count == 0 {
                s.edit_state.find_cursor = None;
                return Ok(json!({"count": 0, "layers": 0}));
            }
            s.edit("Replace All Text", |doc, _| {
                let (dpi, format) = (doc.resolution_dpi, doc.pixel_format());
                for (id, matches) in &plan {
                    if let Some(LayerContent::Text(t)) = doc.layer_mut(*id).map(|l| &mut l.content) {
                        for (a, b) in matches.iter().rev() {
                            crate::type_cmds::replace_text(t, *a, *b, replace);
                        }
                        crate::type_cmds::refresh_at(dpi, format, t);
                    }
                }
                Ok(())
            })?;
            s.edit_state.find_cursor = None;
            Ok(json!({"count": count, "layers": plan.len()}))
        }
        "find" | "change" | "changeFind" => {
            let doc = st.doc.clone();
            let valid_cursor = s.edit_state.find_cursor.as_ref().is_some_and(|c| query.accepts_cursor(&doc, c));
            if !valid_cursor {
                s.edit_state.find_cursor = None;
            }
            let cursor = s.edit_state.find_cursor.as_ref().map(|c| (c.layer, c.start, c.end));
            let mut changed = None;
            if action != "find"
                && let Some((id, a, b)) = cursor
                && text_of(&doc, id).is_some_and(|t| find_matches(t, find, case, whole).contains(&(a, b)))
            {
                crate::type_cmds::check_text_unlocked(&doc, id)?;
                s.edit("Replace Text", |doc, _| {
                    let (dpi, format) = (doc.resolution_dpi, doc.pixel_format());
                    if let Some(LayerContent::Text(t)) = doc.layer_mut(id).map(|l| &mut l.content) {
                        crate::type_cmds::replace_text(t, a, b, replace);
                        crate::type_cmds::refresh_at(dpi, format, t);
                    }
                    Ok(())
                })?;
                let end = a + replace.len();
                // Collapse the cursor on the side the search moves away from, so the next search
                // skips the replaced text even when the replacement still matches.
                let c = if forward { end } else { a };
                s.edit_state.find_cursor =
                    Some(FindCursor { layer: id, start: c, end: c, find: find.to_string(), case_sensitive: case, whole_word: whole, forward, all_layers });
                changed = Some((id, a, end));
                if action == "change" {
                    return Ok(json!({"changed": {"layer": id.0, "start": a, "end": end}}));
                }
            }
            if action == "change" {
                return Ok(json!({"changed": null}));
            }
            // Find the next match after the cursor (wrapping through the layers).
            let doc = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
            let cursor = s.edit_state.find_cursor.as_ref().map(|c| (c.layer, c.start, c.end));
            let start = cursor.and_then(|(id, _, _)| layers.iter().position(|l| *l == id)).unwrap_or(0);
            for k in 0..=layers.len() {
                // `allLayers: false` with a non-type layer active leaves no layers to search (#703).
                let Some(&id) = layers.get((start + k) % layers.len().max(1)) else { break };
                let Some(text) = text_of(&doc, id) else { continue };
                let range = cursor.filter(|(cid, _, _)| k == 0 && *cid == id).map(|(_, a, b)| (a, b));
                if let Some((a, b)) = query.next_match(text, range) {
                    s.edit_state.find_cursor =
                        Some(FindCursor { layer: id, start: a, end: b, find: find.to_string(), case_sensitive: case, whole_word: whole, forward, all_layers });
                    let _ = s.select_layer(id);
                    return Ok(
                        json!({"found": {"layer": id.0, "start": a, "end": b, "text": &text[a..b]}, "changed": changed.map(|(l, a, b)| json!({"layer": l.0, "start": a, "end": b}))}),
                    );
                }
            }
            s.edit_state.find_cursor = None;
            Ok(json!({"found": null, "changed": changed.map(|(l, a, b)| json!({"layer": l.0, "start": a, "end": b}))}))
        }
        other => Err(bad(cmd, format!("unknown action `{other}` (find|change|changeFind|changeAll)"))),
    }
}
