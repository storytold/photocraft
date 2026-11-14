//! #13683: Contact Sheet II shows its fields in the reference application's four groups, with
//! Mode, Bit Depth, Place and the resolution unit as dropdowns, and Units converting the size.

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

pub(super) type H = egui_kittest::Harness<'static, PhotocraftApp>;

/// The whole app's dialogs with Contact Sheet II open from the menu, without a document (the
/// command needs none).
pub(super) fn harness() -> H {
    harness_with(PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default()))
}

/// [`harness`] for `app`.
pub(super) fn harness_with(app: PhotocraftApp) -> H {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, super::COMMAND, json!({})).unwrap();
    h.run_steps(4);
    h
}

pub(super) fn fields(h: &H) -> &Map<String, Value> {
    &h.state().ui.dialogs.last().expect("the dialog is open").fields
}

/// The number the value field `name` shows.
fn shown(h: &H, name: &str) -> f64 {
    h.get_by_role_and_label(Role::SpinButton, name).accesskit_node().numeric_value().unwrap_or(f64::NAN)
}

/// Types `text` into the value field `name`.
pub(super) fn type_into(h: &mut H, name: &str, text: &str) {
    let value = shown(h, name);
    crate::file_ui::tests::type_into_field(h, value, text);
}

fn num(h: &H, key: &str) -> f64 {
    fields(h)[key].as_f64().unwrap_or_else(|| panic!("{key} is a number"))
}

/// Opens the dropdown `name` and picks `entry`.
pub(super) fn pick(h: &mut H, name: &str, entry: &str) {
    h.get_by_role_and_label(Role::ComboBox, name).click();
    h.run_steps(3);
    h.get_all_by_label(entry).rfind(|n| n.accesskit_node().role() != Role::ComboBox).unwrap_or_else(|| panic!("no entry {entry} in {name}")).click();
    h.run_steps(3);
}

/// A new folder in the temp folder holding one 40 x 20 PNG, `a.png`.
pub(super) fn image_folder(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pc-contact-sheet-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut image = photocraft_engine::Session::new();
    image.execute("file.new", json!({"width": 40, "height": 20, "background": "white"})).unwrap();
    let png = photocraft_io::export(&image.active().unwrap().doc, "a.png", &Default::default()).unwrap().bytes;
    std::fs::write(dir.join("a.png"), png).unwrap();
    dir
}

/// The dropdown `name` lists `expected` when opened; a second click closes it.
fn entries(h: &mut H, name: &str, expected: &[&str]) {
    h.get_by_role_and_label(Role::ComboBox, name).click();
    h.run_steps(3);
    for entry in expected {
        assert!(h.query_all_by_label(entry).any(|n| n.accesskit_node().role() != Role::ComboBox), "{name} lists {entry}");
    }
    h.get_by_role_and_label(Role::ComboBox, name).click();
    h.run_steps(3);
}

#[test]
fn the_four_groups_and_their_controls_follow_the_reference_order() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let h = harness();
        let titles = ["Source Images", "Document", "Thumbnails", "Use Filename as Caption"];
        let top = |label: &str| h.get_by_label(label).rect().top();
        for pair in titles.windows(2) {
            assert!(top(pair[0]) < top(pair[1]), "group order: {pair:?}");
        }
        // Each group's controls, top to bottom (Resolution's unit sits on its row).
        let controls = [
            (Role::TextInput, "Input"),
            (Role::ComboBox, "Units"),
            (Role::SpinButton, "Width"),
            (Role::SpinButton, "Height"),
            (Role::SpinButton, "Resolution"),
            (Role::CheckBox, "Flatten All Layers"),
            (Role::ComboBox, "Mode"),
            (Role::ComboBox, "Bit Depth"),
            (Role::ComboBox, "Place"),
            (Role::SpinButton, "Columns"),
            (Role::SpinButton, "Rows"),
            (Role::CheckBox, "Rotate For Best Fit"),
            (Role::CheckBox, "Use Auto-Spacing"),
            (Role::CheckBox, "Use Filename as Caption"),
            (Role::TextInput, "Font"),
            (Role::SpinButton, "Font size"),
        ];
        let y = |(role, label): (Role, &str)| h.get_by_role_and_label(role, label).rect().center().y;
        for pair in controls.windows(2) {
            assert!(y(pair[0]) < y(pair[1]), "control order: {pair:?}");
        }
        let (field, unit) = (h.get_by_role_and_label(Role::SpinButton, "Resolution").rect(), h.get_by_role_and_label(Role::ComboBox, "Resolution Unit").rect());
        assert!((field.center().y - unit.center().y).abs() < 2.0 && field.right() <= unit.left(), "the unit follows the Resolution field");
        // Each group's title comes before its first control and after the last of the group above.
        for (title, first, above) in [
            ("Source Images", (Role::TextInput, "Input"), None),
            ("Document", (Role::ComboBox, "Units"), Some((Role::TextInput, "Input"))),
            ("Thumbnails", (Role::ComboBox, "Place"), Some((Role::ComboBox, "Bit Depth"))),
            ("Use Filename as Caption", (Role::TextInput, "Font"), Some((Role::CheckBox, "Use Auto-Spacing"))),
        ] {
            assert!(top(title) < y(first) && above.is_none_or(|a| y(a) < top(title)), "{title} holds its controls");
        }
    });
}

