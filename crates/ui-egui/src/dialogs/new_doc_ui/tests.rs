use super::*;
use crate::new_doc_remember_tests::{app, fields, harness, open, press_create};
use egui_kittest::kittest::Queryable;
use photocraft_engine::color_cmds::ColorExt;
use photocraft_engine::prefs::SavedPrefsExt;
use photocraft_engine::presets::PresetsExt;

/// The default settings with `edit` applied.
fn options(edit: impl FnOnce(&mut DocumentOptions)) -> DocumentOptions {
    let mut options = DocumentOptions::default();
    edit(&mut options);
    options
}

/// #321: New Document offers the working space and the other profiles of the mode's space.
#[test]
fn color_profile_menu_lists_the_profiles_of_the_modes_space() {
    use craft::cms::Builtin;
    let app = app();
    let lists = profiles(&app);
    let of = |mode: DialogMode| lists.iter().find(|p| p.mode == mode).unwrap_or_else(|| panic!("{mode:?} has a list"));
    let rgb = of(DialogMode::Rgb);
    assert_eq!(rgb.working, app.session.color().working(photocraft_color::ColorMode::Rgb).description);
    assert!(rgb.others.contains(&Builtin::AdobeRgbCompat.id().to_string()));
    assert!(!rgb.others.contains(&Builtin::CoatedCmyk.id().to_string()));
    let cmyk = of(DialogMode::Cmyk);
    assert!(cmyk.others.contains(&Builtin::CoatedCmyk.id().to_string()) && !cmyk.others.contains(&Builtin::Srgb.id().to_string()));
    // Every built-in entry is one `file.new` takes for that mode; an installed one is a path.
    for list in &lists {
        let mode = create_params(&options(|o| o.color_mode = list.mode))["mode"].clone();
        for profile in list.others.iter().filter(|p| !p.ends_with(".icc")) {
            let mut s = photocraft_engine::Session::new();
            s.execute("file.new", json!({"width": 2, "height": 2, "mode": mode, "profile": profile})).unwrap_or_else(|e| panic!("{mode} {profile}: {e}"));
        }
    }
}

/// Our format at the engine boundary: what `file.new` takes for each setting, and nothing of the dialog's own.
#[test]
fn preset_sets_size_resolution_and_create_params_drop_ui_keys() {
    let table = [
        (
            options(|o| {
                o.name = "A4".into();
                o.size = PixelSize::new(2480, 3508);
                o.pixels_per_inch = 300.0;
                o.unit = UnitKind::Inches;
                o.resolution_unit = UnitKind::PixelsPerCentimetre;
            }),
            json!({"name": "A4", "width": 2480, "height": 3508, "resolution": 300.0, "mode": "rgb", "depth": 8, "background": "white", "artboard": false}),
        ),
        (
            options(|o| {
                o.color_mode = DialogMode::Cmyk;
                o.depth = BitDepth::Sixteen;
                o.background = BackgroundContents::Transparent;
                o.profile = Some("coated-cmyk".into());
                o.artboard = true;
            }),
            json!({"name": "", "width": 2100, "height": 1500, "resolution": 300.0, "mode": "cmyk", "depth": 16, "background": "transparent", "artboard": true, "profile": "coated-cmyk"}),
        ),
        (
            options(|o| {
                o.color_mode = DialogMode::Grayscale;
                o.depth = BitDepth::ThirtyTwo;
                o.background = BackgroundContents::Custom([1.0, 0.0, 0.0]);
            }),
            json!({"name": "", "width": 2100, "height": 1500, "resolution": 300.0, "mode": "gray", "depth": 32, "background": "#ff0000", "artboard": false}),
        ),
        (
            options(|o| {
                o.color_mode = DialogMode::Lab;
                o.background = BackgroundContents::BackgroundColor;
            }),
            json!({"name": "", "width": 2100, "height": 1500, "resolution": 300.0, "mode": "lab", "depth": 8, "background": "backgroundColor", "artboard": false}),
        ),
        (
            options(|o| o.background = BackgroundContents::Black),
            json!({"name": "", "width": 2100, "height": 1500, "resolution": 300.0, "mode": "rgb", "depth": 8, "background": "black", "artboard": false}),
        ),
    ];
    for (settings, expected) in table {
        let params = create_params(&settings);
        assert_eq!(params, expected);
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", params).unwrap_or_else(|e| panic!("{expected}: {e}"));
        let d = &s.active().unwrap().doc;
        assert_eq!((d.size.width, d.size.height, d.resolution_dpi), (settings.size.width, settings.size.height, settings.pixels_per_inch as f32));
        assert_eq!(d.artboards().len(), usize::from(settings.artboard));
    }
    // The stored form adds the display units, drops the name and the artboard, and reads back as the same document.
    let settings = options(|o| {
        o.size = PixelSize::new(1600, 800);
        o.pixels_per_inch = 254.0;
        o.unit = UnitKind::Millimetres;
        o.resolution_unit = UnitKind::PixelsPerCentimetre;
        o.color_mode = DialogMode::Cmyk;
        o.background = BackgroundContents::Custom([0.0, 1.0, 0.0]);
    });
    let stored = to_settings(&settings);
    assert_eq!((&stored["unit"], &stored["resolutionUnit"]), (&json!("mm"), &json!("cm")));
    assert!(stored.get("name").is_none() && stored.get("artboard").is_none());
    let read: DocumentSettings = serde_json::from_value(stored).unwrap();
    assert_eq!(to_options(&read), settings);
    // A Background Color stored with its captured colour (a preset saved before the port) keeps that colour; one without follows the toolbox.
    let captured = DocumentSettings { background: "backgroundColor".into(), background_color: Some([0.1, 0.2, 0.3]), ..read.clone() };
    assert_eq!(to_options(&captured).background, BackgroundContents::Custom([0.1, 0.2, 0.3]));
    let following = DocumentSettings { background: "backgroundColor".into(), background_color: None, ..read };
    assert_eq!(to_options(&following).background, BackgroundContents::BackgroundColor);
}

