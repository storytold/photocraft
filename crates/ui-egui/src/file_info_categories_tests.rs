//! File Info categories through the actual menu, dialog host, widgets and control channel.

use std::sync::Arc;

use egui::accesskit::Role;
use egui::{Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::app::framework::BuildApp as _;

type H = Harness<'static, Box<PhotocraftApp>>;

const CATEGORIES: [(&str, &str); 12] = [
    ("basic", "Basic"),
    ("cameraData", "Camera Data"),
    ("origin", "Origin"),
    ("iptc", "IPTC"),
    ("iptcExtension", "IPTC Extension"),
    ("gpsData", "GPS Data"),
    ("audioData", "Audio Data"),
    ("videoData", "Video Data"),
    ("history", "History"),
    ("dicom", "DICOM"),
    ("assetTags", "Asset Tags"),
    ("rawData", "Raw Data"),
];

const CAMERA: [(&str, &str); 10] = [
    ("make", "Make"),
    ("model", "Model"),
    ("owner", "Owner"),
    ("lens", "Lens"),
    ("focalLength", "Focal Length"),
    ("exposure", "Exposure"),
    ("imageSize", "Image Size"),
    ("orientation", "Orientation"),
    ("resolution", "Resolution"),
    ("flash", "Flash"),
];

const PACKET: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:aux="http://ns.adobe.com/exif/1.0/aux/" xmlns:exif="http://ns.adobe.com/exif/1.0/" xmlns:tiff="http://ns.adobe.com/tiff/1.0/" xmlns:fixture="https://example.invalid/metadata-fixture/" xmp:CreateDate="2024-02-17T12:40:19+01:00" xmp:ModifyDate="2024-02-17T12:41:20+01:00" xmp:MetadataDate="2024-02-17T12:42:21+01:00" fixture:opaque="preserve-me" aux:OwnerName="Fixture owner" exif:PixelXDimension="6016" exif:PixelYDimension="4000" tiff:Orientation="1" tiff:XResolution="300" tiff:YResolution="300" tiff:ResolutionUnit="2" exif:Flash="1"/></rdf:RDF></x:xmpmeta>"#;

/// Five camera rows the old nine-field CameraInfo writer does not carry. This is a
/// classic little-endian TIFF EXIF payload, using the same test-writer pattern as
/// engine web_cmds::tests::camera_exif; no parser or production helper is added.
fn extra_camera_exif() -> Vec<u8> {
    let entry = |bytes: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: u32| {
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    };
    const IFD0: u32 = 8;
    const EXIF: u32 = IFD0 + 2 + 5 * 12 + 4;
    const OWNER: u32 = EXIF + 2 + 4 * 12 + 4;
    const X_RESOLUTION: u32 = OWNER + 12;
    const Y_RESOLUTION: u32 = X_RESOLUTION + 8;
    let mut bytes = b"II*\0".to_vec();
    bytes.extend_from_slice(&IFD0.to_le_bytes());
    bytes.extend_from_slice(&5u16.to_le_bytes());
    entry(&mut bytes, 0x0112, 3, 1, 6); // Orientation: rotated 90 degrees clockwise.
    entry(&mut bytes, 0x011a, 5, 1, X_RESOLUTION);
    entry(&mut bytes, 0x011b, 5, 1, Y_RESOLUTION);
    entry(&mut bytes, 0x0128, 3, 1, 3); // ResolutionUnit: centimetres.
    entry(&mut bytes, 0x8769, 4, 1, EXIF);
    bytes.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(bytes.len(), EXIF as usize);
    bytes.extend_from_slice(&4u16.to_le_bytes());
    entry(&mut bytes, 0x9209, 3, 1, 0); // Flash: zero is a present value, not a missing row.
    entry(&mut bytes, 0xa002, 4, 1, 4032);
    entry(&mut bytes, 0xa003, 4, 1, 3024);
    entry(&mut bytes, 0xa430, 2, 11, OWNER);
    bytes.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(bytes.len(), OWNER as usize);
    bytes.extend_from_slice(b"EXIF owner\0\0"); // Eleven ASCII bytes, then one alignment byte.
    assert_eq!(bytes.len(), X_RESOLUTION as usize);
    for numerator in [240u32, 180] {
        bytes.extend_from_slice(&numerator.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
    }
    assert_eq!(bytes.len(), Y_RESOLUTION as usize + 8);
    bytes
}

fn harness(size: egui::Vec2, camera: bool) -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48, "name": "first.psd"})).unwrap();
    if camera {
        app.session
            .edit("Metadata fixture", |doc, _| {
                doc.metadata.xmp = Some(PACKET.into());
                doc.metadata.exif = Some(Arc::new(photocraft_algo::exif::build(&photocraft_algo::exif::CameraInfo {
                    make: Some("Fixture make".into()),
                    model: Some("Fixture model".into()),
                    lens: Some("Fixture lens".into()),
                    exposure_time: Some(1.0 / 250.0),
                    f_number: Some(5.6),
                    iso: Some(200.0),
                    focal_length: Some(35.0),
                    focal_length_35mm: Some(52.0),
                    date_taken: Some("2024:02:17 12:40:19".into()),
                })));
                Ok(())
            })
            .unwrap();
    }
    app.sync_views();
    let mut h = Harness::builder()
        .with_context(app.ctx().clone())
        .with_size(size)
        .with_step_dt(1.0 / 30.0)
        .with_max_steps(64)
        .build_app(|ui, app| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
    h.run_steps(4);
    h
}

