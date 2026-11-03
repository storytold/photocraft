use super::paths::{join, sanitize, stem, write_file};
use super::save::{SaveOpts, encode};
use super::{
    ColorMode, Document, EngineError, Layer, LayerContent, LayerId, Rect, Result, Session, Surface, Value, flattened_checked, from_rgba_into, json, str_param,
};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame as GifFrame};

// ---------- Export ----------

const SPRITE_SHEET_CMD: &str = "file.export.spriteSheet";
const MAX_SPRITE_SHEET_DIMENSION: u32 = 16_384;
const MAX_SPRITE_FRAME_COUNT: usize = 4_096;
const MAX_SPRITE_FRAME_DURATION_TICKS: u32 = 60;

#[derive(Clone, Copy)]
struct SpriteTimelineFrame {
    frame: usize,
    duration_ticks: u32,
}

struct SpriteSheetMetadata {
    sheet_width: u32,
    sheet_height: u32,
    frame_width: u32,
    frame_height: u32,
    padding: u32,
    columns: usize,
    fps: u32,
}

struct SpriteGifConfig {
    frame_width: u32,
    frame_height: u32,
    padding: u32,
    columns: usize,
    fps: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpriteSource {
    Layers,
    Grid,
}

/// Parses the requested sprite source and rejects unsupported source names.
fn sprite_source(p: &Value) -> Result<SpriteSource> {
    match p.get("source").and_then(Value::as_str).unwrap_or("layers") {
        "layers" => Ok(SpriteSource::Layers),
        "grid" => Ok(SpriteSource::Grid),
        other => Err(EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: format!("`source` must be \"layers\" or \"grid\", got {other}") }),
    }
}

/// Reads a positive bounded sprite-sheet dimension parameter.
fn positive_u32(p: &Value, key: &str, default: u32) -> Result<u32> {
    let value = p.get(key).and_then(Value::as_u64).unwrap_or(u64::from(default));
    u32::try_from(value).ok().filter(|v| *v > 0 && *v <= MAX_SPRITE_SHEET_DIMENSION).ok_or_else(|| EngineError::BadParams {
        cmd: SPRITE_SHEET_CMD.into(),
        msg: format!("`{key}` must be an integer from 1 to {MAX_SPRITE_SHEET_DIMENSION}"),
    })
}

/// Reads a non-negative bounded sprite-sheet spacing parameter.
fn bounded_u32(p: &Value, key: &str, default: u32) -> Result<u32> {
    let value = p.get(key).and_then(Value::as_u64).unwrap_or(u64::from(default));
    u32::try_from(value).ok().filter(|v| *v <= MAX_SPRITE_SHEET_DIMENSION).ok_or_else(|| EngineError::BadParams {
        cmd: SPRITE_SHEET_CMD.into(),
        msg: format!("`{key}` must be an integer from 0 to {MAX_SPRITE_SHEET_DIMENSION}"),
    })
}

/// Parses and validates the ordered timeline, defaulting to every source frame once.
fn sprite_timeline(p: &Value, frame_count: usize) -> Result<Vec<SpriteTimelineFrame>> {
    let Some(values) = p.get("selectedFrames").and_then(Value::as_array) else {
        return Ok((0..frame_count).map(|frame| SpriteTimelineFrame { frame, duration_ticks: 1 }).collect());
    };
    let mut timeline = Vec::with_capacity(values.len());
    for value in values {
        let (frame, duration_ticks) = if let Some(frame) = value.as_u64() {
            (usize::try_from(frame).map_err(|_| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "timeline frame index is too large".into() })?, 1)
        } else {
            let object = value
                .as_object()
                .ok_or_else(|| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "timeline frames must be integers or objects".into() })?;
            let frame = object
                .get("frame")
                .and_then(Value::as_u64)
                .ok_or_else(|| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "timeline frame is missing `frame`".into() })?;
            let frame =
                usize::try_from(frame).map_err(|_| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "timeline frame index is too large".into() })?;
            let duration_ticks = object.get("durationTicks").and_then(Value::as_u64).unwrap_or(1);
            let duration_ticks =
                u32::try_from(duration_ticks).ok().filter(|v| (1..=MAX_SPRITE_FRAME_DURATION_TICKS).contains(v)).ok_or_else(|| EngineError::BadParams {
                    cmd: SPRITE_SHEET_CMD.into(),
                    msg: format!("timeline `durationTicks` must be an integer from 1 to {MAX_SPRITE_FRAME_DURATION_TICKS}"),
                })?;
            (frame, duration_ticks)
        };
        if frame >= frame_count {
            return Err(EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: format!("timeline frame {frame} is out of range") });
        }
        timeline.push(SpriteTimelineFrame { frame, duration_ticks });
    }
    if timeline.is_empty() {
        return Ok((0..frame_count).map(|frame| SpriteTimelineFrame { frame, duration_ticks: 1 }).collect());
    }
    Ok(timeline)
}

