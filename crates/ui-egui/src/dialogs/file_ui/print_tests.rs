//! The Print dialog's one Printer list, its Grayscale choice and the paper a local queue is asked for,
//! drawn offscreen with in-process printers (no spooler or device is asked).

use super::*;
use crate::PhotocraftApp;
use egui_kittest::{Harness, kittest::Queryable};

/// Stand-in system printers, in place of the spooler's.
fn printers() -> photocraft_engine::print_cmds::PrinterSource {
    std::sync::Arc::new(|| {
        vec![
            craft_print::Printer { name: "Front Desk".into(), default: false, two_sided: None, description: String::new() },
            craft_print::Printer { name: "Laser Room".into(), default: true, two_sided: None, description: String::new() },
        ]
    })
}

/// An app with a 60 x 40 document and the Print dialog open on the system printers (the Windows driver
/// list is dropped, so nothing asks a driver for its paper).
fn open() -> (Harness<'static, PhotocraftApp>, u64) {
    let mut harness = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.printer_source = Some(printers());
        app.run("file.new", json!({"name": "photo", "width": 60, "height": 40})).unwrap();
        app
    });
    harness.run_steps(4);
    let id = open_print(harness.state_mut());
    harness.state_mut().ui.dialog_mut(id).unwrap().fields.remove("__printers");
    harness.run_steps(3);
    (harness, id)
}

/// How many nodes show `label`, as a label or as a widget's value (a dropdown shows its choice as a value).
fn count(h: &Harness<'_, PhotocraftApp>, label: &str) -> usize {
    h.query_all_by_label(label).count() + h.query_all_by_value(label).count()
}

/// Every label and value on screen, for a failing assertion's message.
#[cfg(network_print)]
fn dump(h: &Harness<'_, PhotocraftApp>) -> String {
    use egui_kittest::kittest::NodeT as _;
    let mut all = String::new();
    let mut stack = vec![h.root()];
    while let Some(n) = stack.pop() {
        if let Some(t) = n.accesskit_node().label().or_else(|| n.accesskit_node().value()) {
            all.push_str(&format!(
                "{:?}:{t}
",
                n.accesskit_node().role()
            ));
        }
        stack.extend(n.children());
    }
    all
}

/// The last node showing `label`: the open list's entry rather than the button that shows the choice.
fn click_last(h: &Harness<'_, PhotocraftApp>, label: &str) {
    match h.query_all_by_label(label).last() {
        Some(n) => n.click(),
        None => h.query_all_by_value(label).last().unwrap_or_else(|| panic!("no `{label}` to click")).click(),
    }
}

fn pick(h: &mut Harness<'_, PhotocraftApp>, label: &str) {
    click_last(h, label);
    h.run_steps(3);
}

#[cfg(network_print)]
fn top(h: &Harness<'_, PhotocraftApp>, label: &str) -> f32 {
    h.query_all_by_label(label).last().or_else(|| h.query_all_by_value(label).last()).unwrap().rect().min.y
}

fn fields(h: &Harness<'_, PhotocraftApp>, id: u64) -> Map<String, Value> {
    h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone()
}

#[test]
#[cfg(network_print)]
fn one_printer_list_holds_the_system_printers_and_the_network_entry() {
    let (mut h, id) = open();
    assert_eq!(count(&h, "Destination"), 0, "there is no second menu to pick the kind of printer from");
    assert!(!h.state().net_print.selected);
    assert_eq!(count(&h, "Printer address:"), 0, "the network section is hidden until its entry is chosen");
    pick(&mut h, "Laser Room");
    for entry in ["Front Desk", "Laser Room", "Network printer…"] {
        assert!(
            count(&h, entry) > 0,
            "the list shows {entry}
{}",
            dump(&h)
        );
    }
    // The network entry comes after the real printers.
    assert!(top(&h, "Front Desk") < top(&h, "Network printer…") && top(&h, "Laser Room") < top(&h, "Network printer…"));
    pick(&mut h, "Network printer…");
    assert!(h.state().net_print.selected);
    assert!(count(&h, "Printer address:") > 0 && count(&h, "Query printer") > 0, "choosing the network entry shows its section");
    assert!(!super::can_confirm(&fields(&h, id)), "nothing queried or confirmed yet");
    // Back to a real printer: the section goes, the printer is the one chosen.
    pick(&mut h, "Network printer…");
    pick(&mut h, "Front Desk");
    assert!(!h.state().net_print.selected);
    assert_eq!(count(&h, "Printer address:"), 0);
    assert_eq!(fields(&h, id)["printer"], "Front Desk");
    assert!(super::can_confirm(&fields(&h, id)));
}