#[test]
fn mode_bit_depth_and_place_are_dropdowns_with_the_reference_entries() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        entries(&mut h, "Mode", &["Grayscale", "RGB Color", "CMYK Color", "Lab Color"]);
        entries(&mut h, "Bit Depth", &["8-bit", "16-bit"]);
        entries(&mut h, "Place", &["across first", "down first"]);
        entries(&mut h, "Units", &["inches", "cm", "pixels"]);
        entries(&mut h, "Resolution Unit", &["pixels/inch", "pixels/cm"]);
        pick(&mut h, "Mode", "Grayscale");
        pick(&mut h, "Bit Depth", "16-bit");
        pick(&mut h, "Place", "down first");
        let f = fields(&h);
        assert_eq!((&f["mode"], &f["depth"], &f["placeAcrossFirst"]), (&json!("gray"), &json!(16), &json!(false)));
    });
}

#[test]
fn switching_units_converts_width_and_height() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        let size = |h: &H| (num(h, "width"), num(h, "height"));
        let close = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6;
        assert_eq!(size(&h), (8.0, 10.0));
        for (unit, want) in [("cm", (20.32, 25.4)), ("pixels", (2400.0, 3000.0)), ("inches", (8.0, 10.0)), ("pixels", (2400.0, 3000.0)), ("cm", (20.32, 25.4))]
        {
            pick(&mut h, "Units", unit);
            assert_eq!(fields(&h)["units"], unit);
            assert!(close(size(&h), want), "{unit}: {:?}", size(&h));
        }
        assert_eq!(num(&h, "resolution"), 300.0, "the resolution stays");
        // A typed value reaches the command as typed, not as the field's f32.
        type_into(&mut h, "Width", "20.33");
        assert_eq!(fields(&h)["width"], json!(20.33));
    });
}

#[test]
fn switching_the_resolution_unit_converts_the_shown_value_only() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        let shown = |h: &H| shown(h, "Resolution");
        assert!((shown(&h) - 300.0).abs() < 1e-3);
        pick(&mut h, "Resolution Unit", "pixels/cm");
        assert!((shown(&h) - 118.11).abs() < 0.01, "300 pixels/inch is 118.11 pixels/cm, got {}", shown(&h));
        assert_eq!(num(&h, "resolution"), 300.0, "the command still gets pixels/inch");
        type_into(&mut h, "Resolution", "100");
        assert!((num(&h, "resolution") - 254.0).abs() < 1e-9, "100 pixels/cm is 254 pixels/inch, got {}", num(&h, "resolution"));
        pick(&mut h, "Resolution Unit", "pixels/inch");
        assert!((shown(&h) - 254.0).abs() < 1e-3);
    });
}

#[test]
fn unticking_the_caption_greys_the_font_settings() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        let greyed = |h: &H| {
            [(Role::TextInput, "Font"), (Role::SpinButton, "Font size")]
                .map(|(role, label)| h.get_by_role_and_label(role, label).accesskit_node().is_disabled())
        };
        assert_eq!(greyed(&h), [false, false]);
        h.get_by_role_and_label(Role::CheckBox, "Use Filename as Caption").click();
        h.run_steps(3);
        assert_eq!(fields(&h)["caption"], false);
        assert_eq!(greyed(&h), [true, true]);
    });
}

/// OK sends the params the form sent before the layout: the same keys and values, the resolution
/// in pixels/inch whatever unit it is shown in, and `placeAcrossFirst` and `depth` as before.
#[test]
fn ok_runs_the_command_with_the_same_params() {
    crate::i18n::with_language(crate::i18n::Lang::EN, || {
        let mut h = harness();
        let before = crate::filter_dialog::params_of(fields(&h));
        assert_eq!(
            before,
            json!({"input": "", "units": "inches", "width": 8.0, "height": 10.0, "resolution": 300.0, "mode": "rgb", "depth": 8, "columns": 5, "rows": 6, "placeAcrossFirst": true, "autoSpacing": true, "rotateForBestFit": false, "caption": true, "font": photocraft_text::fonts::DEFAULT_FAMILY, "fontSize": 12.0, "flatten": false, "includeSubfolders": true, "groupByFolder": false})
        );
        pick(&mut h, "Resolution Unit", "pixels/cm");
        assert_eq!(crate::filter_dialog::params_of(fields(&h)), before, "the unit is view state");

        let dir = image_folder("layout");
        h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("input".into(), json!(dir.to_string_lossy()));
        h.get_by_role_and_label(Role::Button, "OK").click();
        h.run_steps(4);
        std::fs::remove_dir_all(&dir).ok();
        let app = h.state();
        assert!(app.ui.dialogs.is_empty(), "OK closes the dialog");
        assert_eq!(app.session.documents().len(), 1, "one sheet");
        let sheet = &app.session.active().unwrap().doc;
        assert_eq!((sheet.size.width, sheet.size.height, sheet.resolution_dpi), (2400, 3000, 300.0));
    });
}