/// Returns the metadata path associated with a normalized PNG path.
fn sprite_sheet_json_path(path: &str) -> String {
    Path::new(path).with_extension("json").to_string_lossy().into_owned()
}

/// Extracts the portable image filename stored in metadata.
fn sprite_sheet_image_name(path: &str) -> String {
    Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or("sprite-sheet.png").to_owned()
}

/// Writes Aseprite-compatible frame rectangles and durations beside the PNG.
fn write_sprite_sheet_metadata(path: &str, image_path: &str, config: &SpriteSheetMetadata, timeline: &[SpriteTimelineFrame]) -> Result<()> {
    let base_tick_ms = 1000u32 / config.fps.max(1);
    let frames = timeline
        .iter()
        .enumerate()
        .map(|(step, item)| {
            let col = item.frame % config.columns;
            let row = item.frame / config.columns;
            let x = config.padding + u32::try_from(col).unwrap_or(u32::MAX).saturating_mul(config.frame_width.saturating_add(config.padding));
            let y = config.padding + u32::try_from(row).unwrap_or(u32::MAX).saturating_mul(config.frame_height.saturating_add(config.padding));
            let rect = json!({"x": x, "y": y, "w": config.frame_width, "h": config.frame_height});
            (
                format!("frame_{step}"),
                json!({
                    "frame": rect,
                    "rotated": false,
                    "trimmed": false,
                    "spriteSourceSize": rect,
                    "sourceSize": {"w": config.frame_width, "h": config.frame_height},
                    "duration": item.duration_ticks.saturating_mul(base_tick_ms).max(1),
                }),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let payload = json!({
        "frames": frames,
        "meta": {
            "app": "PhotoCraft",
            "version": env!("CARGO_PKG_VERSION"),
            "image": sprite_sheet_image_name(image_path),
            "format": "RGBA8888",
            "size": {"w": config.sheet_width, "h": config.sheet_height},
            "scale": "1",
            "frameTags": [{
                "name": "default",
                "from": 0,
                "to": timeline.len().saturating_sub(1),
                "direction": "forward"
            }]
        }
    });
    let bytes = serde_json::to_vec_pretty(&payload).map_err(|e| EngineError::Other(format!("cannot encode sprite sheet metadata: {e}")))?;
    write_file(&sprite_sheet_json_path(path), &bytes)?;
    Ok(())
}

/// Encodes the ordered timeline as a looping GIF and flushes it to disk.
fn encode_sprite_sheet_gif(output_path: &Path, sheet_png: &[u8], config: &SpriteGifConfig, timeline: &[SpriteTimelineFrame]) -> Result<()> {
    if timeline.is_empty() {
        return Err(EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "cannot export GIF with an empty timeline".into() });
    }
    let sheet = image::load_from_memory_with_format(sheet_png, image::ImageFormat::Png)
        .map_err(|e| EngineError::Other(format!("cannot decode sprite sheet for GIF export: {e}")))?
        .into_rgba8();
    let file = File::create(output_path).map_err(|e| EngineError::Other(format!("cannot create GIF output: {e}")))?;
    let mut writer = BufWriter::new(file);
    {
        let mut encoder = GifEncoder::new(&mut writer);
        encoder.set_repeat(Repeat::Infinite).map_err(|e| EngineError::Other(format!("cannot configure GIF repeat: {e}")))?;
        let base_tick_ms = 1000u32 / config.fps.max(1);
        for item in timeline {
            let col = item.frame % config.columns;
            let row = item.frame / config.columns;
            let x = config
                .padding
                .checked_add(
                    u32::try_from(col)
                        .ok()
                        .and_then(|col| col.checked_mul(config.frame_width.saturating_add(config.padding)))
                        .ok_or_else(|| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "GIF frame x coordinate is too large".into() })?,
                )
                .ok_or_else(|| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "GIF frame x coordinate is too large".into() })?;
            let y = config
                .padding
                .checked_add(
                    u32::try_from(row)
                        .ok()
                        .and_then(|row| row.checked_mul(config.frame_height.saturating_add(config.padding)))
                        .ok_or_else(|| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "GIF frame y coordinate is too large".into() })?,
                )
                .ok_or_else(|| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "GIF frame y coordinate is too large".into() })?;
            let cropped = image::imageops::crop_imm(&sheet, x, y, config.frame_width, config.frame_height).to_image();
            let delay = Delay::from_numer_denom_ms(item.duration_ticks.saturating_mul(base_tick_ms).max(20), 1);
            encoder.encode_frame(GifFrame::from_parts(cropped, 0, 0, delay)).map_err(|e| EngineError::Other(format!("cannot encode GIF frame: {e}")))?;
        }
    }
    writer.flush().map_err(|e| EngineError::Other(format!("cannot flush GIF output: {e}")))?;
    Ok(())
}

