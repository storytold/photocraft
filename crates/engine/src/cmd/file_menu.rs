use super::{Color, ColorMode, CommandSpec, Document, EngineError, Layer, SampleType, Size, Value, always, has_doc, json, parse_hex};

/// File menu commands: New and Close.
pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "file.new",
            "New…",
            ["File"],
            Some("Cmd+N"),
            r##"{"width":1..300000=1920,"height":1..300000=1080 (pixel counts are rounded),"mode":"rgb|gray|grayscale|cmyk|lab"="rgb","depth":8|16|32=8,"background":"white|black|backgroundColor|transparent|#rrggbb"="white","backgroundColor":[r,g,b]? (0..1; defaults to toolbox colour),"resolution":1..30000=72 (ppi),"name":str,"artboard":bool=false (one artboard over the canvas, its background the chosen one, holding an empty Layer 1; no Background layer),"profile":"working"|built-in id|path to an .icc file="working" (the colour profile it is tagged with; it must suit the mode)}"##,
            always,
            |s, p| {
                use crate::document_preset_cmds::{MAX_DIMENSION, MAX_RESOLUTION, background_color, color_mode, sample_type};
                crate::document_preset_cmds::validate_new_params(s, p)?;
                // A size given as a float (`512.0`, as JSON from a UI field) is still that size (#254).
                let px = |k: &str, d: u32| match p.get(k) {
                    Some(v) => v
                        .as_u64()
                        .map(|n| n.clamp(1, MAX_DIMENSION as u64) as u32)
                        .or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f.round().clamp(1.0, MAX_DIMENSION as f64) as u32))
                        .unwrap_or(d),
                    None => d,
                };
                let (w, h) = (px("width", 1920), px("height", 1080));
                let mode = color_mode(p.get("mode").and_then(Value::as_str).unwrap_or("rgb")).unwrap_or(ColorMode::Rgb);
                let depth = sample_type(p.get("depth").and_then(Value::as_u64).unwrap_or(8)).unwrap_or(SampleType::U8);
                let name = p.get("name").and_then(Value::as_str).unwrap_or("Untitled").to_string();
                let res = p.get("resolution").and_then(Value::as_f64).unwrap_or(72.0).clamp(1.0, MAX_RESOLUTION) as f32;
                // The background colour; `None` for Transparent.
                let background = match p.get("background").and_then(Value::as_str).unwrap_or("white") {
                    "backgroundColor" => {
                        let bgc = background_color(p, s.tools.background, "file.new")?;
                        Some(Color::rgba(bgc[0], bgc[1], bgc[2], 1.0))
                    }
                    "transparent" => None,
                    "black" => Some(Color::BLACK),
                    other => {
                        let c = parse_hex(other).unwrap_or([1.0; 4]);
                        Some(Color::rgba(c[0], c[1], c[2], c[3]))
                    }
                };
                let mut doc = if p.get("artboard").and_then(Value::as_bool).unwrap_or(false) {
                    crate::artboard_cmds::new_artboard_document(name, Size::new(w, h), mode, depth, background)
                } else if let Some(c) = background {
                    Document::with_background(name, Size::new(w, h), mode, depth, c)
                } else {
                    let mut d = Document::new(name, Size::new(w, h), mode, depth);
                    d.layers.push(Layer::raster("Layer 1", d.pixel_format()));
                    d
                };
                doc.resolution_dpi = res;
                // Photoshop's New Document › Color Profile: tag the document with another profile
                // than the working space (#321). Checked before the document opens.
                if let Some(spec) = p.get("profile").and_then(Value::as_str).filter(|spec| !matches!(*spec, "working" | "none")) {
                    let prof = s.color.resolve(spec, Some(&doc), None)?;
                    if prof.color_space != crate::color_cmds::mode_space(doc.mode) {
                        return Err(EngineError::BadParams {
                            cmd: "file.new".into(),
                            msg: format!("profile `{}` is {:?}; the document is {:?}", prof.description, prof.color_space, doc.mode),
                        });
                    }
                    doc.icc_profile = Some(prof.to_bytes());
                }
                let i = s.add_document(doc, None);
                Ok(json!({ "document": i }))
            }
        ),
        cmd!("file.close", "Close", ["File"], Some("Cmd+W"), r##"{"document":index?}"##, has_doc, |s, p| {
            let i = p.get("document").and_then(Value::as_u64).map(|v| v as usize).or(s.active_index()).ok_or(EngineError::NoDocument)?;
            s.close(i).ok_or(EngineError::NoDocument)?;
            Ok(Value::Null)
        }),
    ]
}
