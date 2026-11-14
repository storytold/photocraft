//! The develop view's keyboard shortcuts (Camera Raw's keymap) driven with real key events in the
//! shell: the rail's tools, `[` `]` and ⇧`[` ⇧`]` on the active brush, ⇧T Guided Upright, ⌘1 …
//! the Edit sections in the panel's order, none of them while a value is being typed, and none
//! reaching PhotoCraft's own shortcuts behind the dialog.
//!
//! `PHOTOCRAFT_LC_SNAPSHOTS=<dir>` also saves the Geometry section with its Upright icons there
//! (`upright-icons.png`, on the GPU); `PHOTOCRAFT_LC_RAW=<file>` uses that raw.
use egui::{Event, Key, Modifiers, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_ui_egui::{PhotocraftApp, Services, camera_raw_ui, shortcuts::pretty, theme::ThemeKind};
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

/// PhotoCraft's own state behind the dialog that its shortcuts would change: the tool and its
/// options (brush size, …).
fn app_state(h: &Harness<'_, PhotocraftApp>) -> Value {
    let all = photocraft_ui_egui::control::inspect(h.state(), &h.ctx);
    json!({"tool": all["tool"], "toolOptions": all["toolOptions"]})
}

fn settle(h: &mut Harness<'_, PhotocraftApp>) {
    for _ in 0..600 {
        h.run_steps(1);
        let r = inspect(h);
        if !r.is_null() && r["previewPending"] == false && r["detailPending"] == false {
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

/// One press and release of `key` with `modifiers` held, as a keyboard sends them.
fn key(h: &mut Harness<'_, PhotocraftApp>, key: Key, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    for pressed in [true, false] {
        h.event(Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers });
    }
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

fn dialog() -> Harness<'static, PhotocraftApp> {
    dialog_on(false)
}

/// The dialog on a raw, rendered on the GPU if `gpu` (for a snapshot).
fn dialog_on(gpu: bool) -> Harness<'static, PhotocraftApp> {
    let (name, bytes) = match std::env::var("PHOTOCRAFT_LC_RAW") {
        Ok(path) => ("test.CR2".to_string(), std::fs::read(path).unwrap()),
        Err(_) => ("IMG_0001.dng".to_string(), dng()),
    };
    let builder = Harness::builder().with_step_dt(1.0 / 60.0).with_size(vec2(1600.0, 1000.0));
    let builder = if gpu { builder.wgpu() } else { builder };
    let mut h = builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let services = Services {
            import: Some(Box::new(|name: &str, b: &[u8], opts: &photocraft_io::ImportOptions| {
                photocraft_io::import_with_options(name, b, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.open_bytes(&name, &bytes).unwrap();
        app
    });
    h.ctx.all_styles_mut(|s| s.animation_time = 0.0);
    settle(&mut h);
    assert_eq!(inspect(&h)["panel"], "develop");
    h
}

/// E, B, M and ⇧E pick the rail's tools as in Camera Raw (the current tool's key again goes back
/// to Edit), LightCraft's Q does nothing here, the rail's tooltips name the keys, and PhotoCraft's
/// own tool keys behind the dialog never see them.
#[test]
fn camera_raw_tool_keys_pick_the_rails_tools() {
    let mut h = dialog();
    let behind = app_state(&h);
    for label in ["Edit (E)", "Heal (B)", "Masking (K)", &format!("Red Eye ({})", pretty("Shift+E"))] {
        assert!(h.query_by_label(label).is_some(), "the rail button's label names its key: {label}");
    }
    assert_eq!(inspect(&h)["shortcuts"]["tool.heal"], "B", "{}", inspect(&h)["shortcuts"]);
    let tool = |h: &Harness<'_, PhotocraftApp>| inspect(h)["tool"].as_str().unwrap().to_string();
    for (k, m, want) in [
        (Key::B, Modifiers::NONE, "heal"),
        (Key::M, Modifiers::NONE, "masking"),
        (Key::E, Modifiers::SHIFT, "redEye"),
        (Key::E, Modifiers::SHIFT, "edit"),
        (Key::Q, Modifiers::NONE, "edit"),
        (Key::M, Modifiers::NONE, "masking"),
        (Key::E, Modifiers::NONE, "edit"),
    ] {
        key(&mut h, k, m);
        assert_eq!(tool(&h), want, "after {k:?} {m:?}");
    }
    // `[` `]` and the rest neither: the canvas brush and tool are as they were
    for (k, m) in [(Key::CloseBracket, Modifiers::NONE), (Key::OpenBracket, Modifiers::SHIFT), (Key::B, Modifiers::NONE)] {
        key(&mut h, k, m);
    }
    assert_eq!(app_state(&h), behind, "no shortcut reached PhotoCraft behind the dialog");
}

/// `[` `]` size the active brush (Masking's while it paints, Heal's spot, Red Eye's pupil) and
/// ⇧`[` ⇧`]` its feather, a tenth at a time within the sliders' ranges; a spot's or eye's change
/// is one undo step.
#[test]
fn brackets_size_and_feather_the_active_brush() {
    let mut h = dialog();
    // Masking: a brush mask, so the brush paints
    key(&mut h, Key::M, Modifiers::NONE);
    ui(&mut h, json!({"local": {"newMask": "brush"}}));
    assert_eq!(inspect(&h)["maskDraw"], "brush");
    let brush = |h: &Harness<'_, PhotocraftApp>| (inspect(h)["brush"]["size"].as_f64().unwrap() * 400.0, inspect(h)["brush"]["feather"].as_f64().unwrap());
    let (size, feather) = brush(&h);
    assert!((size - 12.0).abs() < 1e-6 && feather == 50.0, "{size} {feather}");
    key(&mut h, Key::CloseBracket, Modifiers::NONE);
    assert!((brush(&h).0 - 13.0).abs() < 1e-6, "{:?}", brush(&h));
    for _ in 0..40 {
        key(&mut h, Key::CloseBracket, Modifiers::NONE);
    }
    assert!((brush(&h).0 - 100.0).abs() < 1e-6, "the Size slider's maximum: {:?}", brush(&h));
    for _ in 0..60 {
        key(&mut h, Key::OpenBracket, Modifiers::NONE);
    }
    assert!((brush(&h).0 - 1.0).abs() < 1e-6, "its minimum: {:?}", brush(&h));
    key(&mut h, Key::CloseBracket, Modifiers::SHIFT);
    assert_eq!(brush(&h).1, 60.0);
    // a US layout's ⇧[ arrives as `{`
    for k in [Key::OpenCurlyBracket, Key::OpenBracket] {
        key(&mut h, k, Modifiers::SHIFT);
    }
    assert_eq!(brush(&h).1, 40.0);

    // Heal: the next spot's size without one selected, the selected spot's with one
    key(&mut h, Key::B, Modifiers::NONE);
    let heal = |h: &Harness<'_, PhotocraftApp>| inspect(h)["healBrush"]["size"].as_f64().unwrap() * 1000.0;
    assert!((heal(&h) - 20.0).abs() < 1e-6);
    key(&mut h, Key::CloseBracket, Modifiers::NONE);
    assert!((heal(&h) - 22.0).abs() < 1e-6, "{}", heal(&h));
    let r = ui(&mut h, json!({"local": {"heal": {"at": [0.4, 0.45]}}}));
    assert_eq!(r["selectedSpot"], 0);
    let spot = |h: &Harness<'_, PhotocraftApp>| {
        (inspect(h)["develop"]["spots"][0]["size"].as_f64().unwrap() * 1000.0, inspect(h)["develop"]["spots"][0]["feather"].as_f64().unwrap())
    };
    let (s0, f0) = spot(&h);
    key(&mut h, Key::OpenBracket, Modifiers::NONE);
    let (s1, _) = spot(&h);
    assert!((s1 - (s0 - (s0 * 0.1).max(1.0)).round()).abs() < 1e-6, "{s0} → {s1}");
    key(&mut h, Key::CloseBracket, Modifiers::SHIFT);
    assert_eq!(spot(&h).1, f0 + 10.0);
    ui(&mut h, json!({"undo": true}));
    assert_eq!(spot(&h).1, f0, "the feather key was one undo step");
    ui(&mut h, json!({"undo": true}));
    assert!((spot(&h).0 - s0).abs() < 1e-6, "and the size key another");

    // Red Eye: the selected eye's pupil size; it has no feather
    key(&mut h, Key::E, Modifiers::SHIFT);
    let r = ui(&mut h, json!({"local": {"redEye": {"at": [0.6, 0.4]}}}));
    assert_eq!(r["selectedEye"], 0);
    let eye = |h: &Harness<'_, PhotocraftApp>| inspect(h)["develop"]["red_eye"][0].clone();
    let before = eye(&h);
    key(&mut h, Key::CloseBracket, Modifiers::NONE);
    assert_eq!(eye(&h)["pupil_size"].as_f64(), Some(before["pupil_size"].as_f64().unwrap() + 5.0), "{}", eye(&h));
    let sized = eye(&h);
    key(&mut h, Key::CloseBracket, Modifiers::SHIFT);
    assert_eq!(eye(&h), sized, "no feather to change");
}

/// ⇧T goes to the Edit panel from another tool, opens Geometry, scrolls it into view and picks
/// Guided Upright with its tool; the Guided button's tooltip names the key.
#[test]
fn shift_t_opens_geometry_on_guided_upright() {
    let mut h = dialog();
    key(&mut h, Key::M, Modifiers::NONE);
    assert_eq!(inspect(&h)["tool"], "masking");
    key(&mut h, Key::T, Modifiers::SHIFT);
    settle(&mut h);
    let r = inspect(&h);
    assert_eq!(r["tool"], "edit");
    assert_eq!(r["develop"]["geometry"]["upright"], "guided", "{}", r["develop"]["geometry"]);
    assert_eq!(r["guidedUprightTool"]["active"], true);
    assert!(r["sections"].as_array().unwrap().iter().any(|s| s == "geometry"), "{}", r["sections"]);
    assert!(r["sectionsInView"].as_array().unwrap().iter().any(|s| s == "geometry"), "scrolled to: {}", r["sectionsInView"]);
    let guided = h.get_by_label("Guided");
    assert_eq!(guided.accesskit_node().toggled(), Some(egui::accesskit::Toggled::True), "Guided is the selected mode");
    // ⇧T again puts the tool down and keeps Guided
    key(&mut h, Key::T, Modifiers::SHIFT);
    let r = inspect(&h);
    assert_eq!(r["guidedUprightTool"]["active"], false);
    assert_eq!(r["develop"]["geometry"]["upright"], "guided");
}

/// ⌘1 … number the Edit panel's section headers in the order it draws them (Light, Color,
/// Effects, then the rest), open and scroll to one, and fold it when it is open and in view.
/// Their tooltips say so with the platform's modifier.
#[test]
fn command_numbers_open_the_edit_sections_in_panel_order() {
    let mut h = dialog();
    let order: Vec<String> = inspect(&h)["sectionOrder"].as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect();
    assert_eq!(&order[..3], ["light", "color", "effects"], "{order:?}");
    assert!(order.iter().any(|s| s == "geometry") && order.len() <= 9, "{order:?}");
    let open = |h: &Harness<'_, PhotocraftApp>| inspect(h)["sections"].as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect::<Vec<_>>();
    let digits = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
    // from Heal, ⌘2 goes to Edit and opens Color
    key(&mut h, Key::B, Modifiers::NONE);
    key(&mut h, Key::Num2, Modifiers::COMMAND);
    assert_eq!(inspect(&h)["tool"], "edit");
    assert!(open(&h).contains(&"color".to_string()), "{:?}", open(&h));
    // from every section folded, each one's number opens it and brings its header into view
    ui(&mut h, json!({"local": {"sections": []}}));
    for (n, id) in order.iter().enumerate() {
        key(&mut h, digits[n], Modifiers::COMMAND);
        h.run_steps(2);
        let r = inspect(&h);
        assert!(open(&h).contains(id), "⌘{} opens {id}: {:?}", n + 1, open(&h));
        assert!(r["sectionsInView"].as_array().unwrap().iter().any(|s| s == id.as_str()), "⌘{} scrolls to {id}: {}", n + 1, r["sectionsInView"]);
    }
    // the last one is open and in view: its number again folds it
    let last = order.len() - 1;
    key(&mut h, digits[last], Modifiers::COMMAND);
    assert!(!open(&h).contains(&order[last]), "folded: {:?}", open(&h));
    // a number past the last section does nothing
    if order.len() < 9 {
        let was = open(&h);
        key(&mut h, Key::Num9, Modifiers::COMMAND);
        assert_eq!(open(&h), was);
    }
    // the headers' tooltips name the keys
    ui(&mut h, json!({"local": {"sections": []}}));
    let light = h.get_by_label("Light");
    light.hover();
    h.run_steps(30);
    let tip = format!("Light ({})", pretty("Cmd+1"));
    assert!(h.query_by_label_contains(&tip).is_some(), "the header's tooltip: {tip}");
}

/// No shortcut fires while a slider's value is being typed: the keys go to the field.
#[test]
fn shortcuts_wait_while_a_value_is_typed() {
    let mut h = dialog();
    // click Exposure's value: its field opens with the keyboard
    let track = h.get_by_label("Exposure").rect();
    let at = egui::pos2(track.right() - 30.0, track.top() - 8.0);
    h.event(Event::PointerMoved(at));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
    assert!(h.ctx.text_edit_focused(), "the value field has the keyboard");
    let behind = app_state(&h);
    let sections = inspect(&h)["sections"].clone();
    for (k, m) in [
        (Key::M, Modifiers::NONE),
        (Key::B, Modifiers::NONE),
        (Key::T, Modifiers::SHIFT),
        (Key::Num3, Modifiers::COMMAND),
        (Key::CloseBracket, Modifiers::NONE),
    ] {
        key(&mut h, k, m);
    }
    let r = inspect(&h);
    assert_eq!(r["tool"], "edit");
    assert_eq!(r["develop"]["geometry"]["upright"], "off");
    assert_eq!(r["sections"], sections);
    assert_eq!(app_state(&h), behind);
    // Return leaves the field; then the keys work again
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert!(!h.ctx.text_edit_focused());
    key(&mut h, Key::M, Modifiers::NONE);
    assert_eq!(inspect(&h)["tool"], "masking");
}

/// Geometry's Upright modes are icon buttons, each with its name as its label and the selected
/// one marked; a click picks one.
#[test]
fn upright_modes_are_labelled_icon_buttons() {
    let gpu = std::env::var_os("PHOTOCRAFT_LC_SNAPSHOTS").is_some();
    let _lock = gpu.then(photocraft_testkit::gpu_lock);
    let mut h = dialog_on(gpu);
    ui(&mut h, json!({"local": {"sections": ["geometry"]}}));
    let names = ["Off", "Auto", "Level", "Vertical", "Full", "Guided"];
    let selected = |h: &Harness<'_, PhotocraftApp>, name: &str| {
        // the Edit header's own Auto shares a name: Upright's is the one in Geometry's row
        h.query_all_by_role_and_label(egui::accesskit::Role::Button, name).any(|n| n.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True))
    };
    for name in names {
        assert!(h.query_all_by_role_and_label(egui::accesskit::Role::Button, name).count() >= 1, "Upright {name}");
    }
    assert!(selected(&h, "Off") && !selected(&h, "Vertical"));
    h.query_all_by_label("Vertical").next().unwrap().click();
    settle(&mut h);
    assert_eq!(inspect(&h)["develop"]["geometry"]["upright"], "vertical");
    assert!(selected(&h, "Vertical") && !selected(&h, "Off"));
    if let Ok(dir) = std::env::var("PHOTOCRAFT_LC_SNAPSHOTS") {
        std::fs::create_dir_all(&dir).unwrap();
        // Geometry alone open and scrolled to the top (its ⌘ number), the pointer off the buttons
        let order = inspect(&h)["sectionOrder"].clone();
        let n = order.as_array().unwrap().iter().position(|s| s == "geometry").unwrap();
        ui(&mut h, json!({"local": {"sections": []}}));
        let digits = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
        key(&mut h, digits[n], Modifiers::COMMAND);
        h.event(Event::PointerMoved(egui::pos2(10.0, 10.0)));
        settle(&mut h);
        h.render().unwrap().save(std::path::Path::new(&dir).join("upright-icons.png")).unwrap();
    }
}
