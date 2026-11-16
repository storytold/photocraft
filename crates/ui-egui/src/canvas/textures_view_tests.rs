//! The canvas composite is not rebuilt for a revision that only changed the view (#4915's
//! `view_revision`), and a real edit is never narrowed away by a view command queued with it.

use serde_json::json;

use super::textures::ensure_texture;
use crate::PhotocraftApp;

/// Draw document 0's CPU canvas texture once, clearing what the previous pass recorded.
fn draw(app: &mut PhotocraftApp, ctx: &egui::Context) -> Option<egui::TextureId> {
    app.perf.last_refresh = "";
    app.perf.last_refresh_px = 0;
    ctx.begin_pass(Default::default());
    let texture = ensure_texture(app, ctx, 0, None).map(|(texture, _scale)| texture);
    // A bare `Context` has no backend to apply the frame's texture uploads, and dropping them unapplied
    // panics (epaint's `TexturesDelta` drop guard): clear them as a backend does after applying.
    let mut output = ctx.end_pass();
    output.textures_delta.clear();
    texture
}

#[test]
fn a_channel_view_change_does_not_recomposite_the_canvas() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.run("edit.fill", json!({"color": "#336699"})).unwrap();
    let ctx = egui::Context::default();
    let first = draw(&mut app, &ctx).expect("canvas texture");
    assert_eq!((app.perf.last_refresh, app.perf.last_refresh_px), ("full", 64 * 48), "the first draw composites the document");

    // A Channels-panel click (and an eye toggle) changes the view only: the texture the canvas
    // holds is already what this frame draws, so nothing is composited or uploaded.
    for (id, params) in [
        ("channel.target", json!({"channel": "red"})),
        ("channel.setVisible", json!({"channel": {"color": 1}})),
        ("channel.target", json!({"channel": "composite"})),
    ] {
        app.run(id, params.clone()).unwrap();
        let texture = draw(&mut app, &ctx).expect("canvas texture");
        assert_eq!(texture, first, "{id} {params}: the same texture");
        assert_eq!((app.perf.last_refresh, app.perf.last_refresh_px), ("", 0), "{id} {params}: no recomposite");
    }

    // A real edit still composites, and a view command queued with it in the same frame does not
    // narrow its damage away.
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    draw(&mut app, &ctx);
    assert!(app.perf.last_refresh_px > 0, "an edit composites: {:?}", (app.perf.last_refresh, app.perf.last_refresh_px));
    app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
    app.run("channel.target", json!({"channel": "green"})).unwrap();
    let texture = draw(&mut app, &ctx).expect("canvas texture");
    assert!(
        app.perf.last_refresh_px > 0,
        "an edit with a channel click queued behind it still redraws: {:?}",
        (app.perf.last_refresh, app.perf.last_refresh_px)
    );
    assert_eq!(texture, first, "drawn into the texture it already had");
}
