//! Help › System Info: what the report says, what it leaves out, and how the dialog and
//! `help.systemInfo` carry it.

use super::*;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

const SECTION_TITLES: [&str; 8] = ["Application", "System", "Graphics", "Displays", "Pen input", "Fonts", "Session", "Files"];

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

fn row<'a>(sections: &'a [Section], title: &str, label: &str) -> Option<&'a str> {
    sections.iter().find(|s| s.title == title)?.rows.iter().find(|(l, _)| l == label).map(|(_, v)| v.as_str())
}

#[test]
fn windows_11_is_named_from_its_build_not_its_product_name() {
    assert_eq!(windows_name(Some("Windows 10 Pro"), Some("25H2"), Some("26200"), Some(6899)), "Windows 11 Pro 25H2 (build 26200.6899)");
    assert_eq!(windows_name(Some("Windows 10 Home"), Some("22H2"), Some("19045"), Some(5737)), "Windows 10 Home 22H2 (build 19045.5737)");
    assert_eq!(windows_name(Some("Windows 10 Pro"), None, Some("22000"), None), "Windows 11 Pro (build 22000)");
    assert_eq!(windows_name(None, None, Some("not a number"), None), "Windows (build not a number)");
    assert_eq!(windows_name(None, None, None, None), "Windows");
}

#[test]
fn linux_os_cpu_and_memory_come_from_their_files() {
    let os = "NAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\n";
    assert_eq!(os_release_name(os).as_deref(), Some("Ubuntu 24.04.1 LTS"));
    assert_eq!(os_release_name("NAME=Fedora Linux\nVERSION_ID=41\n").as_deref(), Some("Fedora Linux 41"));
    assert_eq!(os_release_name("PRETTY_NAME=\"\"\nNAME=Arch Linux\n").as_deref(), Some("Arch Linux"));
    assert_eq!(os_release_name("garbage"), None);
    let x86 = "processor\t: 0\nvendor_id\t: AuthenticAMD\nmodel name\t: AMD Ryzen 9 7950X 16-Core Processor\n";
    assert_eq!(cpuinfo_model(x86).as_deref(), Some("AMD Ryzen 9 7950X 16-Core Processor"));
    assert_eq!(cpuinfo_model("processor\t: 0\nModel\t\t: Raspberry Pi 5 Model B Rev 1.0\n").as_deref(), Some("Raspberry Pi 5 Model B Rev 1.0"));
    assert_eq!(cpuinfo_model("model name\t:\n"), None);
    assert_eq!(meminfo_available("MemTotal: 64000000 kB\nMemAvailable:   1024 kB\n"), Some(1024 * 1024));
    assert_eq!(meminfo_available("MemAvailable: lots\n"), None);
    assert_eq!(meminfo_available(&format!("MemAvailable: {} kB\n", u64::MAX)), None, "an overflow is no number, not a panic");
}

#[test]
fn numbers_and_names_read_plainly() {
    assert_eq!(bytes(512), "512 bytes");
    assert_eq!(bytes(64 << 10), "64 KB");
    assert_eq!(bytes(812 << 20), "812 MB");
    assert_eq!(bytes(66_255_331_328), "61.7 GB");
    assert_eq!(duration(45.9), "45 s");
    assert_eq!(duration(12.0 * 60.0 + 5.0), "12 min");
    assert_eq!(duration(3.0 * 3600.0 + 5.0 * 60.0), "3 h 05 min");
    assert_eq!(duration(f64::NAN), "0 s");
    assert_eq!(duration(-4.0), "0 s");
    assert_eq!(backend_label("dx12"), "DirectX 12");
    assert_eq!(backend_label("gl"), "OpenGL");
    assert_eq!(backend_label(""), "Auto");
    assert_eq!(backend_label("someday"), "someday");
    assert_eq!(device_type_label("discreteGpu"), "Dedicated GPU");
    assert_eq!(device_type_label("cpu"), "Software renderer (no GPU)");
    assert_eq!(vendor_name(0x10de), "NVIDIA");
    assert_eq!(vendor_name(0x1002), "AMD");
    assert_eq!(vendor_name(0xbeef), "vendor beef");
}

