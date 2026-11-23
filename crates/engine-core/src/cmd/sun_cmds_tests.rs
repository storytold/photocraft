use serde_json::{Value, json};

use crate::Session;

const W: i32 = 512;
const H: i32 = 256;

/// A 2:1 sky at `depth` bits with a small bright sun at document pixel (`sx`, `sy`).
fn sky(depth: u64, sx: i32, sy: i32) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": W, "height": H, "depth": depth})).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let sun = (x - sx).pow(2) + (y - sy).pow(2) <= 4;
                let c = if sun {
                    [400.0, 380.0, 350.0, 1.0]
                } else if y < H / 2 {
                    [0.3, 0.45, 0.7, 1.0]
                } else {
                    [0.12, 0.1, 0.08, 1.0]
                };
                data.extend(photocraft_raster::from_rgba(&fmt, c));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s
}

fn vec3(v: &Value) -> [f64; 3] {
    [0, 1, 2].map(|i| v[i].as_f64().unwrap())
}

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.06)
}

#[test]
fn finds_the_sun_measures_it_only_in_32_bit_and_reports_it_in_the_frame_asked_for() {
    // A quarter of the way right of centre, on the horizon line: azimuth 90°, elevation ~0°.
    let (sx, sy) = (W * 3 / 4, H / 2 - 4);
    for depth in [8, 32] {
        let mut s = sky(depth, sx, sy);
        let revision = s.active().unwrap().revision;
        let r = s.execute("image.analysis.findSun", json!({})).unwrap();
        assert!(r["found"].as_bool().unwrap());
        let p = &r["position"];
        assert!((p[0].as_f64().unwrap() - f64::from(sx)).abs() < 3.0 && (p[1].as_f64().unwrap() - f64::from(sy)).abs() < 3.0, "{p}");
        assert_eq!(r["light"]["measured"], json!(depth == 32), "{depth}-bit: {}", r["light"]);
        assert_eq!(s.active().unwrap().revision, revision, "finding the sun changes nothing");

        // Y-up left-handed: image centre +Z, its right +X. Turned 90° right, the centre faces +X
        // and the sun, a quarter to its right, faces −Z.
        assert!(close(vec3(&r["direction"]), [1.0, 0.0, 0.0]), "{}", r["direction"]);
        let turned = s.execute("image.analysis.findSun", json!({"heading": 90})).unwrap();
        assert!(close(vec3(&turned["direction"]), [0.0, 0.0, -1.0]), "{}", turned["direction"]);
        let z_up = s.execute("image.analysis.findSun", json!({"frame": "zUpRightHanded"})).unwrap();
        assert!(close(vec3(&z_up["direction"]), [1.0, 0.0, 0.0]), "{}", z_up["direction"]);
        assert!(z_up["lightRotation"].is_null(), "the light rotation is only given for Y-up left-handed");
    }
}

#[test]
fn at_measures_where_it_is_told_instead_of_the_brightest_spot() {
    let mut s = sky(32, W * 3 / 4, H / 2 - 20);
    let r = s.execute("image.analysis.findSun", json!({"at": [W / 4, H / 4]})).unwrap();
    assert!(r["picked"].as_bool().unwrap());
    assert!((r["azimuth"].as_f64().unwrap() + 90.0).abs() < 1.0, "{}", r["azimuth"]);
    // Nothing bright there: an estimate, not the real sun's measured light, and not a clipped sun either.
    assert_eq!(r["light"]["measured"], json!(false));
    assert_eq!(r["light"]["clipped"], json!(false));
}