#[test]
#[cfg(network_print)]
fn a_queried_network_printer_is_its_own_entry_by_the_name_it_reported() {
    let (mut h, _) = open();
    // An in-process printer answers the query (WS-Print over an in-memory transport).
    let printer = craft_print::wsd::testing::FakePrinter::new(craft_print::wsd::testing::FakeConfig::default());
    let transport: crate::network_glue::Transport = Arc::new(craft_print::wsd::testing::MemTransport(printer));
    {
        let app = h.state_mut();
        app.set_network_transport(transport);
        app.net_print.selected = true;
        app.net_print.state.draft = craft_ui::printing::network::Draft::for_address("127.0.0.1:5357/printer");
        let worker = crate::network_glue::query_printer(app.net_print.transport(), None, Arc::clone(&app.net_print.stations));
        assert!(app.net_print.state.start_query(&worker, true));
    }
    for _ in 0..200 {
        h.run_steps(1);
        if !h.state().net_print.state.draft.querying() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let name = h.state().net_print.state.draft.printer().map(|p| p.name.clone()).filter(|n| !n.is_empty()).expect("the fake printer reports a name");
    let known = format!("{name} (network)");
    h.run_steps(3);
    assert!(count(&h, &known) > 0, "the dropdown shows the printer by name");
    // The list holds the real printers, `Network printer…` and the queried one by name.
    let (app_fields, id) = (fields(&h, h.state().ui.dialogs.last().unwrap().id), h.state().ui.dialogs.last().unwrap().id);
    let entries = printer_entries(h.state(), &app_fields, None);
    let labels: Vec<&str> = entries.iter().map(|e| e.1.as_str()).collect();
    assert_eq!(labels.last().copied(), Some(known.as_str()));
    for entry in ["Network printer…", "Front Desk", "Laser Room"] {
        assert!(labels.contains(&entry), "{entry} stays in the list: {labels:?}");
    }
    assert_eq!(current_printer(h.state(), &app_fields), NETWORK_KNOWN);
    // A system printer leaves the network one; the address and its answer are kept for the session.
    let mut f = fields(&h, id);
    choose_printer(h.state_mut(), &mut f, "Laser Room", false);
    assert!(!h.state().net_print.selected);
    assert_eq!(f["printer"], "Laser Room");
    assert!(h.state().net_print.state.draft.printer().is_some());
    choose_printer(h.state_mut(), &mut f, NETWORK_ENTRY, false);
    assert!(h.state().net_print.selected, "the network entry is chosen again without retyping the address");
    assert_eq!(current_printer(h.state(), &f), NETWORK_KNOWN);
}

#[test]
fn the_windows_driver_list_has_the_default_first_and_marked() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let mut f = Map::new();
    f.insert("__printerDefault".into(), json!("Laser Room"));
    let names = ["Front Desk".to_string(), "Laser Room".to_string(), "Zebra".to_string()];
    let entries = printer_entries(&app, &f, Some(&names));
    assert_eq!(entries[0], ("Laser Room".to_string(), format!("Laser Room ({})", tl!("Default printer"))));
    assert_eq!(entries[1].0, "Front Desk");
    assert_eq!(entries[2].0, "Zebra");
    #[cfg(network_print)]
    assert_eq!(entries.last().map(|e| e.0.as_str()), Some(NETWORK_ENTRY));
}

