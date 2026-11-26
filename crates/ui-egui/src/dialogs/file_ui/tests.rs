use craft::print::ui::HostedPrint;
use craft::ui::dialog::{DialogId, StackDialog as _};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedSaveForWeb, Resample, SAVE_PATH_KEY, WebFormat, WebSlices};
use photocraft_engine::print_cmds::PrintExt;

use super::*;
use std::sync::Arc;

pub(super) fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    *app.session.printer_source_mut() =
        Some(Arc::new(|| vec![craft::print::Printer { name: "Test printer".into(), default: true, two_sided: None, description: String::new() }]));
    app.run("file.new", json!({"width": 120, "height": 80, "name": "web"})).unwrap();
    app
}

/// Runs one pass of the dialogs, so each applies the answers and input it holds.
pub(super) fn pass(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    ctx.run_ui(Default::default(), |ui| crate::dialogs::run(app, ui)).textures_delta.clear();
}

/// Sets a Save for Web field as `ui.dialog.set` does.
pub(super) fn set_option(app: &mut PhotocraftApp, id: DialogId, field: &str, value: Value) {
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, field, value).unwrap();
}

fn web_dialog(app: &mut PhotocraftApp, id: DialogId) -> &mut HostedSaveForWeb {
    app.framework.dialogs.get_mut_by_id::<HostedSaveForWeb>(id).expect("Save for Web is open")
}

/// Answers the Save… request with `path`, as the save picker does.
fn answer_path(app: &mut PhotocraftApp, id: DialogId, path: &str) {
    web_dialog(app, id).answer_path(SAVE_PATH_KEY, std::path::PathBuf::from(path));
    pass(app);
}

/// OK on Save for Web, then the app's takers run the export. The answered Save… finishes the dialog in its pass;
/// a dialog still open takes OK, which needs the path.
fn confirm(app: &mut PhotocraftApp, id: DialogId) {
    let ctx = app.ctx().clone();
    if app.framework.dialogs.get_by_id::<HostedSaveForWeb>(id).is_some() {
        crate::dialogs::confirm(app, &ctx, id).expect("OK needs the path answered");
    }
    take(app, &ctx);
}

fn fv(v: Value) -> FieldValue {
    serde_json::from_value(v).expect("a field value")
}

fn field(f: &DialogFields, key: &str) -> Value {
    f.get(key).map_or(Value::Null, |v| serde_json::to_value(v).unwrap())
}

fn print_fields(app: &mut PhotocraftApp, id: DialogId) -> &mut DialogFields {
    app.framework.dialogs.get_mut_by_id::<HostedPrint>(id).expect("Print is open").fields_mut()
}

/// A default printer named `name`, as a printer source lists it.
fn listed_printer(name: &str) -> craft::print::Printer {
    craft::print::Printer { name: name.into(), default: true, two_sided: None, description: String::new() }
}

/// The printer the open Print dialog has chosen.
fn chosen_printer(app: &PhotocraftApp, id: DialogId) -> Value {
    app.framework.dialogs.get_by_id::<HostedPrint>(id).and_then(|dialog| dialog.json_fields()).map_or(Value::Null, |f| f["printer"].clone())
}

#[test]
fn print_dialog_keeps_saved_settings_when_the_cached_printer_list_is_empty() {
    check_saved_settings_with_cached_printers(&[]);
}

#[test]
fn print_dialog_keeps_saved_settings_when_the_cached_printer_list_is_stale() {
    check_saved_settings_with_cached_printers(&["Old printer"]);
}

