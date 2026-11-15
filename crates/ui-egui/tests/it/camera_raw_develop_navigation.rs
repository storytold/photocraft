//! The develop view's Zoom and Hand tools beside its local tools, and the dialog's own keys (one
//! keymap, `CAMERA_RAW`), driven with real key and pointer events: Z takes the preview from the
//! Masking brush until K (or M) picks it again, a click with the Zoom tool zooms in a step at the
//! pointer and ⌥-click out, a drag scrubs the zoom, H pans, Space pans for as long as it is held,
//! the toolbar's buttons do as Z and H do and their double-click is 100 % (Zoom) or Fit (Hand), Esc
//! puts the Zoom down before it counts as Cancel; ⌘Z ⇧⌘Z, ⌘0 ⌥⌘0 ⌘+ ⌘−, Y, U O S work in the develop
//! view and none while a mask's name is being typed. On the Mac the native menu's key equivalents
//! of those keys stay live while the dialog is open, and reach it.
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_ui_egui::{PhotocraftApp, Services, camera_raw_ui, theme::ThemeKind};
use serde_json::{Value, json};

fn dng() -> Vec<u8> {
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};
    let (w, h) = (96, 64);
    let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 30000));
    spec.as_shot_neutral = Some([0.5, 1.0, 0.7]);
    spec.build()
}

fn inspect(h: &Harness<'_, PhotocraftApp>) -> Value {
    photocraft_ui_egui::control::inspect(h.state(), &h.ctx)["cameraRaw"].clone()
}

fn settle(h: &mut Harness<'_, PhotocraftApp>) {
    for _ in 0..600 {
        h.run_steps(1);
        let r = inspect(h);
        if r.is_null() || r["previewPending"] == false && r["detailPending"] == false {
            h.run_steps(2);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn ui(h: &mut Harness<'_, PhotocraftApp>, v: Value) -> Value {
    let ctx = h.ctx.clone();
    let r = camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({ "ui": v })).unwrap().unwrap();
    settle(h);
    r
}

fn key(h: &mut Harness<'_, PhotocraftApp>, key: Key, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    for pressed in [true, false] {
        h.event(Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers });
    }
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

fn button(h: &mut Harness<'_, PhotocraftApp>, pos: Pos2, pressed: bool, modifiers: Modifiers) {
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers });
}

fn preview_rect(h: &Harness<'_, PhotocraftApp>) -> egui::Rect {
    let r = &inspect(h)["previewRect"];
    let f = |i: usize| r[i].as_f64().unwrap() as f32;
    egui::Rect::from_min_max(egui::pos2(f(0), f(1)), egui::pos2(f(2), f(3)))
}

fn viewport(h: &Harness<'_, PhotocraftApp>) -> egui::Rect {
    let r = &inspect(h)["viewportRect"];
    let f = |i: usize| r[i].as_f64().unwrap() as f32;
    egui::Rect::from_min_max(egui::pos2(f(0), f(1)), egui::pos2(f(2), f(3)))
}

/// A drag across the viewport from `from` to `to` (fractions of it).
fn drag(h: &mut Harness<'_, PhotocraftApp>, from: egui::Vec2, to: egui::Vec2) {
    let r = viewport(h);
    let (a, b) = (r.lerp_inside(from), r.lerp_inside(to));
    h.event(Event::PointerMoved(a));
    h.run_steps(1);
    button(h, a, true, Modifiers::NONE);
    h.run_steps(1);
    for k in 1..=8 {
        h.event(Event::PointerMoved(a + (b - a) * (k as f32 / 8.0)));
        h.run_steps(1);
    }
    button(h, b, false, Modifiers::NONE);
    settle(h);
}

fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(at));
    h.run_steps(1);
    button(h, at, true, modifiers);
    h.run_steps(1);
    button(h, at, false, modifiers);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    // (apart, so the next click is no double-click)
    for _ in 0..40 {
        h.run_steps(1);
    }
    settle(h);
}

/// Two clicks at `at` in quick succession.
fn double_click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    h.run_steps(1);
    for _ in 0..2 {
        button(h, at, true, Modifiers::NONE);
        h.run_steps(1);
        button(h, at, false, Modifiers::NONE);
        h.run_steps(1);
    }
    h.run_steps(2);
}

fn zoom(h: &Harness<'_, PhotocraftApp>) -> f64 {
    inspect(h)["zoom"].as_f64().unwrap()
}

