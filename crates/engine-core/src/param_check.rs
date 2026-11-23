//! Reject params a command does not document (#4336, #7596).
//!
//! Commands read the keys they know from a JSON object and used to ignore the rest, so a
//! misspelled key (`{"ids":[4]}` for `{"layer":4}`) ran the command on its defaults and reported
//! success. The dispatcher checks the keys against the command's parameter schema (its `params`
//! doc, read by [`crate::param_schema`]) once, for every transport (CLI, MCP, control channel,
//! UI, actions), for every command.
//!
//! A doc with a top-level `…` is open-ended and is not checked, and neither is a doc the schema
//! can't read (`param_schema::tests` lists those, so the list only shrinks).

use serde_json::Value;

use crate::commands::CommandSpec;
use crate::param_schema::{self, UNIVERSAL};
use crate::{EngineError, Result};

/// Commands whose doc does not yet name every key they read (the engine's own tests pass
/// them more): not checked until their doc is completed. The list may only shrink.
const INCOMPLETE: &[&str] = &[
    "actions.duplicate",
    "actions.photoshopStep",
    "actions.play",
    "actions.record",
    "brush.mixer.sample",
    "brush.presets.move",
    "brush.presets.moveGroup",
    "count.move",
    "count.remove",
    "document.activate",
    "edit.fill",
    "edit.fillForeground",
    "edit.transform.flipVertical",
    "edit.transform.splitWarpCrosswise",
    "file.automate.batch",
    "file.automate.createDroplet",
    "file.export.renderVideo",
    "file.fileInfo",
    "file.import.videoFramesToLayers",
    "file.printOneCopy",
    "file.printers",
    "file.scripts.browse",
    "gradient.presets.select",
    "history.setSource",
    "image.adjustments.colorLookup",
    "image.adjustments.curves",
    "image.adjustments.equalize",
    "image.adjustments.hdrToning",
    "image.adjustments.levels",
    "image.adjustments.replaceColor",
    "image.adjustments.selectiveColor",
    "image.adjustments.shadowsHighlights",
    "image.analysis.recordMeasurements",
    "image.applyImage",
    "layer.artboard.set",
    "layer.groupLayers",
    "layer.layerStyle.bevelEmboss",
    "layer.layerStyle.colorOverlay",
    "layer.layerStyle.dropShadow",
    "layer.layerStyle.gradientOverlay",
    "layer.layerStyle.innerGlow",
    "layer.layerStyle.innerShadow",
    "layer.layerStyle.outerGlow",
    "layer.layerStyle.patternOverlay",
    "layer.layerStyle.satin",
    "layer.layerStyle.stroke",
    "layer.lockLayers",
    "layer.mergeVisible",
    "layer.moveTo",
    "layer.new.artboard",
    "layer.select",
    "layer.toggleMaskOverlay",
    "layer.videoLayers.interpretFootage",
    "layer.videoLayers.newVideoLayerFromFile",
    "layer.videoLayers.replaceFootage",
    "measurementLog.delete",
    "notes.delete",
    "paint.artHistoryBrush",
    "paint.backgroundEraser",
    "paint.blur",
    "paint.bucket",
    "paint.burn",
    "paint.cloneStamp",
    "paint.colorReplacement",
    "paint.dodge",
    "paint.healingBrush",
    "paint.historyBrush",
    "paint.mixerBrush",
    "paint.patternStamp",
    "paint.pencil",
    "paint.redEye",
    "paint.sharpen",
    "paint.smudge",
    "paint.sponge",
    "paint.spotHealing",
    "paint.stroke",
    "path.addToSelection",
    "path.intersectWithSelection",
    "path.set",
    "path.stroke",
    "path.subtractFromSelection",
    "path.toShape",
    "path.transform",
    "plugin.run",
    "prefs.reset",
    "prefs.set",
    "select.convertToShape",
    "select.modify.border",
    "select.object",
    "select.transformSelection",
    "shape.create",
    "shape.edit",
    "slice.divide",
    "slice.new",
    "slice.promote",
    "slice.set",
    "style.presets.apply",
    "swatches.delete",
    "swatches.use",
    "type.edit",
    "type.setStyle",
];

