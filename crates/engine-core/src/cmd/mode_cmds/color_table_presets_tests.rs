//! #14344: the Spectrum table runs from magenta (index 0) to red (index 255), System (Mac OS)
//! from white down through the web cube to black last, System (Windows) from black through the
//! static colours to white last. Indexed Color conversion with the system palettes still gives
//! each pixel the same colour; only the order of the table changes.

use super::*;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const YELLOW: [f32; 4] = [1.0, 1.0, 0.0, 1.0];

/// Four solid bars red, green, blue and yellow, converted to Indexed Color with `palette`, no
/// dither and no transparency.
fn bars(palette: &str) -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 8, "height": 4, "background": "white"})).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for _ in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                data.extend(photocraft_raster::from_rgba(&fmt, [RED, GREEN, BLUE, YELLOW][x as usize / 2]));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s.execute("image.mode.indexedColor", json!({"palette": palette, "forced": "none", "dither": "none", "transparency": false})).unwrap();
    s
}

fn bar_colours(s: &Session) -> [[f32; 4]; 4] {
    let surf = s.active().unwrap().doc.layers[0].surface().unwrap();
    [0, 2, 4, 6].map(|x| surf.rgba(x, 2))
}

fn unique(table: &[[u8; 3]]) -> bool {
    let mut sorted = table.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    sorted.len() == table.len()
}

#[test]
fn spectrum_runs_from_magenta_to_red() {
    let t = preset_table("spectrum").unwrap();
    assert_eq!(t.len(), 256);
    let [first, last] = [t[0], t[255]];
    assert!(first[0] > first[1] && first[2] > first[1], "index 0 is magenta-ish: {first:?}");
    assert!(last[0] > last[1] && last[0] > last[2], "index 255 is red: {last:?}");
}

#[test]
fn applying_spectrum_makes_the_bars_magenta_ish() {
    let mut s = bars("exact");
    s.execute("image.mode.colorTable", json!({"table": "spectrum"})).unwrap();
    for c in bar_colours(&s) {
        assert!(c[0] > c[1] && c[2] > c[1], "{c:?}");
    }
}

#[test]
fn system_mac_runs_from_white_to_black() {
    let t = preset_table("systemMac").unwrap();
    assert_eq!(t.len(), 256);
    assert!(unique(&t));
    assert_eq!(&t[..4], &[[255, 255, 255], [255, 255, 204], [255, 255, 153], [255, 255, 102]]);
    assert_eq!(t[255], [0, 0, 0]);
}

#[test]
fn system_windows_runs_from_black_to_white() {
    let t = preset_table("systemWindows").unwrap();
    assert_eq!(t.len(), 256);
    assert!(unique(&t));
    assert_eq!(&t[..5], &[[0, 0, 0], [128, 0, 0], [0, 128, 0], [128, 128, 0], [0, 0, 128]]);
    assert_eq!(t[255], [255, 255, 255]);
}

#[test]
fn the_system_palettes_keep_their_colours() {
    // Only the order changed: the same colours as the cube plus the ramps (Mac OS) and the static
    // colours plus the cube (Windows).
    let cube = photocraft_algo::quantize::web_palette();
    let mac = preset_table("systemMac").unwrap();
    assert!(cube.iter().all(|c| mac.contains(c)), "the whole web cube");
    assert!([[17, 0, 0], [0, 238, 0], [0, 0, 119], [34, 34, 34]].iter().all(|c| mac.contains(c)), "the ramps");
    let windows = preset_table("systemWindows").unwrap();
    assert!(cube.iter().all(|c| windows.contains(c)), "the whole web cube");
    assert!([[192, 220, 192], [166, 202, 240], [255, 251, 240], [160, 160, 164]].iter().all(|c| windows.contains(c)), "the static colours");
}

#[test]
fn indexed_conversion_with_the_system_palettes_gives_each_pixel_its_colour() {
    for palette in ["systemMac", "systemWindows"] {
        let s = bars(palette);
        assert_eq!(bar_colours(&s), [RED, GREEN, BLUE, YELLOW], "{palette}");
    }
}
