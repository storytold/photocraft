//! Artboard from Group (#14552): the artboard is Normal at 100 % opacity, as in the reference
//! application, whatever the group had. Mask, name, children and fill opacity are kept, one undo
//! restores the group, and the other two ways of making an artboard keep today's blend and opacity.

use super::*;
use photocraft_color::BlendMode;
use photocraft_doc::LayerMask;

/// 200 x 200 with a top-level group "Group" (one child "Red" with pixels) that has the given
/// blend, opacity, fill opacity and an optional mask.
fn session_with_group(blend: BlendMode, opacity: f32, fill_opacity: f32, mask: bool) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 200, "depth": 8, "background": "white"})).unwrap();
    let gid = s
        .edit("Group", |doc, active| {
            let fmt = doc.pixel_format();
            let mut red = Layer::raster("Red", fmt);
            red.surface_mut().unwrap().fill_rect(Rect { x0: 20, y0: 20, x1: 80, y1: 80 }, &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
            let mut g = Layer::group("Group", vec![red]);
            g.blend = blend;
            g.opacity = opacity;
            g.fill_opacity = fill_opacity;
            if mask {
                g.mask = Some(LayerMask::reveal_all());
            }
            let id = doc.insert_above(None, g);
            *active = Some(id);
            Ok(id)
        })
        .unwrap();
    (s, gid)
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

#[test]
fn a_pass_through_group_becomes_a_normal_full_opacity_artboard() {
    let (mut s, gid) = session_with_group(BlendMode::PassThrough, 1.0, 1.0, false);
    s.execute("layer.new.artboardFromGroup", json!({"layer": gid.0})).unwrap();
    let l = doc(&s).layer(gid).unwrap();
    assert!(l.artboard().is_some());
    assert_eq!(l.blend, BlendMode::Normal);
    assert_eq!(l.opacity, 1.0);
}

#[test]
fn a_multiply_half_opacity_group_is_reset_and_keeps_its_mask_name_children_and_fill() {
    let (mut s, gid) = session_with_group(BlendMode::Multiply, 0.5, 0.25, true);
    s.execute("layer.new.artboardFromGroup", json!({"layer": gid.0})).unwrap();
    let l = doc(&s).layer(gid).unwrap();
    assert!(l.artboard().is_some());
    assert_eq!(l.blend, BlendMode::Normal);
    assert_eq!(l.opacity, 1.0);
    assert!(l.mask.is_some(), "the layer mask stays");
    assert_eq!(l.name, "Group");
    let kids: Vec<&str> = l.children().unwrap().iter().map(|k| k.name.as_str()).collect();
    assert_eq!(kids, ["Red"]);
    assert_eq!(l.fill_opacity, 0.25, "fill opacity is left alone");
}

#[test]
fn one_undo_restores_multiply_at_half_opacity() {
    let (mut s, gid) = session_with_group(BlendMode::Multiply, 0.5, 1.0, true);
    s.execute("layer.new.artboardFromGroup", json!({"layer": gid.0})).unwrap();
    s.undo();
    let l = doc(&s).layer(gid).unwrap();
    assert!(l.artboard().is_none());
    assert_eq!(l.blend, BlendMode::Multiply);
    assert_eq!(l.opacity, 0.5);
    assert!(l.mask.is_some());
}

#[test]
fn the_other_ways_to_make_an_artboard_keep_todays_blend_and_opacity() {
    // Layer > New > Artboard.
    let (mut s, _) = session_with_group(BlendMode::Normal, 1.0, 1.0, false);
    let r = s.execute("layer.new.artboard", json!({"rect": [0, 0, 100, 100]})).unwrap();
    let board = doc(&s).layer(LayerId(r["layer"].as_u64().unwrap())).unwrap();
    assert!(board.artboard().is_some());
    assert_eq!((board.blend, board.opacity), (BlendMode::PassThrough, 1.0));
    // Artboard from Layers: the selected layer goes into a new board.
    let (mut s, gid) = session_with_group(BlendMode::Normal, 1.0, 1.0, false);
    crate::layer_multi_cmds::set_selection(&mut s, vec![gid], Some(gid), Some(gid)).unwrap();
    let r = s.execute("layer.new.artboardFromLayers", json!({})).unwrap();
    let board = doc(&s).layer(LayerId(r["layer"].as_u64().unwrap())).unwrap();
    assert!(board.artboard().is_some());
    assert_eq!((board.blend, board.opacity), (BlendMode::PassThrough, 1.0));
}
