//! Input routing through the real canvas, dialogs and shortcuts (#44, #45): keyboard zoom hits the
//! canvas (never egui's UI scale), and an open dialog keeps the canvas pannable and zoomable but
//! is not cancelled by a click outside it.

use egui::{Key, Modifiers, Pos2, pos2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;

fn harness() -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    // The whole app: its frame hook hands the keys, the wheel and the pointer to the framework's runtime.
    let ctx = app.ctx().clone();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_context(ctx).with_max_steps(64).build_eframe(move |_cc| app);
    h.run_steps(4);
    h
}

fn zoom(h: &Harness<'static, Box<PhotocraftApp>>) -> f32 {
    h.state().current_zoom()
}

/// The highest zoom the ladder allows.
fn max_zoom() -> f32 {
    craft::behavior::canvas::ZoomScheme::PHOTOSHOP.max as f32
}

/// The values of the dialog opened last, as automation lists them (`ui.inspect`).
fn dialog_fields(h: &Harness<'static, Box<PhotocraftApp>>) -> serde_json::Value {
    h.state().framework.dialogs.open_dialogs().into_iter().last().and_then(|d| d.fields).expect("an open dialog with values")
}

fn open_dialog(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    crate::dialogs::open_command_dialog(h.state_mut(), "image.adjustments.brightnessContrast", "Brightness/Contrast…").unwrap();
    h.run_steps(3);
    assert!(h.state().dialog_open());
}

/// A point on the canvas well away from the centred dialog.
fn free_canvas(h: &Harness<'static, Box<PhotocraftApp>>) -> Pos2 {
    let r = h.state().last_canvas_rect;
    pos2(r.left() + 60.0, r.bottom() - 60.0)
}