/// Computes checked packed-sheet dimensions and enforces texture and frame limits.
fn sprite_sheet_dimensions(count: usize, columns: usize, frame_w: u32, frame_h: u32, padding: u32) -> Result<(u32, u32)> {
    if count == 0 || count > MAX_SPRITE_FRAME_COUNT {
        return Err(EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: format!("sprite sheet must contain 1 to {MAX_SPRITE_FRAME_COUNT} frames") });
    }
    let rows = count
        .checked_add(columns - 1)
        .and_then(|n| n.checked_div(columns))
        .ok_or_else(|| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "too many frames".into() })?;
    let cols = u32::try_from(columns).map_err(|_| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "too many columns".into() })?;
    let rows = u32::try_from(rows).map_err(|_| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "too many rows".into() })?;
    let width = cols
        .checked_mul(frame_w)
        .and_then(|v| v.checked_add((cols + 1).checked_mul(padding)?))
        .ok_or_else(|| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "sprite sheet width is too large".into() })?;
    let height = rows
        .checked_mul(frame_h)
        .and_then(|v| v.checked_add((rows + 1).checked_mul(padding)?))
        .ok_or_else(|| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "sprite sheet height is too large".into() })?;
    if width == 0 || height == 0 || width > MAX_SPRITE_SHEET_DIMENSION || height > MAX_SPRITE_SHEET_DIMENSION {
        return Err(EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "sprite sheet dimensions are too large".into() });
    }

    Ok((width, height))
}

