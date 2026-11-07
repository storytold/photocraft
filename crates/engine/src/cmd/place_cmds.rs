//! Place a decoded image file into the active document's selection (`edit.placeImage`).
//!
//! The web preview panel's screenshot button uses it: the captured page is saved to a temporary
//! PNG, then this command scales it to the rectangle the user drew with the marquee tool and
//! inserts it as a new raster layer, so the shot lands exactly inside the drawn box.

use photocraft_color::PixelFormat;
use photocraft_doc::Layer;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// Bilinear RGBA8 resize (`sw × sh` → `dw × dh`): the fit of a capture into the drawn box.
fn resize_rgba(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let mut out = vec![0u8; dw as usize * dh as usize * 4];
    let sx = sw as f32 / dw as f32;
    let sy = sh as f32 / dh as f32;
    let (lx, ly) = ((sw - 1) as f32, (sh - 1) as f32);
    for y in 0..dh {
        let vy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, ly);
        let y0 = vy.floor() as u32;
        let y1 = (y0 + 1).min(sh - 1);
        let ty = vy - y0 as f32;
        for x in 0..dw {
            let vx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, lx);
            let x0 = vx.floor() as u32;
            let x1 = (x0 + 1).min(sw - 1);
            let tx = vx - x0 as f32;
            let at = |px: u32, py: u32, c: u32| src[((py * sw + px) * 4 + c) as usize] as f32;
            let o = ((y * dw + x) * 4) as usize;
            for c in 0..4u32 {
                let a = at(x0, y0, c) * (1.0 - tx) + at(x1, y0, c) * tx;
                let b = at(x0, y1, c) * (1.0 - tx) + at(x1, y1, c) * tx;
                out[o + c as usize] = (a * (1.0 - ty) + b * ty).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

fn place(s: &mut Session, p: &Value) -> Result<Value> {
    let path = p.get("path").and_then(Value::as_str).ok_or_else(|| bad("edit.placeImage", "`path` (an image file) is required"))?;
    let bytes = std::fs::read(path).map_err(|e| EngineError::Other(format!("could not read {path}: {e}")))?;
    let img = photocraft_codecs::decode(&bytes).map_err(|e| EngineError::Other(format!("could not decode {path}: {e}")))?;
    let (sw, sh) = (img.width(), img.height());
    if sw == 0 || sh == 0 {
        return Err(bad("edit.placeImage", "the image has no pixels"));
    }
    let rgba = img.to_rgba8();
    let id = s.edit("Place Image", |doc, active| {
        let canvas = doc.bounds();
        let selection = doc.selection.as_ref().map(|m| m.content_bounds().intersect(&canvas)).filter(|r| !r.is_empty());
        let (cw, ch) = (canvas.width(), canvas.height());
        // With a drawn box the shot fits inside it and is centred there. Without one it still
        // lands — at 1:1 in the middle of the canvas, shrunk only when it would not fit — so the
        // screenshot button always produces something visible instead of an error nobody sees.
        let (dw, dh) = match selection {
            Some(t) => {
                let (tw, th) = (t.width(), t.height());
                let fit = (tw as f32 / sw as f32).min(th as f32 / sh as f32);
                (((sw as f32 * fit).round() as u32).clamp(1, tw), ((sh as f32 * fit).round() as u32).clamp(1, th))
            }
            None => {
                let fit = (0.9 * cw as f32 / sw as f32).min(0.9 * ch as f32 / sh as f32).min(1.0);
                (((sw as f32 * fit).round() as u32).clamp(1, cw.max(1)), ((sh as f32 * fit).round() as u32).clamp(1, ch.max(1)))
            }
        };
        let (dx, dy) = match selection {
            Some(t) => (t.x0 + ((t.width() - dw) / 2) as i32, t.y0 + ((t.height() - dh) / 2) as i32),
            None => (canvas.x0 + ((cw - dw) / 2) as i32, canvas.y0 + ((ch - dh) / 2) as i32),
        };
        let box_rect = Rect::new(dx, dy, dx + dw as i32, dy + dh as i32);
        let scaled = resize_rgba(&rgba, sw, sh, dw, dh);
        let shot = Surface::from_interleaved(PixelFormat::RGBA8, box_rect, &scaled);
        // The document's own pixel format (a 16-bit or float document converts once here).
        let shot = if doc.pixel_format() == PixelFormat::RGBA8 { shot } else { shot.convert(doc.pixel_format()) };
        let mut layer = Layer::raster("Placed Image", doc.pixel_format());
        if let Some(surface) = layer.surface_mut() {
            *surface = shot;
        }
        let bg = crate::background_cmds::background_id(doc);
        let id = crate::layer_place::insert_new(doc, *active, layer, crate::layer_place::Place::Auto);
        crate::background_cmds::keep_background_at_bottom(bg, doc)?;
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({ "layer": id.0 }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "edit.placeImage",
        label: "Place Image File…",
        menu: &[],
        shortcut: None,
        params: r##"{"path":str} (place an image file on the canvas: fitted into the selection when there is one, otherwise at 1:1 in the middle) → {layer}"##,
        enabled: has_doc,
        run: place,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A 64×48 PNG with a red square in its top-left quarter, written to the temp directory.
    fn shot_png() -> std::path::PathBuf {
        let (w, h) = (64u32, 48u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let o = ((y * w + x) * 4) as usize;
                let inside = x < 32 && y < 24;
                rgba[o] = if inside { 255 } else { 0 };
                rgba[o + 1] = 0;
                rgba[o + 2] = if inside { 0 } else { 255 };
                rgba[o + 3] = 255;
            }
        }
        let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgba, rgba).unwrap();
        let png = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).unwrap();
        let path = std::env::temp_dir().join(format!("photocraft-place-test-{}.png", std::process::id()));
        std::fs::write(&path, png).unwrap();
        path
    }

    #[test]
    fn places_the_shot_inside_the_selection() {
        let path = shot_png();
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 160})).unwrap();
        // A 100×80 box at (20, 30) is the drawn frame.
        s.execute("select.rect", json!({"x": 20, "y": 30, "width": 100, "height": 80})).unwrap();
        let before = s.active().unwrap().doc.layers.len();
        let v = s.execute("edit.placeImage", json!({"path": path.to_string_lossy()})).unwrap();
        assert!(v["layer"].as_u64().is_some(), "the command reports the new layer: {v}");
        let after = s.active().unwrap().doc.layers.len();
        assert!(after > before, "a layer was inserted ({before} → {after})");
        // One history step, so the placement is undoable.
        assert!(s.execute("edit.undo", json!({})).is_ok(), "the placement records undo");
        assert_eq!(s.active().unwrap().doc.layers.len(), before, "undo removed the placed layer");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn without_a_selection_the_shot_lands_in_the_middle() {
        let path = shot_png();
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 120, "height": 90})).unwrap();
        let before = s.active().unwrap().doc.layers.len();
        let v = s.execute("edit.placeImage", json!({"path": path.to_string_lossy()})).unwrap();
        assert!(v["layer"].as_u64().is_some(), "the command reports the new layer: {v}");
        assert!(s.active().unwrap().doc.layers.len() > before, "the shot still lands, without a box drawn");
        let _ = std::fs::remove_file(path);
    }
}