#[test]
fn paths_under_the_home_folder_lose_the_account_name() {
    let home = std::path::Path::new(if cfg!(windows) { r"C:\Users\someone" } else { "/home/someone" });
    let log = home.join("AppData").join("photocraft.log");
    let tidy = tidy_path(&log, Some(home));
    assert!(tidy.starts_with('~') && !tidy.contains("someone"), "{tidy}");
    assert_eq!(tidy_path(home, Some(home)), "~");
    let elsewhere = std::path::Path::new(if cfg!(windows) { r"D:\PhotoCraft\PhotoCraftData" } else { "/opt/photocraft/data" });
    assert_eq!(tidy_path(elsewhere, Some(home)), elsewhere.display().to_string());
    assert_eq!(tidy_path(elsewhere, None), elsewhere.display().to_string());
    assert_eq!(tidy_path(&log, Some(std::path::Path::new(""))), log.display().to_string(), "an empty home matches nothing");
}

#[test]
fn the_report_has_every_section_in_order() {
    let sections = report(&app(), None);
    let titles: Vec<&str> = sections.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, SECTION_TITLES);
    assert!(sections.iter().all(|s| !s.rows.is_empty()), "{sections:?}");
    let version = row(&sections, "Application", "Version").unwrap_or("");
    assert!(version.starts_with(photocraft_engine::build_info::VERSION), "{version}");
    assert!(!row(&sections, "System", "OS").unwrap_or("").is_empty());
    assert!(row(&sections, "System", "CPU").unwrap_or("").contains("threads"));
}

#[cfg(windows)]
#[test]
fn windows_reports_its_version_cpu_and_free_memory() {
    let p = detect_platform();
    assert!(p.os.starts_with("Windows") && p.os.contains("build"), "{p:?}");
    assert!(p.cpu.as_deref().is_some_and(|c| !c.is_empty()), "{p:?}");
    let free = available_memory().unwrap_or(0);
    assert!(free > 0 && crate::gpu_canvas::physical_memory().is_some_and(|t| free <= t), "{free}");
    let memory = row(&report(&app(), None), "System", "Memory").unwrap_or("").to_string();
    assert!(memory.contains("installed") && memory.contains("available"), "{memory}");
}

#[test]
fn documents_are_described_by_size_and_mode_never_by_name() {
    let mut app = app();
    app.run("file.new", serde_json::json!({"width": 640, "height": 480, "depth": 16, "name": "Client Secret Portrait"})).unwrap();
    app.run("layer.new.layer", serde_json::json!({})).unwrap();
    let name = app.session.documents().first().map(|d| d.doc.name.clone()).unwrap_or_default();
    assert!(!name.is_empty());
    let sections = report(&app, None);
    let doc = row(&sections, "Session", "Document 1").unwrap_or("");
    assert!(doc.starts_with("640×480 px, RGB 16-bit, 2 layers"), "{doc}");
    assert_eq!(row(&sections, "Session", "Open documents"), Some("1"));
    let all = text(&sections);
    assert!(!all.contains(&name), "the document name {name:?} leaked into\n{all}");
}

#[test]
fn install_paths_are_listed_with_the_home_folder_as_a_tilde() {
    let Some(home) = home_dir() else { return };
    let mut app = app();
    let dir = home.join("PhotocraftTest");
    app.services.install = Some(InstallInfo {
        kind: "Portable".into(),
        settings_dir: Some(dir.clone()),
        log_file: Some(dir.join("logs").join("photocraft.log")),
        recovery_dir: Some(dir.join("Recovery")),
    });
    let sections = report(&app, None);
    assert_eq!(row(&sections, "Application", "Install"), Some("Portable"));
    let log = row(&sections, "Files", "Log file").unwrap_or("");
    assert!(log.starts_with('~') && log.ends_with("photocraft.log"), "{log}");
    let all = text(&sections);
    assert!(!all.contains(&home.display().to_string()), "the home folder leaked into\n{all}");
}

