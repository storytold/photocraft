//! The frame that handles a release on the canvas shows what the release made. The canvas paints
//! after the frame's input, so a tool's preview never disappears before its result is on screen.
//! Painted before the input, a new shape or selection blinked out for that one frame, and a guide
//! moved with the Move tool blinked back to where it was.

use egui::{Color32, Event, PointerButton, Pos2, Rect};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::{Tool, View};

/// A white 64×64 document shown at 4 screen px per document px, centred, with `tool` selected.
fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    // Snapping would move the release point.
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app.ui.extras.rulers = false;
    app.ui.views = vec![View { zoom: 4.0, center: [32.0, 32.0], fit_pending: false, fill_pending: false, doc_size: [64, 64], rotation: 0.0 }];
    app
}

/// Frames at a display's 60 Hz, so two clicks a few frames apart are a double-click.
fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(400.0, 340.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            app.last_canvas_rect = ui.max_rect();
            let v = app.ui.views[0].clone();
            crate::canvas::canvas_view(app, ui, 0, ui.max_rect(), v, true);
        },
        app,
    );
    h.run_steps(2);
    h
}

/// The screen point of document point `p`.
fn at(h: &Harness<'_, PhotocraftApp>, p: [f32; 2]) -> Pos2 {
    let app = h.state();
    let v = &app.ui.views[0];
    let xf = ViewXform { rect: app.last_canvas_rect, zoom: v.zoom / app.ppp, center: v.center, flip: false, rotation: 0.0, aspect: 1.0 };
    xf.to_screen(p[0], p[1])
}

fn button(h: &mut Harness<'_, PhotocraftApp>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
}

/// Press at document point `from`, drag to `to` over four frames, then release in a frame of its
/// own: the last frame drawn is the one that handled the release.
fn drag(h: &mut Harness<'_, PhotocraftApp>, from: [f32; 2], to: [f32; 2]) {
    let (a, b) = (at(h, from), at(h, to));
    h.event(Event::PointerMoved(a));
    h.step();
    button(h, a, true);
    h.step();
    for i in 1..=4 {
        h.event(Event::PointerMoved(a + (b - a) * (i as f32 / 4.0)));
        h.step();
    }
    button(h, b, false);
    h.step();
}

/// The document revision the canvas image was last painted from.
fn shown_revision(h: &Harness<'_, PhotocraftApp>) -> u64 {
    let app = h.state();
    let id = app.session.active().unwrap().doc.id;
    app.canvases.iter().filter(|(key, _)| key.0 == id).map(|(_, cache)| cache.tex_revision).max().unwrap()
}

/// Where the last frame drew the selection's marching ants: the screen rectangle around them
/// (they are one black-and-white mesh), or `None` when it drew none.
fn selection_ants(h: &Harness<'_, PhotocraftApp>) -> Option<Rect> {
    let mut around: Option<Rect> = None;
    for clipped in &h.output().shapes {
        let egui::Shape::Mesh(mesh) = &clipped.shape else { continue };
        let ants = mesh.texture_id == egui::TextureId::default()
            && !mesh.vertices.is_empty()
            && mesh.vertices.iter().all(|v| v.color == Color32::BLACK || v.color == Color32::WHITE);
        if !ants {
            continue;
        }
        for v in &mesh.vertices {
            let point = Rect::from_min_max(v.pos, v.pos);
            around = match around {
                Some(r) => Some(r.union(point)),
                None => Some(point),
            };
        }
    }
    around
}

/// The x of every vertical guide the last frame drew (in the colour Preferences › Guides, Grid &
/// Slices gives guides).
fn vertical_guides_drawn(h: &Harness<'_, PhotocraftApp>) -> Vec<f32> {
    let hex = &h.state().session.prefs().guides_grid_and_slices.guide_color;
    let [r, g, b] = photocraft_engine::prefs::parse_hex(hex).unwrap();
    let guide = Color32::from_rgb(r, g, b);
    let mut xs = Vec::new();
    for clipped in &h.output().shapes {
        if let egui::Shape::LineSegment { points: [a, b], stroke } = &clipped.shape
            && stroke.color == guide
            && a.x == b.x
        {
            xs.push(a.x.round());
        }
    }
    xs
}

#[test]
fn a_released_guide_is_drawn_where_it_was_dropped() {
    let mut app = app(Tool::Move);
    app.run("view.newGuide", json!({"orientation": "vertical", "position": 16})).unwrap();
    let mut h = harness(app);
    assert_eq!(vertical_guides_drawn(&h), vec![at(&h, [16.0, 32.0]).x.round()], "the guide before the drag");
    drag(&mut h, [16.0, 32.0], [48.0, 32.0]);
    assert_eq!(h.state().session.active().unwrap().doc.guides.vertical, vec![48.0], "the release moved the guide");
    let dropped = at(&h, [48.0, 32.0]).x.round();
    assert_eq!(vertical_guides_drawn(&h), vec![dropped], "the release frame draws the guide where it was dropped, only there, not back at x 16");
}

#[test]
fn a_released_rectangle_is_on_screen_in_the_frame_that_makes_it() {
    let mut h = harness(app(Tool::Rectangle));
    let layers = h.state().session.active().unwrap().doc.layers.len();
    drag(&mut h, [10.0, 10.0], [40.0, 30.0]);
    let st = h.state().session.active().unwrap();
    assert_eq!(st.doc.layers.len(), layers + 1, "the release made a shape layer");
    assert_eq!(shown_revision(&h), st.revision, "the release frame paints the document with the new rectangle, not the one from before the release");
}

#[test]
fn a_released_marquee_shows_its_selection_in_the_frame_that_makes_it() {
    let mut h = harness(app(Tool::RectMarquee));
    // A first selection, then a second one elsewhere that replaces it.
    for (from, to) in [([8.0, 8.0], [24.0, 24.0]), ([36.0, 36.0], [56.0, 56.0])] {
        drag(&mut h, from, to);
        let want = Rect::from_min_max(at(&h, from), at(&h, to));
        let Some(drawn) = selection_ants(&h) else { panic!("the release frame draws no marching ants for the selection from {from:?} to {to:?}") };
        // The ants are 1 px quads along the outline, so they reach about a pixel past it.
        let close = (drawn.min - want.min).length() < 2.0 && (drawn.max - want.max).length() < 2.0;
        assert!(close, "the release frame draws the ants around {drawn:?}, not the new selection {want:?}");
    }
}

/// A crop committed by double-clicking inside its frame resizes the document during the frame's
/// input. That frame already shows the cropped document, centred as the next frame would show it.
#[test]
fn a_crop_committed_by_double_click_is_shown_in_the_frame_that_commits_it() {
    let mut app = app(Tool::Crop);
    app.ui.crop_rect = Some([8.0, 8.0, 40.0, 40.0]);
    let mut h = harness(app);
    let p = at(&h, [24.0, 24.0]);
    h.event(Event::PointerMoved(p));
    h.step();
    // Two clicks, a frame for each press and release: egui reports the second as a double-click,
    // and the last frame is the one that commits.
    for pressed in [true, false, true, false] {
        button(&mut h, p, pressed);
        h.step();
    }
    let st = h.state().session.active().unwrap();
    assert_eq!([st.doc.size.width, st.doc.size.height], [32, 32], "the double-click cropped the document");
    assert_eq!(shown_revision(&h), st.revision, "that frame paints the cropped document");
    assert_eq!(h.state().ui.views[0].center, [16.0, 16.0], "and centres it");
}
