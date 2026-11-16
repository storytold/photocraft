//! #11794: a note's icon is 20 × 20 screen points at every zoom, its top-left on the note's
//! position, and the Note tool's click picks a note anywhere on that icon (edges included) and
//! makes a new note just outside it. The Note tool's pointer is a note sheet whose hotspot is its
//! top-left corner.

use egui::{Modifiers, Pos2, Rect, vec2};
use egui_kittest::Harness;
use serde_json::json;

use super::{NOTE_ICON, note_at, note_icon_rect};
use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::state::Tool;

/// The canvas alone at 100 % on a 400 × 300 document, with `tool` active (as the cursor tests in
/// `canvas::cursors` build it).
fn harness(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = tool;
    let mut h = Harness::builder().with_size(vec2(900.0, 600.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [200.0, 150.0];
    v.fit_pending = false;
    h.run_steps(2);
    h
}

fn has_fonts(h: &Harness<'static, PhotocraftApp>) -> bool {
    h.ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into())))
}

/// Hovers the canvas's centre (on the document).
fn hover(h: &mut Harness<'static, PhotocraftApp>) {
    let p = h.state().last_canvas_rect.center();
    h.hover_at(p);
    h.run_steps(2);
}

/// The OS cursor bitmap's size, hotspot and alpha channel.
fn bitmap(h: &Harness<'static, PhotocraftApp>) -> Option<([u16; 2], [u16; 2], Vec<u8>)> {
    h.output().platform_output.cursor_image.as_ref().map(|i| (i.size, i.hotspot, i.rgba.as_chunks::<4>().0.iter().map(|p| p[3]).collect()))
}

fn xform(zoom: f32) -> ViewXform {
    ViewXform { rect: Rect::from_min_size(Pos2::new(30.0, 40.0), vec2(800.0, 600.0)), zoom, center: [300.0, 200.0], flip: false, rotation: 0.0, aspect: 1.0 }
}

/// A 600 × 400 document with one note at 100,100, the Note tool active and the view at `zoom`.
fn note_app(zoom: f32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    app.run("notes.add", json!({"x": 100, "y": 100, "text": "first"})).unwrap();
    app.sync_views();
    app.ui.views[0].zoom = zoom;
    app.ui.tool = Tool::Note;
    app.ui.analysis.note_selected = None;
    app
}

fn click(app: &mut PhotocraftApp, [x, y]: [f64; 2]) {
    crate::canvas::tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, Modifiers::NONE);
    crate::canvas::tool_event(app, ToolEvent::Up { x, y }, Modifiers::NONE);
}

fn notes(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.notes.len()
}

#[test]
fn the_icon_is_twenty_points_square_at_every_zoom() {
    for zoom in [1.0, 4.0, 0.25] {
        let xf = xform(zoom);
        let r = note_icon_rect(&xf, [100.0, 50.0]);
        assert_eq!(r.min, xf.to_screen(100.0, 50.0), "zoom {zoom}: the icon's top-left is the note's position");
        assert_eq!(r.size(), vec2(20.0, 20.0), "zoom {zoom}");
    }
    assert_eq!(NOTE_ICON, 20.0);
}

#[test]
fn the_hit_test_covers_the_drawn_icon_and_its_edges() {
    let app = note_app(1.0);
    let n = &app.session.active().unwrap().doc.notes;
    for zoom in [1.0, 4.0] {
        let side = 20.0 / zoom;
        for p in [[100.0, 100.0], [100.0 + side, 100.0], [100.0, 100.0 + side], [100.0 + side, 100.0 + side], [100.0 + side * 0.9, 100.0 + side * 0.5]] {
            assert_eq!(note_at(n, p, zoom), Some(0), "zoom {zoom}: {p:?} is on the icon");
        }
        let past = 0.5 / zoom;
        for p in [[100.0 - past, 101.0], [100.0 + side + past, 101.0], [101.0, 100.0 + side + past]] {
            assert_eq!(note_at(n, p, zoom), None, "zoom {zoom}: {p:?} is just outside the icon");
        }
    }
}

#[test]
fn a_click_on_the_icon_selects_the_note_and_just_outside_it_makes_a_new_one() {
    for zoom in [1.0, 4.0] {
        let side = 20.0 / f64::from(zoom);
        // The icon's right and bottom edges: a 16-point-wide box missed the right part of the icon.
        let mut app = note_app(zoom);
        click(&mut app, [100.0 + side * 0.95, 100.0 + side * 0.95]);
        assert_eq!(notes(&app), 1, "zoom {zoom}: a click on the icon's lower right makes no note");
        assert_eq!(app.ui.analysis.note_selected, Some(0), "zoom {zoom}: it selects the note");
        // Just past the right edge: a new note.
        let mut app = note_app(zoom);
        click(&mut app, [100.0 + side + 1.0 / f64::from(zoom), 105.0]);
        assert_eq!(notes(&app), 2, "zoom {zoom}: a click just outside the icon makes a new note");
    }
}