fn call(h: &mut H, method: &str, params: Value) -> Value {
    let ctx = h.ctx.clone();
    let (request, _rx) = crate::control::ControlRequest::new(method, params);
    match crate::control::handle(h.state_mut(), &ctx, &request) {
        crate::control::Reply::Done(reply) => reply,
        _ => panic!("{method} should complete immediately"),
    }
}

fn dialog(h: &mut H) -> Value {
    let inspection = call(h, "ui.inspect", json!({}));
    let dialogs = inspection["result"]["dialogs"].as_array().unwrap();
    let matching: Vec<_> = dialogs.iter().filter(|dialog| dialog["key"] == "file.fileInfo").cloned().collect();
    assert_eq!(matching.len(), 1, "{inspection}");
    matching[0].clone()
}

fn open(h: &mut H) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.fileInfo", json!({})).unwrap();
    h.run_steps(5);
    assert_eq!(dialog(h)["key"], "file.fileInfo");
}

fn set(h: &mut H, field: &str, value: Value) {
    let reply = call(h, "ui.dialog.set", json!({"dialog": "top", "field": field, "value": value}));
    assert_eq!(reply["ok"], true, "{reply}");
    h.run_steps(3);
}

fn camera_value_node(node: &egui_kittest::kittest::AccessKitNode<'_>, label: &str) -> bool {
    use egui::accesskit::Role;

    fn value_input(node: egui_kittest::kittest::AccessKitNode<'_>) -> bool {
        (node.role() == Role::MultilineTextInput && node.is_hidden()) || node.children().any(value_input)
    }

    // The selectable value part owns both this label and its folded text input.
    // A side caption has only its label; stay inside the immediate part container.
    node.role() == Role::Label
        && node.label().as_deref() == Some(label)
        && node.value().is_some()
        && node.parent().is_some_and(|part| part.role() == Role::GenericContainer && part.children().any(value_input))
}

fn camera_value(h: &H, label: &str) -> String {
    h.get_by(|node| camera_value_node(node, label)).accesskit_node().value().unwrap().to_owned()
}