/// Opens Print while another thread holds the spooler. The printer list kept from before (`cached`) does not name
/// the remembered printer. The dialog keeps the remembered printer and its driver settings while the list is
/// pending, and still keeps them once the spooler is free and the fresh list names the printer.
fn check_saved_settings_with_cached_printers(cached: &'static [&'static str]) {
    use std::sync::atomic::{AtomicBool, Ordering};
    let mut app = app();
    let ctx = app.ctx().clone();
    let available = Arc::new(AtomicBool::new(false));
    let listed = Arc::clone(&available);
    *app.session.printer_source_mut() = Some(Arc::new(move || {
        if listed.load(Ordering::SeqCst) { vec![listed_printer("Saved printer")] } else { cached.iter().map(|name| listed_printer(name)).collect() }
    }));
    app.session.prefs.edit(|p| {
        p.dialogs.insert(photocraft_engine::print_cmds::REMEMBERED.into(), json!({"printer": "Saved printer", "printerSettings": "0011"}));
    });
    let mut lister = photocraft_engine::Session::new();
    *lister.printer_source_mut() = app.session.printer_source().cloned();
    let (held_tx, held) = std::sync::mpsc::channel();
    let (release, released_rx) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        photocraft_engine::print_cmds::with_spooler(|| {
            // Both lists the dialog reads while the spooler is busy are kept from this listing.
            let _ = photocraft_engine::print_cmds::rgb_printers(&lister);
            let kept = photocraft_engine::print_cmds::printers_without_waiting(&mut lister);
            available.store(true, Ordering::SeqCst);
            held_tx.send(kept["printers"].as_array().map_or(0, Vec::len)).unwrap();
            let _ = released_rx.recv();
        });
    });
    let kept = held.recv().unwrap();
    let id = open_print(&mut app);
    pass(&mut app);
    take(&mut app, &ctx);
    pass(&mut app);
    let pending = (chosen_printer(&app, id), field(print_fields(&mut app, id), "printerSettings"), print_ready(&mut app, id));
    let _ = release.send(());
    holder.join().unwrap();
    assert_eq!(kept, cached.len(), "the printers kept from before");
    assert_eq!(pending, (json!("Saved printer"), json!("0011"), false), "while the list is pending, cached {cached:?}");
    photocraft_engine::print_cmds::with_spooler(|| {
        pass(&mut app);
        take(&mut app, &ctx);
        pass(&mut app);
    });
    let fresh = (chosen_printer(&app, id), field(print_fields(&mut app, id), "printerSettings"), print_ready(&mut app, id));
    assert_eq!(fresh, (json!("Saved printer"), json!("0011"), true), "after the fresh list, cached {cached:?}");
}

/// A fresh, empty list leaves no printer chosen and Print unavailable, and drops the remembered printer's driver
/// settings, so a printer listed later does not get them.
#[test]
fn an_empty_fresh_printer_list_drops_the_saved_driver_settings() {
    let mut app = app();
    let ctx = app.ctx().clone();
    *app.session.printer_source_mut() = Some(Arc::new(Vec::new));
    app.session.prefs.edit(|p| {
        p.dialogs.insert(photocraft_engine::print_cmds::REMEMBERED.into(), json!({"printer": "Saved printer", "printerSettings": "0011"}));
    });
    let id = open_print(&mut app);
    photocraft_engine::print_cmds::with_spooler(|| {
        pass(&mut app);
        take(&mut app, &ctx);
        pass(&mut app);
    });
    let shown = (chosen_printer(&app, id), field(print_fields(&mut app, id), "printerSettings"), print_ready(&mut app, id));
    assert_eq!(shown, (json!(""), Value::Null, false));
}

/// Whether the open Print dialog's Print would start a job.
fn print_ready(app: &mut PhotocraftApp, id: DialogId) -> bool {
    let ctx = app.ctx().clone();
    app.framework.dialogs.get_mut_by_id::<HostedPrint>(id).expect("Print is open").confirm(&ctx).is_some()
}

/// A fresh list without the remembered printer replaces it with the default printer, and the driver settings of
/// the replaced printer are dropped. Settings remembered without a printer (the default one) stay.
#[test]
fn a_fresh_printer_list_without_the_saved_printer_drops_its_driver_settings() {
    for (remembered, settings) in
        [(json!({"printer": "Gone printer", "printerSettings": "0011"}), Value::Null), (json!({"printerSettings": "0011"}), json!("0011"))]
    {
        let mut app = app();
        let ctx = app.ctx().clone();
        app.session.prefs.edit(|p| {
            p.dialogs.insert(photocraft_engine::print_cmds::REMEMBERED.into(), remembered.clone());
        });
        let id = open_print(&mut app);
        photocraft_engine::print_cmds::with_spooler(|| {
            pass(&mut app);
            take(&mut app, &ctx);
            pass(&mut app);
        });
        assert_eq!(chosen_printer(&app, id), json!("Test printer"), "{remembered}");
        assert_eq!(field(print_fields(&mut app, id), "printerSettings"), settings, "{remembered}");
    }
}

