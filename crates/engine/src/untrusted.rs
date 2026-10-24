//! Commands that reach outside the document they edit: they read or write files by path, or
//! resolve settings that name files. Untrusted command sources are kept to the document by
//! refusing them: automation requests, and a design timeline replayed from a file.

use serde_json::Value;

/// Whether command `id` with `params` would read or write the filesystem beyond the document.
/// Filesystem-bearing commands have not yet been converted to consume directory capabilities, so
/// untrusted callers use capability-scoped document methods for file effects instead.
pub fn reaches_outside(id: &str, params: &Value) -> bool {
    let safe_file_command = matches!(
        id,
        "file.new"
            | "file.newFromClipboard"
            | "file.close"
            | "file.closeAll"
            | "file.closeOthers"
            | "file.fileInfo"
            | "file.automate.fitImage"
            | "file.automate.conditionalModeChange"
            | "file.automate.cropAndStraightenPhotos"
            | "file.scripts.flattenAllLayerEffects"
            | "file.scripts.flattenAllMasks"
            | "file.scripts.deleteAllEmptyLayers"
            | "file.export.exportPreferences"
    );
    if id.starts_with("file.") && !safe_file_command {
        return true;
    }
    // Contents commands re-check the resolved source in the engine before any disk read.
    // Photoshop's embedded linked-layer blocks count as in-memory sources too.
    let in_memory_smart_command = matches!(
        id,
        "layer.smartObjects.convertToSmartObject"
            | "layer.smartObjects.editContents"
            | "layer.smartObjects.convertToLayers"
            | "layer.smartObjects.saveContents"
    );
    (id.starts_with("layer.smartObjects.") && !in_memory_smart_command)
        || matches!(
            id,
            "pattern.import"
                | "pattern.export"
                | "edit.presets.migratePresets"
                | "measurementLog.export"
                | "layer.videoLayers.newVideoLayerFromFile"
                | "layer.videoLayers.replaceFootage"
                | "layer.videoLayers.reloadFrame"
        )
        || command_uses_ambient_path(id, params)
        || profile_command_may_read_ambient(id, params)
        || preferences_may_grant_ambient_paths(id, params)
        || params_contain_ambient_path(id, params)
}

/// [`reaches_outside`] for a desktop session, which may already hold user-configured
/// colour-profile paths: commands that implicitly resolve those settings count too, even when
/// their parameters contain no path. Fresh headless sessions can't acquire such settings.
pub fn reaches_outside_desktop(id: &str, params: &Value) -> bool {
    reaches_outside(id, params) || id.starts_with("image.mode.")
}

fn params_contain_ambient_path(id: &str, params: &Value) -> bool {
    let keys: &[&str] = match id {
        "image.adjustments.colorLookup" | "layer.newAdjustmentLayer.colorLookup" | "layer.setAdjustment" => &["file"],
        "filter.distort.displace" => &["mapPath"],
        "layer.quickExportAsPng" | "layer.exportAs" | "image.applyDataSet" | "layer.smartObjects.editContents" | "layer.smartObjects.convertToLayers" => {
            &["path"]
        }
        "image.mode.rgb" | "image.mode.grayscale" | "image.mode.cmyk" | "image.mode.lab" => &["profile"],
        "edit.assignProfile" | "edit.convertToProfile" | "edit.profileInfo" | "view.proofSetup" | "view.gamutWarning" => &["profile"],
        // New Document's Color Profile (#321) can name an .icc file.
        "file.new" => &["profile"],
        "edit.colorSettings" => &["workingRgb", "workingCmyk", "workingGray"],
        _ => &[],
    };
    keys.iter().any(|key| {
        params
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(|value| if matches!(*key, "file" | "mapPath" | "path") { !value.is_empty() } else { looks_like_path(value) })
    })
}

fn profile_command_may_read_ambient(id: &str, params: &Value) -> bool {
    // Installed profiles scan OS and user directories even without a request path.
    if matches!(id, "color.profileMismatch" | "edit.colorSettings" | "edit.installedProfiles" | "view.proofSetup") {
        return true;
    }
    // New Document skips exact `working` and `none`; the other working aliases resolve
    // the configured working space, which may be an .icc path.
    if id == "file.new" {
        return params.get("profile").and_then(Value::as_str).is_some_and(|profile| {
            matches!(profile, "default" | "working-cmyk" | "workingCmyk" | "working-rgb" | "workingRgb" | "working-gray" | "workingGray")
        });
    }
    matches!(id, "edit.assignProfile" | "edit.convertToProfile" | "edit.profileInfo")
        && params.get("profile").and_then(Value::as_str).is_some_and(|profile| {
            matches!(profile, "working" | "default" | "working-cmyk" | "workingCmyk" | "working-rgb" | "workingRgb" | "working-gray" | "workingGray")
        })
}

fn preferences_may_grant_ambient_paths(id: &str, params: &Value) -> bool {
    if id != "prefs.set" {
        return false;
    }
    let direct = params.get("path").and_then(Value::as_str).is_some_and(preference_uses_ambient_filesystem);
    let batch = params.get("values").and_then(Value::as_object).is_some_and(|values| values.keys().any(|path| preference_uses_ambient_filesystem(path)));
    direct || batch
}

fn preference_uses_ambient_filesystem(path: &str) -> bool {
    // The engine skips empty segments (`.scriptEvents.enabled` sets `scriptEvents.enabled`), so
    // judge the first non-empty one; no segment at all is the whole-preferences update.
    let mut segments = path.split('.').filter(|segment| !segment.is_empty());
    let Some(section) = segments.next() else { return true };
    // File Handling's LUT folder (#6558) is listed from disk; the rest of that section is not.
    matches!(section, "colorSettings" | "scriptEvents" | "historyLog" | "plugIns" | "scratchDisks")
        || (section == "fileHandling" && segments.next().is_none_or(|key| key == "lutFolder"))
}

fn command_uses_ambient_path(id: &str, params: &Value) -> bool {
    match id {
        "actions.load"
        | "brush.presets.importAbr"
        | "brush.presets.importImage"
        | "gradient.presets.importGrd"
        | "plugin.install"
        | "swatches.import"
        | "style.presets.importAsl"
        | "layer.layerStyle.contours.load" => {
            // `data` wins over `path` in these commands; any `path` without it reads the filesystem.
            params.get("data").is_none() && params.get("path").is_some()
        }
        // With a `path` the file is written there; without one the bytes come back as `data`.
        "actions.save" | "swatches.export" | "style.presets.exportAsl" | "layer.layerStyle.contours.save" => params.get("path").is_some(),
        // Shortcut sets: a .kys read or written by path, or a Summarize page written by path.
        "edit.keyboardShortcuts" => {
            params.get("path").is_some() || ["importKys", "exportKys", "summarize"].iter().any(|k| params.get(*k).and_then(|v| v.get("path")).is_some())
        }
        "plugin.reload" => true,
        _ => false,
    }
}

fn looks_like_path(value: &str) -> bool {
    value.contains('/')
        || value.contains('\\')
        || value.contains(':')
        || value.to_ascii_lowercase().ends_with(".icc")
        || value.to_ascii_lowercase().ends_with(".icm")
}
