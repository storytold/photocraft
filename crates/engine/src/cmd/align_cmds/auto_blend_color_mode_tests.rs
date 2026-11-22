//! #14203: Auto-Blend Layers is disabled and rejected in CMYK and Lab documents, while
//! RGB and Grayscale at 8/16/32-bit keep working and Auto-Align stays enabled in CMYK/Lab.

use super::*;

fn two_layer_doc(mode_cmd: Option<&str>, depth: u8) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 60, "depth": depth, "background": "transparent"})).unwrap();
    if let Some(cmd) = mode_cmd {
        s.execute(cmd, json!({})).unwrap();
    }
    s.execute("layer.new.layer", json!({"name": "Left"})).unwrap();
    let a = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.new.layer", json!({"name": "Right"})).unwrap();
    let b = s.active().unwrap().active_layer.unwrap();
    s.edit("paint", |doc, _| {
        let mode = doc.mode;
        for (id, x0) in [(a, 0), (b, 20)] {
            let surf = doc.layer_mut(id).unwrap().surface_mut().unwrap();
            let px: Vec<f32> = match mode {
                ColorMode::Grayscale => vec![0.4, 1.0],
                ColorMode::Cmyk => vec![0.2, 0.4, 0.6, 0.1, 1.0],
                ColorMode::Lab => vec![0.5, 0.5, 0.5, 1.0],
                _ => vec![0.4, 0.5, 0.6, 1.0],
            };
            surf.fill_rect(Rect::new(x0, 0, x0 + 60, 60), &px);
        }
        Ok(())
    })
    .unwrap();
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    s
}

#[test]
fn auto_blend_disabled_and_rejected_in_cmyk_and_lab() {
    for mode_cmd in ["image.mode.cmyk", "image.mode.lab"] {
        let mut s = two_layer_doc(Some(mode_cmd), 8);
        assert!(!s.is_enabled("edit.autoBlendLayers"), "{mode_cmd}: autoBlendLayers should be disabled");
        assert!(s.is_enabled("edit.autoAlignLayers"), "{mode_cmd}: autoAlignLayers should stay enabled");

        let before_layers = s.active().unwrap().doc.layer_count();
        let before_rev = s.active().unwrap().revision;
        let err = s.execute("edit.autoBlendLayers", json!({})).unwrap_err().to_string();
        assert!(err.contains("Auto-Blend Layers"), "{mode_cmd}: unexpected error {err}");
        assert_eq!(s.active().unwrap().doc.layer_count(), before_layers);
        assert_eq!(s.active().unwrap().revision, before_rev);
    }
}

#[test]
fn auto_blend_works_in_rgb_and_grayscale_at_8_16_32_bit() {
    for mode_cmd in [None, Some("image.mode.grayscale")] {
        for depth in [8, 16, 32] {
            let mut s = two_layer_doc(mode_cmd, depth);
            assert!(s.is_enabled("edit.autoBlendLayers"), "{mode_cmd:?} {depth}-bit: should be enabled");
            s.execute("edit.autoBlendLayers", json!({"method": "stack", "seamlessTones": false})).unwrap();
        }
    }
}