#[test]
#[cfg(network_print)]
fn grayscale_is_in_the_dialog_the_preview_and_the_network_colour_setting() {
    let (mut h, id) = open();
    assert_eq!(count(&h, "Black and white"), 0);
    assert!(count(&h, "Grayscale") > 0, "the local box says Grayscale");
    let key = |h: &Harness<'_, PhotocraftApp>, gray| {
        let st = h.state().session.active().unwrap();
        thumb_key(st.doc.id.0, st.revision, gray)
    };
    let cached = |h: &Harness<'_, PhotocraftApp>, gray| h.ctx.data(|d| d.get_temp::<Arc<egui::TextureHandle>>(key(h, gray))).is_some();
    assert!(cached(&h, false) && !cached(&h, true), "the colour preview comes first");
    pick(&mut h, "Grayscale");
    assert_eq!(fields(&h, id)["grayscale"], true);
    assert!(cached(&h, true), "the gray preview is drawn from the first frame after the tick");
    assert!(effective_grayscale(h.state(), &fields(&h, id)));
    pick(&mut h, "Grayscale");
    assert_eq!(fields(&h, id)["grayscale"], false);
    assert!(cached(&h, false), "colour and gray thumbnails are cached apart");
    assert!(!effective_grayscale(h.state(), &fields(&h, id)));
    // The network printer: its Colour setting is the choice; the local box is not shown.
    h.state_mut().net_print.selected = true;
    h.state_mut().net_print.state.draft.color = Some(craft_ui::printing::network::ColorMode::Monochrome);
    h.run_steps(3);
    assert!(effective_grayscale(h.state(), &fields(&h, id)));
    h.state_mut().net_print.state.draft.color = Some(craft_ui::printing::network::ColorMode::Color);
    assert!(!effective_grayscale(h.state(), &fields(&h, id)));
}

#[test]
fn the_thumbnail_keys_for_gray_and_colour_never_meet() {
    let mut seen = std::collections::HashSet::new();
    for doc in [0u64, 1, 1 << 60, u64::MAX] {
        for rev in [0u64, 1, 1 << 60, u64::MAX] {
            for gray in [false, true] {
                assert!(seen.insert(thumb_key(doc, rev, gray)), "{doc} {rev} {gray}");
            }
        }
    }
}

#[test]
fn a_gray_preview_pixel_is_rec_709_luma_with_its_alpha() {
    let g = gray_pixels(&[255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0]);
    assert_eq!(g, [54, 54, 54, 255, 182, 182, 182, 128, 18, 18, 18, 0]);
}

#[test]
#[cfg(network_print)]
fn the_network_colour_choice_and_its_reason_say_grayscale() {
    let l = crate::network_glue::labels();
    assert_eq!(l.color_monochrome, "Grayscale");
    assert!(l.unsupported_color.contains("grayscale") && !l.unsupported_color.to_lowercase().contains("black"), "{}", l.unsupported_color);
}

#[test]
fn a_queue_may_be_allowed_its_own_paper_in_the_dialog_and_the_choice_reaches_the_command() {
    let (mut h, id) = open();
    let label = "Use the printer's own paper if it lacks this one";
    assert!(count(&h, label) > 0, "offered where the page goes to a queue");
    assert_ne!(fields(&h, id).get("paperFallback"), Some(&json!(true)), "off until the person allows it");
    pick(&mut h, label);
    let f = fields(&h, id);
    assert_eq!(f["paperFallback"], true);
    assert_eq!(params(&f)["paperFallback"], true, "it is a command parameter, not a hidden field");
    // Not for a PDF file: no queue is asked for a paper there.
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("output".into(), json!("x.pdf"));
    h.run_steps(3);
    assert_eq!(count(&h, label), 0);
}

/// Driver settings belong to the printer they were chosen for: picking another printer drops them, also
/// when the pick comes while the network printer is the destination (no driver list is shown then).
#[test]
fn another_printer_never_prints_with_the_last_ones_driver_settings() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let mut f = Map::new();
    f.insert("printer".into(), json!("Front Desk"));
    f.insert("printerSettings".into(), json!("00ff"));
    #[cfg(network_print)]
    choose_printer(&mut app, &mut f, NETWORK_ENTRY, false);
    assert_eq!(f["printerSettings"], "00ff", "the network entry leaves the system printer's settings alone");
    choose_printer(&mut app, &mut f, "Laser Room", false);
    assert_eq!(f["printer"], "Laser Room");
    assert!(!f.contains_key("printerSettings"), "Front Desk's settings went with Laser Room: {f:?}");
    // Picking the printer already chosen keeps what was set for it.
    f.insert("printerSettings".into(), json!("00ff"));
    choose_printer(&mut app, &mut f, "Laser Room", false);
    assert_eq!(f["printerSettings"], "00ff");
}