#[test]
fn clicking_outside_a_dialog_keeps_it_open_and_the_canvas_pans_and_zooms() {
    let mut h = harness();
    open_dialog(&mut h);
    let p = free_canvas(&h);
    h.hover_at(p);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert!(h.state().dialog_open(), "a click outside must not cancel the dialog");

    // Scrolling over the canvas pans it.
    let c0 = h.state().ui.views[0].center;
    h.event(egui::Event::MouseWheel {
        source: egui::MouseWheelSource::Unknown,
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -40.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(8);
    let c1 = h.state().ui.views[0].center;
    assert!(c1[1] > c0[1], "scroll pans under a dialog: {c0:?} -> {c1:?}");

    // Space-drag pans too.
    h.event(egui::Event::Key { key: Key::Space, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for i in 1..=5 {
        h.hover_at(p + vec2(10.0 * i as f32, 0.0));
        h.run_steps(1);
    }
    h.event(egui::Event::PointerButton { pos: p + vec2(50.0, 0.0), button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.event(egui::Event::Key { key: Key::Space, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    let c2 = h.state().ui.views[0].center;
    let z = zoom(&h);
    assert!((c2[0] - (c1[0] - 50.0 / z)).abs() < 0.5, "space-drag pans by 50 pt: {c1:?} -> {c2:?}");
    assert!(h.state().dialog_open());

    // Esc still cancels.
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(!h.state().dialog_open());
}

fn press(h: &mut Harness<'static, Box<PhotocraftApp>>, p: Pos2, pressed: bool) {
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

fn picker_color(h: &Harness<'static, Box<PhotocraftApp>>) -> String {
    dialog_fields(h)["color"].as_str().unwrap().to_string()
}

/// The Color Picker's eyedropper, as in Photoshop: a
/// click or drag there samples into the picker's new colour, whatever the tool. OK applies it.
#[test]
fn color_picker_samples_the_image_under_its_pipette() {
    let mut h = harness();
    h.state_mut().run("shape.create", json!({"kind": "rect", "rect": [0, 0, 200, 300], "fill": "#ff0000"})).unwrap();
    h.state_mut().run("shape.create", json!({"kind": "rect", "rect": [200, 0, 200, 300], "fill": "#00ff00"})).unwrap();
    // 400 %: the image covers the whole canvas, red on the left of the dialog, green on its right.
    let v = &mut h.state_mut().ui.views[0];
    (v.zoom, v.center) = (4.0, [200.0, 150.0]);
    v.clear_pending();
    crate::color_picker_ui::open(h.state_mut(), "foreground");
    h.run_steps(3);
    let r = h.state().last_canvas_rect;
    let (red, green) = (pos2(r.left() + 60.0, r.center().y), pos2(r.right() - 60.0, r.center().y));
    assert_eq!(doc_at(&h, red)[0] < 200.0, doc_at(&h, green)[0] > 200.0);
    let foreground = h.state().session.tools.foreground;

    // A click samples; the press and release may land in one frame (`ui.click`).
    h.hover_at(red);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: red, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    press(&mut h, red, false);
    assert_eq!(picker_color(&h), "#ff0000");
    assert_eq!(h.state().session.tools.foreground, foreground, "only OK sets the foreground");

    // A drag keeps sampling, skipping the dialog on the way.
    press(&mut h, red, true);
    for i in 1..=6 {
        h.hover_at(red + (green - red) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    press(&mut h, green, false);
    assert_eq!(picker_color(&h), "#00ff00");

    // Space-drag still pans instead of sampling.
    h.hover_at(red);
    h.run_steps(1);
    let c0 = h.state().ui.views[0].center;
    h.event(egui::Event::Key { key: Key::Space, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    press(&mut h, red, true);
    h.hover_at(red + vec2(40.0, 0.0));
    h.run_steps(1);
    press(&mut h, red + vec2(40.0, 0.0), false);
    h.event(egui::Event::Key { key: Key::Space, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    let c1 = h.state().ui.views[0].center;
    assert!((c1[0] - (c0[0] - 10.0)).abs() < 0.5, "space-drag pans by 40 pt at 400 %: {c0:?} -> {c1:?}");
    assert_eq!(picker_color(&h), "#00ff00");

    // With the Hand tool a click samples too (the tool doesn't matter); OK applies the sample.
    h.state_mut().ui.tool = crate::state::Tool::Hand;
    h.hover_at(red);
    h.run_steps(1);
    press(&mut h, red, true);
    press(&mut h, red, false);
    assert_eq!(picker_color(&h), "#ff0000");
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert!(!h.state().dialog_open());
    assert_eq!(h.state().session.tools.foreground, [1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn curves_picker_samples_document_coordinates_through_the_view_transform() {
    let mut h = harness();
    h.state_mut().run("select.rect", json!({"x": 0, "y": 0, "width": 200, "height": 300})).unwrap();
    h.state_mut().run("edit.fill", json!({"color": "#804020"})).unwrap();
    h.state_mut().run("select.rect", json!({"x": 200, "y": 0, "width": 200, "height": 300})).unwrap();
    h.state_mut().run("edit.fill", json!({"color": "#20a0e0"})).unwrap();
    h.state_mut().run("select.deselect", json!({})).unwrap();
    let committed = h.state().session.active().unwrap().doc.clone();
    let history = h.state().session.active().unwrap().history.past_len();
    let view = &mut h.state_mut().ui.views[0];
    (view.zoom, view.center) = (4.0, [200.0, 150.0]);
    view.clear_pending();
    h.state_mut().ui.view.flip_horizontal = true;
    let dialog = crate::adjust_dialog::open(h.state_mut(), "image.adjustments.curves").unwrap();
    // Arms the black-point eyedropper, as its button in the dialog does.
    h.state_mut()
        .framework
        .dialogs
        .get_mut_by_id::<crate::adjust_dialog::HostedAdjustDialog>(dialog)
        .expect("the Curves dialog is open")
        .set_picker(Some(craft::imaging::ui::adjust::TonePicker::Black));
    h.run_steps(3);
    let canvas = h.state().last_canvas_rect;
    let left = pos2(canvas.left() + 60.0, canvas.center().y);
    h.hover_at(left);
    h.run_steps(1);
    press(&mut h, left, true);
    press(&mut h, left, false);
    h.run_steps(3);
    assert!(std::sync::Arc::ptr_eq(&h.state().session.active().unwrap().doc, &committed));
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    assert!(crate::adjust_preview::display_doc(h.state_mut(), 0).is_some(), "the new curve is visible only through the preview");
    let red = dialog_fields(&h)["red"][0][0].as_f64().unwrap();
    assert!((red - 32.0).abs() < 0.6, "sampled document-right blue patch through the flipped 400% view: {red}");
}

/// A wheel event of `dy` lines with `modifiers` held.
fn wheel(h: &Harness<'static, Box<PhotocraftApp>>, dy: f32, modifiers: Modifiers) {
    h.event_modifiers(
        egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, dy),
            phase: egui::TouchPhase::Move,
            modifiers,
        },
        modifiers,
    );
}

/// The document point under screen point `p` (no flip).
fn doc_at(h: &Harness<'static, Box<PhotocraftApp>>, p: Pos2) -> [f32; 2] {
    let v = &h.state().ui.views[0];
    let c = h.state().last_canvas_rect.center();
    [v.center[0] + (p.x - c.x) / v.zoom, v.center[1] + (p.y - c.y) / v.zoom]
}

/// The whole window (status bar and Navigator included) on a 400 × 300 document.
fn app_window() -> Harness<'static, Box<PhotocraftApp>> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(cc.egui_ctx.clone(), s, crate::Services::default())
    });
    h.run_steps(8);
    h
}

/// Photoshop 25.4: ⌥ + wheel goes on past 3200 % to exactly 12800 %, and further notches there
/// change nothing. The status bar's zoom field used to stop at 3200 %, clamping the zoom back
/// every frame: past it, each notch zoomed around the pointer and the field pulled the zoom
/// back, so the image slid towards the pointer.
#[test]
fn the_wheel_reaches_12800_percent_and_stops_dead_there() {
    let mut h = app_window();
    {
        let v = &mut h.state_mut().ui.views[0];
        (v.zoom, v.center) = (40.0, [200.0, 150.0]);
        v.clear_pending();
    }
    h.run_steps(4);
    assert_eq!(zoom(&h), 40.0, "4000 % holds: no zoom control clamps it back");
    let r = h.state().last_canvas_rect;
    let p = pos2(r.center().x - 150.0, r.center().y - 90.0);
    // The document point under `p`, through the canvas view as its last pass showed it.
    let doc_at = |h: &Harness<'static, Box<PhotocraftApp>>, p: Pos2| {
        let d = crate::canvas::testing::view(h.state()).expect("the canvas has shown").to_doc(p);
        [d.x as f32, d.y as f32]
    };
    h.hover_at(p);
    h.run_steps(2);
    let d0 = doc_at(&h, p);
    // Frames after each notch (0.25 s each, the harness's default step): a notch lands within two (egui's wheel
    // smoothing reaches 90 % in 0.1 s and takes the rest under one point at once), and the old clamp pulled the zoom
    // back on every frame, so three show it: the zoom and centre
    // after them are the ones the old thirty left.
    const NOTCH_STEPS: usize = 3;
    for _ in 0..14 {
        wheel(&h, 1.0, Modifiers::ALT);
        h.run_steps(NOTCH_STEPS);
    }
    assert_eq!(zoom(&h), max_zoom(), "a notch past the limit lands on it exactly");
    let d1 = doc_at(&h, p);
    assert!((d1[0] - d0[0]).abs() < 0.02 && (d1[1] - d0[1]).abs() < 0.02, "zoomed around the pointer: {d0:?} -> {d1:?}");
    let c = h.state().ui.views[0].center;
    for _ in 0..4 {
        wheel(&h, 1.0, Modifiers::ALT);
        h.run_steps(NOTCH_STEPS);
    }
    assert_eq!(zoom(&h), max_zoom());
    assert_eq!(h.state().ui.views[0].center, c, "at the limit the image doesn't move");
    // One notch back: 12800 / 1.1, still around the pointer.
    wheel(&h, -1.0, Modifiers::ALT);
    h.run_steps(NOTCH_STEPS);
    assert!((zoom(&h) - max_zoom() / 1.1).abs() < 1e-2, "{}", zoom(&h));
    let d2 = doc_at(&h, p);
    assert!((d2[0] - d0[0]).abs() < 0.02 && (d2[1] - d0[1]).abs() < 0.02, "{d0:?} -> {d2:?}");
}

/// Animated Zoom on: notches flicked past 12800 % stop there, and the first notch back zooms out
/// at once by its ×1/1.1. The rest of the notches the limit swallowed must not eat it.
#[test]
fn the_first_animated_notch_back_from_the_limit_zooms_out() {
    let mut h = harness();
    {
        let v = &mut h.state_mut().ui.views[0];
        v.zoom = max_zoom() / 1.05;
        v.clear_pending();
    }
    let r = h.state().last_canvas_rect;
    h.hover_at(r.center());
    h.run_steps(2);
    wheel(&h, 3.0, Modifiers::ALT);
    h.step();
    assert_eq!(zoom(&h), max_zoom(), "the flick reaches the limit");
    wheel(&h, -1.0, Modifiers::ALT);
    h.run_steps(40);
    assert!((zoom(&h) - max_zoom() / 1.1).abs() < 1e-2, "one notch back: {}", zoom(&h));
}

#[test]
fn alt_scroll_zooms_while_a_temporary_tool_is_held() {
    let mut h = harness();
    let p = h.state().last_canvas_rect.center();
    h.hover_at(p);
    // Space (the temporary Hand, #249) is down: ⌥ + scroll still zooms by notches, and the
    // held key never changes the current tool.
    h.event(egui::Event::Key { key: Key::Space, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    let z0 = zoom(&h);
    wheel(&h, 2.0, Modifiers::ALT);
    h.run_steps(40);
    assert!((zoom(&h) / z0 - 1.1f32.powi(2)).abs() < 1e-3, "{z0} -> {}", zoom(&h));
    h.event(egui::Event::Key { key: Key::Space, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(h.state().ui.tool, crate::state::Tool::Move);
}

/// Type tool (#206): Alt+←/→ at a collapsed caret kerns the pair before it by 20/1000 em (100
/// with ⌘/Ctrl), one history step per press.
#[test]
fn type_tool_alt_arrows_kern_the_pair() {
    use photocraft_doc::text::Kerning;
    let mut h = harness();
    let id = h.state_mut().run("type.create", json!({"x": 20, "y": 80, "text": "AVA To", "size": 40, "font": "Inter"})).unwrap()["layer"].as_u64().unwrap();
    let text = |h: &Harness<'static, Box<PhotocraftApp>>| {
        let st = h.state().session.active().unwrap();
        match &st.doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
            photocraft_doc::LayerContent::Text(t) => t.clone(),
            _ => panic!("not text"),
        }
    };
    let kern = |h: &Harness<'static, Box<PhotocraftApp>>| {
        let t = text(h);
        let r = t.char_runs();
        (r[0].style.kerning, r[0].style.kern, r.len())
    };
    let steps = |h: &Harness<'static, Box<PhotocraftApp>>| h.state().session.active().unwrap().history.entries().len();
    let metric = photocraft_text::shared().lock().unwrap().pair_kerning(&text(&h), 72.0, 0).unwrap().round();
    edit_type(&mut h, id, 1, 1);
    let s0 = steps(&h);
    h.key_press_modifiers(Modifiers::ALT, Key::ArrowRight);
    h.run_steps(2);
    assert_eq!(kern(&h).0, Kerning::Off);
    assert_eq!(kern(&h).1, metric + 20.0);
    h.key_press_modifiers(Modifiers::ALT | Modifiers::COMMAND, Key::ArrowRight);
    h.run_steps(2);
    assert_eq!(kern(&h).1, metric + 120.0);
    h.key_press_modifiers(Modifiers::ALT, Key::ArrowLeft);
    h.run_steps(2);
    assert_eq!(kern(&h).1, metric + 100.0);
    assert_eq!(steps(&h), s0 + 3, "one history step per press");
    // The caret didn't move; at the text end there's no pair to kern.
    assert_eq!(h.state().ui.text_edit.as_ref().map(|e| (e.caret, e.anchor)), Some((1, 1)));
    let end = text(&h).text.chars().count();
    edit_type(&mut h, id, end, end);
    let before = kern(&h);
    h.key_press_modifiers(Modifiers::ALT, Key::ArrowRight);
    h.run_steps(2);
    assert_eq!(kern(&h), before);
    assert_eq!(steps(&h), s0 + 3);
    // Undo walks back one press at a time.
    assert!(h.state_mut().session.undo());
    h.run_steps(1);
    assert_eq!(kern(&h).1, metric + 120.0);
}

/// Starts a type editing session on `layer` with `anchor..caret` selected, as a click on the text
/// with the Type tool does.
fn edit_type(h: &mut Harness<'static, Box<PhotocraftApp>>, layer: u64, anchor: usize, caret: usize) {
    let app = h.state_mut();
    app.run("layer.select", json!({"layer": layer, "mode": "replace"})).unwrap();
    let ctx = crate::canvas::testing::ctx(app);
    crate::type_tool::edit_active(app, &ctx).unwrap();
    let ed = app.ui.text_edit.as_mut().expect("the session starts");
    ed.anchor = anchor;
    ed.caret = caret;
    h.run_steps(2);
}

fn type_layer(h: &Harness<'static, Box<PhotocraftApp>>, id: u64) -> photocraft_doc::TextLayer {
    match &h.state().session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
        photocraft_doc::LayerContent::Text(t) => t.clone(),
        _ => panic!("not text"),
    }
}

/// The character style of character `c` of a type layer.
fn char_style(h: &Harness<'static, Box<PhotocraftApp>>, id: u64, c: usize) -> photocraft_doc::text::CharStyle {
    let t = type_layer(h, id);
    let byte = t.text.char_indices().nth(c).map_or(t.text.len(), |(i, _)| i);
    let mut end = 0;
    t.char_runs()
        .into_iter()
        .find(|r| {
            end += r.len;
            end > byte || t.text.is_empty()
        })
        .unwrap()
        .style
}

/// The paragraph style of the paragraph holding character `c`.
fn para_style(h: &Harness<'static, Box<PhotocraftApp>>, id: u64, c: usize) -> photocraft_doc::text::ParagraphStyle {
    let t = type_layer(h, id);
    let byte = t.text.char_indices().nth(c).map_or(t.text.len(), |(i, _)| i);
    let mut end = 0;
    t.paragraph_runs()
        .into_iter()
        .find(|r| {
            end += r.len;
            end > byte
        })
        .unwrap()
        .style
}

fn history(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<String> {
    h.state().session.active().unwrap().history.entries()
}

/// Photoshop's ⇧⌘K/U/B/I/: toggle All Caps, Underline, Faux Bold, Faux Italic and Strikethrough
/// on the selected characters while typing, one step per press, and never reach the global
/// ⇧⌘K/U/B/I (Color Settings, Desaturate, Auto Color, Inverse).
#[test]
fn type_tool_format_keys_toggle_character_styles() {
    use photocraft_doc::text::{Caps, CharStyle};
    let mut h = harness();
    // A selection, so Select ▸ Inverse would be enabled if the key reached it.
    h.state_mut().run("select.all", json!({})).unwrap();
    let id = h.state_mut().run("type.create", json!({"x": 20, "y": 80, "text": "Hello world", "size": 20})).unwrap()["layer"].as_u64().unwrap();
    edit_type(&mut h, id, 0, 5);
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    type Has = fn(&CharStyle) -> bool;
    let keys: [(Key, Has); 6] = [
        (Key::K, |s| s.caps == Caps::AllCaps),
        (Key::U, |s| s.underline),
        (Key::B, |s| s.faux_bold),
        (Key::I, |s| s.faux_italic),
        (Key::Slash, |s| s.strikethrough),
        (Key::Questionmark, |s| s.strikethrough),
    ];
    for (key, has) in keys {
        let before = history(&h).len();
        let was = has(&char_style(&h, id, 0));
        h.key_press_modifiers(cmd_shift, key);
        h.run_steps(2);
        assert_eq!(has(&char_style(&h, id, 0)), !was, "{key:?} toggles the selection");
        assert_eq!(has(&char_style(&h, id, 4)), !was, "{key:?}");
        assert!(!has(&char_style(&h, id, 6)), "{key:?} leaves the unselected characters");
        let steps = history(&h);
        assert_eq!(steps.len(), before + 1, "{key:?}: one history step");
        assert_eq!(steps.last().map(String::as_str), Some("Set Type Style"), "{key:?}");
        assert!(!h.state().dialog_open(), "{key:?} must not open Color Settings");
        assert!(h.state().ui.text_edit.is_some());
    }
    // The second strikethrough key turned it off again; press the rest again: all off.
    for key in [Key::K, Key::U, Key::B, Key::I] {
        h.key_press_modifiers(cmd_shift, key);
        h.run_steps(2);
    }
    let s = char_style(&h, id, 2);
    assert!(s.caps == Caps::Normal && !s.underline && !s.faux_bold && !s.faux_italic && !s.strikethrough);
    // Partly underlined: the key underlines the whole selection rather than flipping each part.
    h.state_mut().run("type.setStyle", json!({"layer": id, "range": [0, 2], "underline": true})).unwrap();
    h.key_press_modifiers(cmd_shift, Key::U);
    h.run_steps(2);
    assert!((0..5).all(|c| char_style(&h, id, c).underline));
    // Undo takes back one press.
    assert!(h.state_mut().session.undo());
    h.run_steps(1);
    assert!(char_style(&h, id, 0).underline && !char_style(&h, id, 3).underline);
    // A collapsed caret in non-empty text: nothing changes, and nothing global runs.
    edit_type(&mut h, id, 3, 3);
    let before = history(&h);
    for key in [Key::K, Key::U, Key::B, Key::I] {
        h.key_press_modifiers(cmd_shift, key);
        h.run_steps(2);
    }
    assert_eq!(history(&h), before);
    assert!(!h.state().dialog_open());
    // Empty text: the key sets the style the next characters typed get.
    let empty = h.state_mut().run("type.create", json!({"x": 20, "y": 160, "text": "", "size": 20})).unwrap()["layer"].as_u64().unwrap();
    edit_type(&mut h, empty, 0, 0);
    h.key_press_modifiers(cmd_shift, Key::B);
    h.run_steps(2);
    h.event(egui::Event::Text("ab".into()));
    h.run_steps(2);
    assert_eq!(type_layer(&h, empty).text, "ab");
    assert!(char_style(&h, empty, 0).faux_bold && char_style(&h, empty, 1).faux_bold);
}

/// Outside text editing ⇧⌘K and ⇧⌘I keep their global meanings.
#[test]
fn global_cmd_shift_keys_still_work_outside_type_editing() {
    let mut h = harness();
    h.state_mut().run("select.all", json!({})).unwrap();
    let id = h.state_mut().run("type.create", json!({"x": 20, "y": 80, "text": "Hello", "size": 20})).unwrap()["layer"].as_u64().unwrap();
    h.run_steps(2);
    assert!(h.state().ui.text_edit.is_none());
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    h.key_press_modifiers(cmd_shift, Key::I);
    h.run_steps(2);
    assert_eq!(history(&h).last().map(String::as_str), Some("Inverse"));
    h.key_press_modifiers(cmd_shift, Key::K);
    h.run_steps(3);
    assert!(h.state().dialog_open(), "Color Settings opens");
    assert!(!char_style(&h, id, 0).faux_italic && char_style(&h, id, 0).caps == photocraft_doc::text::Caps::Normal);
}

/// ⌥↑/↓ leading (2 pt, 10 with ⌘), ⇧⌥↑/↓ baseline shift and ⌥←/→ tracking over a selection
/// (20, 100 with ⌘): one step per press, from each character's own value, the selection kept.
#[test]
fn type_tool_format_keys_step_leading_baseline_and_tracking() {
    let mut h = harness();
    let id = h.state_mut().run("type.create", json!({"x": 20, "y": 80, "text": "Hello world", "size": 10})).unwrap()["layer"].as_u64().unwrap();
    edit_type(&mut h, id, 0, 5);
    let before = history(&h).len();
    let press = |h: &mut Harness<'static, Box<PhotocraftApp>>, m: Modifiers, k: Key| {
        h.key_press_modifiers(m, k);
        h.run_steps(2);
    };
    // Auto leading is 120% of 10 pt: ⌥↓ makes it 14.
    press(&mut h, Modifiers::ALT, Key::ArrowDown);
    assert_eq!(char_style(&h, id, 0).leading_pt, Some(14.0));
    press(&mut h, Modifiers::ALT, Key::ArrowUp);
    assert_eq!(char_style(&h, id, 4).leading_pt, Some(12.0));
    press(&mut h, Modifiers::ALT | Modifiers::COMMAND, Key::ArrowDown);
    assert_eq!(char_style(&h, id, 0).leading_pt, Some(22.0));
    press(&mut h, Modifiers::ALT | Modifiers::SHIFT, Key::ArrowUp);
    assert_eq!(char_style(&h, id, 0).baseline_shift_pt, 2.0);
    press(&mut h, Modifiers::ALT | Modifiers::SHIFT | Modifiers::COMMAND, Key::ArrowDown);
    assert_eq!(char_style(&h, id, 0).baseline_shift_pt, -8.0);
    press(&mut h, Modifiers::ALT, Key::ArrowRight);
    assert_eq!(char_style(&h, id, 0).tracking, 20.0);
    press(&mut h, Modifiers::ALT | Modifiers::COMMAND, Key::ArrowLeft);
    assert_eq!(char_style(&h, id, 4).tracking, -80.0);
    assert_eq!(history(&h).len(), before + 7, "one history step per press");
    assert_eq!(h.state().ui.text_edit.as_ref().map(|e| (e.anchor, e.caret)), Some((0, 5)), "the selection stays");
    let rest = char_style(&h, id, 7);
    assert_eq!((rest.leading_pt, rest.baseline_shift_pt, rest.tracking), (None, 0.0, 0.0));
    // A collapsed caret: ⌥↑/↓ change nothing and don't move the caret.
    edit_type(&mut h, id, 8, 8);
    let steps = history(&h).len();
    press(&mut h, Modifiers::ALT, Key::ArrowDown);
    press(&mut h, Modifiers::ALT | Modifiers::SHIFT, Key::ArrowUp);
    assert_eq!(history(&h).len(), steps);
    assert_eq!(h.state().ui.text_edit.as_ref().map(|e| (e.anchor, e.caret)), Some((8, 8)));
}

/// ⌥⇧⌘H toggles Hyphenate for the paragraphs the selection or caret touches.
#[test]
fn type_tool_format_keys_toggle_hyphenate() {
    let mut h = harness();
    let id = h.state_mut().run("type.create", json!({"box": [10, 10, 300, 200], "text": "one\ntwo\nthree", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    let keys = Modifiers::COMMAND | Modifiers::ALT | Modifiers::SHIFT;
    // The caret in the second paragraph.
    edit_type(&mut h, id, 5, 5);
    let before = history(&h).len();
    h.key_press_modifiers(keys, Key::H);
    h.run_steps(2);
    assert_eq!([0, 5, 9].map(|c| para_style(&h, id, c).hyphenate), [false, true, false]);
    // A selection over the first two: not all hyphenate, so both do; again, neither.
    edit_type(&mut h, id, 1, 5);
    h.key_press_modifiers(keys, Key::H);
    h.run_steps(2);
    assert_eq!([0, 5, 9].map(|c| para_style(&h, id, c).hyphenate), [true, true, false]);
    h.key_press_modifiers(keys, Key::H);
    h.run_steps(2);
    assert_eq!([0, 5, 9].map(|c| para_style(&h, id, c).hyphenate), [false, false, false]);
    assert_eq!(history(&h).len(), before + 3);
}

/// The formatting keys never panic: an edit of a layer that has gone, or with no document.
#[test]
fn type_tool_format_keys_survive_hostile_state() {
    let mut h = harness();
    let id = h.state_mut().run("type.create", json!({"x": 20, "y": 80, "text": "abc", "size": 20})).unwrap()["layer"].as_u64().unwrap();
    edit_type(&mut h, id, 0, 3);
    h.state_mut().run("layer.delete", json!({"layer": id})).unwrap();
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::U);
    h.key_press_modifiers(Modifiers::ALT, Key::ArrowDown);
    h.run_steps(2);
    assert!(h.state().ui.text_edit.is_none(), "an edit of a missing layer ends");
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for ed in [None, Some(1u64)] {
        app.ui.text_edit =
            ed.map(|layer| crate::state::TextEdit { layer, document: None, caret: 4, anchor: 0, session: "x".into(), created: false, mask: false });
        // PLAN: assumed these three keep their signatures (tools-w20); the literal lists the fields tools-w20 keeps
        crate::type_tool::toggle_char_style(&mut app, crate::type_tool::CharToggle::AllCaps);
        crate::type_tool::step_char_metric(&mut app, "leadingBy", 2.0);
        crate::type_tool::step_char_metric(&mut app, "trackingBy", f32::NAN);
        crate::type_tool::toggle_hyphenate(&mut app);
    }
    assert!(app.session.active().is_none());
}

#[test]
fn move_alt_copy_cursor_follows_the_latched_duplication_drag() {
    let mut h = harness();
    h.state_mut().run("shape.create", json!({"kind": "rect", "rect": [150, 100, 100, 100], "fill": "#ff0000"})).unwrap();
    h.state_mut().ui.tool = crate::state::Tool::Move;
    h.state_mut().ui.tool_options.move_show_transform = false;
    let before = h.state().session.active().unwrap().doc.layers.len();
    // Without ⌥ the Move tool asks for its painted arrow with the cross (#16819), which the window shows as an image.
    let move_cursor = Some(craft::behavior::Cursor::Painted(craft::behavior::PaintedCursor::pointer_tool(
        craft::behavior::PointerGlyph::Arrow,
        crate::icons::tool_icon(crate::state::Tool::Move),
    )));
    let requested = |h: &Harness<'static, Box<PhotocraftApp>>| h.state().canvas_tools.last_response.as_ref().and_then(|r| r.cursor.clone());
    let p = h.state().last_canvas_rect.center();
    h.hover_at(p);
    h.run_steps(2);
    assert_eq!(requested(&h), move_cursor);
    h.event(egui::Event::ModifiersChanged(Modifiers::ALT));
    h.run_steps(2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Copy);
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), before, "hovering never duplicates");
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::ALT });
    h.run_steps(1);
    let end = p + vec2(25.0, 15.0);
    h.hover_at(end);
    h.run_steps(2);
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::Copy, "releasing Alt does not undo an active copy");
    h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(requested(&h), move_cursor, "back to the Move cursor");
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), before + 1);
}

/// Esc while drawing with the Pen ends the path where it is, left open, and keeps it as the work
/// path, as ↩ does (#1769).
#[test]
fn esc_ends_a_pen_path_and_keeps_it() {
    let mut h = harness();
    h.state_mut().ui.tool = crate::state::Tool::Pen;
    h.state_mut().ui.views[0].rotation = 40.0;
    let owner = crate::vector_ui::capture_pen_owner(h.state());

    h.state_mut().ui.pen = Some(crate::vector_ui::PenPath::corners(&[[50.0, 50.0], [150.0, 50.0], [150.0, 120.0]], owner));
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.pen.is_none(), "the Pen leaves drawing state");
    assert_eq!(h.state().ui.views[0].rotation, 40.0, "the active path owns Escape");
    let doc = &h.state().session.active().unwrap().doc;
    let path = doc.work_path.as_ref().expect("the path is kept as the work path");
    assert_eq!(path.subpaths.len(), 1);
    assert!(!path.subpaths[0].closed, "left open");
    assert_eq!(path.subpaths[0].knots.len(), 3);
    assert_eq!(h.state().ui.selected_path.as_deref(), Some("work"));
}

/// A rotated view takes the first idle Esc; the next retains normal deselection.
#[test]
fn escape_resets_rotation_before_idle_deselection_for_paint_tools() {
    for tool in [crate::state::Tool::Brush, crate::state::Tool::Pencil] {
        let mut h = harness();
        h.state_mut().ui.tool = tool;
        h.state_mut().ui.views[0].rotation = 40.0;
        h.state_mut().run("select.all", json!({})).unwrap();
        h.run_steps(1);
        let history = h.state().session.active().unwrap().history.entries().len();
        h.key_press(Key::Escape);
        h.run_steps(2);
        assert_eq!(h.state().ui.views[0].rotation, 0.0, "{tool:?}");
        assert!(h.state().session.active().unwrap().doc.selection.is_some(), "first Esc keeps the selection");
        assert_eq!(h.state().session.active().unwrap().history.entries().len(), history, "view reset is not an edit");
        h.key_press(Key::Escape);
        h.run_steps(2);
        assert!(h.state().session.active().unwrap().doc.selection.is_none(), "second Esc deselects normally");
    }
}

/// Esc with no active operation deselects (#2674), like ⌘D.
#[test]
fn escape_deselects_when_nothing_else_needs_cancelling() {
    let mut h = harness();
    h.state_mut().run("select.all", json!({})).unwrap();
    h.run_steps(1);
    assert!(h.state().session.active().unwrap().doc.selection.is_some());
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(h.state().session.active().unwrap().doc.selection.is_none(), "Esc deselects");
    assert!(!h.state().ui.status_error, "no error: {}", h.state().ui.status);
    // With nothing selected, Esc stays untouched (no history step, no error).
    let steps = h.state().session.active().unwrap().history.entries().len();
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().history.entries().len(), steps, "no Deselect step");
}

/// The dialog owns Esc: it cancels the dialog and the selection stays.
#[test]
fn escape_with_a_dialog_open_cancels_the_dialog_not_the_selection() {
    let mut h = harness();
    h.state_mut().run("select.all", json!({})).unwrap();
    h.state_mut().ui.views[0].rotation = 40.0;
    open_dialog(&mut h);
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(!h.state().dialog_open(), "Esc cancelled the dialog");
    assert!(h.state().session.active().unwrap().doc.selection.is_some(), "the selection stays");
    assert_eq!(h.state().ui.views[0].rotation, 40.0, "the dialog owns the first Esc");
}

/// How much of the 400 × 300 document is on screen: the visible document span on each axis,
/// clipped to the image (0 when the image has left the view on that axis).
fn visible_image(h: &Harness<'static, Box<PhotocraftApp>>) -> [f32; 2] {
    let r = h.state().last_canvas_rect;
    let (a, b) = (doc_at(h, r.min), doc_at(h, r.max));
    [(b[0].min(400.0) - a[0].max(0.0)).max(0.0), (b[1].min(300.0) - a[1].max(0.0)).max(0.0)]
}

#[test]
fn wheel_zoom_over_the_pasteboard_keeps_the_image_in_view() {
    // #3720: zooming in with the pointer on the gray pasteboard anchored on the pointer, so a few
    // notches pushed the image out of the window. The anchor is now the nearest point of the image.
    for (zoom_with_wheel, m) in [(true, Modifiers::NONE), (false, Modifiers::ALT)] {
        let mut h = harness();
        h.state_mut().session.edit_prefs(|p| p.general.zoom_with_scroll_wheel = zoom_with_wheel);
        let r = h.state().last_canvas_rect;
        let p = pos2(r.left() + 20.0, r.top() + 20.0);
        let d = doc_at(&h, p);
        assert!(d[0] < 0.0 && d[1] < 0.0, "the pointer starts on the pasteboard: {d:?}");
        h.hover_at(p);
        h.run_steps(2);
        let z0 = zoom(&h);
        for _ in 0..20 {
            wheel(&h, 1.0, m);
            h.run_steps(3);
        }
        h.run_steps(20);
        assert!(zoom(&h) > z0 * 5.0, "the notches zoomed in: {z0} -> {}", zoom(&h));
        let v = visible_image(&h);
        assert!(v[0] > 0.0 && v[1] > 0.0, "zoom_with_wheel {zoom_with_wheel}: the image left the view {v:?}");
        // The image's corner nearest the pointer is the anchor: it stays where it was on screen.
        let c = doc_at(&h, pos2(r.left() + 20.0, r.top() + 20.0));
        assert!(c[0] < 0.0 && c[1] < 0.0, "still pasteboard under the pointer, the corner held: {c:?}");
        // A pinch from there behaves the same.
        h.event(egui::Event::Zoom(4.0));
        h.run_steps(3);
        let v = visible_image(&h);
        assert!(v[0] > 0.0 && v[1] > 0.0, "pinch: the image left the view {v:?}");
    }
}

#[test]
fn assignable_pick_layer_shortcut_follows_transformed_canvas_without_editing() {
    let mut h = harness();
    let base = h.state().session.active().unwrap().active_layer.unwrap();
    h.state_mut().run("layer.new.layer", json!({"name": "Paint"})).unwrap();
    let paint = h.state().session.active().unwrap().active_layer.unwrap();
    h.state_mut().run("select.rect", json!({"x": 100, "y": 100, "width": 40, "height": 40})).unwrap();
    h.state_mut().run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    h.state_mut().run("select.deselect", json!({})).unwrap();
    h.state_mut().run("layer.new.layer", json!({"name": "Empty"})).unwrap();
    h.state_mut().run("prefs.set", json!({"path": "shortcuts.layer.selectUnderCursor", "value": "Cmd+Alt+K"})).unwrap();
    h.state_mut().ui.tool = crate::state::Tool::Brush;
    for (zoom, rotation, flip) in [(1.0, 0.0, false), (2.0, 37.0, true), (0.5, -90.0, false)] {
        h.state_mut().ui.views[0].zoom = zoom;
        h.state_mut().ui.views[0].rotation = rotation;
        h.state_mut().ui.views[0].center = [180.0, 145.0];
        h.state_mut().ui.view.flip_horizontal = flip;
        h.run_steps(2);
        for (point, expected) in [([120.0, 120.0], paint), ([155.0, 155.0], base)] {
            let pos = crate::canvas::testing::to_screen(h.state(), point);
            let history = h.state().session.active().unwrap().history.past_len();
            h.hover_at(pos);
            h.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::K);
            h.run_steps(2);
            assert_eq!(h.state().session.active().unwrap().active_layer, Some(expected));
            assert_eq!(h.state().session.active().unwrap().history.past_len(), history, "picking never adds a document edit");
            assert!(!h.state().session.journal.iter().any(|(id, _)| id == "paint.stroke"));
        }
    }
    h.state_mut().run("layer.setProps", json!({"layer": paint.0, "visible": false})).unwrap();
    let p = crate::canvas::testing::to_screen(h.state(), [120.0, 120.0]);
    h.hover_at(p);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::K);
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(base));
    // Outside the document and modal dialogs preserve the current selection.
    h.state_mut().run("layer.select", json!({"layer": paint.0})).unwrap();
    h.hover_at(pos2(0.0, 0.0));
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::K);
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(paint));
    open_dialog(&mut h);
    h.hover_at(p);
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::ALT, Key::K);
    h.run_steps(2);
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(paint));
}

#[test]
fn marquee_drag_after_hiding_extras_restores_selection_visibility() {
    let mut h = harness();
    h.state_mut().ui.tool = crate::state::Tool::RectMarquee;
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "view.extras", json!({})).unwrap();
    assert!(!h.state().ui.view.extras);
    let start = h.state().last_canvas_rect.center() - vec2(30.0, 20.0);
    let end = start + vec2(60.0, 40.0);
    h.hover_at(start);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for i in 1..=4 {
        h.hover_at(start + (end - start) * (i as f32 / 4.0));
        h.run_steps(1);
    }
    h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert!(h.state().session.active().unwrap().doc.selection.is_some(), "the pointer drag created a selection");
    assert!(h.state().ui.view.shows(h.state().ui.view.show.selection_edges), "a new marquee must become visible after hiding Extras");
    crate::menus::invoke(h.state_mut(), &ctx, "view.extras", json!({})).unwrap();
    h.run_steps(2);
    assert!(!h.state().ui.view.extras, "the user can hide the existing selection again");
    h.state_mut().run("edit.fill", json!({"color": "#22cc44"})).unwrap();
    assert!(!h.state().ui.view.extras, "painting an existing selection must preserve the user's hide choice");
}
