//! #14940: the Inner Shadow page's ranges on the command side. The page types Distance
//! 0..30000 px, Size 0..250 px and an Angle of -360..360; `layer.layerStyle.innerShadow` keeps
//! those values inside the page's ranges and wraps an Angle past half a turn into -180..180 when
//! it stores it. Values already inside keep today's params and pixels.

use super::*;
use crate::Session;
use serde_json::json;

fn session(width: u32, height: u32) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": width, "height": height, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

/// The one Inner Shadow the active layer carries.
fn inner_shadow(s: &Session) -> Shadow {
    let d = s.active().unwrap();
    let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
    match l.effects.items.as_slice() {
        [Effect::InnerShadow(sh)] => sh.clone(),
        other => panic!("no inner shadow: {other:?}"),
    }
}

/// #14940: a typed Angle of 200 is stored as -160 and -200 as 160; past the field's ±360 the
/// value is clamped first, as every other out-of-range entry on the page is.
#[test]
fn inner_shadow_wraps_an_angle_past_half_a_turn() {
    for (typed, stored) in [(200.0, -160.0), (-200.0, 160.0), (270.0, -90.0), (360.0, 0.0), (400.0, 0.0), (-400.0, 0.0)] {
        let mut s = session(32, 32);
        s.execute("layer.layerStyle.innerShadow", json!({"angle": typed, "useGlobalLight": false})).unwrap();
        let angle = inner_shadow(&s).angle;
        assert!((angle - stored).abs() < 1e-4, "angle {typed} stored as {angle}, not {stored}");
    }
}

/// The other half of #14940: an Angle already inside half a turn each way is stored exactly as
/// it is today, so in-range params, imported PSD values and the shared light angle stay as they
/// were. 180 stays 180 rather than becoming -180.
#[test]
fn an_inner_shadow_angle_inside_half_a_turn_is_kept_exactly() {
    for angle in [0.0, 45.0, -45.0, 179.5, 180.0, -180.0] {
        let mut s = session(32, 32);
        s.execute("layer.layerStyle.innerShadow", json!({"angle": angle, "useGlobalLight": false})).unwrap();
        assert_eq!(inner_shadow(&s).angle, angle, "angle {angle}");
    }
}

/// #14940: the command keeps Distance inside the page's 0..30000 px, a negative value at 0.
#[test]
fn inner_shadow_clamps_distance_to_the_page_range() {
    for (sent, kept) in [(40000.0, 30000.0), (-5.0, 0.0), (30000.0, 30000.0)] {
        let mut s = session(32, 32);
        s.execute("layer.layerStyle.innerShadow", json!({"distance": sent})).unwrap();
        assert_eq!(inner_shadow(&s).distance, kept, "distance {sent}");
    }
}

/// #14940: and Size inside 0..250 px, 0 included.
#[test]
fn inner_shadow_clamps_size_to_the_page_range() {
    for (sent, kept) in [(300.0, 250.0), (-3.0, 0.0), (0.0, 0.0), (250.0, 250.0)] {
        let mut s = session(32, 32);
        s.execute("layer.layerStyle.innerShadow", json!({"size": sent})).unwrap();
        assert_eq!(inner_shadow(&s).size, kept, "size {sent}");
    }
}

/// #14940 names the Drop Shadow's ranges for #14893, so that path stores what it is given
/// today: no clamp on Distance 40000 or Size 300, no wrap on Angle 200.
#[test]
fn a_drop_shadow_still_stores_what_it_is_given() {
    let mut s = session(32, 32);
    s.execute("layer.layerStyle.dropShadow", json!({"distance": 40000.0, "size": 300.0, "angle": 200.0, "useGlobalLight": false})).unwrap();
    let d = s.active().unwrap();
    let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
    let Effect::DropShadow(sh) = &l.effects.items[0] else { panic!("{:?}", l.effects.items) };
    assert_eq!((sh.distance, sh.size, sh.angle), (40000.0, 300.0, 200.0));
}