/// #4789: the swatch beside Background Contents opens the Color Picker; its colour becomes the new document's
/// background.
#[test]
fn a_custom_background_color_is_picked_and_used() {
    let mut h = harness(app());
    let id = open(&mut h);
    h.get_by_label("Custom background color").click();
    h.run_steps(3);
    let opened = h.state().framework.dialogs.open_dialogs();
    let picker = opened.iter().find(|d| d.id != id).map(|d| d.id).expect("the Color Picker opens over the dialog");
    assert_eq!(opened.len(), 2);
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, picker, "color", json!("#336699")).unwrap();
    crate::dialogs::confirm(h.state_mut(), &ctx, picker).expect("OK is enabled");
    h.run_steps(3);
    assert!(matches!(crate::new_doc_remember_tests::options(h.state(), id).background, BackgroundContents::Custom(_)), "{}", fields(h.state(), id));
    assert!(press_create(&mut h, id));
    let doc = &h.state().session.active().unwrap().doc;
    let px = photocraft_compose::flatten(doc).px[0];
    assert!((px[0] * 255.0 - 51.0).abs() < 1.0 && (px[1] * 255.0 - 102.0).abs() < 1.0 && (px[2] * 255.0 - 153.0).abs() < 1.0, "{px:?}");
}

/// Saving and deleting a preset in the dialog reach the engine's store, which keeps them across a restart and
/// gives them back in the next dialog's Saved tab.
#[test]
fn save_restart_select_create_and_delete_through_the_dialog() {
    let mut h = harness(app());
    let id = open(&mut h);
    // The icon beside Name opens the naming pane with its proposed name selected, so typing replaces it.
    h.get_by_label("Save Document Preset").click();
    h.run_steps(3);
    h.event(egui::Event::Text("Wide".into()));
    h.run_steps(1);
    h.get_by_label("Save Preset").click();
    h.run_steps(3);
    let stored = h.state().session.presets().documents.clone();
    assert_eq!(stored.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Wide"]);
    let size = (stored[0].settings.width, stored[0].settings.height);
    assert_eq!(size, (1920, 1080), "the dialog's settings are stored");

    // A restart: the preset comes back from the preferences into the next dialog's library.
    let mut restarted = photocraft_engine::Session::new();
    restarted.load_prefs_json(&h.state().session.prefs_to_json()).unwrap();
    let mut h2 = harness(PhotocraftApp::new(restarted, crate::Services::default()));
    let id2 = open(&mut h2);
    let library = h2
        .state()
        .framework
        .dialogs
        .get_by_id::<HostedNewDocument>(id2)
        .expect("the dialog is open")
        .saved_presets()
        .items()
        .into_iter()
        .map(|i| i.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(library, ["Wide"]);
    assert_eq!(fields(h2.state(), id2)["preset"], "Wide", "the dialog opens on the saved preset's card");

    // Delete it in the first dialog: the engine drops it too.

    assert_eq!(fields(h.state(), id)["tab"], "saved", "a saved preset opens the Saved tab");
    h.get_by_label("Delete").click();
    h.run_steps(3);
    assert!(h.state().session.presets().documents.is_empty());
}

/// Opens New Document, applies `edit` to the settings it shows and presses Create.
fn create_with(h: &mut crate::new_doc_remember_tests::AppHarness, edit: impl FnOnce(&mut DocumentOptions)) {
    let id = open(h);
    let mut shown = crate::new_doc_remember_tests::options(h.state(), id);
    edit(&mut shown);
    crate::new_doc_remember_tests::set_options(h, id, &shown);
    assert!(press_create(h, id), "Create is enabled");
}

/// #14030: Create with Bitmap makes a 1-bit Bitmap document, and the created settings become the first Recent card.
#[test]
fn create_makes_a_bitmap_document() {
    let mut h = harness(app());
    create_with(&mut h, |o| {
        o.size = PixelSize::new(32, 24);
        o.color_mode = DialogMode::Bitmap;
        o.depth = BitDepth::One;
        o.background = BackgroundContents::Black;
    });
    let doc = &h.state().session.active().expect("a document was created").doc;
    assert_eq!((doc.size.width, doc.size.height), (32, 24));
    assert_eq!(doc.mode, ColorMode::Bitmap, "not a silent RGB document");
    let recent = &h.state().session.presets().recent_documents;
    assert_eq!((recent[0].mode.as_str(), recent[0].depth, recent[0].background.as_str()), ("bitmap", 1, "black"));
    assert_eq!(to_options(&recent[0]).color_mode, DialogMode::Bitmap, "the Recent card opens on Bitmap again");
}

/// #14034: Create gives View › Pixel Aspect Ratio the picked ratio, a saved custom one included, and Square Pixels
/// resets a ratio picked in the view menu before.
#[test]
fn create_gives_the_new_document_the_picked_pixel_aspect() {
    let mut app = app();
    app.ui.shell.custom_pars.push(crate::workspace_ui::CustomPar { name: "Cinema".into(), ratio: 1.25 });
    assert_eq!(custom_pixel_aspects(&app), [("custom:1.25:Cinema".to_owned(), "Cinema".to_owned())], "saved ratios follow the presets");
    let mut h = harness(app);
    create_with(&mut h, |o| o.pixel_aspect = "d1DvNtsc".into());
    assert_eq!(h.state().ui.view.pixel_aspect, "d1DvNtsc");
    create_with(&mut h, |o| o.pixel_aspect = "custom:1.25:Cinema".into());
    assert_eq!(crate::view_cmds::pixel_aspect_ratio(&h.state().ui.view.pixel_aspect), 1.25);
    h.state_mut().ui.view.pixel_aspect = "anamorphic2To1".into();
    create_with(&mut h, |o| o.pixel_aspect = SQUARE_PIXELS.into());
    assert_eq!(h.state().ui.view.pixel_aspect, SQUARE_PIXELS);
    // A custom ratio deleted since the dialog remembered it shows as Square Pixels, and Create gives that.
    h.state_mut().ui.shell.custom_pars.clear();
    create_with(&mut h, |o| o.pixel_aspect = "custom:1.25:Cinema".into());
    assert_eq!(h.state().ui.view.pixel_aspect, SQUARE_PIXELS);
}
