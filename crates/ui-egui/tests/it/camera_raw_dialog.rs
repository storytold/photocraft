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
    let image = rectangle(&h, "previewRect");
    key(&mut h, Key::Space, true);
    drag(&mut h, at, at + vec2(80.0, 40.0));
    key(&mut h, Key::Space, false);
    assert_eq!(dabs(&h), 0);
    let moved = rectangle(&h, "previewRect").min - image.min;
    assert!(moved.x > 40.0 && moved.y > 20.0, "the preview panned: {moved:?}");
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

/// The components of the first mask.
fn components(h: &Harness<'_, PhotocraftApp>) -> Vec<Value> {
    inspect(h)["params"]["masks"][0]["components"].as_array().cloned().unwrap_or_default()
}
fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    press(h, at, true);
    press(h, at, false);
}

/// Before (Y) shows the image without Geometry, where the masks do not apply: the brush paints
/// nothing there, and paints again on After.
#[test]
fn the_before_view_takes_no_mask_edits() {
    let mut h = fixture();
    brush(&mut h);
    menu(&mut h, json!({"before":true}));
    let at = rectangle(&h, "previewRect").center();
    click(&mut h, at);
    assert_eq!(dabs(&h), 0);
    menu(&mut h, json!({"before":false}));
    click(&mut h, at);
    assert_eq!(dabs(&h), 1);
}

/// Split view: Before covers the left half, so only the After half takes the brush.
#[test]
fn split_view_edits_only_the_after_half() {
    let mut h = fixture();
    brush(&mut h);
    menu(&mut h, json!({"compare":"split"}));
    let image = rectangle(&h, "previewRect");
    click(&mut h, image.lerp_inside(vec2(0.25, 0.5)));
    assert_eq!(dabs(&h), 0, "Before half");
    click(&mut h, image.lerp_inside(vec2(0.75, 0.5)));
    assert_eq!(dabs(&h), 1, "After half");
}

/// A Color Range click beside the image (the preview smaller than the view) samples nothing.
#[test]
fn color_range_clicks_beside_the_image_do_nothing() {
    let mut h = fixture();
    menu(&mut h, json!({"set":{"masks":[{"name":"Mask 1","components":[]}]},"masks":{"open":true,"selected":0,"tool":"colorPick"}}));
    let (image, view) = (rectangle(&h, "previewRect"), rectangle(&h, "viewportRect"));
    let beside = if image.left() - view.left() > 4.0 { pos2(view.left() + 2.0, image.center().y) } else { pos2(image.center().x, view.top() + 2.0) };
    assert!(view.contains(beside) && !image.contains(beside), "{image:?} in {view:?}");
    click(&mut h, beside);
    assert!(components(&h).is_empty(), "{:?}", components(&h));
    click(&mut h, image.center());
    assert_eq!(components(&h).len(), 1);
}

/// When the pointer moves on the release frame, the gradient ends where it was let go.
#[test]
fn a_gradient_ends_where_it_is_released() {
    let mut h = fixture();
    menu(&mut h, json!({"set":{"masks":[{"name":"Mask 1","components":[]}]},"masks":{"open":true,"selected":0,"tool":"linear"}}));
    let image = rectangle(&h, "previewRect");
    let (start, mid, end) = (image.lerp_inside(vec2(0.3, 0.3)), image.lerp_inside(vec2(0.5, 0.5)), image.lerp_inside(vec2(0.7, 0.6)));
    press(&mut h, start, true);
    for n in 1..=4 {
        h.event(Event::PointerMoved(start + (mid - start) * n as f32 / 4.0));
        h.run_steps(2);
    }
    // Released at `end` without a move event before it.
    press(&mut h, end, false);
    let c = components(&h);
    let zero = c[0]["zero"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect::<Vec<_>>();
    assert!((zero[0] - 0.7).abs() < 0.01 && (zero[1] - 0.6).abs() < 0.01, "{c:?}");
}

/// A keyboard edit (an arrow on a focused value box) is its own undo step.
#[test]
fn a_keyboard_edit_is_one_undo_step() {
    let mut h = fixture();
    menu(&mut h, json!({"panels":{"open":["light"]}}));
    let field = h.query_by_role_and_label(Role::SpinButton, "Exposure").or_else(|| h.query_by_role_and_label(Role::TextInput, "Exposure")).unwrap().rect();
    click(&mut h, field.center());
    key(&mut h, Key::ArrowUp, true);
    key(&mut h, Key::ArrowUp, false);
    let exposure = || inspect(&h)["params"]["exposure"].as_f64().unwrap();
    let edited = exposure();
    assert!(edited > 0.0, "the arrow stepped Exposure");
    menu(&mut h, json!({"undo":true}));
    assert_eq!(inspect(&h)["params"]["exposure"].as_f64(), Some(0.0), "undone");
}
