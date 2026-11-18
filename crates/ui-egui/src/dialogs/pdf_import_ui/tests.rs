use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_engine::Session;
use serde_json::{Value, json};

use super::*;
use crate::{PhotocraftApp, Services};

/// A hand-written PDF: `pages` are (width, height, content stream).
fn pdf(pages: &[(f32, f32, &str)]) -> Vec<u8> {
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    let mut objs: Vec<Vec<u8>> =
        vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(), format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).into_bytes()];
    for (i, (w, h, content)) in pages.iter().enumerate() {
        objs.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).into_bytes());
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents {} 0 R >>", 3 + 2 * i).into_bytes());
    }
    write(objs)
}

/// A one-page PDF drawing one uncompressed RGB image of `w × h` pixels, `placed` wide in points.
fn image_pdf(w: u32, h: u32, placed: f32) -> Vec<u8> {
    let px: Vec<u8> = (0..w * h * 3).map(|k| (k % 251) as u8).collect();
    let mut img =
        format!("<< /Type /XObject /Subtype /Image /Width {w} /Height {h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n", px.len())
            .into_bytes();
    img.extend_from_slice(&px);
    img.extend_from_slice(b"\nendstream");
    let content = format!("q {placed} 0 0 {} 50 50 cm /Im0 Do Q", placed * h as f32 / w as f32);
    let objs = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 300] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).into_bytes(),
        img,
    ];
    write(objs)
}