#[test]
fn file_info_lists_twelve_categories_in_order_with_only_measured_tooltips() {
    let mut h = harness(vec2(1200.0, 900.0), false);
    open(&mut h);
    assert_eq!(dialog(&mut h)["fields"]["category"], "basic");
    let mut rows: Vec<_> = h
        .query_all_by_role(Role::ListItem)
        .filter_map(|row| {
            row.accesskit_node().label().filter(|name| CATEGORIES.iter().any(|(_, label)| *label == name.as_str())).map(|label| (row.rect().top(), label))
        })
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(rows.iter().map(|(_, label)| label.as_str()).collect::<Vec<_>>(), CATEGORIES.map(|(_, label)| label));
    for (label, expected) in [
        ("Camera Data", "Camera Data :: Contains the most important TIFF and Exif properties."),
        ("Origin", "Origin :: Displays information about the origin of the asset"),
        ("IPTC", "IPTC :: International Press Telecommunications Council Core schema properties"),
        ("Raw Data", "Raw Data :: Displays the XMP packet in raw RDF/XML form."),
    ] {
        assert_eq!(h.get_by_role_and_label(Role::ListItem, label).accesskit_node().description().as_deref(), Some(expected));
    }
    for label in ["Basic", "IPTC Extension", "GPS Data", "Audio Data", "Video Data", "History", "DICOM", "Asset Tags"] {
        assert_eq!(h.get_by_role_and_label(Role::ListItem, label).accesskit_node().description(), None, "unmeasured {label}");
    }
    h.get_by_role_and_label(Role::TextInput, "Document Title");
    assert!(h.query_by_role_and_label(Role::Heading, "Camera Information").is_none(), "Camera Data isn't appended to Basic");
    h.get_by_role_and_label(Role::Button, "Preferences");
    h.get_by_role_and_label(Role::Button, "Template Folder");
}

#[test]
fn category_click_and_control_switch_are_inspectable_and_reopen_on_the_last_page() {
    let mut h = harness(vec2(1200.0, 900.0), false);
    open(&mut h);
    let before = h.state().session.active().unwrap().doc.clone();
    h.get_by_role_and_label(Role::ListItem, "Camera Data").click();
    h.run_steps(4);
    assert_eq!(dialog(&mut h)["fields"]["category"], "cameraData");
    for (key, label) in CATEGORIES {
        set(&mut h, "category", json!(key));
        assert_eq!(dialog(&mut h)["fields"]["category"], key);
        assert_eq!(h.get_by_role_and_label(Role::ListItem, label).accesskit_node().is_selected(), Some(true));
    }
    let rejected = call(&mut h, "ui.dialog.set", json!({"dialog": "top", "field": "category", "value": "invented"}));
    assert_eq!(rejected["ok"], false);
    assert_eq!(dialog(&mut h)["fields"]["category"], "rawData");
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run_steps(3);
    assert!(h.state().framework.dialogs.is_empty());
    assert!(Arc::ptr_eq(&before, &h.state().session.active().unwrap().doc));
    open(&mut h);
    assert_eq!(dialog(&mut h)["fields"]["category"], "rawData");
    assert!(h.query_by_role_and_label(Role::TextInput, "Document Title").is_none());
}

#[test]
fn camera_page_shows_ten_empty_rows_and_keeps_existing_camera_formatting() {
    for populated in [false, true] {
        let mut h = harness(vec2(1200.0, 1000.0), populated);
        let info = h.state_mut().session.execute("file.fileInfo", json!({})).unwrap();
        open(&mut h);
        h.get_by_role_and_label(Role::ListItem, "Camera Data").click();
        h.run_steps(4);
        let mut last = f32::NEG_INFINITY;
        for (key, label) in CAMERA {
            let node = h.get_by(|node| camera_value_node(node, label));
            assert!(node.rect().top() >= last, "camera order {label}");
            last = node.rect().top();
            assert_eq!(camera_value(&h, label), info["camera"][key].as_str().unwrap_or_default(), "{label}");
        }
        h.get_by_role_and_label(Role::Heading, "Camera Information");
        h.get_by_role_and_label(Role::Heading, "Shot Information");
        assert_eq!(call(&mut h, "ui.dialog.set", json!({"dialog":"top", "field":"camera/owner", "value":"changed"}))["ok"], false);
        if populated {
            for (label, value) in [
                ("Make", "Fixture make"),
                ("Model", "Fixture model"),
                ("Lens", "Fixture lens"),
                ("Focal Length", "35 mm"),
                ("Exposure", "1/250 s"),
                ("Date Taken", "2024-02-17 12:40:19"),
                ("F-Stop", "f/5.6"),
                ("ISO Speed", "ISO 200"),
                ("Focal Length (35 mm)", "52 mm"),
            ] {
                assert_eq!(camera_value(&h, label), value);
            }
            for key in ["owner", "imageSize", "orientation", "resolution", "flash"] {
                assert!(info["camera"][key].as_str().is_some_and(|value| !value.is_empty()), "fixture supplies {key}: {info}");
            }
        }
    }
    // EXIF-only and conflicting XMP both use EXIF, including the present Flash=0.
    for conflicting_xmp in [false, true] {
        let mut h = harness(vec2(1200.0, 1000.0), false);
        h.state_mut()
            .session
            .edit("Five EXIF camera fields", |doc, _| {
                doc.metadata.exif = Some(Arc::new(extra_camera_exif()));
                // PACKET disagrees with EXIF on all five rows; EXIF must win, including Flash=0.
                doc.metadata.xmp = conflicting_xmp.then(|| PACKET.into());
                Ok(())
            })
            .unwrap();
        let info = h.state_mut().session.execute("file.fileInfo", json!({})).unwrap();
        let expected = [
            ("owner", "Owner", "EXIF owner"),
            ("imageSize", "Image Size", "4032 x 3024"),
            ("orientation", "Orientation", "Rotated 90° CW"),
            ("resolution", "Resolution", "240 x 180 / centimetres"),
            ("flash", "Flash", "No flash"),
        ];
        for (key, _, value) in expected {
            assert_eq!(info["camera"][key], value, "EXIF row {key}, conflicting XMP {conflicting_xmp}");
        }
        open(&mut h);
        h.get_by_role_and_label(Role::ListItem, "Camera Data").click();
        h.run_steps(4);
        for (_, label, value) in expected {
            assert_eq!(camera_value(&h, label), value, "shown EXIF row {label}, conflicting XMP {conflicting_xmp}");
        }
    }
}

