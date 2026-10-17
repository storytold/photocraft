//! Records a tool keeps in a document: free-form user data stored with it, and layer ids that
//! stay the same when the file is saved and opened again.
//!
//! A tool that tracks what a document holds (a studio's publishing pipeline, an asset manager,
//! a batch script) leaves notes in the file ("this group is published as …", "this layer was
//! loaded from version 12") and has to find them again later, here or after a trip through
//! Photoshop.
//!
//! * `document.userData` keeps JSON values under caller-chosen keys in the document's XMP packet
//!   ([`photocraft_io::user_data`]), so the files that keep the layers (PSD, PSB, layered TIFF,
//!   `.pcraft`) keep it; flat exports leave it out. XMP asks other applications to preserve
//!   properties they don't know.
//! * `layer.persistentId` reads the layer's persistent id (the PSD `lyid`), which every layer gets
//!   when it is created ([`crate::layer_ids`]). The `id` of `document.inspect` is a session id: a
//!   PSD's layers get new ones every time the file opens, so a note that names a layer uses this id.

use photocraft_doc::LayerId;
use photocraft_io::user_data::{self, UserDataError};
use serde_json::{Map, Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const USER_DATA: &str = "document.userData";
const PERSISTENT_ID: &str = "layer.persistentId";

/// Longest key, in bytes.
const MAX_KEY: usize = 64;
/// Largest value, as stored: its JSON text, XML-escaped.
const MAX_VALUE: usize = 1 << 20;
/// Largest user data of one document, as stored: the packet is written into every layered save.
const MAX_TOTAL: usize = 4 << 20;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_layers(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if d.doc.layers.is_empty() { Err("the document has no layers".into()) } else { Ok(()) }
}

/// The optional `key` param: 1 to [`MAX_KEY`] characters from `A-Z a-z 0-9 . _ -`.
fn key_param(p: &Value) -> Result<Option<&str>> {
    match p.get("key") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(k)) if !k.is_empty() && k.len() <= MAX_KEY && k.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) => Ok(Some(k)),
        Some(_) => Err(bad(USER_DATA, format!("`key` must be 1 to {MAX_KEY} characters from A-Z a-z 0-9 . _ -"))),
    }
}

/// The active document's user data. Data that is there but can't be read is an error (never an
/// empty map a set would then silently replace), saying how to clear it.
fn current(s: &Session) -> Result<Map<String, Value>> {
    let xmp = s.active().ok_or(EngineError::NoDocument)?.doc.metadata.xmp.as_deref();
    user_data::read(xmp).map_err(|e| EngineError::Other(format!("{e}; pass {{\"remove\": true}} without a `key` to clear it")))
}

/// Store `data` as the active document's user data (one undo step).
fn store(s: &mut Session, data: &Map<String, Value>) -> Result<()> {
    // Not journaled (a pipeline's notes must not replay from a recorded action), so the
    // dispatcher doesn't refuse this while a background job runs on the document; a change made
    // then would be lost when the job applies its result.
    if let Some(job) = s.active_job() {
        let busy = format!("“{}” is still running on this document; wait for it to finish or cancel it (Esc)", job.label);
        return Err(EngineError::Disabled(USER_DATA.to_string(), busy));
    }
    let xmp = s.active().ok_or(EngineError::NoDocument)?.doc.metadata.xmp.clone();
    let new = user_data::write(xmp.as_deref(), data).map_err(|e| match e {
        UserDataError::Character(_) => bad(USER_DATA, format!("`value`: {e}")),
        _ => EngineError::Other(e.to_string()),
    })?;
    s.edit("User Data", |doc, _| {
        doc.metadata.xmp = Some(new);
        Ok(())
    })
}

fn user_data(s: &mut Session, p: &Value) -> Result<Value> {
    let key = key_param(p)?;
    let remove = match p.get("remove") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool().ok_or_else(|| bad(USER_DATA, "`remove` must be true or false"))?,
    };
    let value = p.get("value").filter(|v| !v.is_null());
    let Some(key) = key else {
        return match (value, remove) {
            (Some(_), _) => Err(bad(USER_DATA, "`value` needs a `key`")),
            (None, false) => Ok(json!({"data": current(s)?})),
            // Everything, readable or not: the way out of data that can't be read.
            (None, true) => {
                let xmp = s.active().ok_or(EngineError::NoDocument)?.doc.metadata.xmp.clone();
                let present = !matches!(user_data::read(xmp.as_deref()), Ok(data) if data.is_empty());
                if present {
                    store(s, &Map::new())?;
                }
                Ok(json!({"data": {}, "removed": present}))
            }
        };
    };
    let mut data = current(s)?;
    match (value, remove) {
        (Some(_), true) => Err(bad(USER_DATA, "pass `value` or `remove`, not both")),
        (None, false) => Ok(json!({"key": key, "value": data.get(key)})),
        (None, true) => {
            let removed = data.remove(key).is_some();
            if removed {
                store(s, &data)?;
            }
            Ok(json!({"key": key, "value": null, "removed": removed}))
        }
        (Some(value), false) => {
            if user_data::stored_len(value) > MAX_VALUE {
                return Err(bad(USER_DATA, format!("`value` is larger than {MAX_VALUE} bytes as stored (JSON, XML-escaped)")));
            }
            // Storing what is already there is not an edit: no undo step, a clean document stays clean.
            if data.get(key) != Some(value) {
                data.insert(key.to_string(), value.clone());
                if user_data::stored_len(&Value::Object(data.clone())) > MAX_TOTAL {
                    return Err(bad(USER_DATA, format!("the document's user data would be larger than {MAX_TOTAL} bytes as stored (JSON, XML-escaped)")));
                }
                store(s, &data)?;
            }
            Ok(json!({"key": key, "value": value}))
        }
    }
}

fn persistent_id(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = match p.get("layer") {
        None | Some(Value::Null) => st.active_layer.ok_or_else(|| bad(PERSISTENT_ID, "no layer given and no active layer"))?,
        Some(v) => LayerId(v.as_u64().ok_or_else(|| bad(PERSISTENT_ID, "`layer` must be a layer id"))?),
    };
    let layer = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let persistent = layer.psd_id.ok_or_else(|| EngineError::Other(format!("layer {} has no persistent id", id.0)))?;
    Ok(json!({"layer": id.0, "persistentId": persistent}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: USER_DATA,
            label: "Document User Data",
            menu: &[],
            shortcut: None,
            params: r##"{"key":str?,"value":json?,"remove":bool=false} (no key: everything, as {"data":{key:value}}; key alone: read it; value: store any JSON under the key; remove: delete the key, or without a key all of it, readable or not. Keys are 1..64 characters from A-Z a-z 0-9 . _ -; a value is at most 1 MiB and all of it 4 MiB as stored. Kept in the document's XMP, so PSD, PSB, layered TIFF and .pcraft files keep it and flat exports leave it out; a change is one undo step)"##,
            enabled: has_doc,
            run: user_data,
            // A pipeline's notes about this document must not replay onto others from a recorded action.
            journal: false,
        },
        CommandSpec {
            id: PERSISTENT_ID,
            label: "Persistent Layer ID",
            menu: &[],
            shortcut: None,
            params: r##"{"layer":id?} → {"layer","persistentId"}: the layer's persistent id (the PSD layer id), given when the layer is created and the same after the file is saved and reopened as PSD, PSB, layered TIFF or .pcraft (the "id" of document.inspect is a session id)"##,
            enabled: has_layers,
            run: persistent_id,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests;
