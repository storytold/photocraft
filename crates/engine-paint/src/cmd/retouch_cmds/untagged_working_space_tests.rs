//! #14224: cloning from another document into an untagged one lands in the space that document is
//! shown in, not in the mode's built-in default.

use super::*;
use crate::color_cmds::ColorExt;
use craft_cms::{Builtin, Transform};
use photocraft_color::ColorMode;

/// Cross-document Clone Stamp over a flat sRGB block, into an untagged RGB document whose working
/// RGB is ProPhoto-compatible. The stamp keeps its look, so its samples are re-encoded into the
/// space the destination is displayed in.
#[test]
fn a_cloned_block_lands_in_the_working_space_of_the_untagged_destination() {
    let mut s = crate::session();
    let filled = [0.9f32, 0.2, 0.2, 1.0];
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    s.execute("edit.assignProfile", json!({"profile": "srgb"})).unwrap();
    s.edit("block", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 64, 64), &filled);
        Ok(())
    })
    .unwrap();
    s.execute("cloneSource.set", json!({"source": [16, 16]})).unwrap();
    // Opened under the default settings, so the document stays untagged.
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.colorSettings", json!({"workingRgb": "prophoto-compat"})).unwrap();
    let dst = s.active().unwrap().doc.clone();
    assert!(dst.icc_profile.is_none(), "the destination document is untagged");

    s.execute("paint.cloneStamp", json!({"points": [[48, 48]], "size": 8, "hardness": 100})).unwrap();
    let st = s.active().unwrap();
    let got = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().read_region(Rect::new(48, 48, 49, 49));

    let mut out = [0.0f32; 16];
    let working = s.color().working(ColorMode::Rgb);
    Transform::new(Builtin::Srgb.profile(), &working, s.color().settings.intent(), s.color().settings.bpc).unwrap().eval(&filled[..3], &mut out).unwrap();
    let want = [out[0], out[1], out[2]];
    assert!((0..3).any(|c| (got[c] - filled[c]).abs() > 8.0 / 255.0), "the fixture must tell the two spaces apart: the stamp kept the source samples {got:?}");
    for c in 0..3 {
        assert!((got[c] - want[c]).abs() < 0.01, "the stamp landed at {:?}, not in the destination's space {want:?}", &got[..3]);
    }
}