#[test]
fn switching_category_resets_only_the_page_scroll_and_keeps_the_footer_fixed() {
    let mut h = harness(vec2(1100.0, 430.0), true);
    open(&mut h);
    h.get_by_role_and_label(Role::ListItem, "Camera Data").click();
    h.run_steps(8);
    let make_y = h.get_by(|node| camera_value_node(node, "Make")).rect().top();
    let footer = h.get_by_role_and_label(Role::Button, "OK").rect();
    let categories = h.get_by_role_and_label(Role::ListItem, "Basic").rect();
    h.hover_at(h.get_by(|node| camera_value_node(node, "Lens")).rect().center());
    for _ in 0..12 {
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        });
        h.run_steps(1);
    }
    let moved_y = h.query_by(|node| camera_value_node(node, "Make")).map(|node| node.rect().top());
    assert_ne!(moved_y, Some(make_y), "the page really scrolled");
    assert_eq!(h.get_by_role_and_label(Role::Button, "OK").rect(), footer);
    assert_eq!(h.get_by_role_and_label(Role::ListItem, "Basic").rect(), categories);
    h.get_by_role_and_label(Role::ListItem, "Basic").click();
    h.run_steps(4);
    h.get_by_role_and_label(Role::ListItem, "Camera Data").click();
    h.run_steps(32);
    let reset = h.get_by(|node| camera_value_node(node, "Make")).rect().top();
    assert!((reset - make_y).abs() < 1.0, "returned to page top: {make_y} -> {reset}");
    assert_eq!(h.get_by_role_and_label(Role::Button, "OK").rect(), footer);
}

#[test]
fn basic_widget_edits_cancel_without_mutation_then_ok_applies_once() {
    let mut h = harness(vec2(1200.0, 900.0), true);
    for commit in [false, true] {
        open(&mut h);
        let before = h.state().session.active().unwrap().doc.clone();
        let history = h.state().session.active().unwrap().history.entries();
        h.get_by_role_and_label(Role::TextInput, "Document Title").click();
        h.run_steps(1);
        h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text("Typed title".into()));
        h.run_steps(2);
        h.key_press(egui::Key::Tab);
        h.run_steps(2);
        assert_eq!(dialog(&mut h)["fields"]["title"], "Typed title");
        h.get_by_role_and_label(Role::Button, if commit { "OK" } else { "Cancel" }).click();
        h.run_steps(5);
        assert!(h.state().framework.dialogs.is_empty());
        let state = h.state().session.active().unwrap();
        if commit {
            assert_eq!(state.history.entries().len(), history.len() + 1);
            let info = h.state_mut().session.execute("file.fileInfo", json!({})).unwrap();
            assert_eq!(info["title"], "Typed title");
        } else {
            assert!(Arc::ptr_eq(&before, &state.doc));
            assert_eq!(state.history.entries(), history);
        }
    }
}

