//! #12198: Structure 1 keeps flat content, as the higher values do; busy content is still
//! re-synthesised (the existing structure test in `retouch_cmds/tests.rs`).

use serde_json::json;

use crate::Session;

/// 800 × 600: grey left half, green right half, an 80 × 80 checker of 10 px cells at 100,100;
/// the 40 × 40 selection at 120,120 moved by (460, 280) onto the green. Row y 420, as 0..255.
fn row_after(structure: u64) -> Vec<(i32, [f32; 3])> {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 600})).unwrap();
    let fill = |s: &mut Session, x: i32, y: i32, w: i32, h: i32, c: &str| {
        s.execute("select.rect", json!({"x": x, "y": y, "width": w, "height": h, "antiAlias": false})).unwrap();
        s.execute("edit.fill", json!({"color": c})).unwrap();
    };
    fill(&mut s, 0, 0, 400, 600, "#808080");
    fill(&mut s, 400, 0, 400, 600, "#28a03c");
    for cy in 0..8 {
        for cx in 0..8 {
            fill(&mut s, 100 + 10 * cx, 100 + 10 * cy, 10, 10, if (cx + cy) % 2 == 0 { "#3c3c3c" } else { "#c8c8c8" });
        }
    }
    s.execute("select.rect", json!({"x": 120, "y": 120, "width": 40, "height": 40, "antiAlias": false})).unwrap();
    s.execute("paint.contentAwareMove", json!({"offset": [460, 280], "structure": structure, "color": 0})).unwrap();
    let d = s.active().unwrap();
    let surf = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap();
    (575..=620)
        .step_by(5)
        .map(|x| {
            (x, {
                let c = surf.rgba(x, 420);
                [c[0] * 255.0, c[1] * 255.0, c[2] * 255.0]
            })
        })
        .collect()
}

fn at(row: &[(i32, [f32; 3])], x: i32) -> [f32; 3] {
    row.iter().find(|(px, _)| *px == x).map(|(_, c)| *c).unwrap()
}

fn near(a: [f32; 3], b: [f32; 3], within: f32) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() <= within)
}

#[test]
fn structure_1_keeps_a_flat_checker_like_structure_4() {
    let (one, four) = (row_after(1), row_after(4));
    for x in (590..=610).step_by(5) {
        assert!(near(at(&one, x), at(&four, x), 10.0), "x {x}: structure 1 {:?}, structure 4 {:?}", at(&one, x), at(&four, x));
    }
    // The cells themselves, as the reference keeps them.
    for (x, v) in [(590, 200.0), (595, 200.0), (600, 60.0), (605, 60.0), (610, 200.0)] {
        assert!(near(at(&one, x), [v; 3], 10.0), "x {x}: {:?}", at(&one, x));
    }
    // The edge band blends into the green exactly as Structure 4's does today. (Structure 4's
    // own edge is 23 to 39 levels off the reference's at x 585 and 615; Structure 3 to 7 stay as
    // they are here.)
    for x in [580, 585, 615] {
        assert_eq!(at(&one, x), at(&four, x), "x {x}");
    }
    assert!(near(at(&one, 585), [58.0, 72.0, 60.0], 25.0), "x 585 blends towards the green: {:?}", at(&one, 585));
}

#[test]
fn structure_3_to_7_and_2_on_flat_content_match() {
    let three = row_after(3);
    for structure in [2, 5, 7] {
        assert_eq!(row_after(structure), three, "structure {structure}");
    }
}
