//! File › Export › Export for Publication: one step writes the flattened image as TIFF and/or PNG
//! at a print resolution (300 dpi by default), optionally resampled to a journal column width,
//! with the resolution stored in each file so page-layout and submission systems size it right.
//! The document itself is not changed: the resample happens on a copy.
//!
//! TIFF is written LZW-compressed (lossless, accepted by journal submission systems) and without
//! layers; PNG is lossless. Column widths are the common journal sizes; `widthMm` sets any other.

use photocraft_algo::resample::Resample;
use photocraft_codecs::TiffCompression;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

const CMD: &str = "file.export.publication";

/// Journal figure widths in mm: single column, 1.5 column, double column (full page width).
const COLUMNS: [(&str, f64); 3] = [("single", 85.0), ("oneAndHalf", 120.0), ("double", 175.0)];

const MM_PER_INCH: f64 = 25.4;

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// The print width asked for, in mm (None: keep the pixels).
fn width_mm(p: &Value) -> Result<Option<f64>> {
    match p.get("width").and_then(Value::as_str).unwrap_or("keep") {
        "keep" => Ok(None),
        "custom" => {
            let mm = p.get("widthMm").and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad("`widthMm` is needed for a custom width"))?;
            if !(5.0..=2000.0).contains(&mm) {
                return Err(bad("`widthMm` must be between 5 and 2000 mm"));
            }
            Ok(Some(mm))
        }
        w => COLUMNS
            .iter()
            .find(|(name, _)| *name == w)
            .map(|(_, mm)| Some(*mm))
            .ok_or_else(|| bad(format!("unknown width `{w}` (keep|single|oneAndHalf|double|custom)"))),
    }
}

/// The formats asked for, in a fixed order, each once.
fn formats(p: &Value) -> Result<Vec<&'static str>> {
    let Some(v) = p.get("formats") else {
        // The dialog's two checkboxes; neither given means both.
        let on = |k: &str| p.get(k).map(|v| v.as_bool().ok_or_else(|| bad(format!("`{k}` must be true or false")))).transpose();
        return match (on("tiff")?, on("png")?) {
            (None, None) => Ok(vec!["tiff", "png"]),
            (t, g) => {
                let out: Vec<&'static str> = [(t, "tiff"), (g, "png")].into_iter().filter(|(on, _)| *on == Some(true)).map(|(_, f)| f).collect();
                if out.is_empty() { Err(bad("choose at least one format")) } else { Ok(out) }
            }
        };
    };
    let list = v.as_array().ok_or_else(|| bad("`formats` must be a list such as [\"tiff\", \"png\"]"))?;
    let mut out = Vec::new();
    for f in list {
        let f = match f.as_str() {
            Some("tiff" | "tif") => "tiff",
            Some("png") => "png",
            _ => return Err(bad(format!("unknown format {f} (tiff|png)"))),
        };
        if !out.contains(&f) {
            out.push(f);
        }
    }
    if out.is_empty() {
        return Err(bad("choose at least one format"));
    }
    Ok(out)
}

fn export_publication(s: &mut Session, p: &Value) -> Result<Value> {
    let dpi = match p.get("dpi") {
        None => 300.0,
        Some(v) => v.as_f64().filter(|d| d.is_finite() && (72.0..=2400.0).contains(d)).ok_or_else(|| bad("`dpi` must be between 72 and 2400"))?,
    };
    let mm = width_mm(p)?;
    let formats = formats(p)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let folder = match p.get("folder").and_then(Value::as_str).filter(|f| !f.is_empty()) {
        Some(f) => f.to_string(),
        None => d
            .path
            .as_deref()
            .and_then(|path| std::path::Path::new(path).parent())
            .map(|dir| dir.to_string_lossy().into_owned())
            .ok_or_else(|| bad("the document has not been saved yet: choose a `folder` for the files"))?,
    };
    let stem = match p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.to_string(),
        None => format!("{}_{}dpi", d.doc.name.rsplit_once('.').map_or(d.doc.name.as_str(), |(a, _)| a), dpi.round()),
    };
    if stem.contains(['/', '\\']) {
        return Err(bad("`name` is a file name, not a path"));
    }
    let mut doc = photocraft_doc::Document::clone(&d.doc);
    let mut warnings = Vec::new();
    if let Some(mm) = mm {
        let (ow, oh) = (doc.size.width, doc.size.height);
        let nw = (mm / MM_PER_INCH * dpi).round().clamp(1.0, 300_000.0) as u32;
        let nh = (f64::from(oh) * f64::from(nw) / f64::from(ow.max(1))).round().clamp(1.0, 300_000.0) as u32;
        if nw != ow {
            crate::image_cmds::check_resample_budget(CMD, &doc, nw, nh, "; choose a smaller width or resolution")?;
            let up = nw > ow;
            if f64::from(nw) > f64::from(ow) * 1.5 {
                warnings.push(format!("enlarged {:.1}× ({ow} → {nw} px): fine detail is interpolated, not added", f64::from(nw) / f64::from(ow)));
            }
            crate::image_cmds::resample_doc(&mut doc, nw, nh, if up { Resample::PreserveDetails } else { Resample::BicubicSharper });
        }
    }
    doc.resolution_dpi = dpi as f32;
    let (w, h) = (doc.size.width, doc.size.height);
    let mut files = Vec::new();
    for f in formats {
        let ext = if f == "tiff" { "tif" } else { "png" };
        let path = crate::file_cmds::join(&folder, &format!("{stem}.{ext}"));
        crate::allocation::checkpoint("exporting for publication")?;
        let mut opts = photocraft_io::ExportOptions::default();
        opts.encode.tiff_compression = TiffCompression::Lzw;
        let r = photocraft_io::export(&doc, &path, &opts).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
        crate::file_cmds::write_file(&path, &r.bytes)?;
        warnings.extend(r.warnings);
        files.push(json!({"path": path, "format": f, "bytes": r.bytes.len()}));
    }
    crate::automate_cmds::fire_event(s, "export");
    // The dialog opens with these choices next time (not the folder or name).
    let saved: serde_json::Map<String, Value> =
        ["tiff", "png", "dpi", "width", "widthMm"].iter().filter_map(|k| p.get(*k).filter(|v| !v.is_null()).map(|v| ((*k).to_string(), v.clone()))).collect();
    if !saved.is_empty() {
        s.edit_prefs(|pr| {
            pr.dialogs.insert(CMD.to_string(), Value::Object(saved));
        });
    }
    let size_mm = |px: u32| (f64::from(px) / dpi * MM_PER_INCH * 10.0).round() / 10.0;
    Ok(json!({"files": files, "width": w, "height": h, "dpi": dpi, "widthMm": size_mm(w), "heightMm": size_mm(h), "warnings": warnings}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Export for Publication…",
        menu: &["File", "Export"],
        shortcut: None,
        params: r##"{"formats":["tiff","png"]=both (or "tiff":bool,"png":bool),"dpi":72..2400=300,"width":"keep|single|oneAndHalf|double|custom"="keep" (85, 120, 175 mm),"widthMm":mm (custom),"folder":dir (default: the document's folder),"name":file name without extension (default: "<document>_<dpi>dpi")} → {files:[{path,format,bytes}],width,height,dpi,widthMm,heightMm,warnings}. Flattened copy, LZW TIFF and PNG, resolution stored in the files; the document is unchanged; existing files of that name are replaced"##,
        enabled: has_doc,
        run: export_publication,
        journal: true,
    }]
}

#[cfg(test)]
mod tests;
