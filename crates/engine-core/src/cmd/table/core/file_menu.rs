use crate::cmd::{CommandSpec, EngineError, Value, bad, has_doc};

/// File menu: Close.
pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "file.close",
        "Close",
        ["File"],
        Some("Cmd+W"),
        r##"{"document":index?,"save":"discard|ask"|false="discard","force":bool?,"discard":bool?} (false means discard; ask refuses to close a changed document in headless playback; force is an older recorded Close and is ignored; discard:true is an older spelling of save:discard)"##,
        has_doc,
        |s, p| {
            let i = p.get("document").and_then(Value::as_u64).map(|v| v as usize).or(s.active_index()).ok_or(EngineError::NoDocument)?;
            match p.get("save").filter(|v| !v.is_null()) {
                Some(v) if v == "ask" && s.documents().get(i).ok_or(EngineError::NoDocument)?.is_dirty() => {
                    return Err(bad("file.close", "the document has unsaved changes; save it or explicitly discard them"));
                }
                Some(v) if v == "ask" => {}
                // `false` (don't save) is the older spelling of discard.
                Some(Value::Bool(false)) => {}
                Some(v) if v != "discard" => return Err(bad("file.close", "`save` must be ask, discard or false")),
                _ => {}
            }
            s.close(i).ok_or(EngineError::NoDocument)?;
            Ok(Value::Null)
        }
    )]
}