/// Exports source frames to PNG, Aseprite JSON, and optionally a looping GIF.
pub fn export_sprite_sheet(s: &mut Session, p: &Value) -> Result<Value> {
    let requested_path = str_param(p, "path", SPRITE_SHEET_CMD)?;
    let requested = Path::new(requested_path);
    let png_path = if requested.extension().and_then(|ext| ext.to_str()) == Some("png") { requested.to_path_buf() } else { requested.with_extension("png") };
    let path = png_path.to_string_lossy().into_owned();
    let json_path = png_path.with_extension("json");
    if png_path == json_path {
        return Err(EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "PNG and JSON metadata destinations resolved to identical paths".into() });
    }
    let source = sprite_source(p)?;
    let frame_w = positive_u32(p, "frameWidth", 32)?;
    let frame_h = positive_u32(p, "frameHeight", 32)?;
    let padding = bounded_u32(p, "padding", 0)?;
    let columns = positive_u32(p, "columns", 8)? as usize;
    let fps = p.get("fps").and_then(Value::as_u64).unwrap_or(8);
    let fps = u32::try_from(fps)
        .ok()
        .filter(|v| (1..=240).contains(v))
        .ok_or_else(|| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "`fps` must be an integer from 1 to 240".into() })?;
    let _authorization = crate::jobs::render_authorization_scope(s.authorize);
    let active = s.active().ok_or(EngineError::NoDocument)?;
    let doc = active.doc.clone();
    let fmt = photocraft_color::PixelFormat::RGBA32F;
    let mut frames = Vec::new();
    match source {
        SpriteSource::Layers => {
            for layer in doc.layers.iter().filter(|layer| layer.visible) {
                if matches!(layer.content, LayerContent::Adjustment(_)) {
                    continue;
                }
                let mut one = (*doc).clone();
                one.layers = vec![layer.clone()];
                frames.push(flattened_checked(&one, fmt, SPRITE_SHEET_CMD)?);
                if frames.len() > MAX_SPRITE_FRAME_COUNT {
                    return Err(EngineError::BadParams {
                        cmd: SPRITE_SHEET_CMD.into(),
                        msg: format!("sprite sheet exceeds safety limit of {MAX_SPRITE_FRAME_COUNT} frames"),
                    });
                }
            }
        }
        SpriteSource::Grid => {
            let bounds = doc.bounds();
            let frame_w_i =
                i32::try_from(frame_w).map_err(|_| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "frame width is too large".into() })?;
            let frame_h_i =
                i32::try_from(frame_h).map_err(|_| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "frame height is too large".into() })?;
            let cols = (bounds.width().saturating_add(frame_w - 1) / frame_w).max(1) as usize;
            let rows = (bounds.height().saturating_add(frame_h - 1) / frame_h).max(1) as usize;
            let count =
                cols.checked_mul(rows).ok_or_else(|| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "grid frame count is too large".into() })?;
            if count > MAX_SPRITE_FRAME_COUNT {
                return Err(EngineError::BadParams {
                    cmd: SPRITE_SHEET_CMD.into(),
                    msg: format!("grid settings produce {count} frames, exceeding safety limit of {MAX_SPRITE_FRAME_COUNT}"),
                });
            }
            let canvas = flattened_checked(&doc, fmt, SPRITE_SHEET_CMD)?;
            for row in 0..rows {
                for col in 0..cols {
                    let x = bounds.x0 + (col as i32) * frame_w_i;
                    let y = bounds.y0 + (row as i32) * frame_h_i;
                    let mut frame = Surface::new(fmt);
                    let area = Rect::new(x, y, x + frame_w_i, y + frame_h_i);
                    frame.write_region(Rect::new(0, 0, frame_w_i, frame_h_i), &canvas.read_region(area));
                    frames.push(frame);
                }
            }
        }
    }
    if frames.is_empty() {
        return Err(EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "no frames are available".into() });
    }
    let timeline = sprite_timeline(p, frames.len())?;
    let (sheet_w, sheet_h) = sprite_sheet_dimensions(frames.len(), columns, frame_w, frame_h, padding)?;
    let mut sheet = Surface::new(fmt);
    for (index, frame) in frames.iter().enumerate() {
        let col = index % columns;
        let row = index / columns;
        let col = u32::try_from(col).map_err(|_| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "too many columns".into() })?;
        let row = u32::try_from(row).map_err(|_| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "too many rows".into() })?;
        let x = padding
            .checked_add(
                col.checked_mul(
                    frame_w
                        .checked_add(padding)
                        .ok_or_else(|| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "sprite sheet spacing is too large".into() })?,
                )
                .ok_or_else(|| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "sprite sheet x coordinate is too large".into() })?,
            )
            .ok_or_else(|| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "sprite sheet x coordinate is too large".into() })?;
        let y = padding
            .checked_add(
                row.checked_mul(
                    frame_h
                        .checked_add(padding)
                        .ok_or_else(|| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "sprite sheet spacing is too large".into() })?,
                )
                .ok_or_else(|| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "sprite sheet y coordinate is too large".into() })?,
            )
            .ok_or_else(|| EngineError::BadParams { cmd: SPRITE_SHEET_CMD.into(), msg: "sprite sheet y coordinate is too large".into() })?;
        let data = frame.read_region(Rect::new(0, 0, frame_w as i32, frame_h as i32));
        sheet.write_region(Rect::new(x as i32, y as i32, (x + frame_w) as i32, (y + frame_h) as i32), &data);
    }
    let mut out = Document::new("Sprite Sheet", photocraft_doc::Size::new(sheet_w, sheet_h), ColorMode::Rgb, photocraft_color::SampleType::F32);
    let mut layer = Layer::raster("Sprite Sheet", fmt);
    if let Some(target) = layer.surface_mut() {
        *target = sheet;
    }
    out.layers = vec![layer];
    let result = photocraft_io::export(&out, "sprite-sheet.png", &photocraft_io::ExportOptions::default())
        .map_err(|e| EngineError::Other(format!("cannot encode sprite sheet: {e}")))?;
    write_file(&path, &result.bytes)?;
    let metadata = SpriteSheetMetadata { sheet_width: sheet_w, sheet_height: sheet_h, frame_width: frame_w, frame_height: frame_h, padding, columns, fps };
    write_sprite_sheet_metadata(&path, &path, &metadata, &timeline)?;
    let export_gif = p.get("exportGif").and_then(Value::as_bool).unwrap_or(false);
    let gif_path = png_path.with_extension("gif").to_string_lossy().into_owned();
    if export_gif {
        let gif_config = SpriteGifConfig { frame_width: frame_w, frame_height: frame_h, padding, columns, fps };
        encode_sprite_sheet_gif(Path::new(&gif_path), &result.bytes, &gif_config, &timeline)?;
    }
    Ok(
        json!({"path": path, "metadataPath": json_path.to_string_lossy(), "gifPath": export_gif.then_some(gif_path), "width": sheet_w, "height": sheet_h, "frames": frames.len(), "timelineFrames": timeline.len()}),
    )
}

