use super::optimizer::{optimize, web_document, web_exif, web_warnings, web_xmp};
use super::settings::WebSettings;
use super::{EngineError, Result, Session, Value, bad, join, json, other, stem, write_file};

// ---------- Export Preferences / Quick Export ----------

/// File › Export › Export Preferences…: sets the given `export.*` preferences through `prefs.set`
/// (validated like any other preference) and returns the whole section.
pub(super) fn export_preferences(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.export.exportPreferences";
    let mut values = serde_json::Map::new();
    for (k, key) in [
        ("quickExportFormat", "export.quickExportFormat"),
        ("quickExportLocation", "export.quickExportLocation"),
        ("quickExportNaming", "export.quickExportNaming"),
        ("jpegQuality", "export.jpegQuality"),
        ("webpLossless", "export.webpLossless"),
        ("webpQuality", "export.webpQuality"),
        ("metadata", "export.metadata"),
        ("convertToSrgb", "export.convertToSrgb"),
    ] {
        if let Some(v) = p.get(k) {
            values.insert(key.into(), v.clone());
        }
    }
    if !values.is_empty() {
        s.execute("prefs.set", json!({"values": values})).map_err(|e| bad(cmd, e.to_string()))?;
    }
    Ok(json!({"section": "export", "values": s.prefs().get("export").unwrap_or(Value::Null)}))
}

/// The Quick Export format's extension (Export Preferences › Format).
fn quick_export_ext(s: &Session) -> String {
    serde_json::to_value(s.prefs().export.quick_export_format).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "png".into())
}

/// Quick Export's base destination without `path`: `<name>.<format>` beside the document's file
/// for Same Folder. Timestamp naming and collision handling are applied when the export writes.
pub fn quick_export_target(s: &Session) -> Option<String> {
    let same = s.prefs().export.quick_export_location == crate::prefs::ExportLocation::SameFolder;
    let dp = s.active()?.path.as_deref().filter(|_| same)?;
    let dir = std::path::Path::new(dp).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    Some(join(&dir, &format!("{}.{}", stem(dp), quick_export_ext(s))))
}

/// Whether the base [`quick_export_target`] names the document's own file. Default naming must
/// ask before replacing it (#6599); Timestamp naming publishes under a distinct, exclusive name.
pub fn quick_export_target_is_own_file(s: &Session) -> bool {
    let own = s.active().and_then(|d| d.path.as_deref());
    quick_export_target(s).zip(own).is_some_and(|(target, own)| crate::file_cmds::same_file(&target, own))
}

