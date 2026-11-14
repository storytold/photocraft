use super::*;

fn setup_reference_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "depth": 8, "name": "target"})).unwrap();

    // Background: left half x 0..200 RGB 200/100/50, right half x 200..400 RGB 50/150/250.
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 300, "mode": "replace"})).unwrap();
    s.execute("edit.fill", json!({"color": "#c86432"})).unwrap();
    s.execute("select.rect", json!({"x": 200, "y": 0, "width": 200, "height": 300, "mode": "replace"})).unwrap();
    s.execute("edit.fill", json!({"color": "#3296fa"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();

    // Layer "Top": 100x100 square at (150, 100) of 0/255/0 at alpha 128.
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    s.execute("select.rect", json!({"x": 150, "y": 100, "width": 100, "height": 100, "mode": "replace"})).unwrap();
    s.execute("edit.fill", json!({"color": "#00ff00", "opacity": 128.0 * 100.0 / 255.0})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();

    // Layer "Masked": solid red with a layer mask showing only the left half.
    s.execute("layer.new.layer", json!({"name": "Masked"})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 300, "mode": "replace"})).unwrap();
    s.execute("layer.layerMask.revealSelection", json!({})).unwrap();

    // Alpha 1 mask: white rectangle (0,0)-(200,150), black elsewhere.
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 150, "mode": "replace"})).unwrap();
    s.execute("select.saveSelection", json!({"name": "Alpha 1"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();

    s
}

fn sample_u8(s: &Session, chan_idx: usize) -> [u8; 4] {
    let ch = &s.active().unwrap().doc.channels[chan_idx].surface;
    let pts = [(50, 50), (300, 50), (50, 200), (225, 150)];
    pts.map(|(x, y)| (ch.sample_channel(x, y, 0).clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn run_calc(s: &mut Session, blending: &str, opacity: Option<f64>, mask_invert: Option<bool>) -> Result<[u8; 4]> {
    let mut p = json!({
        "source1": {"layer": "merged", "channel": "red"},
        "source2": {"layer": {"background": true}, "channel": "green"},
        "blending": blending,
        "result": "newChannel"
    });
    if let Some(op) = opacity {
        p["opacity"] = json!(op);
    }
    if let Some(inv) = mask_invert {
        p["mask"] = json!({"channel": "Alpha 1", "invert": inv});
    }
    let r = s.execute("image.calculations", p)?;
    let idx = r["channel"].as_u64().unwrap() as usize;
    Ok(sample_u8(s, idx))
}

#[test]
fn darker_and_lighter_color_match_normal_at_full_opacity() {
    let mut s = setup_reference_doc();

    assert_eq!(run_calc(&mut s, "normal", None, None).unwrap(), [255, 50, 255, 25]);
    assert_eq!(run_calc(&mut s, "darkerColor", None, None).unwrap(), [255, 50, 255, 25]);
    assert_eq!(run_calc(&mut s, "lighterColor", None, None).unwrap(), [255, 50, 255, 25]);
    assert_eq!(run_calc(&mut s, "Darker Color", None, None).unwrap(), [255, 50, 255, 25]);
    assert_eq!(run_calc(&mut s, "Lighter Color", None, None).unwrap(), [255, 50, 255, 25]);

    // Contrast with Darken and Lighten to prove it is Normal, not Darken/Lighten.
    assert_eq!(run_calc(&mut s, "darken", None, None).unwrap(), [100, 50, 100, 25]);
    assert_eq!(run_calc(&mut s, "lighten", None, None).unwrap(), [255, 150, 255, 150]);
}

#[test]
fn darker_and_lighter_color_match_normal_with_opacity_and_mask() {
    let mut s = setup_reference_doc();

    // 50% opacity
    let normal_50 = run_calc(&mut s, "normal", Some(50.0), None).unwrap();
    assert_eq!(normal_50, [178, 100, 178, 87]);
    assert_eq!(run_calc(&mut s, "darkerColor", Some(50.0), None).unwrap(), normal_50);
    assert_eq!(run_calc(&mut s, "lighterColor", Some(50.0), None).unwrap(), normal_50);

    // Mask "Alpha 1" (plain)
    let normal_mask = run_calc(&mut s, "normal", None, Some(false)).unwrap();
    assert_eq!(normal_mask, [255, 150, 100, 150]);
    assert_eq!(run_calc(&mut s, "darkerColor", None, Some(false)).unwrap(), normal_mask);
    assert_eq!(run_calc(&mut s, "lighterColor", None, Some(false)).unwrap(), normal_mask);

    // Mask "Alpha 1" (inverted)
    let normal_mask_inv = run_calc(&mut s, "normal", None, Some(true)).unwrap();
    assert_eq!(run_calc(&mut s, "darkerColor", None, Some(true)).unwrap(), normal_mask_inv);
    assert_eq!(run_calc(&mut s, "lighterColor", None, Some(true)).unwrap(), normal_mask_inv);
}

#[test]
fn unknown_blending_still_errors_and_apply_image_unchanged() {
    let mut s = setup_reference_doc();

    assert!(run_calc(&mut s, "notABlendMode", None, None).is_err());

    // #14478 covers Apply Image separately; Apply Image must keep refusing darkerColor / lighterColor.
    s.execute("layer.select", json!({"layer": {"background": true}})).unwrap();
    assert!(
        s.execute(
            "image.applyImage",
            json!({
                "source": {"layer": "merged", "channel": "red"},
                "blending": "darkerColor"
            })
        )
        .is_err()
    );
    assert!(
        s.execute(
            "image.applyImage",
            json!({
                "source": {"layer": "merged", "channel": "red"},
                "blending": "lighterColor"
            })
        )
        .is_err()
    );
}