#[test]
fn print_dialog_retries_an_uncached_printer_list_after_the_spooler_is_released() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let mut app = app();
    let ctx = app.ctx().clone();
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&asked);
    *app.session.printer_source_mut() = Some(Arc::new(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        vec![craft::print::Printer { name: "Retry printer".into(), default: true, two_sided: None, description: String::new() }]
    }));
    let (held_tx, held) = std::sync::mpsc::channel();
    let (release, released_rx) = std::sync::mpsc::channel::<()>();
    let released = Arc::new(AtomicBool::new(false));
    let done = Arc::clone(&released);
    let holder = std::thread::spawn(move || {
        photocraft_engine::print_cmds::with_spooler(|| {
            held_tx.send(()).unwrap();
            let _ = released_rx.recv_timeout(std::time::Duration::from_secs(5));
            done.store(true, Ordering::SeqCst);
        });
    });
    held.recv().unwrap();
    let id = open_print(&mut app);
    print_fields(&mut app, id).set("dryRun", true);
    pass(&mut app);
    take(&mut app, &ctx);
    let waited = released.load(Ordering::SeqCst);
    let calls_while_busy = asked.load(Ordering::SeqCst);
    let _ = release.send(());
    holder.join().unwrap();
    assert!(!waited, "opening or drawing Print waited for the spooler");
    assert_eq!(calls_while_busy, 0);
    photocraft_engine::print_cmds::with_spooler(|| {
        pass(&mut app);
        take(&mut app, &ctx);
        assert!(asked.load(Ordering::SeqCst) > 0, "the printer list is asked for again once the spooler is free");
    });
}

/// The image-box drag reports the box's top-left corner on the paper in paper points (as drawn, so Emulsion Down
/// is mirrored): no jump on the first update, the position replaces what an earlier update set, and Emulsion
/// Down mirrors x.
// PLAN: assumed `drag_print_position(fields, position: Pos2, layout: &PrintLayout) -> bool` takes the position as drawn (dialogs-w58).
#[test]
fn print_drag_uses_the_resolved_layout_and_travel_in_paper_points_without_accumulation() {
    let app = app();
    let doc = app.session.active().unwrap().doc.clone();
    let layout = |f: &DialogFields| photocraft_engine::print_cmds::layout(&doc, &print_params(f), "file.print").unwrap();
    let top_left = |l: &photocraft_engine::print_cmds::PrintLayout| {
        let (x, y, w, h) = l.rect;
        let x = if l.emulsion_down { l.paper.0 - x - w } else { x };
        egui::pos2(x as f32, (l.paper.1 - y - h) as f32)
    };
    for fit in [false, true] {
        for mirror in [false, true] {
            for orientation in ["portrait", "landscape"] {
                let mut f = DialogFields::from_pairs([
                    ("center", fv(json!(true))),
                    ("scale", fv(json!(175.0))),
                    ("scaleToFit", fv(json!(fit))),
                    ("emulsionDown", fv(json!(mirror))),
                    ("orientation", fv(json!(orientation))),
                ]);
                let start = layout(&f);
                let press = top_left(&start);
                let before = serde_json::to_value(&f).unwrap();
                assert!(!drag_print_position(&mut f, press, &start), "a press without travel changes nothing, so a click leaves Center on");
                assert_eq!(serde_json::to_value(&f).unwrap(), before);
                let manual = layout(&f);
                assert!((manual.rect.0 - start.rect.0).abs() < 1e-9, "no jump on the first update");
                assert!((manual.rect.1 - start.rect.1).abs() < 1e-9);
                assert_eq!(manual.scale, start.scale);
                let sign = if mirror { -1.0 } else { 1.0 };
                for delta in [egui::vec2(-90.0, 45.0), egui::vec2(22.5, -11.25), egui::vec2(180.0, 90.0)] {
                    assert!(drag_print_position(&mut f, press + delta, &start));
                    let moved = layout(&f);
                    assert!((moved.rect.0 - start.rect.0 - sign * f64::from(delta.x)).abs() < 0.001, "x travel, mirror {mirror}");
                    assert!((moved.rect.1 - start.rect.1 + f64::from(delta.y)).abs() < 0.001, "y travel");
                    assert_eq!((moved.rect.2, moved.rect.3, moved.scale), (start.rect.2, start.rect.3, start.scale));
                    let once = serde_json::to_value(&f).unwrap();
                    assert!(drag_print_position(&mut f, press + delta, &start));
                    assert_eq!(serde_json::to_value(&f).unwrap(), once, "total drag travel must not accumulate");
                }
            }
        }
    }
}