#[test]
fn the_note_tool_shows_a_note_sheet_with_its_hotspot_top_left() {
    let rings = crate::tool_cursor::note_sheet_rings();
    let pts: Vec<_> = rings.iter().flatten().collect();
    assert!(pts.iter().any(|p| **p == egui::Vec2::ZERO), "the sheet's corner is on the hotspot");
    assert!(pts.iter().all(|p| p.x >= 0.0 && p.y >= 0.0), "the whole sheet is right of and below the hotspot");

    let mut h = harness(Tool::Note);
    if !has_fonts(&h) {
        return;
    }
    hover(&mut h);
    let (size, hotspot, alpha) = bitmap(&h).expect("the Note tool's pointer is an OS cursor bitmap");
    let w = usize::from(size[0]);
    let (mut lo, mut hi) = ([usize::MAX; 2], [0; 2]);
    for (i, _) in alpha.iter().enumerate().filter(|(_, a)| **a > 0) {
        for (k, v) in [i % w, i / w].into_iter().enumerate() {
            lo[k] = lo[k].min(v);
            hi[k] = hi[k].max(v);
        }
    }
    let scale = crate::tool_cursor::native_scale(&h.ctx);
    for k in 0..2 {
        let at = usize::from(hotspot[k]);
        assert!(at.abs_diff(lo[k]) as f32 <= 2.0 * scale, "axis {k}: the hotspot {at} is the sheet's top-left edge {}", lo[k]);
        assert!((hi[k] - at) as f32 >= 10.0 * scale, "axis {k}: the sheet lies right of and below the hotspot");
    }
}

/// CodeRabbit on #14783: the icon is a screen-space square at the note's screen position, so the
/// hit test works there: flipped, rotated and with a non-unit aspect a click on the drawn icon
/// picks the note and a click just off it does not.
#[test]
fn the_hit_test_follows_the_drawn_icon_when_the_view_is_flipped_rotated_or_stretched() {
    let app = note_app(1.0);
    let n = &app.session.active().unwrap().doc.notes;
    for (flip, rotation, aspect) in [(true, 0.0, 1.0), (false, 30.0, 1.0), (false, 0.0, 1.5), (true, 30.0, 1.5)] {
        let xf = ViewXform { flip, rotation, aspect, ..xform(2.0) };
        let r = note_icon_rect(&xf, n[0].position);
        let doc = |p: Pos2| xf.to_doc(p);
        for p in
            [r.center(), r.left_top() + vec2(1.0, 1.0), r.right_bottom() - vec2(1.0, 1.0), r.right_top() + vec2(-1.0, 1.0), r.left_bottom() + vec2(1.0, -1.0)]
        {
            assert_eq!(super::note_at_screen(n, &xf, doc(p)), Some(0), "flip {flip}, rotation {rotation}, aspect {aspect}: {p:?} is on the icon");
        }
        for p in [r.left_top() - vec2(2.0, 0.0), r.right_top() + vec2(2.0, 0.0), r.right_bottom() + vec2(0.0, 2.0), r.left_bottom() - vec2(0.0, -2.0)] {
            assert_eq!(super::note_at_screen(n, &xf, doc(p)), None, "flip {flip}, rotation {rotation}, aspect {aspect}: {p:?} is off the icon");
        }
    }
}

/// CodeRabbit on #14783: during an animated zoom the icon is drawn at the displayed zoom, so the
/// click is tested at that zoom too: shown at half size, the zoomed-out icon covers 40 document
/// pixels and a click 30 px along picks the note instead of making a new one.
#[test]
fn a_click_during_an_animated_zoom_uses_the_displayed_zoom() {
    let mut app = note_app(1.0);
    let center = app.ui.views[0].center;
    app.ui.views[0].zoom_animating = true;
    app.ui.views[0].display_camera = Some((0.5, center));
    click(&mut app, [130.0, 105.0]);
    assert_eq!(notes(&app), 1, "no new note: the click is on the icon as shown");
    assert_eq!(app.ui.analysis.note_selected, Some(0));
}

/// CodeRabbit on #14783: Preferences › Cursors › Other Cursors Precise keeps the plain crosshair
/// for the Note tool, as before this change, and so does Caps Lock (not measured): its pointer is
/// the marquee's.
#[test]
fn precise_other_cursors_and_caps_lock_keep_the_crosshair_for_the_note_tool() {
    for caps in [false, true] {
        let (mut note, mut marquee) = (harness(Tool::Note), harness(Tool::RectMarquee));
        if !has_fonts(&note) {
            return;
        }
        for h in [&mut note, &mut marquee] {
            if caps {
                h.state_mut().caps_lock = true;
            } else {
                h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": "precise"})).unwrap();
            }
            h.run_steps(2);
            hover(h);
        }
        assert_eq!(note.output().platform_output.cursor_icon, egui::CursorIcon::Crosshair, "caps {caps}");
        assert_eq!(bitmap(&note), bitmap(&marquee), "caps {caps}: the marquee's crosshair, not the note sheet");
    }
}

/// CodeRabbit on #14783: with the view flipped, a click on the visible icon picks the note (a
/// document-space box made a duplicate note there).
#[test]
fn a_click_on_the_icon_in_a_flipped_view_selects_the_note() {
    let mut app = note_app(1.0);
    app.ui.view.flip_horizontal = true;
    let xf = ViewXform::active(&app).expect("the view's mapping");
    let r = note_icon_rect(&xf, app.session.active().unwrap().doc.notes[0].position);
    click(&mut app, xf.to_doc(r.center()));
    assert_eq!(notes(&app), 1, "no new note on the icon");
    assert_eq!(app.ui.analysis.note_selected, Some(0));
}