pub(super) fn layers_to_files(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.export.layersToFiles";
    let dir = str_param(p, "dir", cmd).or_else(|_| str_param(p, "output", cmd))?.to_string();
    let report = export_layers_report_with(s, p, |name, bytes| write_file(&join(&dir, name), bytes))?;
    let files: Vec<String> = report.files.iter().map(|name| join(&dir, name)).collect();
    Ok(json!({"files": files, "skipped": report.skipped}))
}

/// The bodies of `file.export.layersToFiles`, streamed to `write` as a single-component filename
/// plus its bytes: one file per exported layer (top to bottom, like Photoshop's numbering),
/// encoded but never touched as a path. Automation writes exactly these bytes beneath its write
/// root (#3216), so the two paths cannot drift. Returns the filenames written, in order.
pub fn export_layers_with(s: &Session, p: &Value, write: impl FnMut(&str, &[u8]) -> Result<()>) -> Result<Vec<String>> {
    export_layers_report_with(s, p, write).map(|report| report.files)
}

/// A top-level layer omitted from a successful Layers to Files export.
#[derive(Debug, serde::Serialize)]
pub struct SkippedExportLayer {
    /// Session layer id (names need not be unique).
    pub layer: u64,
    /// Original name, not the sanitized filename component.
    pub name: String,
    /// Why the layer produced no file.
    pub reason: &'static str,
}

/// Filenames written and layers skipped, each in top-to-bottom order.
#[derive(Debug, Default)]
pub struct LayerExportReport {
    pub files: Vec<String>,
    pub skipped: Vec<SkippedExportLayer>,
}

