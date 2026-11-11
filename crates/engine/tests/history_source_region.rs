//! #11594: a small History Brush stroke on a large image whose history source is in another
//! format (after an Image › Mode change to CMYK or Lab) converts only the tiles the stroke reads, not
//! the whole source layer. The timing test is `#[ignore]`d; run it in release:
//!
//! ```sh
//! cargo test -p photocraft-engine --release --test history_source_region -- --ignored --nocapture
//! ```

use photocraft_engine::Session;
use serde_json::json;
use std::time::Instant;

/// A 36 MP (6000×6000) 8-bit RGB document covered in noise (every tile distinct), switched to
/// `mode`: the History Brush's source (state 1, the noise) is then RGB. A depth change alone is
/// refused (#12842), so it can't be the case measured here.
fn large_session(mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 6000, "height": 6000, "depth": 8, "mode": "rgb"})).unwrap();
    s.execute("filter.noise.addNoise", json!({"amount": 30, "seed": 42})).unwrap();
    s.execute(&format!("image.mode.{mode}"), json!({})).unwrap();
    s
}

#[test]
#[ignore = "timing on a 36 MP image: run in release with --ignored --nocapture"]
fn small_history_brush_stroke_on_a_36_mp_image_after_a_mode_change() {
    for mode in ["cmyk", "lab"] {
        let mut s = large_session(mode);
        let mut best = f64::MAX;
        for i in 0..3 {
            let y = 3000 + i * 40;
            let t = Instant::now();
            s.execute("paint.historyBrush", json!({"points": [[2990, y], [3010, y]], "size": 20, "hardness": 100, "state": 1})).unwrap();
            best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        }
        println!("RGB source -> {mode} layer, 20 px stroke on 6000x6000: best of 3 = {best:.2} ms");
    }
}
