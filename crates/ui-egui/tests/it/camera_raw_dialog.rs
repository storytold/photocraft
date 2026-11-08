//! Camera Raw dialog layout and preview drawing through the real modal shell.
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, accesskit::Role, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_ui_egui::{PhotocraftApp, camera_raw_ui, control, theme::ThemeKind};
use serde_json::{Value, json};

/// The settings column's right edge at 1440 pt: the tool strip (44 pt) and the column's 18 pt
/// inner margin.
const SETTINGS_RIGHT: f32 = 1440.0 - 44.0 - 18.0;

fn fixture() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1440.0, 1000.0)).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":1200,"height":900})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color":"#808080"})).unwrap();
        // English labels whatever the host's language (the queries below are by label).
        app.session.prefs.edit(|p| p.interface.language = "en".into());
        camera_raw_ui::open(&mut app, &cc.egui_ctx).unwrap();
        app
    });
    h.ctx.all_styles_mut(|style| style.animation_time = 0.0);
    h.run_steps(4);
    h
}
fn inspect(h: &Harness<'_, PhotocraftApp>) -> Value {
    control::inspect(h.state(), &h.ctx)["cameraRaw"].clone()
}
fn rectangle(h: &Harness<'_, PhotocraftApp>, key: &str) -> Rect {
    let v = inspect(h);
    let a = v[key].as_array().unwrap();
    Rect::from_min_max(pos2(a[0].as_f64().unwrap() as f32, a[1].as_f64().unwrap() as f32), pos2(a[2].as_f64().unwrap() as f32, a[3].as_f64().unwrap() as f32))
}
fn menu(h: &mut Harness<'_, PhotocraftApp>, ui: Value) -> Value {
    let ctx = h.ctx.clone();
    let r = camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui":ui})).unwrap().unwrap();
    h.run_steps(3);
    r
}
fn press(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, down: bool) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: down, modifiers: Modifiers::NONE });
    h.run_steps(2);
}
fn drag(h: &mut Harness<'_, PhotocraftApp>, start: Pos2, end: Pos2) {
    press(h, start, true);
    for n in 1..=4 {
        h.event(Event::PointerMoved(start + (end - start) * n as f32 / 4.0));
        h.run_steps(2);
    }
    press(h, end, false);
}
fn key(h: &mut Harness<'_, PhotocraftApp>, key: Key, down: bool) {
    h.event(Event::Key { key, physical_key: None, pressed: down, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}
fn dabs(h: &Harness<'_, PhotocraftApp>) -> usize {
    let masks = inspect(h)["params"]["masks"].clone();
    masks[0]["components"].as_array().map_or(0, |c| c.iter().map(|c| c["dabs"].as_array().map_or(0, Vec::len)).sum())
}
/// One mask, selected, with the brush tool on.
fn brush(h: &mut Harness<'_, PhotocraftApp>) {
    menu(h, json!({"set":{"masks":[{"name":"Mask 1","components":[]}]},"masks":{"open":true,"selected":0,"tool":"brush"}}));
}

/// Side by side, the image is fitted to the After half: `zoom` is the scale it is shown at.
#[test]
fn side_by_side_zoom_is_the_shown_scale() {
    let mut h = fixture();
    menu(&mut h, json!({"compare":"sideBySide"}));
    let shown = rectangle(&h, "previewRect").width() / 1200.0;
    let zoom = inspect(&h)["zoom"].as_f64().unwrap() as f32;
    assert!((zoom - shown).abs() < 1e-3, "zoom {zoom}, shown at {shown}");
}

/// Color Grading's 3-Way view: the Shadows and Highlights Luminance rows each fit their column.
#[test]
fn three_way_luminance_rows_fit_their_columns() {
    let mut h = fixture();
    menu(&mut h, json!({"panels":{"open":["colorGrading"]},"grading":"threeWay"}));
    let mut rows: Vec<Rect> = h.query_all_by_role_and_label(Role::Slider, "Luminance").map(|n| n.rect()).collect();
    assert_eq!(rows.len(), 3, "Midtones, Shadows and Highlights");
    rows.sort_by(|a, b| b.top().total_cmp(&a.top()).then(a.left().total_cmp(&b.left())));
    let (shadows, highlights) = (rows[0], rows[1]);
    assert!((shadows.top() - highlights.top()).abs() < 1.0, "{shadows:?} {highlights:?}");
    assert!(shadows.right() <= highlights.left(), "Shadows runs into Highlights: {shadows:?} {highlights:?}");
    assert!(highlights.right() <= SETTINGS_RIGHT, "Highlights is cut off: {highlights:?}");
}

/// Color Mixer › Color: all eight colour chips on one row inside the column.
#[test]
fn color_mixer_chips_fit_the_column() {
    let mut h = fixture();
    menu(&mut h, json!({"panels":{"open":["colorMixer"]},"mixer":{"adjust":"color"}}));
    let magenta = h.get_by_role_and_label(Role::Button, "Magenta").rect();
    let red = h.get_by_role_and_label(Role::Button, "Red").rect();
    assert!(magenta.right() <= SETTINGS_RIGHT, "{magenta:?}");
    assert!((magenta.top() - red.top()).abs() < 1.0, "one row");
}

/// A brush click without a drag paints one dab.
#[test]
fn a_brush_click_paints_a_dab() {
    let mut h = fixture();
    brush(&mut h);
    let at = rectangle(&h, "previewRect").center();
    press(&mut h, at, true);
    press(&mut h, at, false);
    assert_eq!(dabs(&h), 1);
}

/// Space + drag pans the preview; the brush paints nothing meanwhile.
#[test]
fn a_space_drag_pans_without_painting() {
    let mut h = fixture();
    brush(&mut h);
    menu(&mut h, json!({"view":{"zoom":2.0}}));
    let at = rectangle(&h, "viewportRect").center();
    key(&mut h, Key::Space, true);
    drag(&mut h, at, at + vec2(80.0, 40.0));
    key(&mut h, Key::Space, false);
    assert_eq!(dabs(&h), 0);
}

/// With the Masks panel open and a mask selected, the guide tool still adds Guided Upright guides.
#[test]
fn the_guide_tool_draws_guides_with_the_masks_panel_open() {
    let mut h = fixture();
    menu(
        &mut h,
        json!({
            "set":{"perspectiveUpright":"guided","uprightGuides":[[[0.3,0.1],[0.32,0.9]]],"masks":[{"name":"Mask 1","components":[{"kind":"sky"}]}]},
            "masks":{"open":true,"selected":0,"tool":"guides"},
        }),
    );
    let image = rectangle(&h, "previewRect");
    drag(&mut h, image.lerp_inside(vec2(0.7, 0.2)), image.lerp_inside(vec2(0.68, 0.8)));
    assert_eq!(inspect(&h)["params"]["uprightGuides"].as_array().map(Vec::len), Some(2));
}