#[test]
fn the_text_lines_every_value_up_in_one_column() {
    let sections = report(&app(), None);
    let all = text(&sections);
    assert!(all.starts_with("PhotoCraft System Info\n"));
    let mut columns = std::collections::BTreeSet::new();
    for s in &sections {
        assert!(all.contains(&format!("\n{}\n", s.title)), "{} heading", s.title);
        for (label, value) in &s.rows {
            let line = all.lines().find(|l| l.starts_with(&format!("  {label} ")) && l.ends_with(value.as_str())).unwrap_or("");
            assert!(!line.is_empty(), "no line for {label}: {value}");
            columns.insert(line.chars().count() - value.chars().count());
        }
    }
    assert_eq!(columns.len(), 1, "values start at columns {columns:?}");
}

#[test]
fn a_pen_is_reported_once_its_pressure_or_tilt_arrives() {
    let mut app = app();
    assert_eq!(row(&report(&app, None), "Pen input", "Seen this session"), Some("nothing (no pen or touch pressure yet)"));
    let touch = egui::Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(1),
        phase: egui::TouchPhase::Start,
        pos: egui::pos2(10.0, 10.0),
        force: Some(0.4),
    };
    app.stylus.update(&[touch]);
    assert_eq!(row(&report(&app, None), "Pen input", "Seen this session"), Some("pressure"));
    app.stylus.feed.set(Some(crate::stylus::PenSample { pressure: 0.5, tilt_x: 20.0, eraser: true, ..Default::default() }));
    app.stylus.update(&[]);
    assert_eq!(row(&report(&app, None), "Pen input", "Seen this session"), Some("pressure, tilt, eraser end"));
}

