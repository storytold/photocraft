//! Color Lookup by library id. Agents cannot name files (the automation interface refuses the
//! `file` param), so `lut` also takes the id `lut.library` lists, `Pack/Name.cube`. The id is
//! resolved here, where the session and its LUT library are, before the command's own params are
//! read: the LUT's text is handed on as the `data` and `fileName` params, which Color Lookup
//! already embeds in the layer. The document therefore never depends on the library, and
//! [`adjust_cmds::lookup_from_params`](crate::adjust_cmds::lookup_from_params) stays free of
//! session access.
//!
//! An id is only ever compared with the library's listing ([`LutLibrary::read_by_id`]); it is never
//! turned into a path. Anything the listing does not hold (including `..`, absolute paths and
//! backslashes) is left for `lookup_from_params` to refuse as an unknown look.

use serde_json::Value;

use crate::{EngineError, Result, Session};
use photocraft_doc::{Adjustment, LayerContent};

/// The commands whose Color Lookup `lut` param may be a library id.
fn is_lookup_command(s: &Session, id: &str, p: &Value) -> bool {
    match id {
        "image.adjustments.colorLookup" | "layer.newAdjustmentLayer.colorLookup" => true,
        // Other adjustment kinds have no `lut` param, so only a Color Lookup layer is looked at.
        "layer.setAdjustment" => crate::commands::layer_param(s, p).ok().is_some_and(|layer| {
            s.active().and_then(|st| st.doc.layer(layer)).is_some_and(|l| matches!(l.content, LayerContent::Adjustment(Adjustment::ColorLookup { .. })))
        }),
        _ => false,
    }
}

/// `p` for command `id`, with a library id in `lut` replaced by the LUT's embedded text. Params
/// that name no library LUT (built-in looks, `"none"`, no library, an unknown id, or `data` given
/// as well) come back unchanged; an installed LUT that cannot be read is an error.
pub(crate) fn resolve(s: &Session, id: &str, mut p: Value) -> Result<Value> {
    let Some(lut) = p.get("lut").and_then(Value::as_str).map(str::to_string) else { return Ok(p) };
    let builtin = lut.is_empty() || lut == "none" || photocraft_cms::lutfile::BUILTIN.iter().any(|b| b.0 == lut);
    if builtin || p.get("data").and_then(Value::as_str).is_some() || !is_lookup_command(s, id, &p) {
        return Ok(p);
    }
    let Some(lib) = s.lut_library.as_ref() else { return Ok(p) };
    let found = lib.read_by_id(&lut).map_err(|msg| EngineError::BadParams { cmd: "colorLookup".into(), msg })?;
    let Some((file_name, bytes)) = found else { return Ok(p) };
    // Refuse invalid UTF-8 rather than let the parser's lossy conversion silently change the table.
    let text =
        String::from_utf8(bytes).map_err(|_| EngineError::BadParams { cmd: "colorLookup".into(), msg: format!("{file_name}: LUT must contain UTF-8 text") })?;
    if let Some(obj) = p.as_object_mut() {
        obj.remove("lut");
        obj.insert("data".into(), Value::String(text));
        obj.insert("fileName".into(), Value::String(file_name));
    }
    Ok(p)
}

#[cfg(test)]
mod tests;