fn strokes(h: &Harness<'_, PhotocraftApp>) -> usize {
    inspect(h)["develop"]["masks"].as_array().map_or(0, |m| m.iter().map(|m| m["components"][0]["shape"]["strokes"].as_array().map_or(0, Vec::len)).sum())
}

fn dialog() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1600.0, 1000.0)).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let services = Services {
            import: Some(Box::new(|name: &str, b: &[u8], opts: &photocraft_io::ImportOptions| {
                photocraft_io::import_with_options(name, b, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.open_bytes("IMG_0001.dng", &dng()).unwrap();
        app
    });
    h.ctx.all_styles_mut(|s| s.animation_time = 0.0);
    settle(&mut h);
    assert_eq!(inspect(&h)["panel"], "develop");
    h
}

/// Z, H, Space and the toolbar take the preview from the Masking brush; K or M give it back;
/// Esc puts the Zoom down first.
#[test]
fn zoom_and_hand_take_the_preview_from_the_brush_until_it_is_picked_again() {
    let mut h = dialog();
    key(&mut h, Key::K, Modifiers::NONE);
    assert_eq!(inspect(&h)["brush"]["armed"], true);
    let fit = zoom(&h);
    // Z: a drag scrubs the zoom (right: in) and paints nothing
    key(&mut h, Key::Z, Modifiers::NONE);
    let r = inspect(&h);
    assert_eq!((r["toolPaused"].as_bool(), r["tool"].as_str()), (Some(true), Some("masking")), "Z puts the brush on hold");
    assert_eq!(r["view"]["zoomTool"], true);
    drag(&mut h, vec2(0.4, 0.5), vec2(0.6, 0.5));
    assert!(zoom(&h) > fit * 1.2, "a drag with the Zoom tool zooms in: {} from {fit}", zoom(&h));
    assert_eq!(strokes(&h), 0, "and paints nothing");
    // a click zooms in a step at the pointer, ⌥-click out
    ui(&mut h, json!({"view": {"zoom": 1.0}}));
    let fit = zoom(&h);
    let at = preview_rect(&h).lerp_inside(vec2(0.4, 0.45));
    click(&mut h, at, Modifiers::NONE);
    let z1 = zoom(&h);
    assert!(z1 > fit, "a click zooms in: {z1} from {fit}");
    click(&mut h, at, Modifiers::NONE);
    assert!(zoom(&h) > z1, "and again: {} from {z1}", zoom(&h));
    let z2 = zoom(&h);
    click(&mut h, at, Modifiers::ALT);
    assert!(zoom(&h) < z2, "⌥-click zooms out");
    // K gives the brush back: a drag paints
    ui(&mut h, json!({"view": {"zoom": "fit"}}));
    key(&mut h, Key::K, Modifiers::NONE);
    let r = inspect(&h);
    assert_eq!((r["toolPaused"].as_bool(), r["tool"].as_str()), (Some(false), Some("masking")), "K picks the brush again, not Edit");
    drag(&mut h, vec2(0.3, 0.3), vec2(0.5, 0.35));
    assert_eq!(strokes(&h), 1, "the brush paints again");
    // H: a drag pans and paints nothing; M gives the brush back
    ui(&mut h, json!({"view": {"zoom": 16.0}}));
    key(&mut h, Key::H, Modifiers::NONE);
    assert_eq!(inspect(&h)["toolPaused"], true);
    let center = inspect(&h)["view"]["center"].clone();
    drag(&mut h, vec2(0.6, 0.6), vec2(0.4, 0.4));
    assert_ne!(inspect(&h)["view"]["center"], center, "the Hand pans");
    assert_eq!(strokes(&h), 1);
    key(&mut h, Key::M, Modifiers::NONE);
    assert_eq!(inspect(&h)["toolPaused"], false);
    // Space held pans over the live brush, painting nothing
    let center = inspect(&h)["view"]["center"].clone();
    h.event(Event::Key { key: Key::Space, physical_key: Some(Key::Space), pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    drag(&mut h, vec2(0.4, 0.4), vec2(0.55, 0.6));
    h.event(Event::Key { key: Key::Space, physical_key: Some(Key::Space), pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_ne!(inspect(&h)["view"]["center"], center, "Space pans");
    assert_eq!(strokes(&h), 1, "Space-dragging paints nothing");
    assert_eq!(inspect(&h)["toolPaused"], false, "and leaves the brush up");
    // the toolbar's Zoom button does as Z does; its double-click is 100 %, the Hand's Fit
    h.get_by_label("Zoom Tool").click();
    h.run_steps(2);
    assert_eq!(inspect(&h)["toolPaused"], true, "the Zoom button takes the preview from the brush");
    let at = h.get_by_label("Zoom Tool").rect().center();
    double_click(&mut h, at);
    let z = inspect(&h)["view"]["zoom"].as_f64().unwrap_or(0.0);
    assert!((z - 1.0).abs() < 1e-6, "double-clicking the Zoom tool is 100 %: {}", inspect(&h)["view"]);
    for _ in 0..40 {
        h.run_steps(1);
    }
    let at = h.get_by_label("Hand Tool").rect().center();
    double_click(&mut h, at);
    assert!(inspect(&h)["view"]["zoom"].is_null(), "double-clicking the Hand is Fit: {}", inspect(&h)["view"]);
    // Esc puts the Hand / Zoom down (the brush comes back) before it is Cancel
    assert_eq!(inspect(&h)["toolPaused"], true);
    key(&mut h, Key::Escape, Modifiers::NONE);
    let r = inspect(&h);
    assert!(r.is_object(), "Esc didn't close the dialog");
    assert_eq!((r["toolPaused"].as_bool(), r["tool"].as_str()), (Some(false), Some("masking")));
}

/// The dialog's own keys work in the develop view from its one keymap: undo and redo, the zoom keys,
/// before / after, the clipping warnings and the sampler; O is the mask overlay's while masking; and
/// none of them fire while a mask's name is typed (nor does Esc close the dialog then).
#[test]
fn the_dialogs_own_keys_work_in_the_develop_view() {
    let mut h = dialog();
    let shortcuts = inspect(&h)["shortcuts"].clone();
    for (id, chord) in [
        ("edit.undo", "Cmd+Z"),
        ("edit.redo", "Cmd+Shift+Z"),
        ("view.fit", "Cmd+0"),
        ("view.actualPixels", "Cmd+Alt+0"),
        ("view.zoomIn", "Cmd+="),
        ("view.zoomOut", "Cmd+-"),
        ("view.hand", "H"),
        ("view.zoom", "Z"),
        ("view.beforeAfter", "Y"),
        ("view.shadowClipping", "U"),
        ("view.highlightClipping", "O"),
        ("view.colorSampler", "S"),
        ("filmstrip.selectAll", "Cmd+A"),
        ("dialog.cancel", "Escape"),
        ("mask.overlay", "O"),
    ] {
        assert_eq!(shortcuts[id], chord, "{id} in the keymap");
    }
    // undo / redo of an edit
    ui(&mut h, json!({"develop": {"light": {"exposure": 1.0}}}));
    key(&mut h, Key::Z, Modifiers::COMMAND);
    assert_eq!(inspect(&h)["develop"]["light"]["exposure"], 0.0, "⌘Z undoes");
    key(&mut h, Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(inspect(&h)["develop"]["light"]["exposure"], 1.0, "⇧⌘Z redoes");
    // zoom keys
    key(&mut h, Key::Num0, Modifiers::COMMAND | Modifiers::ALT);
    assert_eq!(inspect(&h)["view"]["zoom"], 1.0, "⌥⌘0 is 100 %");
    key(&mut h, Key::Minus, Modifiers::COMMAND);
    assert!(inspect(&h)["view"]["zoom"].as_f64().unwrap() < 1.0, "⌘− zooms out");
    key(&mut h, Key::Equals, Modifiers::COMMAND);
    key(&mut h, Key::Plus, Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(inspect(&h)["view"]["zoom"].as_f64().unwrap() > 1.0, "⌘= and ⌘+ zoom in");
    key(&mut h, Key::Num0, Modifiers::COMMAND);
    assert!(inspect(&h)["view"]["zoom"].is_null(), "⌘0 fits");
    // before / after, clipping, sampler (Edit)
    key(&mut h, Key::Y, Modifiers::NONE);
    assert_eq!(inspect(&h)["before"], true, "Y shows before");
    key(&mut h, Key::Y, Modifiers::NONE);
    assert_eq!(inspect(&h)["before"], false);
    for (k, field) in [(Key::U, "shadows"), (Key::O, "highlights"), (Key::S, "samplerTool")] {
        key(&mut h, k, Modifiers::NONE);
        assert_eq!(inspect(&h)["scope"][field], true, "{field}");
        key(&mut h, k, Modifiers::NONE);
        assert_eq!(inspect(&h)["scope"][field], false, "{field} off");
    }
    // while masking O is the overlay's, not the highlight warning's
    key(&mut h, Key::K, Modifiers::NONE);
    let r = ui(&mut h, json!({"local": {"brush": {"points": [[0.2, 0.3], [0.4, 0.35]]}}}));
    assert_eq!(r["maskOverlay"], true);
    key(&mut h, Key::O, Modifiers::NONE);
    assert_eq!((inspect(&h)["maskOverlay"].as_bool(), inspect(&h)["scope"]["highlights"].as_bool()), (Some(false), Some(false)));
    // typing a mask's name: Y, O, Z and Esc are letters there (Esc leaves the field, not the dialog)
    let name = inspect(&h)["develop"]["masks"][0]["name"].as_str().unwrap().to_string();
    let at = h.get_by_label(&name).rect().center();
    double_click(&mut h, at);
    for k in [Key::Y, Key::Z] {
        key(&mut h, k, Modifiers::NONE);
    }
    let r = inspect(&h);
    assert_eq!((r["before"].as_bool(), r["toolPaused"].as_bool()), (Some(false), Some(false)), "the keys typed into the name");
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(inspect(&h).is_object(), "Esc in the name field leaves the dialog open");
}

/// On the Mac the menu bar takes ⌘ chords before a window sees them, and swallows the key of a
/// disabled item. While the dialog is open the items whose keys are the dialog's stay enabled, a
/// key equivalent comes back as the key press, and the dialog takes it; the others stay disabled,
/// and a click on a menu item still runs nothing under the dialog.
#[test]
fn the_mac_menu_bar_hands_the_dialogs_keys_to_it() {
    use photocraft_ui_egui::native_menu::{Backend, Chord, Event as MenuEvent, MenuBar, NativeMenu};
    use std::{cell::RefCell, rc::Rc};
    struct Menu {
        synced: Rc<RefCell<Vec<MenuBar>>>,
        events: Rc<RefCell<Vec<MenuEvent>>>,
    }
    impl Backend for Menu {
        fn sync(&mut self, bar: &MenuBar) {
            self.synced.borrow_mut().push(bar.clone());
        }
        fn drain(&mut self) -> Vec<MenuEvent> {
            std::mem::take(&mut *self.events.borrow_mut())
        }
    }
    let synced = Rc::new(RefCell::new(Vec::new()));
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut h = dialog();
    h.state_mut().services.native_menu = Some(NativeMenu::new(Box::new(Menu { synced: synced.clone(), events: events.clone() })));
    key(&mut h, Key::Y, Modifiers::NONE);
    key(&mut h, Key::Y, Modifiers::NONE);
    let enabled = |id: &str| synced.borrow().last().and_then(|bar| bar.find(id).map(|it| it.enabled));
    for id in ["edit.undo", "view.zoomIn", "view.zoomOut", "view.fitOnScreen", "select.all"] {
        assert_eq!(enabled(id), Some(true), "{id}'s key equivalent stays live under the dialog");
    }
    for id in ["file.save", "image.imageRotation.90cw"] {
        assert_eq!(enabled(id), Some(false), "{id} is disabled under the dialog");
    }
    // AppKit chose Undo by its key equivalent: the dialog undoes its own edit
    ui(&mut h, json!({"develop": {"light": {"exposure": 1.0}}}));
    let document = h.state().session.active().map(|s| s.history.past_len());
    let chosen = |h: &mut Harness<'_, PhotocraftApp>, ev: MenuEvent| {
        events.borrow_mut().push(ev);
        let mut raw = egui::RawInput::default();
        if let Some(menu) = h.state_mut().services.native_menu.as_mut() {
            menu.raw_input(&mut raw);
        }
        for e in raw.events {
            h.event(e);
        }
        h.run_steps(2);
    };
    chosen(&mut h, MenuEvent::Key(Chord::parse("Cmd+Z").unwrap()));
    assert_eq!(inspect(&h)["develop"]["light"]["exposure"], 0.0, "⌘Z from the menu bar undoes in the dialog");
    assert_eq!(h.state().session.active().map(|s| s.history.past_len()), document, "not the document");
    chosen(&mut h, MenuEvent::Click("edit.undo".into()));
    assert_eq!(h.state().session.active().map(|s| s.history.past_len()), document, "a menu click runs nothing under the dialog");
}