/// Err when `params` has a key the command's schema does not document.
pub(crate) fn check(spec: &CommandSpec, params: &Value) -> Result<()> {
    let Some(map) = params.as_object() else { return Ok(()) };
    let Some(schema) = param_schema::of(spec) else { return Ok(()) };
    if schema.open() || INCOMPLETE.contains(&spec.id) {
        return Ok(());
    }
    for key in map.keys() {
        if UNIVERSAL.contains(&key.as_str()) || schema.has_key(key) || implied(spec.id, key) {
            continue;
        }
        let keys = schema.keys();
        let expected = if keys.is_empty() { "no parameters".to_string() } else { format!("expected: {}", keys.join(", ")) };
        return Err(EngineError::BadParams { cmd: spec.id.to_string(), msg: format!("unknown parameter `{key}` ({expected})") });
    }
    Ok(())
}

/// Keys accepted beyond the docs: every pixel filter runs on a named `"layer"` (the Filter
/// Gallery and Smart Filters record it), every adjustment on a `"layer"` and a
/// `"channel"`/`"targetChannel"` (`adjust_params` reads them for all kinds), and `_`-prefixed
/// keys are the engine's own bookkeeping (a PSD-embedded displacement map, an imported action's
/// descriptor), never typed by a caller.
/// Whether `id` is on the incomplete-docs list: unknown keys are not rejected yet.
pub fn is_incomplete(id: &str) -> bool {
    INCOMPLETE.contains(&id)
}

fn implied(id: &str, key: &str) -> bool {
    (key == "layer" && id.starts_with("filter."))
        || (id.starts_with("image.adjustments.") && matches!(key, "layer" | "channel" | "targetChannel"))
        || key.starts_with('_')
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::Session;

    #[test]
    fn unknown_param_is_an_error_naming_the_key_and_the_accepted_ones() {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        let a = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        let visible = |s: &Session| s.active().unwrap().doc.layer(photocraft_doc::LayerId(a)).unwrap().visible;
        for cmd in ["layer.hideLayers", "layer.showLayers"] {
            let err = s.execute(cmd, json!({"ids": [a]})).unwrap_err().to_string();
            assert!(err.contains("unknown parameter `ids`") && err.contains("expected: layer"), "{err}");
            assert!(visible(&s), "{cmd}: nothing ran");
        }
        s.execute("layer.hideLayers", json!({"layer": a, "coalesce": "k"})).unwrap();
        assert!(!visible(&s));
    }

    #[test]
    fn every_command_with_a_closed_doc_rejects_an_unknown_key() {
        // #7596: an undocumented key used to run the command on its defaults and report success.
        crate::install().unwrap();
        let mut checked = 0;
        for spec in crate::commands::command_specs() {
            let Some(schema) = param_schema::of(spec) else { continue };
            let typo = json!({"zzTypo": 1});
            if schema.open() || INCOMPLETE.contains(&spec.id) {
                assert!(check(spec, &typo).is_ok(), "{}", spec.id);
                continue;
            }
            let err = check(spec, &typo).expect_err(spec.id).to_string();
            assert!(err.contains("unknown parameter `zzTypo`"), "{err}");
            for key in UNIVERSAL {
                assert!(check(spec, &json!({ *key: "k" })).is_ok(), "{}: {key}", spec.id);
            }
            for key in schema.keys() {
                if !key.contains('<') {
                    assert!(check(spec, &json!({ key: null })).is_ok(), "{}: {key}", spec.id);
                }
            }
            checked += 1;
        }
        assert!(checked > 850, "{checked}");
    }

    #[test]
    fn incomplete_docs_are_registered_closed_docs() {
        crate::install().unwrap();
        for id in INCOMPLETE {
            let spec = crate::commands::find(id).unwrap_or_else(|| panic!("{id} is not registered"));
            assert!(param_schema::of(spec).is_some_and(|p| !p.open()), "{id}: open or unparsed docs are not checked anyway");
        }
        assert!(INCOMPLETE.windows(2).all(|w| w[0] < w[1]), "sorted, no duplicates");
    }

    #[test]
    fn a_misspelled_key_fails_through_dispatch_instead_of_running_on_defaults() {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let err = s.execute("paint.gradient", json!({"from": [0, 0], "to": [7, 7], "colour": "red"})).unwrap_err().to_string();
        assert!(err.contains("unknown parameter `colour`") && err.contains("paint.gradient"), "{err}");
        let err = s.precheck("filter.blur.gaussianBlur", &json!({"radus": 3})).unwrap_err().to_string();
        assert!(err.contains("unknown parameter `radus`") && err.contains("expected: radius"), "{err}");
    }
}