#[test]
fn help_system_info_replies_with_the_report_it_shows() {
    let mut app = app();
    let ctx = egui::Context::default();
    let reply = crate::menus::invoke(&mut app, &ctx, "help.systemInfo", Value::Null).unwrap();
    let info = &reply["info"];
    for key in ["version", "os", "arch", "gpu", "monitor", "displays", "sections", "lines"] {
        assert!(!info[key].is_null(), "{key} missing from {info}");
    }
    let lines: Vec<&str> = info["lines"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    assert_eq!(lines.first(), Some(&"PhotoCraft System Info"));
    assert!(lines.iter().any(|l| l.trim_start().starts_with("Graphics API")), "{lines:?}");
    let dialog = app.ui.dialogs.last().map(|d| d.fields.clone()).unwrap_or_default();
    assert_eq!(dialog.get("report"), Some(&info["sections"]), "the dialog shows the report the reply carries");
}

#[test]
fn the_report_copy_and_api_stay_english_in_a_czech_ui() {
    let mut app = app();
    app.perf.gpu_info.adapter = "Test GPU".into();
    // Labels, and the Displays section's values (free memory changes between two reports).
    let english_parts = |sections: &[Section]| {
        let labels: Vec<String> = sections.iter().flat_map(|s| s.rows.iter().map(|(l, _)| format!("{}/{l}", s.title))).collect();
        (labels, sections.iter().find(|s| s.title == "Displays").cloned())
    };
    let english = crate::i18n::with_language(crate::i18n::Lang::EN, || english_parts(&report(&app, None)));
    crate::i18n::with_language(crate::i18n::Lang::from_code("cs").unwrap(), || {
        let sections = report(&app, None);
        assert_eq!(row(&sections, "Application", "Language").map(|v| v.starts_with("Čeština")), Some(true), "{sections:?}");
        assert_eq!(english_parts(&sections), english, "the monitor profile lines are English too");
        assert!(sections.iter().flat_map(|s| &s.rows).any(|(l, _)| l.starts_with("Monitor profile")), "{sections:?}");
        let reply = crate::menus::invoke(&mut app, &egui::Context::default(), "help.systemInfo", Value::Null).unwrap();
        let lines: Vec<&str> = reply["info"]["lines"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        assert!(lines.iter().any(|l| l.trim_start().starts_with("Monitor profile")), "{lines:?}");
        assert!(!lines.iter().any(|l| l.contains("Profil monitoru")), "{lines:?}");
        assert_eq!(crate::i18n::current().code(), "cs", "the report restores the drawing language");
    });
}

#[test]
fn the_dialog_shows_each_section_and_a_copy_button() {
    let mut app = app();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "help.systemInfo", Value::Null).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                return;
            }
            crate::dialogs::show(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    for title in SECTION_TITLES {
        assert!(h.query_by_label(title).is_some(), "no {title} heading");
    }
    assert!(h.query_by_label("Graphics API").is_some());
    assert!(h.query_by_label("Copy").is_some());
}

/// The dialog's labels are built at run time, so the `tl!` scanner can't see them: every section
/// title and row label (with a document open, and the per-display monitor lines) has a System Info
/// entry in every complete language.
#[test]
fn every_label_has_a_translation_in_every_complete_language() {
    let mut app = app();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let sections = report(&app, None);
    let mut labels: Vec<&str> = sections.iter().flat_map(|s| std::iter::once(s.title.as_str()).chain(s.rows.iter().map(|(l, _)| l.as_str()))).collect();
    labels.retain(|l| *l != "Document 1" && !l.starts_with("Monitor profile on "));
    // Rows that only some machines or sessions have.
    let rare = ["Kernel", "Desktop", "CPU", "Startup fallback", "This session", "GPU memory budget", "Window's display", "UI scale", "Window", "Running for"];
    let templates = ["Document {n}", "Monitor profile on {display}", "Monitor profile on {display} (main window)", "Display profiles"];
    labels.extend(rare.iter().chain(&templates));
    for lang in crate::i18n::Lang::all().filter(|l| l.complete_menus()) {
        let missing: Vec<&&str> = labels.iter().filter(|l| !crate::i18n::has_ctx(lang, CONTEXT, l)).collect();
        assert!(missing.is_empty(), "{}: System Info labels without a translation: {missing:?}", lang.code());
    }
}

/// #3196 review: the dialog shows its labels in the UI language; the values and Copy stay English.
#[test]
fn the_dialog_labels_follow_the_ui_language() {
    let cs = crate::i18n::Lang::from_code("cs").unwrap();
    assert_eq!(label(cs, "Graphics"), "Grafika");
    assert_eq!(label(cs, "Type"), "Typ", "the System Info sense, not the Type tool");
    assert_eq!(label(cs, "Document 3"), "Dokument 3");
    assert_eq!(label(cs, "Monitor profile on DELL U2720Q (main window)"), "Profil monitoru na DELL U2720Q (hlavní okno)");
    assert_eq!(label(cs, "Document x"), "Document x", "not a numbered document: unchanged");
    assert_eq!(label(crate::i18n::Lang::EN, "Monitor profile on Built-in"), "Monitor profile on Built-in");
    let mut app = app();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "help.systemInfo", Value::Null).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                return;
            }
            crate::i18n::with_language(crate::i18n::Lang::from_code("cs").unwrap(), || crate::dialogs::show(app, &ctx));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    for title in ["Aplikace", "Systém", "Grafika", "Displeje", "Vstup pera", "Písma", "Relace", "Soubory", "Grafické API"] {
        assert!(h.query_by_label(title).is_some(), "no {title} label");
    }
    assert!(h.query_by_label("Graphics API").is_none(), "the English label is replaced");
    let fields = &h.state().ui.dialogs[0].fields;
    let sections: Vec<Section> = serde_json::from_value(fields["report"].clone()).unwrap();
    assert!(text(&sections).contains("Graphics API"), "Copy stays English");
}
