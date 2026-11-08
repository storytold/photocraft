//! The open-time Camera Raw dialog's filmstrip through the real app shell: clicks with Shift and
//! ⌘ select raws, ⌘A selects all of them and never the document behind the dialog.
use egui::{Key, Modifiers, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_ui_egui::{PhotocraftApp, Services, theme::ThemeKind};
use serde_json::json;

const NAMES: [&str; 3] = ["IMG_0.dng", "IMG_1.dng", "IMG_2.dng"];

/// The app with the real importer, three synthetic raws opened together and the dialog up.
fn fixture() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1200.0, 800.0)).build_eframe(|cc| {
        use photocraft_raw::testgen::{DngSpec, mosaic, scene};
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let services = Services {
            import: Some(Box::new(|name: &str, b: &[u8], opts: &photocraft_io::ImportOptions| {
                photocraft_io::import_with_options(name, b, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let (w, h) = (48, 32);
        let dng = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000)).build();
        for name in NAMES {
            app.open_bytes(name, &dng).unwrap();
        }
        app
    });
    h.run_steps(4);
    h
}

fn selected(h: &Harness<'_, PhotocraftApp>) -> Vec<bool> {
    NAMES.iter().map(|n| h.get_by_label(n).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)).collect()
}

fn no_document_selection(h: &Harness<'_, PhotocraftApp>) -> bool {
    h.state().session.documents().iter().all(|st| st.doc.selection.is_none())
}

#[test]
fn filmstrip_clicks_and_command_a_select_raws_not_the_document() {
    let mut h = fixture();
    assert_eq!(selected(&h), [true, false, false], "the shown raw is selected");
    h.get_by_label(NAMES[2]).click_modifiers(Modifiers::SHIFT);
    h.run_steps(3);
    assert_eq!(selected(&h), [true, true, true], "Shift-click selects the range");
    h.get_by_label(NAMES[1]).click_modifiers(Modifiers::COMMAND);
    h.run_steps(3);
    assert_eq!(selected(&h), [true, false, true], "⌘-click takes one out");
    h.get_by_label(NAMES[1]).click();
    h.run_steps(3);
    assert_eq!(selected(&h), [false, true, false], "a click selects only that raw");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(3);
    assert_eq!(selected(&h), [true, true, true], "⌘A selects every raw in the filmstrip");
    assert!(no_document_selection(&h), "and nothing in the document behind the dialog");
    // Without the dialog, ⌘A is the document's Select All.
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().session.documents().is_empty(), "Cancel opened none of the raws");
    h.state_mut().run("file.new", json!({"width": 16, "height": 16})).unwrap();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    assert!(!no_document_selection(&h));
}