/// The same export as [`export_layers_with`], with an explanation for every omitted top-level
/// layer. Groups export merged, so their children are not separate exports or skips. Encoding
/// and write failures remain errors, never successful skips. Existing numbering is preserved.
pub fn export_layers_report_with(s: &Session, p: &Value, mut write: impl FnMut(&str, &[u8]) -> Result<()>) -> Result<LayerExportReport> {
    let cmd = "file.export.layersToFiles";
    let format = p.get("format").and_then(Value::as_str).unwrap_or("png").trim_start_matches('.').to_ascii_lowercase();
    let trim = p.get("trim").and_then(Value::as_bool).unwrap_or(false);
    let visible_only = p.get("visibleOnly").and_then(Value::as_bool).unwrap_or(true);
    // Callers outside `Session::execute` (headless workspace exports) carry the session's gate
    // into `encode`, so a PSD embeds no file a linked smart object names.
    let _authorization = crate::jobs::render_authorization_scope(s.authorize);
    let save = SaveOpts::from_params(s, p, cmd)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let prefix = p.get("prefix").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| stem(&d.doc.name));
    let doc = d.doc.clone();
    let mut report = LayerExportReport::default();
    // Hidden layers never consumed a filename index; adjustments that pass the visibility
    // filter did. Keep those filenames stable while reporting both kinds of omission.
    let mut i = 0usize;
    for l in doc.layers.iter().rev() {
        if !l.visible && visible_only {
            report.skipped.push(SkippedExportLayer { layer: l.id.0, name: l.name.clone(), reason: "hidden by visibleOnly" });
            continue;
        }
        let index = i;
        i += 1;
        if matches!(l.content, LayerContent::Adjustment(_)) {
            report.skipped.push(SkippedExportLayer { layer: l.id.0, name: l.name.clone(), reason: "adjustment layer has no pixels of its own" });
            continue;
        }
        let mut one = (*doc).clone();
        let mut only = l.clone();
        only.visible = true;
        only.clipped = false;
        one.layers = vec![only];
        if trim {
            let mut bounds = Rect::EMPTY;
            let _ = photocraft_compose::render_bands(&one, one.bounds(), 0, |band| -> std::result::Result<(), ()> {
                let width = band.rect.width() as usize;
                for (i, rgba) in band.px.iter().enumerate() {
                    if rgba[3] > 0.0 && width > 0 {
                        let x = band.rect.x0 + (i % width) as i32;
                        let y = band.rect.y0 + (i / width) as i32;
                        bounds = bounds.union(&Rect::new(x, y, x + 1, y + 1));
                    }
                }
                Ok(())
            });
            if !bounds.is_empty() && bounds != one.bounds() {
                crate::image_cmds::crop_doc(&mut one, cmd, bounds, true, [0.0; 4])?;
            }
        }

        let name = format!("{}_{:04}_{}.{format}", sanitize(&prefix), index, sanitize(&l.name));
        let (bytes, _warnings) = encode(&one, &name, save)?;
        write(&name, &bytes)?;
        report.files.push(name);
    }
    Ok(report)
}

/// Adjustment layers that can be represented as an RGB-only LUT, in document stack order.
/// A LUT has no spatial coordinates, so masks and clipping geometry are deliberately stripped
/// at bake time. Group adjustments need their enclosing group's compositing semantics and are
/// not silently flattened as independent layers.
fn lut_layers<'a>(doc: &'a Document, ids: Option<&[LayerId]>) -> Result<Vec<&'a Layer>> {
    const CMD: &str = "file.export.colorLookupTables";
    let mut selected = Vec::new();
    if let Some(ids) = ids {
        if ids.is_empty() {
            return Err(EngineError::BadParams { cmd: CMD.into(), msg: "select at least one adjustment layer".into() });
        }
        for id in ids {
            if selected.contains(id) {
                return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("duplicate layer id {}", id.0) });
            }
            let Some(layer) = doc.layer(*id) else { return Err(EngineError::NoLayer(*id)) };
            if !matches!(layer.content, LayerContent::Adjustment(_)) {
                return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("layer {} is not an adjustment layer", id.0) });
            }
            if !layer.visible {
                return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("layer {} is hidden", id.0) });
            }
            if !doc.layers.iter().any(|root| root.id == *id) {
                return Err(EngineError::BadParams {
                    cmd: CMD.into(),
                    msg: format!("layer {} is nested in a group; export top-level adjustment layers", id.0),
                });
            }
            selected.push(*id);
        }
    }
    let mut layers = Vec::new();
    for l in &doc.layers {
        if l.visible && matches!(l.content, LayerContent::Adjustment(_)) && (ids.is_none() || selected.contains(&l.id)) {
            layers.push(l);
        }
    }
    if layers.is_empty() {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: "no visible adjustment layers to export".into() });
    }
    Ok(layers)
}