#[test]
fn command_control_held_ok_keeps_captured_document_and_retries_after_both_authorization_failures() {
    let mut h = harness(vec2(1200.0, 900.0), true);
    let first = h.state().session.active().unwrap().doc.id;
    let opened = call(&mut h, "ui.dialog.open", json!({"kind":"command", "command":"file.fileInfo"}));
    assert_eq!(opened["ok"], true, "{opened}");
    h.run_steps(4);
    set(&mut h, "title", json!("Captured title"));
    set(&mut h, "author", json!("A; B"));
    set(&mut h, "keywords", json!("A; B, C"));
    set(&mut h, "copyrightStatus", json!("copyrighted"));
    assert_eq!(call(&mut h, "ui.dialog.set", json!({"dialog":"top", "field":"copyrightStatus", "value":"invented"}))["ok"], false);
    assert_eq!(dialog(&mut h)["fields"]["copyrightStatus"], "copyrighted");
    h.state_mut().run("file.new", json!({"width": 32, "height": 24, "name":"second.psd"})).unwrap();
    let second = h.state().session.active().unwrap().doc.id;
    let before: Vec<_> = h.state().session.documents().iter().map(|state| (state.doc.clone(), state.revision, state.history.entries())).collect();
    h.state_mut().services.automation_command = Some(Box::new(|id, _| if id == "file.fileInfo" { Err("command denied".into()) } else { Ok(()) }));
    let denied = call(&mut h, "ui.dialog.confirm", json!({"dialog":"top"}));
    assert_eq!(denied["ok"], false, "{denied}");
    assert_eq!(dialog(&mut h)["fields"]["title"], "Captured title");
    h.state_mut().services.automation_command = None;
    h.state_mut().services.automation_authorize =
        Some(|id, _| if id == "file.fileInfo" { Err(photocraft_engine::EngineError::Other("engine denied".into())) } else { Ok(()) });
    let failed = call(&mut h, "ui.dialog.confirm", json!({"dialog":"top"}));
    assert_eq!(failed["ok"], false, "{failed}");
    assert_eq!(dialog(&mut h)["fields"]["title"], "Captured title");
    for (state, (doc, revision, history)) in h.state().session.documents().iter().zip(&before) {
        assert!(Arc::ptr_eq(doc, &state.doc));
        assert_eq!(state.revision, *revision);
        assert_eq!(&state.history.entries(), history);
    }
    h.state_mut().services.automation_authorize = None;
    let from = h.state().session.journal.len();
    assert_eq!(call(&mut h, "ui.dialog.confirm", json!({"dialog":"top"}))["ok"], true);
    h.run_steps(4);
    assert!(h.state().framework.dialogs.is_empty());
    assert_eq!(h.state().session.active().unwrap().doc.id, second, "active tab restored");
    assert!(Arc::ptr_eq(&before[1].0, &h.state().session.active().unwrap().doc));
    let captured = h.state().session.documents().iter().find(|state| state.doc.id == first).unwrap();
    assert_eq!(captured.history.entries().len(), before[0].2.len() + 1);
    assert_eq!(h.state().session.journal[from..].iter().filter(|(id, _)| id == "file.fileInfo").count(), 1);
    let info = photocraft_engine::file_cmds::read_file_info(captured.doc.metadata.xmp.as_deref());
    assert_eq!(info["title"], "Captured title");
    assert_eq!(info["author"], "A; B");
    assert_eq!(info["keywords"], json!(["A", "B", "C"]));
    assert_eq!(info["copyrightStatus"], "copyrighted");
    assert_eq!(call(&mut h, "ui.dialog.confirm", json!({"dialog":"top"}))["ok"], false, "cannot apply twice");
}