/// Mean brightness of the thumbnail in raw `name`'s tile, as the window shows it.
fn tile_brightness(h: &mut Harness<'_, PhotocraftApp>, name: &str) -> f32 {
    let tile = h.get_by_label(name).rect();
    let ppp = h.ctx.pixels_per_point();
    let image = h.render().unwrap();
    // The thumbnail's middle, clear of the name, the outline and the edited dot.
    let inner = egui::Rect::from_center_size(tile.center() - vec2(0.0, 12.0), vec2(tile.width() * 0.5, tile.height() * 0.4));
    let (x0, y0, x1, y1) = ((inner.left() * ppp) as u32, (inner.top() * ppp) as u32, (inner.right() * ppp) as u32, (inner.bottom() * ppp) as u32);
    let mut sum = 0.0;
    let mut n = 0.0;
    for y in y0..y1 {
        for x in x0..x1 {
            let p = image.get_pixel(x, y);
            sum += (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0;
            n += 1.0;
        }
    }
    sum / n
}

/// A slider dragged in the dialog with several raws selected redraws the other selected raws'
/// thumbnails once the drag ends, without further input, and as the preview would show them.
#[test]
fn a_dragged_slider_redraws_the_other_selected_thumbnails() {
    let mut h = fixture();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(3);
    let (before_other, before_unselected) = (tile_brightness(&mut h, NAMES[1]), tile_brightness(&mut h, NAMES[2]));
    h.get_by_label(NAMES[2]).click_modifiers(Modifiers::COMMAND);
    h.run_steps(3);
    // Exposure dragged well down with real pointer events.
    let label = photocraft_ui_egui::i18n::tr_ctx(photocraft_ui_egui::i18n::current(), "cameraRaw", "Exposure");
    let track = h.get_by_role_and_label(egui::accesskit::Role::Slider, label).rect();
    let at = egui::pos2(track.center().x, track.center().y);
    h.event(egui::Event::PointerMoved(at));
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(2);
    for n in 1..=6 {
        h.event(egui::Event::PointerMoved(at - vec2(15.0 * n as f32, 0.0)));
        h.run_steps(1);
    }
    let end = at - vec2(90.0, 0.0);
    h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    // Only the frames egui asks for: nothing else wakes the window after the release. The redraw
    // runs on a worker, and until it is shown the dialog asks for the next frame.
    h.run_steps(1);
    for _ in 0..1000 {
        let pending = photocraft_ui_egui::control::inspect(h.state(), &h.ctx)["cameraRaw"]["filmstrip"][1]["thumbnailPending"] == true;
        if !pending {
            break;
        }
        let delay = h.output().viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay);
        assert!(delay.is_some_and(|d| d <= std::time::Duration::from_millis(50)), "a pending thumbnail keeps frames coming: {delay:?}");
        std::thread::sleep(std::time::Duration::from_millis(5));
        h.run_steps(1);
    }
    // One more frame loads the redrawn thumbnail's texture.
    h.run_steps(1);
    let exposure = photocraft_ui_egui::control::inspect(h.state(), &h.ctx)["cameraRaw"]["params"]["exposure"].as_f64().unwrap();
    assert!(exposure < -0.5, "the drag lowered Exposure: {exposure}");
    let film = photocraft_ui_egui::control::inspect(h.state(), &h.ctx)["cameraRaw"]["filmstrip"].clone();
    assert_eq!(film[1]["edited"], true, "{film}");
    assert_eq!(film[1]["thumbnailPending"], false, "{film}");
    let (after_other, after_unselected) = (tile_brightness(&mut h, NAMES[1]), tile_brightness(&mut h, NAMES[2]));
    assert!(after_other < before_other - 10.0, "the selected raw's thumbnail darkened: {before_other} → {after_other}");
    assert!((after_unselected - before_unselected).abs() < 1.0, "the unselected one didn't: {before_unselected} → {after_unselected}");
}

/// The shown raw's own tile follows an edit too, as Camera Raw's does: once the drag ends and the
/// preview has landed, without further input.
#[test]
fn the_shown_raws_tile_follows_its_edit() {
    let mut h = fixture();
    h.run_steps(3);
    let (before_shown, before_other) = (tile_brightness(&mut h, NAMES[0]), tile_brightness(&mut h, NAMES[1]));
    let label = photocraft_ui_egui::i18n::tr_ctx(photocraft_ui_egui::i18n::current(), "cameraRaw", "Exposure");
    let track = h.get_by_role_and_label(egui::accesskit::Role::Slider, label).rect();
    let at = egui::pos2(track.center().x, track.center().y);
    h.event(egui::Event::PointerMoved(at));
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(2);
    for n in 1..=6 {
        h.event(egui::Event::PointerMoved(at + vec2(15.0 * n as f32, 0.0)));
        h.run_steps(1);
    }
    let end = at + vec2(90.0, 0.0);
    h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    // Only the frames the dialog asks for until the preview has landed and the tile shows it.
    h.run_steps(1);
    for _ in 0..1000 {
        let d = photocraft_ui_egui::control::inspect(h.state(), &h.ctx)["cameraRaw"].clone();
        if d["previewPending"] == false && d["filmstrip"][0]["thumbnailPending"] == false {
            break;
        }
        let delay = h.output().viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay);
        assert!(delay.is_some_and(|d| d <= std::time::Duration::from_millis(50)), "the dialog keeps frames coming: {delay:?}");
        std::thread::sleep(std::time::Duration::from_millis(5));
        h.run_steps(1);
    }
    h.run_steps(1);
    let exposure = photocraft_ui_egui::control::inspect(h.state(), &h.ctx)["cameraRaw"]["params"]["exposure"].as_f64().unwrap();
    assert!(exposure > 0.5, "the drag raised Exposure: {exposure}");
    let (after_shown, after_other) = (tile_brightness(&mut h, NAMES[0]), tile_brightness(&mut h, NAMES[1]));
    assert!(after_shown > before_shown + 10.0, "the shown raw's tile brightened: {before_shown} → {after_shown}");
    assert!((after_other - before_other).abs() < 1.0, "an unselected raw's didn't: {before_other} → {after_other}");
}