/// File › Export › Quick Export as <format>: the format, quality, metadata, colour space and
/// location from Export Preferences. `path` overrides the location ("ask" needs one).
pub(super) fn quick_export(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.export.quickExport";
    let mut prefs = s.prefs().export.clone();
    // A pending picker owns its naming policy even if Preferences changes before it answers.
    // Override only this command's copy; the user's newer preference must remain stored.
    if let Some(naming) = p.get("naming") {
        prefs.quick_export_naming =
            naming.as_str().and_then(crate::prefs::QuickExportNaming::parse).ok_or_else(|| bad(cmd, "`naming` must be `default` or `timestamp`"))?;
    }
    let prepared_path = match p.get("preparedPath") {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(_) => return Err(bad(cmd, "`preparedPath` must be a boolean")),
    };
    // A prepared picker name must never turn Default's replacing writer into an implicit override.
    if prepared_path
        && (prefs.quick_export_naming != crate::prefs::QuickExportNaming::Timestamp || p.get("path").and_then(Value::as_str).is_none_or(str::is_empty))
    {
        return Err(bad(cmd, "`preparedPath` requires Timestamp naming and an explicit `path`"));
    }
    let fmt = quick_export_ext(s);
    let path = match p.get("path").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        Some(v) => v.to_string(),
        None => {
            let Some(target) = quick_export_target(s) else {
                return Err(bad(cmd, "missing `path` (Export Preferences › Location is \"ask\", or the document was never saved)"));
            };
            // The document's own file is never replaced by an export the user didn't name (#6599).
            if prefs.quick_export_naming == crate::prefs::QuickExportNaming::Default && quick_export_target_is_own_file(s) {
                return Err(bad(cmd, format!("the export would replace the document's own file {target}; give a `path`")));
            }
            target
        }
    };
    // The name typed in the save dialog wins over Export Preferences' format: Quick Export as PNG
    // renamed to `photo.jpeg` must write a JPEG, not PNG bytes upload checks refuse (#8519).
    let fmt = match crate::file_cmds::extension(&path).as_deref() {
        None => fmt,
        Some("jpg" | "jpeg") => "jpg".into(),
        Some(e @ ("png" | "gif" | "webp" | "exr")) => e.into(),
        Some(e) => return Err(bad(cmd, format!("Quick Export writes PNG, JPEG, GIF, WebP or EXR, not .{e}; use Export As"))),
    };
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let doc = d.doc.clone();
    if fmt == "exr" {
        // EXR keeps floating-point scene values. The ordinary exporter converts to linear
        // light; the web/sRGB pipeline would instead clamp HDR and negative values.
        let opts = photocraft_io::ExportOptions { xmp: photocraft_io::XmpEmbed::None, ..Default::default() };
        let out = photocraft_io::export(&doc, "exr", &opts).map_err(other)?;
        let path = write_quick_export(&path, &out.bytes, prefs.quick_export_naming, prepared_path)?;
        crate::automate_cmds::fire_event(s, "export");
        return Ok(json!({"path": path, "format": fmt, "bytes": out.bytes.len(), "warnings": out.warnings}));
    }
    let metadata = serde_json::to_value(prefs.metadata).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    let mut o = serde_json::Map::new();
    o.insert(
        "metadata".into(),
        json!(if metadata == "all" {
            "all"
        } else if metadata == "none" {
            "none"
        } else {
            "copyright"
        }),
    );
    o.insert("convertToSrgb".into(), json!(prefs.convert_to_srgb));
    o.insert("transparency".into(), json!(true));
    o.insert("matte".into(), json!("#ffffff"));
    match fmt.as_str() {
        "jpg" => {
            o.insert("format".into(), json!("jpeg"));
            o.insert("quality".into(), json!(prefs.jpeg_quality.min(100)));
        }
        "gif" => {
            o.insert("format".into(), json!("gif"));
        }
        "webp" => {
            // WebP uses the regular exporter. Its optional quality is in the Photoshop-style
            // 0–12 save scale: invert that mapping so the codec receives the chosen 1–100.
            // Keeping quality absent preserves the historical lossless Quick Export behavior.
            let (wdoc, _, _) = web_document(&doc, &json!({}), &WebSettings { convert_to_srgb: prefs.convert_to_srgb, ..Default::default() })?;
            let quality = if prefs.webp_lossless { None } else { Some((f64::from(prefs.webp_quality.clamp(1, 100)) - 1.0) * 12.0 / 99.0) };
            let (bytes, warnings) = crate::file_cmds::encode(&wdoc, &path, quality)?;
            let path = write_quick_export(&path, &bytes, prefs.quick_export_naming, prepared_path)?;
            crate::automate_cmds::fire_event(s, "export");
            return Ok(json!({"path": path, "format": fmt, "warnings": warnings}));
        }
        _ => {
            o.insert("format".into(), json!("png24"));
        }
    }
    let q = Value::Object(o);
    let st = WebSettings::from_params(&q, cmd)?;
    let (wdoc, _, _) = web_document(&doc, &json!({}), &st)?;
    let buf = photocraft_compose::flatten(&wdoc);
    let xmp = web_xmp(&doc, st.metadata);
    let exif = web_exif(&doc, st.metadata);
    let opt = optimize(
        &buf.px,
        wdoc.size.width as usize,
        wdoc.bounds(),
        &st,
        wdoc.icc_profile.as_deref().map(|v| v.as_slice()),
        exif,
        xmp.as_deref(),
        wdoc.resolution_dpi,
        false,
    )?;
    let path = write_quick_export(&path, &opt.bytes, prefs.quick_export_naming, prepared_path)?;
    crate::automate_cmds::fire_event(s, "export");
    Ok(json!({"path": path, "format": fmt, "bytes": opt.bytes.len(), "warnings": web_warnings(&st, exif, wdoc.resolution_dpi)}))
}

/// Publish Timestamp names exclusively, preserving a picker's prepared name without restamping.
fn write_quick_export(path: &str, bytes: &[u8], naming: crate::prefs::QuickExportNaming, prepared_path: bool) -> Result<String> {
    if naming == crate::prefs::QuickExportNaming::Default {
        write_file(path, bytes)?;
        return Ok(path.into());
    }
    #[cfg(not(target_arch = "wasm32"))]
    let target = if prepared_path { std::path::PathBuf::from(path) } else { crate::quick_export_naming::timestamped_path(std::path::Path::new(path))? };
    #[cfg(not(target_arch = "wasm32"))]
    let target = photocraft_format::atomic::write_unique(&target, bytes).map_err(other)?;
    #[cfg(target_arch = "wasm32")]
    {
        let _ = prepared_path;
        return Err(other("Quick Export filesystem writing is unavailable on the web"));
    }
    #[cfg(not(target_arch = "wasm32"))]
    target.into_os_string().into_string().map_err(|_| other("Quick Export path is not valid Unicode"))
}
