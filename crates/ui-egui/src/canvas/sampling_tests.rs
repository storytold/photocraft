//! #13505: a flipped view is always drawn on the CPU canvas, which magnified its texture with
//! nearest sampling at every zoom above 100 %. Between 100 % and 200 % the GPU canvas is bilinear,
//! so at 108.3 % (the report's zoom) one row and column in twelve showed twice and a diagonal
//! stroke looked torn only while the view was flipped. The CPU canvas now samples as the GPU
//! canvas does at the view's zoom, and when only the zoom changes it sends one texel again with
//! the new sampling rather than the whole document.

use super::textures::CRISP;
use super::*;
use crate::gpu_canvas::{Sampling, sampling};
use serde_json::json;

/// One upload of `tex` this frame: where (`None` for the whole texture), its size and sampling.
type Upload = (Option<[usize; 2]>, [usize; 2], TextureOptions);

/// The uploads of `tex` since the last call.
fn take_uploads(ctx: &egui::Context, tex: egui::TextureId) -> Vec<Upload> {
    let mut delta = ctx.tex_manager().write().take_delta();
    let seen = delta.set.get(&tex).into_iter().flatten().map(|d| (d.pos, d.image.size(), d.options)).collect();
    delta.clear();
    seen
}

/// A white `w` × `h` document with a black diagonal stroke.
fn stroked(w: u32, h: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": w, "height": h, "background": "white"})).unwrap();
    app.run("paint.stroke", json!({"points": [[4, 4], [60, 30]], "size": 3, "hardness": 1.0, "color": "#000000"})).unwrap();
    app
}

/// What the GPU canvas would do at `zoom`, as the CPU texture's sampling.
fn gpu_like(zoom: f32) -> TextureOptions {
    if sampling(zoom) == Sampling::Bilinear { TextureOptions::LINEAR } else { CRISP }
}

#[test]
fn cpu_canvas_samples_like_the_gpu_canvas_at_every_zoom() {
    let mut app = stroked(64, 48);
    let ctx = egui::Context::default();
    let view = |zoom: f32| Some(InView { visible: DRect::new(0, 0, 64, 48), zoom, key: 1 });
    // The report's zoom: bilinear from the first upload.
    let tex = ensure_texture_in_view(&mut app, &ctx, 0, None, view(1.083)).unwrap().texture;
    assert_eq!(take_uploads(&ctx, tex), vec![(None, [64, 48], TextureOptions::LINEAR)]);
    let mut was = TextureOptions::LINEAR;
    for (zoom, want) in [
        (1.083, TextureOptions::LINEAR),
        (1.0, CRISP),
        (1.5, TextureOptions::LINEAR),
        (1.9, TextureOptions::LINEAR),
        (2.0, CRISP),
        (2.5, CRISP),
        (1.25, TextureOptions::LINEAR),
        (3.0, CRISP),
        (0.5, CRISP),
    ] {
        assert_eq!(gpu_like(zoom), want, "the GPU canvas's sampling at {zoom}");
        assert_eq!(ensure_texture_in_view(&mut app, &ctx, 0, None, view(zoom)).unwrap().texture, tex);
        // A change is one texel sent again with the new sampling; no change sends nothing.
        let expected = if want == was { vec![] } else { vec![(Some([0, 0]), [1, 1], want)] };
        assert_eq!(take_uploads(&ctx, tex), expected, "zoom {zoom}");
        was = want;
    }
    // Back to the report's zoom, then the Navigator's call (no view) keeps the view's sampling,
    // and so does an edit it uploads.
    ensure_texture_in_view(&mut app, &ctx, 0, None, view(1.083)).unwrap();
    take_uploads(&ctx, tex);
    ensure_texture(&mut app, &ctx, 0, None).unwrap();
    assert!(take_uploads(&ctx, tex).is_empty());
    app.run("select.rect", json!({"x": 10, "y": 10, "width": 4, "height": 4})).unwrap();
    ensure_texture(&mut app, &ctx, 0, None).unwrap();
    take_uploads(&ctx, tex);
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    ensure_texture(&mut app, &ctx, 0, None).unwrap();
    let up = take_uploads(&ctx, tex);
    assert!(!up.is_empty() && up.iter().all(|u| u.0.is_some() && u.2 == TextureOptions::LINEAR), "{up:?}");
    // An edit shown at 200 % is uploaded crisp: the sampling rides on the edit's own upload.
    app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
    ensure_texture_in_view(&mut app, &ctx, 0, None, view(2.0)).unwrap();
    let up = take_uploads(&ctx, tex);
    assert!(!up.is_empty() && up.iter().all(|u| u.0.is_some() && u.2 == CRISP), "{up:?}");
    assert!(ensure_texture_in_view(&mut app, &ctx, 0, None, view(2.0)).is_some());
    assert!(take_uploads(&ctx, tex).is_empty(), "nothing left to send");
}

/// The part in view of a document too large for one full-resolution texture (#7568) samples the
/// same way: bilinear at 108.3 %, crisp at 100 % and 200 %, switched with one texel.
#[test]
fn cpu_canvas_detail_samples_like_the_gpu_canvas() {
    let mut app = stroked(3 * MAX_TEXTURE, 40);
    let ctx = egui::Context::default();
    ctx.input_mut(|i| i.max_texture_side = 16384);
    let view = |zoom: f32| Some(InView { visible: DRect::new(0, 0, 1000, 40), zoom, key: 1 });
    let t = ensure_texture_in_view(&mut app, &ctx, 0, None, view(1.083)).unwrap();
    assert!(t.scale < 1.0, "the whole document is reduced");
    let (detail, _) = t.detail.expect("a detail texture above 100 %");
    let up = take_uploads(&ctx, detail);
    assert_eq!(up.len(), 1);
    assert_eq!((up[0].0, up[0].2), (None, TextureOptions::LINEAR));
    for (zoom, want) in [(1.0, CRISP), (1.083, TextureOptions::LINEAR), (2.0, CRISP)] {
        assert_eq!(gpu_like(zoom), want);
        let t = ensure_texture_in_view(&mut app, &ctx, 0, None, view(zoom)).unwrap();
        // The same plan, so the same texture: only its sampling changes.
        assert_eq!(t.detail.map(|d| d.0), Some(detail), "zoom {zoom}");
        assert_eq!(take_uploads(&ctx, detail), vec![(Some([0, 0]), [1, 1], want)], "zoom {zoom}");
    }
    ensure_texture_in_view(&mut app, &ctx, 0, None, view(2.0)).unwrap();
    assert!(take_uploads(&ctx, detail).is_empty(), "unchanged: nothing sent");
}
