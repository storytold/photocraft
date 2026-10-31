//! Embedded image library; original bytes are independent of their source files.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_doc::Document;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
pub const MAX_ASSET_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_LIBRARY_BYTES: usize = 512 * 1024 * 1024;
#[derive(Clone)]
pub struct Resource {
    pub name: String,
    pub bytes: Arc<Vec<u8>>,
}
#[derive(Clone)]
pub struct Asset {
    pub id: u64,
    pub source: Resource,
    pub document: Arc<Document>,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Packed {
    pub id: u64,
    pub name: String,
    pub entry: String,
}
fn err(e: impl Into<String>) -> EngineError {
    EngineError::Other(e.into())
}
pub(crate) fn decode(id: u64, name: String, bytes: Vec<u8>) -> Result<Asset> {
    if bytes.len() as u64 > MAX_ASSET_BYTES || name.is_empty() || name.len() > 4096 || name.contains('\0') {
        return Err(err("Invalid library image"));
    }
    let imported = photocraft_io::import(&name, &bytes).map_err(|e| err(e.to_string()))?;
    if imported.preview_only {
        return Err(err("This image can only be opened as a preview"));
    }
    Ok(Asset { id, source: Resource { name, bytes: Arc::new(bytes) }, document: Arc::new(imported.document) })
}
pub fn import_bytes(s: &mut Session, name: &str, bytes: Vec<u8>) -> Result<Value> {
    if s.uv.assets.len() >= 256 || s.uv.assets.iter().map(|a| a.source.bytes.len()).sum::<usize>().saturating_add(bytes.len()) > MAX_LIBRARY_BYTES {
        return Err(err("Image library exceeds 256 images or 512 MiB"));
    }
    let name = std::path::Path::new(name).file_name().map_or_else(|| name.to_owned(), |n| n.to_string_lossy().into_owned());
    let id = s.uv.assets.iter().map(|a| a.id).max().unwrap_or(0).checked_add(1).ok_or_else(|| err("Image library id overflow"))?;
    let asset = decode(id, name, bytes)?;
    s.uv.assets.push(asset);
    s.uv.generation = s.uv.generation.wrapping_add(1);
    Ok(json!({"asset":id}))
}
fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let path = p.get("path").and_then(Value::as_str).ok_or_else(|| err("path is required"))?;
    let bytes = crate::uv_cmds::read_limited(path, MAX_ASSET_BYTES)?;
    import_bytes(s, path, bytes)
}
fn id(p: &Value) -> Result<u64> {
    p.get("asset").and_then(Value::as_u64).ok_or_else(|| err("asset is required"))
}
fn place(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id(p)?;
    let source = s.uv.assets.iter().find(|a| a.id == id).ok_or_else(|| err("Image no longer exists"))?.source.clone();
    crate::file_cmds::place_bytes(s, &source.name, (*source.bytes).clone(), None, &json!({"smartObject":false}))
}
fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id(p)?;
    let index = s.uv.assets.iter().position(|a| a.id == id).ok_or_else(|| err("Image no longer exists"))?;
    s.uv.assets.remove(index);
    s.uv.generation = s.uv.generation.wrapping_add(1);
    Ok(Value::Null)
}
pub fn specs() -> Vec<CommandSpec> {
    [
        ("uv.asset.import", "Import library image", r#"{"path":string}"#, import as fn(&mut Session, &Value) -> Result<Value>),
        ("uv.asset.place", "Place library image", r#"{"asset":number}"#, place),
        ("uv.asset.remove", "Remove library image", r#"{"asset":number}"#, remove),
    ]
    .into_iter()
    .map(|(id, label, params, run)| CommandSpec {
        id,
        label,
        menu: &[],
        shortcut: None,
        params,
        enabled: |s| if s.uv.scene.is_some() { Ok(()) } else { Err("Import a model first".into()) },
        run,
        journal: false,
    })
    .collect()
}