/// Bake the chosen adjustment stack against an identity RGB lattice.
/// Source order always follows the document stack, even if an explicit list is reversed.
fn bake_cube_layers(size: usize, title: &str, layers: &[&Layer]) -> String {
    let n = size;
    let (w, h) = ((n * n) as u32, n as u32);
    let mut lattice = Document::new("lut", photocraft_doc::Size::new(w, h), ColorMode::Rgb, photocraft_color::SampleType::F32);
    let fmt = lattice.pixel_format();
    let step = 1.0 / (n - 1) as f32;
    let mut data = Vec::with_capacity((w * h) as usize * fmt.channels());
    for g in 0..n {
        for b in 0..n {
            for r in 0..n {
                let mut px = [0.0f32; 8];
                let k = from_rgba_into(&fmt, [r as f32 * step, g as f32 * step, b as f32 * step, 1.0], &mut px);
                data.extend_from_slice(&px[..k]);
            }
        }
    }
    let mut surf = Surface::new(fmt);
    surf.write_region(Rect::new(0, 0, w as i32, h as i32), &data);
    lattice.layers.push(Layer::new("Lattice", LayerContent::Raster(surf)));
    for l in layers {
        let mut a = (*l).clone();
        // Masks and clipping are spatial; a LUT is the adjustment stack's colour mapping.
        a.mask = None;
        a.vector_mask = None;
        a.clipped = false;
        lattice.layers.push(a);
    }
    let out = photocraft_compose::flatten(&lattice);
    let mut s = format!("TITLE \"{}\"\n# Created by PhotoCraft\nLUT_3D_SIZE {n}\nDOMAIN_MIN 0.0 0.0 0.0\nDOMAIN_MAX 1.0 1.0 1.0\n", title.replace('"', "'"));
    // .cube order: red changes fastest, then green, then blue.
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                let (x, y) = (b * n + r, g);
                let px = out.px[y * w as usize + x];
                s.push_str(&format!("{:.6} {:.6} {:.6}\n", px[0].clamp(0.0, 1.0), px[1].clamp(0.0, 1.0), px[2].clamp(0.0, 1.0)));
            }
        }
    }
    s
}

/// Backwards-compatible helper for exporting all visible top-level adjustments.
pub fn bake_cube(doc: &Document, size: usize, title: &str) -> String {
    let layers: Vec<&Layer> = doc.layers.iter().filter(|l| l.visible && matches!(l.content, LayerContent::Adjustment(_))).collect();
    bake_cube_layers(size.clamp(2, photocraft_cms::lutfile::MAX_SIZE), title, &layers)
}

/// Export either the whole visible stack (backward-compatible default), the Layers panel's
/// current selection, or an explicit list supplied by scripts/CLI/automation.
pub(super) fn color_lookup_tables(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "file.export.colorLookupTables";
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let size = match p.get("size") {
        None | Some(Value::Null) => 33,
        Some(v) => v.as_u64().ok_or_else(|| EngineError::BadParams { cmd: CMD.into(), msg: "size must be an integer".into() })?,
    };
    if !(2..=photocraft_cms::lutfile::MAX_SIZE as u64).contains(&size) {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: format!("size must be 2..={}", photocraft_cms::lutfile::MAX_SIZE) });
    }
    let scope = match p.get("scope") {
        None | Some(Value::Null) => "all",
        Some(Value::String(s)) => s.as_str(),
        Some(_) => return Err(EngineError::BadParams { cmd: CMD.into(), msg: "scope must be all or selected".into() }),
    };
    if !matches!(scope, "all" | "selected") {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: "scope must be all or selected".into() });
    }
    if p.get("layers").is_some() && scope == "selected" {
        return Err(EngineError::BadParams { cmd: CMD.into(), msg: "use either layers or scope=selected, not both".into() });
    }
    let explicit: Option<Vec<LayerId>> = if let Some(v) = p.get("layers") {
        let Some(a) = v.as_array() else { return Err(EngineError::BadParams { cmd: CMD.into(), msg: "layers must be an array of layer IDs".into() }) };
        let mut out = Vec::with_capacity(a.len());
        for id in a {
            let Some(id) = id.as_u64() else { return Err(EngineError::BadParams { cmd: CMD.into(), msg: "layer IDs must be unsigned integers".into() }) };
            out.push(LayerId(id));
        }
        Some(out)
    } else if scope == "selected" {
        Some(d.selected_layers())
    } else {
        None
    };
    let layers = lut_layers(&d.doc, explicit.as_deref())?;
    let count = layers.len();
    let title = p.get("title").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| stem(&d.doc.name));
    let cube = bake_cube_layers(size as usize, &title, &layers);
    match p.get("path").and_then(Value::as_str) {
        Some(path) => {
            write_file(path, cube.as_bytes())?;
            Ok(json!({"path": path, "size": size, "layerCount": count}))
        }
        None => Ok(json!({"cube": cube, "size": size, "layerCount": count})),
    }
}

// ---------- guides ----------

pub(super) fn push_unique(v: &mut Vec<f64>, x: f64) {
    if !v.iter().any(|g| (g - x).abs() < 0.01) {
        v.push(x);
    }
}
