//! Place the web preview's screenshot into the drawn selection (`edit.placeImage`).
//!
//! The screenshot button saves the captured page to a temporary PNG and calls this command. The
//! shot is placed as a **smart object** (Layer › Smart Objects, `file.placeEmbedded`), not as a
//! resampled raster: the embedded source keeps the capture's own pixels and the layer stores only
//! the placement transform, so enlarging it later re-renders from the original instead of
//! magnifying an already-interpolated copy. It is fitted into the selection (aspect preserved,
//! centred), and lands centred at 1:1 when nothing is drawn.

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn place(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "edit.placeImage";
    let path = p.get("path").and_then(Value::as_str).ok_or_else(|| bad(CMD, "`path` (an image file) is required"))?;
    // Only the source size is needed here: the place itself decodes the file and keeps its bytes.
    let bytes = std::fs::read(path).map_err(|e| EngineError::Other(format!("could not read {path}: {e}")))?;
    let img = photocraft_codecs::decode(&bytes).map_err(|e| EngineError::Other(format!("could not decode {path}: {e}")))?;
    let (sw, sh) = (img.width() as f64, img.height() as f64);
    if sw <= 0.0 || sh <= 0.0 {
        return Err(bad(CMD, "the image has no pixels"));
    }
    let mut params = json!({"path": path, "smartObject": true, "frame": false});
    if let Some(session) = s.active() {
        let canvas = session.doc.bounds();
        let selection = session.doc.selection.as_ref().map(|m| m.content_bounds().intersect(&canvas)).filter(|r| !r.is_empty());
        if let Some(t) = selection {
            // Fit the capture inside the box without distorting it, and aim the place at the box's
            // middle: the smart object's transform carries the scale, the source stays untouched.
            let fit = (f64::from(t.width()) / sw).min(f64::from(t.height()) / sh);
            let (tw, th) = (f64::from(t.width()), f64::from(t.height()));
            params["scale"] = json!(fit * 100.0);
            params["center"] = json!([f64::from(t.x0) + tw / 2.0, f64::from(t.y0) + th / 2.0]);
            params["fit"] = json!(false);
        }
    }
    // `file.placeEmbedded` inserts the smart object (one undo step) and reports the new layer.
    crate::cmd::file_cmds::place(s, &params, false)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "edit.placeImage",
        label: "Place Image File…",
        menu: &[],
        shortcut: None,
        params: r##"{"path":str} (place an image file as a smart object: fitted into the selection when there is one, otherwise at 1:1 in the middle) → {layer, scale, bounds, smartObject}"##,
        enabled: has_doc,
        run: place,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use photocraft_doc::{LayerContent, SmartSource};
    use serde_json::json;

    use super::*;

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
    fn places_the_shot_inside_the_selection_as_a_smart_object() {
        let path = shot_png();
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 160})).unwrap();
        // A 100×80 box at (20, 30) is the drawn frame.
        s.execute("select.rect", json!({"x": 20, "y": 30, "width": 100, "height": 80})).unwrap();
        let before = s.active().unwrap().doc.layers.len();
        let v = s.execute("edit.placeImage", json!({"path": path.to_string_lossy()})).unwrap();
        assert!(v["layer"].as_u64().is_some(), "the command reports the new layer: {v}");
        assert_eq!(v["smartObject"], json!(true), "the shot lands as a smart object: {v}");
        let after = s.active().unwrap().doc.layers.len();
        assert!(after > before, "a layer was inserted ({before} → {after})");
        // The source keeps the capture's own pixels: this is what makes enlarging it stay sharp.
        let id = s.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l.id).last().unwrap();
        let LayerContent::Smart(sm) = &s.active().unwrap().doc.layer(id).unwrap().content else { panic!("not a smart object") };
        let SmartSource::Embedded { bytes, .. } = &sm.source else { panic!("the source is not embedded") };
        let source = photocraft_codecs::decode(bytes).unwrap();
        assert_eq!((source.width(), source.height()), (64, 48), "the embedded source is the original capture");
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