fn write(objs: Vec<Vec<u8>>) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn app() -> Box<PhotocraftApp> {
    PhotocraftApp::new(
        Session::new(),
        Services {
            import: Some(Box::new(|name: &str, bytes: &[u8], opts: &photocraft_io::ImportOptions| {
                photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        },
    )
}

fn three_pages() -> Vec<u8> {
    pdf(&[(612.0, 792.0, "1 0 0 rg 72 72 468 648 re f"), (595.0, 420.0, "0 1 0 rg 30 30 535 360 re f"), (300.0, 300.0, "0 0 1 rg 20 20 260 260 re f")])
}

/// The open PDF dialogs (Import PDF and password), oldest first: (id, key, values).
fn pdf_dialogs(app: &PhotocraftApp) -> Vec<(DialogId, String, Value)> {
    app.framework
        .dialogs
        .open_dialogs()
        .into_iter()
        .filter_map(|d| {
            let key = d.key?;
            (owns_import(&key) || owns_password(&key)).then(|| (d.id, key, d.fields.unwrap_or(Value::Null)))
        })
        .collect()
}

/// The one open PDF dialog.
fn dialog(app: &PhotocraftApp) -> Option<(DialogId, String, Value)> {
    let mut all = pdf_dialogs(app);
    assert!(all.len() <= 1, "at most one PDF dialog is open");
    all.pop()
}

fn id(app: &PhotocraftApp) -> DialogId {
    dialog(app).expect("the PDF dialog").0
}

/// Sets value `key` of dialog `id`, as `ui.dialog.set` does.
fn set_on(app: &mut PhotocraftApp, id: DialogId, key: &str, v: Value) {
    let ctx = app.ctx().clone();
    app.framework.dialogs.set_json_field(&ctx, id, key, v).expect("the dialog takes the value");
}

/// Sets value `key` of the one PDF dialog.
fn set(app: &mut PhotocraftApp, key: &str, v: Value) {
    let id = id(app);
    set_on(app, id, key, v);
}

fn confirm(app: &mut PhotocraftApp, id: DialogId) -> Result<(), String> {
    crate::dialogs::testing::confirm(app, id)
}

/// The file the first held PDF holds.
fn held(app: &PhotocraftApp) -> Arc<Mutex<Held>> {
    Arc::clone(&app.pdf_import.first().expect("a held PDF").held)
}

#[test]
fn opening_a_pdf_shows_the_dialog_with_photoshop_defaults() {
    let mut app = app();
    let warnings = app.open_file("/tmp/pc-test/multi.pdf", &three_pages()).expect("open");
    assert!(warnings.is_empty());
    assert!(app.session.documents().is_empty(), "nothing opens before OK");
    let (_, key, f) = dialog(&app).expect("the Import PDF dialog");
    assert!(owns_import(&key), "{key}");
    assert_eq!(f.get("name"), Some(&json!("multi.pdf")), "the Name field starts as the file name");
    assert_eq!(f.get("select"), Some(&json!("pages")));
    assert_eq!(f.get("pages"), Some(&json!([1])));
    assert_eq!(f.get("crop"), Some(&json!("boundingBox")));
    assert_eq!(f.get("antiAlias"), Some(&json!(true)));
    assert_eq!(f.get("resolution"), Some(&json!(300.0)));
    assert_eq!(f.get("mode"), Some(&json!("rgb")));
    assert_eq!(f.get("depth"), Some(&json!(8)));
    assert_eq!(f.get("constrain"), Some(&json!(false)));
    assert_eq!(f.get("suppressWarnings"), Some(&json!(false)));
    assert_eq!(f.get("reverseOrder"), Some(&json!(true)));
    assert_eq!(f.get("thumbnailSize"), Some(&json!("small")));
    let held = held(&app);
    let st = lock(&held);
    assert_eq!(st.info.as_ref().map(|i| i.pages.len()), Some(3));
    assert_eq!(st.path.as_deref(), Some("/tmp/pc-test/multi.pdf"));
}

#[test]
fn ok_opens_the_selected_pages_in_reverse_order_as_unsaved_documents() {
    let mut app = app();
    app.open_file("/tmp/pc-test/multi.pdf", &three_pages()).expect("open");
    let id = id(&app);
    set(&mut app, "pages", json!([1, 2, 3]));
    set(&mut app, "resolution", json!(72.0));
    set(&mut app, "crop", json!("mediaBox"));
    confirm(&mut app, id).expect("OK");
    assert!(dialog(&app).is_none() && app.pdf_import.is_empty());
    let names: Vec<&str> = app.session.documents().iter().map(|d| d.doc.name.as_str()).collect();
    assert_eq!(names, ["multi-3.pdf", "multi-2.pdf", "multi-1.pdf"], "last page first, so page 1 ends up on top");
    let active = app.session.active().expect("a document");
    assert_eq!(active.doc.name, "multi-1.pdf");
    assert_eq!((active.doc.size.width, active.doc.size.height, active.doc.resolution_dpi), (612, 792, 72.0));
    assert!(active.source_read_only && active.path.is_none(), "Save must not write into the PDF");
    assert_eq!(app.session.documents()[0].doc.size.width, 300);
    assert!(app.ui.recent_files.iter().any(|p| p.ends_with("multi.pdf")), "the PDF goes to Open Recent");
}

#[test]
fn the_name_field_and_the_page_options_reach_the_documents() {
    let mut app = app();
    app.open_bytes("multi.pdf", &three_pages()).expect("open");
    let id = id(&app);
    set(&mut app, "name", json!("Foo"));
    set(&mut app, "pages", json!([2]));
    set(&mut app, "resolution", json!(72.0));
    set(&mut app, "mode", json!("gray"));
    set(&mut app, "depth", json!(16));
    set(&mut app, "crop", json!("boundingBox"));
    set(&mut app, "reverseOrder", json!(false));
    let fields = app.framework.dialogs.get_by_id::<craft::ui::patterns::form_dialogs::HostedParamDialog>(id).expect("dialog").fields().clone();
    confirm(&mut app, id).expect("OK");
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.name, "Foo-2.pdf");
    assert_eq!((doc.size.width, doc.size.height), (535, 360), "the bounding box of the green rectangle");
    assert_eq!((doc.mode, doc.depth), (photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::U16));
    // The options the values describe, read from the JSON automation set.
    let o = options(&fields, 2, false, None);
    assert_eq!((o.page, o.crop, o.resolution, o.anti_alias, o.image), (2, CropTo::BoundingBox, 72.0, true, None));
    assert_eq!((o.mode, o.depth, o.name.as_deref()), (photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::U16, Some("Foo")));
}

#[test]
fn cancel_opens_nothing_and_lets_the_file_go() {
    let mut app = app();
    app.open_bytes("multi.pdf", &three_pages()).expect("open");
    let id = id(&app);
    crate::dialogs::testing::cancel(&mut app, id).expect("cancel");
    assert!(app.session.documents().is_empty());
    assert!(app.pdf_import.is_empty() && dialog(&app).is_none());
}

/// Two PDFs opened together (Open with both selected, a drop, the command line) each keep their
/// own file behind their own dialog: OK on either opens that file, Cancel on one leaves the other.
#[test]
fn two_pdfs_keep_their_own_dialogs_and_files() {
    let a = pdf(&[(100.0, 80.0, "1 0 0 rg 0 0 100 80 re f")]);
    let b = pdf(&[(60.0, 40.0, "0 0 1 rg 0 0 60 40 re f")]);
    let open_both = |app: &mut PhotocraftApp| {
        app.open_file("/tmp/pc-test/a.pdf", &a).expect("open a");
        app.open_file("/tmp/pc-test/b.pdf", &b).expect("open b");
        let dialogs = pdf_dialogs(app);
        assert_eq!((dialogs.len(), app.pdf_import.len()), (2, 2), "a dialog and a held file each");
        let names: Vec<String> = app.pdf_import.iter().map(|p| lock(&p.held).file_name.clone()).collect();
        assert_eq!(names, ["a.pdf", "b.pdf"]);
        // The dialogs in the order of the files they hold.
        let id_of = |n: u64| dialogs.iter().find(|d| number_of(&d.1) == Some(n)).expect("its dialog").0;
        let (da, db) = (id_of(app.pdf_import[0].number), id_of(app.pdf_import[1].number));
        for id in [da, db] {
            set_on(app, id, "resolution", json!(72.0));
        }
        (da, db)
    };
    let opened = |app: &PhotocraftApp| -> Vec<(String, u32, u32, Vec<f32>)> {
        app.session
            .documents()
            .iter()
            .map(|d| (d.doc.name.clone(), d.doc.size.width, d.doc.size.height, d.doc.layers[0].surface().unwrap().pixel(5, 5)))
            .collect()
    };
    // B's dialog first: B's file, by name and content; A's dialog and file stay.
    let mut app = app();
    let (da, db) = open_both(&mut app);
    confirm(&mut app, db).expect("OK on b");
    assert_eq!(opened(&app), [("b".to_string(), 60, 40, vec![0.0, 0.0, 1.0, 1.0])]);
    assert_eq!(app.pdf_import.iter().map(|p| lock(&p.held).file_name.clone()).collect::<Vec<_>>(), ["a.pdf"], "a's file is still held");
    assert_eq!(pdf_dialogs(&app).iter().map(|d| d.0).collect::<Vec<_>>(), [da], "a's dialog is still up");
    confirm(&mut app, da).expect("OK on a");
    assert_eq!(opened(&app).last().map(|d| (d.0.as_str(), d.1, d.2, d.3.clone())), Some(("a", 100, 80, vec![1.0, 0.0, 0.0, 1.0])));
    assert!(app.pdf_import.is_empty() && pdf_dialogs(&app).is_empty());
    // Cancel on one lets only its file go.
    let mut app = self::app();
    let (da, db) = open_both(&mut app);
    crate::dialogs::testing::cancel(&mut app, da).expect("cancel a");
    assert_eq!(app.pdf_import.iter().map(|p| lock(&p.held).file_name.clone()).collect::<Vec<_>>(), ["b.pdf"]);
    confirm(&mut app, db).expect("OK on b");
    assert_eq!(opened(&app), [("b".to_string(), 60, 40, vec![0.0, 0.0, 1.0, 1.0])]);
    assert!(app.pdf_import.is_empty());
}

#[test]
fn ok_with_nothing_selected_keeps_the_dialog_up() {
    let mut app = app();
    app.open_bytes("multi.pdf", &three_pages()).expect("open");
    let id = id(&app);
    set(&mut app, "pages", json!([]));
    let refusal = confirm(&mut app, id).expect_err("OK is refused");
    assert_eq!(refusal, "Select one or more pages.");
    assert!(app.session.documents().is_empty());
    assert_eq!(dialog(&app).map(|d| d.0), Some(id), "still up");
    assert_eq!(app.pdf_import.len(), 1);
}

#[test]
fn a_one_image_pdf_opens_like_an_image_without_asking() {
    let mut app = app();
    assert!(!wants_dialog("scan.pdf", &image_pdf(100, 50, 200.0)));
    app.open_file("/tmp/pc-test/scan.pdf", &image_pdf(100, 50, 200.0)).expect("open");
    assert!(dialog(&app).is_none());
    let st = app.session.active().expect("the image document");
    assert_eq!((st.doc.name.as_str(), st.doc.size.width, st.doc.size.height, st.doc.resolution_dpi), ("scan.pdf", 100, 50, 36.0));
    assert_eq!(st.doc.layers[0].name, "Background");
    assert!(!st.source_read_only && st.path.as_deref() == Some("/tmp/pc-test/scan.pdf"), "like any image file");
}

/// Two pages: the second draws an 8 × 4 image beside a rectangle, so the file isn't image-only.
fn pair_pdf() -> Vec<u8> {
    let px: Vec<u8> = (0..8 * 4 * 3).map(|k| (k % 251) as u8).collect();
    let mut img = format!("<< /Type /XObject /Subtype /Image /Width 8 /Height 4 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n", px.len())
        .into_bytes();
    img.extend_from_slice(&px);
    img.extend_from_slice(b"\nendstream");
    let c1 = "1 0 0 rg 0 0 50 50 re f";
    let c2 = "q 80 0 0 40 100 100 cm /Im0 Do Q 0 0 1 rg 0 0 10 10 re f";
    let objs = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R >>".to_vec(),
        format!("<< /Length {} >>\nstream\n{c1}\nendstream", c1.len()).into_bytes(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 300] /Resources << /XObject << /Im0 7 0 R >> >> /Contents 6 0 R >>".to_vec(),
        format!("<< /Length {} >>\nstream\n{c2}\nendstream", c2.len()).into_bytes(),
        img,
    ];
    write(objs)
}

#[test]
fn images_mode_opens_the_embedded_images() {
    let mut app = app();
    app.open_bytes("pair.pdf", &pair_pdf()).expect("open");
    let id = id(&app);
    set(&mut app, "select", json!("images"));
    set(&mut app, "images", json!([1]));
    confirm(&mut app, id).expect("OK");
    let doc = &app.session.active().unwrap().doc;
    assert_eq!((doc.name.as_str(), doc.size.width, doc.size.height), ("pair-1", 8, 4));
    assert_eq!(doc.layers[0].name, "Background");
}

#[test]
fn what_import_pdf_opens_from_a_file_stays_untitled() {
    let dir = std::env::temp_dir().join(format!("photocraft-pdf-untitled-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pair.pdf").to_string_lossy().into_owned();
    std::fs::write(&path, pair_pdf()).unwrap();
    let ctx = egui::Context::default();
    // Opened here, then on a worker: the image (Images mode), then page 2.
    for background in [false, true] {
        let mut app = app();
        app.background_jobs = background;
        for (select, key) in [("images", "images"), ("pages", "pages")] {
            app.open_path(&path).expect("open");
            let id = dialog(&app).expect("Import PDF asks every time").0;
            set(&mut app, "select", json!(select));
            set(&mut app, key, json!([if select == "images" { 1 } else { 2 }]));
            confirm(&mut app, id).expect("OK");
            for _ in 0..400 {
                crate::jobs_ui::tick(&mut app, &ctx);
                if app.jobs.opens.is_empty() && !app.session.has_jobs() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let st = app.session.active().expect("the opened document");
            assert!(st.path.is_none() && st.source_read_only, "{select} (background {background}): Save must not write into the PDF");
            assert!(app.ui.recent_files.iter().any(|p| p.ends_with("pair.pdf")), "the PDF goes to Open Recent");
        }
        let names: Vec<&str> = app.session.documents().iter().map(|d| d.doc.name.as_str()).collect();
        assert_eq!(names, ["pair-1", "pair-2.pdf"], "background {background}: the second open asked again instead of switching tabs");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_svg_mentioning_pdf_opens_as_an_svg() {
    let svg = b"<?xml version=\"1.0\"?>\n<!-- exported from %PDF-1.4 source -->\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"4\"><rect width=\"8\" height=\"4\" fill=\"red\"/></svg>";
    assert!(!wants_dialog("drawing.svg", svg));
    let mut app = app();
    app.open_file("/tmp/pc-test/drawing.svg", svg).expect("open");
    assert!(dialog(&app).is_none());
    let st = app.session.active().expect("the SVG document");
    assert_eq!((st.doc.size.width, st.doc.size.height), (8, 4));
    assert_eq!(st.path.as_deref(), Some("/tmp/pc-test/drawing.svg"));
}

#[test]
fn size_label_prints_three_significant_digits_like_photoshop() {
    assert_eq!(size_label(2550, 2856, "rgb", 8), "20.8M");
    assert_eq!(size_label(1000, 1120, "rgb", 8), "3.20M");
    assert_eq!(size_label(1000, 1120, "cmyk", 16), "8.54M");
    assert_eq!(size_label(100, 100, "gray", 8), "9.77K");
    assert_eq!(size_label(7000, 7000, "rgb", 8), "140M");
}

/// A harness drawing the hosted dialogs over `app`.
fn dialog_harness(app: Box<PhotocraftApp>) -> Harness<'static, Box<PhotocraftApp>> {
    let h = Harness::builder().with_context(app.ctx().clone()).with_size(egui::vec2(1100.0, 800.0)).build_ui_state(
        |ui, app: &mut Box<PhotocraftApp>| {
            // The app's font families arrive with `setup_context`, after the first frame.
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::dialogs::run(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::studio());
    h
}

#[test]
fn the_dialog_draws_its_controls() {
    let mut app = app();
    app.open_bytes("multi.pdf", &three_pages()).expect("open");
    let mut h = dialog_harness(app);
    h.run_steps(6);
    for label in ["Name:", "Width:", "Height:", "Resolution:"] {
        assert!(h.query_all_by_label(label).next().is_some(), "missing accessible field: {label}");
    }
    for label in [
        "Import PDF",
        "Select:",
        "Pages",
        "Images",
        "Page Options",
        "Anti-aliased",
        "Constrain Proportions",
        "Suppress Warnings",
        "Open In Reverse Order",
        "Thumbnail Size:",
        "1 of 3 page(s) selected",
        "Page 1",
    ] {
        assert!(h.query_all_by_label(label).next().is_some(), "missing {label}");
    }
    // The thumbnails were rendered.
    let held = held(h.state());
    assert!(lock(&held).thumbs.iter().all(|s| s.ready().is_some()), "every page has its thumbnail");
}

/// A click on a thumbnail selects that page, as the values say; ⌘/Ctrl-click adds one.
#[test]
fn clicking_thumbnails_selects_pages() {
    let mut app = app();
    app.open_bytes("multi.pdf", &three_pages()).expect("open");
    let mut h = dialog_harness(app);
    h.run_steps(6);
    h.get_by_label("Page 3").click();
    h.run_steps(2);
    assert_eq!(dialog(h.state()).map(|d| d.2["pages"].clone()), Some(json!([3])));
    assert!(h.query_all_by_label("1 of 3 page(s) selected").next().is_some());
}

#[test]
fn a_failed_thumbnail_is_not_retried_and_does_not_hold_back_the_rest() {
    // Pages 1–3 each carry an inline image past the decode budget, so their thumbnails are
    // refused; page 4 is plain. Three tries a frame used to go to the same three failures every
    // frame (re-parsing the file each time), and page 4 never got its thumbnail.
    let huge = "q 200 0 0 200 0 0 cm BI /W 40000 /H 40000 /CS /RGB /BPC 8 /F /Fl ID x EI Q";
    let bytes = pdf(&[(300.0, 300.0, huge), (300.0, 300.0, huge), (300.0, 300.0, huge), (300.0, 300.0, "0 0 1 rg 20 20 260 260 re f")]);
    let mut app = app();
    app.open_file("/tmp/pc-test/bad-thumbs.pdf", &bytes).expect("open");
    let held = held(&app);
    let mut st = lock(&held);
    let ctx = egui::Context::default();
    render_thumbs(&mut st, &ctx, false, "small");
    assert!(st.thumbs[..3].iter().all(|s| matches!(s, Slot::Failed)), "the refused pages are recorded");
    assert!(st.thumbs[3].is_pending(), "one frame tries three pages");
    render_thumbs(&mut st, &ctx, false, "small");
    assert!(st.thumbs[3].ready().is_some(), "the next frame reaches page 4");
    assert!(!st.thumbs.iter().any(Slot::is_pending), "nothing is left to retry");
    // A new Thumbnail Size tries every page again.
    render_thumbs(&mut st, &ctx, false, "large");
    assert!(st.thumbs[..3].iter().all(|s| matches!(s, Slot::Failed)));
    assert!(st.thumbs[3].is_pending());
}

/// A PDF that needs a user password: a Standard security handler whose `/U` entry doesn't
/// match the empty password, so opening it without one fails with "password required".
fn encrypted_pdf() -> Vec<u8> {
    let plain = String::from_utf8(three_pages()).expect("the hand-written PDF is ASCII");
    let filler = "AB".repeat(32);
    let encrypt = format!("/Encrypt << /Filter /Standard /V 1 /R 2 /Length 40 /P -4 /O <{filler}> /U <{filler}> >> /ID [<{filler}> <{filler}>]");
    plain.replace("/Root 1 0 R >>", &format!("/Root 1 0 R {encrypt} >>")).into_bytes()
}

/// An encrypted PDF asks for its password first; a wrong one is refused with the reason and the
/// prompt stays up, and Cancel lets the file go.
#[test]
fn an_encrypted_pdf_asks_for_its_password() {
    let bytes = encrypted_pdf();
    assert!(pdf_import::needs_password(&bytes), "the fixture must ask for a password");
    let mut app = app();
    app.open_bytes("locked.pdf", &bytes).expect("open");
    let (id, key, _) = dialog(&app).expect("the password dialog");
    assert!(owns_password(&key), "{key}");
    set(&mut app, "password", json!("wrong"));
    assert_eq!(confirm(&mut app, id), Err("That isn't the PDF's password.".to_string()));
    assert_eq!(dialog(&app).map(|d| d.0), Some(id), "the prompt stays up");
    assert!(lock(&held(&app)).info.is_none(), "nothing was read");
    crate::dialogs::testing::cancel(&mut app, id).expect("cancel");
    assert!(app.pdf_import.is_empty() && dialog(&app).is_none());
}

#[test]
fn the_password_prompt_names_its_field() {
    let mut app = app();
    app.open_bytes("locked.pdf", &encrypted_pdf()).expect("open");
    let mut h = dialog_harness(app);
    h.run_steps(4);
    assert!(h.query_all_by_label("The PDF is protected by a password.").next().is_some(), "the password prompt is shown");
    let named: Vec<String> = h.query_all_by_role(egui::accesskit::Role::PasswordInput).filter_map(|n| n.accesskit_node().label()).collect();
    assert!(named.iter().any(|l| l == "Password"), "the password field has no accessible name: {named:?}");
}

/// The password is masked as it is typed: the field never shows (or tells assistive technology)
/// the characters.
#[test]
fn the_password_field_is_masked() {
    let mut app = app();
    app.open_bytes("locked.pdf", &encrypted_pdf()).expect("open");
    set(&mut app, "password", json!("hunter2"));
    let mut h = dialog_harness(app);
    h.run_steps(4);
    let fields: Vec<_> =
        h.query_all_by_role(egui::accesskit::Role::PasswordInput).filter(|n| n.accesskit_node().label().as_deref() == Some("Password")).collect();
    assert_eq!(fields.len(), 1, "one masked password field");
    let shown = fields[0].accesskit_node().value().unwrap_or_default();
    assert!(!shown.contains("hunter2"), "the password shows as {shown:?}");
    assert!(h.query_all_by_label_contains("hunter2").next().is_none(), "no label shows it either");
}

/// A `pixelSize` side past `u32` keeps `u32::MAX` (the importer's page cap then scales it down
/// with its warning); `as u32` wrapped 2^32 + 5 to 5 pixels.
#[test]
fn an_oversized_pixel_size_is_not_wrapped() {
    let mut f = DialogFields::new();
    f.set("pixelSize", FieldValue::Json(json!([(1u64 << 32) + 5, 600])));
    assert_eq!(pixel_size_field(&f), Some((u32::MAX, 600)));
    assert_eq!(options(&f, 1, false, None).pixel_size, Some((u32::MAX, 600)));
    f.set("pixelSize", FieldValue::Json(json!([800, "tall"])));
    assert_eq!(pixel_size_field(&f), None);
}

/// When the images can't be listed, the reason is kept for the panel instead of an empty list
/// that looks like a PDF without images.
#[test]
fn a_failed_image_listing_keeps_its_error() {
    let mut app = app();
    app.open_bytes("locked.pdf", &encrypted_pdf()).expect("open");
    let held = held(&app);
    let mut st = lock(&held);
    render_thumbs(&mut st, &egui::Context::default(), true, "small");
    assert_eq!(st.images.as_ref().map(Vec::len), Some(0));
    let e = st.images_error.as_deref().expect("the listing error is kept");
    assert!(e.contains(pdf_import::PASSWORD_REQUIRED), "{e}");
}

/// With background opens, the chosen pages are one ordered set: their tabs keep the chosen order
/// (here, Open In Reverse Order) however the decodes finish, and a PDF dropped between tabs puts
/// its pages there.
#[test]
fn background_pages_keep_their_order_and_the_drop_slot() {
    use crate::jobs_ui::{self, OpenTab};
    let mut app = app();
    for name in ["a.psd", "b.psd"] {
        app.session.add_document(
            photocraft_doc::Document::new(name, photocraft_geom::Size::new(4, 4), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8),
            None,
        );
    }
    app.background_jobs = true;
    app.open_bytes_placed("multi.pdf", &three_pages(), None, Some(1), None).expect("dialog");
    let id = id(&app);
    set(&mut app, "pages", json!([1, 2, 3]));
    confirm(&mut app, id).expect("OK");
    let orders: Vec<_> = app.jobs.opens.iter().map(|o| o.order.map(|x| x.seq)).collect();
    assert_eq!(orders, [Some(0), Some(1), Some(2)], "every page is a member of one set, in the chosen order");
    // The decodes finish out of order: the middle page first, then the last, then the first.
    for seq in [1usize, 2, 0] {
        let i = app.jobs.opens.iter().position(|o| o.order.is_some_and(|x| x.seq == seq)).expect("still opening");
        let tab: OpenTab = app.jobs.opens.remove(i);
        let index = app.session.add_document(
            photocraft_doc::Document::new(
                format!("seq{seq}"),
                photocraft_geom::Size::new(4, 4),
                photocraft_color::ColorMode::Rgb,
                photocraft_color::SampleType::U8,
            ),
            None,
        );
        let doc = app.session.documents()[index].doc.id;
        jobs_ui::finish_open_tab(&mut app, &tab, &json!({"document": index, "docId": doc.0})).expect("placed");
    }
    let docs: Vec<&str> = app.session.documents().iter().map(|d| d.doc.name.as_str()).collect();
    // seq 0 is page 3 (Open In Reverse Order), so page 1 ends up last, as in the inline open.
    assert_eq!(docs, ["a.psd", "seq0", "seq1", "seq2", "b.psd"], "the set's order, at the drop slot");
    assert!(app.jobs.batches.is_empty(), "the set is forgotten once every page opened");
}

/// Under Crop To, a line says what the chosen box keeps, and follows the choice: Bounding Box cuts
/// a page's blank margins, which surprised a tester expecting the page's full size (#11414).
#[test]
fn crop_to_says_what_the_chosen_box_keeps() {
    let mut app = app();
    app.open_bytes("multi.pdf", &three_pages()).expect("open");
    let mut h = dialog_harness(app);
    h.run_steps(4);
    assert!(h.query_all_by_label(CropTo::BoundingBox.description()).next().is_some(), "the default's hint");
    set(h.state_mut(), "crop", json!("mediaBox"));
    h.run_steps(2);
    assert!(h.query_all_by_label(CropTo::MediaBox.description()).next().is_some(), "the hint follows the choice");
    assert!(h.query_all_by_label(CropTo::BoundingBox.description()).next().is_none(), "one hint at a time");
    let hints: std::collections::HashSet<&str> = CropTo::ALL.iter().map(|c| c.description()).collect();
    assert_eq!(hints.len(), CropTo::ALL.len(), "every choice has its own hint");
}

/// The Crop To hint and the Anti-aliased box start where the dropdown above them starts, in the
/// column of the fields, not at the labels' edge.
#[test]
fn crop_to_hint_and_anti_aliased_line_up_with_the_dropdown() {
    let mut app = app();
    app.open_bytes("multi.pdf", &three_pages()).expect("open");
    let mut h = dialog_harness(app);
    h.run_steps(4);
    let hint = h.query_all_by_label(CropTo::BoundingBox.description()).next().expect("the hint").rect();
    let combo = h
        .query_all_by_role(egui::accesskit::Role::ComboBox)
        .map(|n| n.rect())
        .filter(|r| r.bottom() <= hint.top() + 0.5)
        .max_by(|a, b| a.bottom().total_cmp(&b.bottom()))
        .expect("the Crop To dropdown above the hint");
    assert!((hint.left() - combo.left()).abs() < 0.5, "hint at x {} but the dropdown at x {}", hint.left(), combo.left());
    let aa = h
        .query_all_by_role(egui::accesskit::Role::CheckBox)
        .find(|n| n.accesskit_node().label().as_deref() == Some("Anti-aliased"))
        .expect("the Anti-aliased box")
        .rect();
    assert!((aa.left() - combo.left()).abs() < 0.5, "Anti-aliased box at x {} but the dropdown at x {}", aa.left(), combo.left());
}

/// A ⇧-click range starts only from an item of the list shown: an index from Pages must not
/// start a range among the Images.
#[test]
fn a_range_does_not_start_from_the_other_list() {
    let mut app = app();
    app.open_bytes("pair.pdf", &pair_pdf()).expect("open");
    let mut h = dialog_harness(app);
    h.run_steps(4);
    h.get_by_label("Page 2").click();
    h.run_steps(2);
    assert_eq!(lock(&held(h.state())).anchor, Some((false, 1)), "the click anchors a range in Pages");
    h.get_by_label("Images").click();
    h.run_steps(4);
    h.get_by_label("Image 1").click_modifiers(egui::Modifiers::SHIFT);
    h.run_steps(2);
    assert_eq!(dialog(h.state()).map(|d| d.2["images"].clone()), Some(json!([1])), "only the clicked image");
}

/// Each labelled row of the right column is one line high, so the whole column fits in the
/// dialog: the framework's fields fill the height a horizontal row offers, which stretched every
/// row to the rest of the dialog and pushed Crop To and everything after it out of sight.
#[test]
fn the_option_rows_fit_in_the_dialog() {
    let mut app = app();
    app.open_bytes("multi.pdf", &three_pages()).expect("open");
    let mut h = dialog_harness(app);
    h.run_steps(6);
    let rect = |role: egui::accesskit::Role, label: &str| {
        h.query_all_by_role(role).find(|n| n.accesskit_node().label().as_deref() == Some(label)).unwrap_or_else(|| panic!("{label}")).rect()
    };
    let dialog = rect(egui::accesskit::Role::Dialog, "Import PDF");
    let name = rect(egui::accesskit::Role::TextInput, "Name:");
    let crop = rect(egui::accesskit::Role::ComboBox, "Crop To");
    assert!(crop.top() - name.bottom() < 16.0, "Crop To at y {} but Name ends at y {}", crop.top(), name.bottom());
    let last = rect(egui::accesskit::Role::CheckBox, "Open In Reverse Order");
    assert!(last.bottom() <= dialog.bottom(), "Open In Reverse Order ends at y {} below the dialog's {}", last.bottom(), dialog.bottom());
}