/// A non-finite position leaves the fields untouched and an extreme finite one clamps both inch offsets to the
/// limits numeric positioning has.
#[test]
fn print_drag_rejects_nonfinite_input_and_bounds_offsets() {
    let app = app();
    let doc = &app.session.active().unwrap().doc;
    let start = photocraft_engine::print_cmds::layout(doc, &json!({}), "file.print").unwrap();
    let mut f = DialogFields::new();
    for position in [egui::pos2(f32::NAN, 0.0), egui::pos2(0.0, f32::INFINITY)] {
        assert!(!drag_print_position(&mut f, position, &start));
        assert!(f.is_empty());
    }
    assert!(drag_print_position(&mut f, egui::pos2(-f32::MAX, f32::MAX), &start));
    assert_eq!(f.number("left"), Some(-photocraft_engine::print_cmds::MAX_POSITION_INCHES));
    assert_eq!(f.number("top"), Some(photocraft_engine::print_cmds::MAX_POSITION_INCHES));
}

#[test]
fn save_for_web_dialog_runs_the_command() {
    let mut app = app();
    let ctx = app.ctx().clone();
    let r = crate::menus::invoke(&mut app, &ctx, "file.export.saveForWebLegacy", json!({})).unwrap();
    let id = serde_json::from_value::<DialogId>(r["dialog"].clone()).expect("the command opened a dialog");
    let dir = std::env::temp_dir().join(format!("pc-webui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("web.gif").to_string_lossy().into_owned();
    set_option(&mut app, id, "format", json!(WebFormat::Gif));
    answer_path(&mut app, id, &path);
    confirm(&mut app, id);
    assert!(std::fs::read(&path).unwrap().starts_with(b"GIF89a"));
    // The next dialog starts from those settings.
    let id = open_web(&mut app).expect("an open document");
    assert_eq!(web_dialog(&mut app, id).current().format, WebFormat::Gif);
    // WebP, with the dialog's own defaults (lossy at quality 60).
    let path = dir.join("web.webp").to_string_lossy().into_owned();
    set_option(&mut app, id, "format", json!(WebFormat::WebP));
    answer_path(&mut app, id, &path);
    confirm(&mut app, id);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[8..12], b"WEBP");
    assert!(bytes.windows(4).any(|w| w == b"VP8 "), "lossy by default");
}

/// A fresh session's dialog seeds from the engine's remembered export (`web_cmds::remember_web_settings`).
#[test]
fn the_save_for_web_dialog_seeds_from_the_remembered_preferences() {
    let mut app = app();
    app.session.prefs.edit(|p| {
        p.dialogs.insert(WEB_COMMAND.into(), json!({"format": "png8", "colors": 64}));
    });
    let id = open_web(&mut app).expect("an open document");
    let o = web_dialog(&mut app, id).current().clone();
    assert_eq!((o.format, o.colors), (WebFormat::Png8, 64));
}

/// #5305: Save for Web with the suggested name runs (and so is recorded) with only the
/// folder, so an action saves each document under its own name; another name is kept.
#[test]
fn save_for_web_with_the_suggested_name_records_only_the_folder() {
    let dir = std::env::temp_dir().join(format!("pc-webui-kept-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, expected) in [
        ("web.png", json!({"folder": dir.to_string_lossy()})),
        ("other.png", json!({"path": dir.join("other.png").to_string_lossy()})),
        ("web.gif", json!({"path": dir.join("web.gif").to_string_lossy()})),
    ] {
        let mut app = app();
        let target = dir.join(name).to_string_lossy().into_owned();
        let id = open_web(&mut app).expect("an open document");
        set_option(&mut app, id, "format", json!(WebFormat::Png24));
        answer_path(&mut app, id, &target);
        confirm(&mut app, id);
        assert!(std::path::Path::new(&target).exists(), "{target}");
        let (command, params) = app.session.journal.last().unwrap();
        assert_eq!(command, WEB_COMMAND);
        for (k, v) in expected.as_object().unwrap() {
            assert_eq!(&params[k], v, "{params}");
        }
        assert!(params.get(if name == "web.png" { "path" } else { "folder" }).is_none(), "{params}");
        let _ = std::fs::remove_file(&target);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A document renamed while the picker is open writes the picked file, not one named after the new name.
#[test]
fn save_for_web_writes_the_picked_file_when_the_name_changed() {
    let dir = std::env::temp_dir().join(format!("pc-webui-renamed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for rename in [false, true] {
        let mut app = app();
        let target = dir.join("web.png").to_string_lossy().into_owned();
        let id = open_web(&mut app).expect("an open document");
        set_option(&mut app, id, "format", json!(WebFormat::Png24));
        answer_path(&mut app, id, &target);
        if rename {
            Arc::make_mut(app.session.active_mut().unwrap().doc_mut()).name = "renamed".into();
        }
        confirm(&mut app, id);
        assert!(std::path::Path::new(&target).exists(), "rename {rename}: {target}");
        assert!(!dir.join("renamed.png").exists(), "rename {rename}");
        let _ = std::fs::remove_file(&target);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// #6388: a width sets the height in proportion, the file comes out at that size, and the document itself is
/// never resized.
#[test]
fn save_for_web_width_keeps_proportions_and_sizes_the_file() {
    let mut app = app();
    let id = open_web(&mut app).expect("an open document");
    let revision = app.session.active().unwrap().revision;
    set_option(&mut app, id, "format", json!(WebFormat::Png24));
    // The dialog keeps one size as a percent of the document, so the width follows the height.
    set_option(&mut app, id, "percent", json!(50.0));
    let path = std::env::temp_dir().join(format!("pc-webui-wh-{}.png", std::process::id())).to_string_lossy().into_owned();
    answer_path(&mut app, id, &path);
    confirm(&mut app, id);
    let written = photocraft_io::import("wh.png", &std::fs::read(&path).unwrap()).unwrap().document;
    assert_eq!((written.size.width, written.size.height), (60, 40));
    let st = app.session.active().unwrap();
    assert_eq!((st.revision, st.doc.size), (revision, photocraft_doc::Size::new(120, 80)));
    let _ = std::fs::remove_file(path);
}

/// #15484: with Constrain Proportions off, a typed width and height both reach the file; the exact size replaces the
/// percent the dialog started with, and the document itself is never resized.
#[test]
fn save_for_web_unlinked_width_and_height_size_the_file() {
    let mut app = app();
    let id = open_web(&mut app).expect("an open document");
    let revision = app.session.active().unwrap().revision;
    set_option(&mut app, id, "format", json!(WebFormat::Png24));
    set_option(&mut app, id, "__constrain", json!(false));
    set_option(&mut app, id, "width", json!(90));
    set_option(&mut app, id, "height", json!(20));
    let path = std::env::temp_dir().join(format!("pc-webui-unlinked-{}.png", std::process::id())).to_string_lossy().into_owned();
    answer_path(&mut app, id, &path);
    confirm(&mut app, id);
    let written = photocraft_io::import("unlinked.png", &std::fs::read(&path).unwrap()).unwrap().document;
    assert_eq!((written.size.width, written.size.height), (90, 20));
    let st = app.session.active().unwrap();
    assert_eq!((st.revision, st.doc.size), (revision, photocraft_doc::Size::new(120, 80)));
    let _ = std::fs::remove_file(path);
}

/// #15485: the panes get the ignored-slices notice only when the document has more slices than are resolved.
#[test]
fn save_for_web_warns_only_past_the_slice_limit() {
    let mut doc = photocraft_doc::Document::new("slices", photocraft_doc::Size::new(8, 8), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8);
    let slice = |i: usize| photocraft_doc::Slice { id: i as u32 + 1, rect: photocraft_doc::Rect::new(0, 0, 8, 8), ..Default::default() };
    doc.slices.list = (0..3).map(slice).collect();
    assert!(super::web::slice_warning(&doc).is_none(), "a few slices: no notice");
    doc.slices.list = (0..photocraft_doc::slices::MAX_RESOLVED_SLICES + 2).map(slice).collect();
    let warning = super::web::slice_warning(&doc).expect("past the limit: a notice");
    assert_eq!(warning.as_str(), "2 stored slices past the first 10000 were ignored");
}

/// Reopened on another document, Save for Web starts at that document's full size again
/// (#1857) but keeps the resampling method, like the other encoding choices.
#[test]
fn save_for_web_reopens_at_full_size_with_the_resample_method() {
    let mut app = app();
    let id = open_web(&mut app).expect("an open document");
    set_option(&mut app, id, "format", json!(WebFormat::Png24));
    set_option(&mut app, id, "percent", json!(50.0));
    set_option(&mut app, id, "resample", json!(Resample::NearestNeighbor));
    let path = std::env::temp_dir().join(format!("pc-webui-reopen-{}.png", std::process::id())).to_string_lossy().into_owned();
    answer_path(&mut app, id, &path);
    confirm(&mut app, id);
    app.run("file.new", json!({"width": 240, "height": 160, "name": "next"})).unwrap();
    let id = open_web(&mut app).expect("an open document");
    let o = web_dialog(&mut app, id).current().clone();
    assert_eq!((o.percent, o.resample), (100.0, Resample::NearestNeighbor));
    let _ = std::fs::remove_file(path);
}

#[test]
fn save_for_web_downloads_images_and_html_through_the_platform_writer() {
    let mut app = app();
    let downloads = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let captured = downloads.clone();
    app.services.write = Some(Box::new(move |path, bytes| {
        captured.borrow_mut().push((path.to_string(), bytes.to_vec()));
        Ok(())
    }));
    download_for_web(&mut app, json!({"path": "web.png", "format": "png24"})).unwrap();
    assert_eq!(downloads.borrow()[0].0, "web.png");
    assert!(downloads.borrow()[0].1.starts_with(b"\x89PNG\r\n\x1a\n"));
    downloads.borrow_mut().clear();

    app.run("slice.new", json!({"rect": [0, 0, 60, 80], "name": "left"})).unwrap();
    download_for_web(&mut app, json!({"dir": ".", "format": "png24", "html": true, "imagesFolder": "images"})).unwrap();
    let files = downloads.borrow();
    let html = std::str::from_utf8(&files.iter().find(|(path, _)| path.ends_with(".html")).unwrap().1).unwrap();
    assert!(!html.contains("images/"), "downloaded files are siblings, not an images directory");
    for (path, bytes) in files.iter().filter(|(path, _)| !path.ends_with(".html")) {
        let name = std::path::Path::new(path).file_name().unwrap().to_str().unwrap();
        assert!(html.contains(&format!("src=\"{name}\"")), "missing downloaded image {name}");
        assert!(!bytes.is_empty());
    }
    assert!(files.iter().any(|(path, _)| path.ends_with("spacer.gif")));
    drop(files);

    app.services.write = Some(Box::new(|_, _| Err("download failed".into())));
    assert_eq!(download_for_web(&mut app, json!({"path": "fail.png", "format": "png24"})).unwrap_err(), "download failed");
    app.services.write = None;
    assert_eq!(download_for_web(&mut app, json!({"path": "missing.png"})).unwrap_err(), "no download service");
}

/// Presses Save… in the dialog's own UI and hands the app back.
fn press_save(app: Box<PhotocraftApp>) -> Box<PhotocraftApp> {
    use egui_kittest::kittest::Queryable;
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1600.0, 1100.0))
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    h.run_steps(5);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Save…").click();
    h.run_steps(3);
    h.into_state()
}

/// The export policy judges the parameters when Save… asks for a path: a denied export opens no picker and writes nothing.
#[test]
fn save_for_web_authorizes_before_opening_a_picker() {
    fn deny(_: &str, _: &Value) -> photocraft_engine::Result<()> {
        Err(photocraft_engine::EngineError::Other("export denied".into()))
    }
    for synthetic_input in [false, true] {
        let mut app = app();
        open_web(&mut app).expect("an open document");
        if synthetic_input {
            app.automation_input = true;
            app.services.automation_authorize = Some(deny);
        } else {
            app.session.authorize = Some(deny);
        }
        app.services.write = Some(Box::new(|_, _| panic!("denied export must not write")));
        let mut app = press_save(app);
        let ctx = app.ctx().clone();
        take(&mut app, &ctx);
        assert_eq!(app.ui.status, "export denied");
        assert!(app.ui.status_error);
        assert!(app.framework.dialogs.is_open::<HostedSaveForWeb>(), "the dialog stays open for another try");
    }
}

#[test]
fn save_for_web_authorizes_generated_palette_before_opening_a_picker() {
    fn deny_palette(_: &str, p: &Value) -> photocraft_engine::Result<()> {
        if p.get("colorTable").is_some() { Err(photocraft_engine::EngineError::Other("palette denied".into())) } else { Ok(()) }
    }
    for synthetic_input in [false, true] {
        let mut app = app();
        let id = open_web(&mut app).expect("an open document");
        set_option(&mut app, id, "format", json!(WebFormat::Gif));
        if synthetic_input {
            app.automation_input = true;
            app.services.automation_command = Some(Box::new(|cmd, p| deny_palette(cmd, p).map_err(|e| e.to_string())));
        } else {
            app.session.authorize = Some(deny_palette);
        }
        app.services.write = Some(Box::new(|_, _| panic!("denied palette must not write")));
        let mut app = press_save(app);
        let ctx = app.ctx().clone();
        take(&mut app, &ctx);
        assert_eq!(app.ui.status, "palette denied");
    }
}

/// The printer list handed to the dialog: the system's printers by name with `_` shown as a space, and a saved
/// printer the spooler no longer lists kept.
#[test]
fn printer_menu_lists_the_spooler_printers_after_the_default() {
    let list = json!([{"name": "HP_OfficeJet_3830_series", "default": true}, {"name": "DNP_DP_DS620", "default": false}]);
    let shown = |p: &craft::print::ui::Printer| if p.label.is_empty() { p.name.replace('_', " ") } else { p.label.clone() };
    let printers = printer_list(Some(&list));
    let names: Vec<&str> = printers.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["HP_OfficeJet_3830_series", "DNP_DP_DS620"]);
    assert!(printers[0].default && !printers[1].default);
    assert_eq!(shown(&printers[1]), "DNP DP DS620");
    assert_eq!(printer_list(None).len(), 0);
}

/// An output path or a preview typed and confirmed in one frame doesn't take the driver's area.
#[test]
fn the_printable_area_goes_only_with_a_print_to_the_printer() {
    let area = json!([0.0, 0.0, 842.29, 1191.14]);
    let mut f = DialogFields::new();
    f.set("printable", fv(area.clone()));
    // Four numbers read back as the dialog fields' four-component (f32) value, so they come back within f32's
    // precision; `file.print` allows 2 pt between the driver's area and the paper.
    // Every element must be a number: one that is not fails here rather than being dropped.
    let numbers = |v: &Value| -> Option<[f64; 4]> { <[f64; 4]>::try_from(v.as_array()?.iter().map(Value::as_f64).collect::<Option<Vec<f64>>>()?).ok() };
    let params = print_params(&f);
    let sent = numbers(&params["printable"]).unwrap_or_else(|| panic!("the area is four numbers [x, y, w, h]: {}", params["printable"]));
    let want = numbers(&area).expect("the test's area is four numbers");
    assert!(sent.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-3), "the driver's area goes with the print: {sent:?}");
    f.set("output", "/tmp/out.pdf");
    assert!(print_params(&f).get("printable").is_none());
    f.set("output", "");
    f.set("dryRun", true);
    assert!(print_params(&f).get("printable").is_none());
}

/// #7410: File › Print opens with the settings of the last print, also those remembered from
/// an earlier session (the preferences), before anything was printed in this one.
#[test]
fn print_dialog_opens_with_the_settings_remembered_from_an_earlier_session() {
    let mut app = app();
    app.session.prefs.edit(|p| {
        p.dialogs
            .insert(photocraft_engine::print_cmds::REMEMBERED.into(), json!({"paper": "a5", "scale": 50, "orientation": "landscape", "intent": "perceptual"}));
    });
    let id = open_print(&mut app);
    let f = print_fields(&mut app, id);
    assert_eq!((field(f, "paper"), field(f, "orientation"), field(f, "intent")), (json!("a5"), json!("landscape"), json!("perceptual")));
    assert_eq!(f.number("scale"), Some(50.0));
    assert_eq!(f.bool("center"), Some(true), "what wasn't remembered keeps its default");
}

#[test]
fn print_dialog_does_not_inherit_scripted_send_override() {
    for send in [false, true] {
        let mut app = app();
        let ctx = app.ctx().clone();
        let out = std::env::temp_dir().join(format!("pc-printui-state-{}-{send}.pdf", std::process::id())).to_string_lossy().into_owned();
        app.session.execute("file.print", json!({"output": out, "send": send, "dryRun": true})).unwrap();
        assert!(app.session.file_menu.last_print.as_ref().unwrap().get("send").is_none());
        let r = crate::menus::invoke(&mut app, &ctx, "file.print", json!({})).unwrap();
        let id = serde_json::from_value::<DialogId>(r["dialog"].clone()).expect("the command opened a dialog");
        assert!(!print_fields(&mut app, id).contains("send"));
        std::fs::remove_file(out).unwrap();
    }
}

/// Selected Slices exports only the chosen slices: the numbers go with the parameters, nothing selected is an
/// error, and a document without slices ignores the choice.
#[test]
fn the_web_preview_selects_slices_and_saves_only_the_selected_ones() {
    // A 120 × 80 document with a user slice on its left half; the right half is auto slice 2.
    let mut app = app();
    app.run("slice.new", json!({"rect": [0, 0, 60, 80], "name": "left"})).unwrap();
    let id = open_web(&mut app).expect("an open document");
    set_option(&mut app, id, "format", json!(WebFormat::Png24));
    set_option(&mut app, id, "slices", json!(WebSlices::Selected(vec![2])));
    let mut options = web_dialog(&mut app, id).current().clone();
    assert_eq!(options.slices, WebSlices::Selected(vec![2]));
    let p = web_params(&options, true).unwrap();
    assert_eq!((p["slices"].clone(), p["numbers"].clone()), (json!("selected"), json!([2])));
    options.slices = WebSlices::Selected(Vec::new());
    assert!(web_params(&options, true).is_err(), "Selected Slices with nothing selected is an error, not every slice");
    assert!(web_params(&options, false).unwrap().get("numbers").is_none(), "a document without slices ignores the choice");
    assert_eq!(options_of(&p).slices, WebSlices::Selected(vec![2]));

    let downloads = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let captured = downloads.clone();
    app.services.write = Some(Box::new(move |path, _| {
        captured.borrow_mut().push(path.to_string());
        Ok(())
    }));
    let mut p = p;
    p["dir"] = json!(".");
    p["html"] = json!(false);
    download_for_web(&mut app, p).unwrap();
    let files = downloads.borrow();
    assert_eq!(files.len(), 1, "only the selected slice is saved: {files:?}");
    assert!(!files[0].contains("left"), "{files:?}");
}