#[test]
fn no_op_basic_confirmation_preserves_dates_and_matches_the_current_command_history_contract() {
    let mut ui = harness(vec2(1200.0, 900.0), true);
    let mut direct = harness(vec2(1200.0, 900.0), true);
    open(&mut ui);
    let fields = dialog(&mut ui)["fields"].clone();
    let mut params = serde_json::Map::new();
    for key in ["title", "author", "authorTitle", "description", "keywords", "copyrightStatus", "copyright", "copyrightUrl"] {
        params.insert(key.to_owned(), fields[key].clone());
    }
    let before = ui.state().session.active().unwrap().history.entries().len();
    let direct_before = direct.state().session.active().unwrap().history.entries().len();
    direct.state_mut().run("file.fileInfo", Value::Object(params)).unwrap();
    assert_eq!(call(&mut ui, "ui.dialog.confirm", json!({"dialog":"top"}))["ok"], true);
    ui.run_steps(4);
    let actual = ui.state().session.active().unwrap();
    let expected = direct.state().session.active().unwrap();
    assert_eq!(actual.doc.metadata.xmp, expected.doc.metadata.xmp);
    assert_eq!(actual.history.entries().len() - before, expected.history.entries().len() - direct_before);
    let xmp = actual.doc.metadata.xmp.as_deref().unwrap();
    for unchanged in ["2024-02-17T12:40:19+01:00", "2024-02-17T12:41:20+01:00", "2024-02-17T12:42:21+01:00", "preserve-me"] {
        assert!(xmp.contains(unchanged), "kept {unchanged}: {xmp}");
    }
}

#[test]
fn xmp_attribute_forms_supply_camera_values_and_are_removed_when_basic_fields_change() {
    // Our own synthetic XML: ordinary double quotes, single quotes, and XML whitespace variants.
    for (before, around, quote) in [(" ", "", '"'), (" ", "", '\''), ("\t", " \t ", '"'), ("\n", "\r\n", '\''), ("\r", " ", '"')] {
        let other = if quote == '"' { '\'' } else { '"' };
        let note = format!("fixture:note={other} aux:OwnerName={quote}quoted-decoy{quote}{other}");
        let packet = format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><!-- aux:OwnerName={quote}comment-decoy{quote} --><rdf:Description rdf:about="" xmlns:aux="http://ns.adobe.com/exif/1.0/aux/" xmlns:tiff="http://ns.adobe.com/tiff/1.0/" xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/" xmlns:xmpRights="http://ns.adobe.com/xap/1.0/rights/" xmlns:fixture="https://example.invalid/metadata-fixture/" {note}{before}aux:OwnerName{around}={around}{quote}Ada &amp; Co{quote}{before}tiff:Make{around}={around}{quote}Fixture make{quote}{before}photoshop:AuthorsPosition{around}={around}{quote}Old role{quote}{before}xmpRights:Marked{around}={around}{quote}True{quote} fixture:opaque="preserve-me"/></rdf:RDF></x:xmpmeta>"#
        );
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        session
            .edit("XMP attribute fixture", |doc, _| {
                doc.metadata.xmp = Some(packet.clone().into());
                Ok(())
            })
            .unwrap();
        let original = session.active().unwrap().doc.metadata.xmp.clone();
        let read = session.execute("file.fileInfo", json!({})).unwrap();
        assert_eq!(read["camera"]["owner"], "Ada & Co", "{before:?} {around:?} {quote}");
        assert_eq!(read["camera"]["make"], "Fixture make");
        assert_eq!(read["authorTitle"], "Old role");
        assert_eq!(read["copyrightStatus"], "copyrighted", "existing Marked behavior");
        assert_eq!(session.active().unwrap().doc.metadata.xmp, original, "reading must not edit metadata");

        let changed = session.execute("file.fileInfo", json!({"authorTitle": "New role"})).unwrap();
        assert_eq!(changed["authorTitle"], "New role");
        assert_eq!(changed["camera"], read["camera"], "a Basic edit keeps Camera Data");
        assert_eq!(changed["copyrightStatus"], "copyrighted");
        let xmp = session.active().unwrap().doc.metadata.xmp.as_deref().unwrap();
        assert!(!xmp.contains("Old role"), "the old attribute span must be removed, not merely shadowed");
        assert!(xmp.contains(&note), "unrelated quoted attribute content survives");
        assert!(xmp.contains("fixture:opaque=\"preserve-me\""));
        assert!(xmp.contains(&format!("<!-- aux:OwnerName={quote}comment-decoy{quote} -->")));
    }
}
