use super::{CommandSpec, EngineError, Rect, Value, always, bad, command_specs, has_doc, inspect, int, json, list_entry};

/// Commands that report session and document state or switch documents.
pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "session.inspect",
            label: "Inspect Session",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: |s, _| Ok(inspect::session(s)),
            journal: false,
        },
        CommandSpec {
            id: "document.inspect",
            label: "Inspect Document",
            menu: &[],
            shortcut: None,
            params: r##"{"document":index?}"##,
            enabled: has_doc,
            run: |s, p| {
                let d = match p.get("document") {
                    Some(v) => {
                        let i = v.as_u64().and_then(|v| usize::try_from(v).ok()).ok_or_else(|| bad("document.inspect", "`document` must be an index"))?;
                        s.documents().get(i).ok_or_else(|| EngineError::Other(format!("no document at index {i}")))?
                    }
                    None => s.active().ok_or(EngineError::NoDocument)?,
                };
                Ok(inspect::document(d))
            },
            journal: false,
        },
        CommandSpec {
            id: "document.activate",
            label: "Activate Document",
            menu: &[],
            shortcut: None,
            params: r##"{"document":index|{"index":uint}} or {"offset":int} (numeric document: zero-based tab index; typed index: one-based opening order; offset: places after the active document in opening order, before it when negative) → {document}"##,
            enabled: has_doc,
            run: |s, p| {
                // A null key counts as omitted.
                let given = |k: &str| p.get(k).filter(|v| !v.is_null());
                let idx = match (given("document"), given("offset")) {
                    (Some(_), Some(_)) => return Err(bad("document.activate", "give `document` or `offset`, not both")),
                    (None, None) => return Err(bad("document.activate", "needs `document` (a tab index) or `offset`")),
                    // Recorded offsets wrap in opening order, independently of dragged tabs.
                    (None, Some(o)) => {
                        let o = o.as_i64().ok_or_else(|| bad("document.activate", "`offset` must be an integer"))?;
                        s.document_at_offset(o)?
                    }
                    (Some(d), None) => {
                        if let Some(selector) = d.as_object() {
                            let index = selector
                                .get("index")
                                .filter(|_| selector.len() == 1)
                                .and_then(Value::as_u64)
                                .ok_or_else(|| bad("document.activate", "document reference needs one non-negative index"))?;
                            return s
                                .document_at_index(index)
                                .and_then(|idx| if s.set_active(idx) { Ok(json!({"document": idx})) } else { Err(EngineError::NoDocument) });
                        }
                        let i = d.as_u64().ok_or_else(|| bad("document.activate", "`document` must be a tab index"))?;
                        usize::try_from(i).map_err(|_| bad("document.activate", "`document` out of range"))?
                    }
                };
                if s.set_active(idx) { Ok(json!({"document": idx})) } else { Err(EngineError::Other(format!("no document at index {idx}"))) }
            },
            journal: true,
        },
        CommandSpec {
            id: "document.move",
            label: "Move Document",
            menu: &[],
            shortcut: None,
            params: r##"{"document":index?,"to":index}"##,
            enabled: has_doc,
            run: |s, p| {
                let index = |key: &str| {
                    p.get(key)
                        .map(|v| v.as_u64().and_then(|v| usize::try_from(v).ok()).ok_or_else(|| bad("document.move", format!("`{key}` must be a tab index"))))
                };
                let from = match index("document") {
                    Some(i) => i?,
                    None => s.active_index().ok_or(EngineError::NoDocument)?,
                };
                let to = index("to").ok_or_else(|| bad("document.move", "missing `to`"))??;
                let to = s.move_document(from, to).ok_or(EngineError::NoDocument)?;
                Ok(json!({"document": to}))
            },
            journal: false,
        },
        CommandSpec {
            id: "command.list",
            label: "List Commands",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: |s, _| Ok(Value::Array(command_specs().iter().map(|c| list_entry(s, c, (c.enabled)(s).err())).collect())),
            journal: false,
        },
        CommandSpec {
            id: "document.pixel",
            label: "Read Composite Pixel",
            menu: &[],
            shortcut: None,
            params: r##"{"x":i32,"y":i32}"##,
            enabled: has_doc,
            run: |s, p| {
                let coord = |k: &str| i32::try_from(int(p, k).unwrap_or(0)).map_err(|_| bad("document.pixel", format!("{k} must fit in 32 bits")));
                let (x, y) = (coord("x")?, coord("y")?);
                let d = s.active().ok_or(EngineError::NoDocument)?;
                // At i32::MAX the 1x1 rect saturates to empty: a pixel that far out is transparent.
                let px = photocraft_compose::render(&d.doc, Rect::from_xywh(x, y, 1, 1)).px.first().copied().unwrap_or_default();
                Ok(json!(px))
            },
            journal: false,
        },
    ]
}
