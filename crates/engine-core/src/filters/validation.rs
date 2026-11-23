//! Validate new filter and adjustment invocations against their parameter schema (the
//! command's `params` doc, read by [`crate::param_schema`]), without changing how old
//! smart-filter records decode. The doc is the one copy of the bounds and choices: there is no
//! table beside it to keep in sync (#7597).

use serde_json::Value;

use crate::{EngineError, Result};

/// Params whose documented range is the dialog's, while commands still accept values outside
/// it and clamp or place them: Lens Flare's centre may sit off the canvas; Camera Raw
/// exposure and Selective Color ink sliders clamp ATN or legacy values to the documented range.
const DIALOG_RANGES: &[(&str, &str)] = &[
    ("filter.render.lensFlare", "centerX"),
    ("filter.render.lensFlare", "centerY"),
    ("filter.cameraRaw", "exposure"),
    ("image.adjustments.selectiveColor", "cyan"),
    ("image.adjustments.selectiveColor", "magenta"),
    ("image.adjustments.selectiveColor", "yellow"),
    ("image.adjustments.selectiveColor", "black"),
];

/// The commands whose values are checked against their schema: the pixel filters (the gallery
/// filters have their own typed catalogue, checked in `gallery_cmds`) and the adjustments.
fn checked(id: &str) -> bool {
    (id.starts_with("filter.") && !id.starts_with("filter.gallery.") && id != "filter.filterGallery") || id.starts_with("image.adjustments.")
}

/// Runs both at dispatch (before committing a floating selection) and at the shared runner
/// (direct specs/Last Filter). Commands outside [`checked`] are unaffected.
pub(crate) fn validate_params(id: &str, params: &Value) -> Result<()> {
    if !checked(id) {
        return crate::gallery_cmds::validate_params(id, params);
    }
    let bad = |msg| EngineError::BadParams { cmd: id.into(), msg };
    let Some(spec) = crate::commands::find(id) else { return Ok(()) };
    if params.is_null() {
        return Ok(());
    }
    let map = params.as_object().ok_or_else(|| bad("params must be a JSON object".into()))?;
    crate::param_check::check(spec, params)?;
    let Some(schema) = crate::param_schema::of(spec) else { return Ok(()) };
    for (key, value) in map {
        let (valid, expected) = match (schema.param(key), key.as_str()) {
            (Some(p), _) if DIALOG_RANGES.contains(&(id, key.as_str())) => (value.is_number() || (value.is_null() && p.optional()), "a number".into()),
            (Some(_), "shapeId" | "shapeName") if id == "filter.blur.shapeBlur" => (value.as_str().is_some_and(|s| !s.is_empty()), "a nonempty string".into()),
            (Some(_), "shapePath") if id == "filter.blur.shapeBlur" => {
                (crate::vector_cmds::parse_path(value).is_ok_and(|path| !path.is_empty() && path.control_bounds().is_some()), "a nonempty vector path".into())
            }
            (Some(p), _) => (p.accepts(value), p.expected()),
            // The same selectors `commands::explicit_layer` already reads: an id, a name, or a
            // recorded Action object (`{"name":"Layer 1"}`, `{"background":true}`, index/ordinal).
            (None, "layer") => (
                value.as_u64().is_some()
                    || value.is_string()
                    || value.as_object().is_some_and(|o| {
                        (o.len() == 1 && (o.get("name").and_then(Value::as_str).is_some() || o.get("background") == Some(&Value::Bool(true))))
                            || o.contains_key("index")
                            || o.contains_key("ordinal")
                    }),
                r#"a layer id, name, {"name":string}, {"background":true}, or an index/ordinal selector"#.into(),
            ),
            (None, "coalesce") => (value.is_string(), "a string".into()),
            (None, "target") if id.starts_with("filter.") => (
                value.as_str().is_some_and(|s| matches!(s, "pixels" | "mask" | "filterMask" | "quickMask"))
                    || value.as_object().is_some_and(|o| {
                        o.len() == 1
                            && (o.get("channel").and_then(Value::as_u64).is_some()
                                || o.get("color").and_then(Value::as_u64).is_some()
                                || o.get("colors")
                                    .and_then(Value::as_array)
                                    .is_some_and(|a| !a.is_empty() && a.len() <= 64 && a.iter().all(|v| v.as_u64().is_some())))
                    }),
                "pixels, mask, filterMask, quickMask, {\"channel\":index}, {\"color\":index} or {\"colors\":[indices]}".into(),
            ),
            (None, _) => (true, String::new()),
        };
        if !valid {
            let got = if value.is_array() || value.is_object() { String::new() } else { format!(", not {value}") };
            return Err(bad(format!("`{key}` must be {expected}{got}")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
