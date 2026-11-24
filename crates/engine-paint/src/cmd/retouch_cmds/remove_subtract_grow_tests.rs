//! The hole's two-pixel growth and the Subtract footprints (#12543): the Subtract difference is
//! taken out of the hole AFTER the Add area has grown, so the growth cannot restore pixels inside
//! a Subtract footprint, and the Subtract footprints themselves do not grow. A pixel inside the
//! subtracted footprint within GROW pixels of the Add edge stays out of the hole, and the
//! unchanged pixels under it stay as they were.

use super::*;

/// The Remove Tool's test document strokes: three Add drags across the red square at y 250, 280
/// and 310 (x 290..370 each) and the Alt-drag subtracting x 345..375 at y 280, all with the 30 px
/// hard brush. The Subtract footprint is the capsule x 330..390, y 265..295, plus its coverage
/// edge at a hair more.
fn plan_of(s: &Session) -> Plan {
    let p = json!({
        "strokes": [
            {"points": [[290, 250], [330, 250], [370, 250]], "size": 30.0, "hardness": 100.0},
            {"points": [[290, 280], [330, 280], [370, 280]], "size": 30.0, "hardness": 100.0},
            {"points": [[290, 310], [330, 310], [370, 310]], "size": 30.0, "hardness": 100.0},
            {"points": [[345, 280], [375, 280]], "size": 30.0, "hardness": 100.0, "subtract": true},
        ],
    });
    super::plan(s, &p).unwrap()
}

/// A session on the empty 800 x 600 canvas the strokes are drawn on.
fn session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    s
}

/// Whether the document pixel (`x`, `y`) is in the mask of the window.
fn in_hole(m: &[bool], win: Rect, x: i32, y: i32) -> bool {
    m[usize::try_from(y - win.y0).unwrap() * (win.width() as usize) + usize::try_from(x - win.x0).unwrap()]
}

#[test]
fn the_hole_growth_never_reaches_into_a_subtracted_footprint() {
    let s = session();
    let plan = plan_of(&s);
    let m = hole(&plan, None).unwrap();
    let win = plan.window;

    // The subtracted area itself stays out of the hole, and the Add area is a hole as before.
    assert!(!in_hole(&m, win, 350, 280), "the middle of the subtracted footprint is not in the hole");
    assert!(in_hole(&m, win, 300, 280), "the Add area between the square's edge and the subtracted footprint is in the hole");

    // The fringe: pixels inside the subtracted footprint within GROW pixels of the surviving Add
    // hole around it. Taking the Subtract difference after the growth keeps them out; growing
    // first and cutting after restores them into the hole, and the apply would change them.
    for at in [(329, 280), (350, 264), (350, 296)] {
        assert!(!in_hole(&m, win, at.0, at.1), "({},{}) is inside the subtracted footprint next to the Add edge and stays out of the hole", at.0, at.1);
    }

    // The Add area's own growth is untouched: the hole still reaches GROW pixels beyond the
    // Add strokes' footprint where no Subtract stroke cuts it.
    assert!(in_hole(&m, win, 274, 280), "the hole still grows {} px past the Add strokes", GROW);
}
