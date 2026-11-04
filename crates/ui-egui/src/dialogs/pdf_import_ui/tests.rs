use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
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

fn app() -> PhotocraftApp {
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

fn dialog(app: &PhotocraftApp) -> Option<&crate::state::Dialog> {
    app.ui.dialogs.iter().find(|d| owns(&d.fields))
}

fn set(app: &mut PhotocraftApp, key: &str, v: Value) {
    let id = dialog(app).expect("the Import PDF dialog").id;
    app.ui.dialog_mut(id).expect("dialog").fields.insert(key.into(), v);
}

#[test]
fn opening_a_pdf_shows_the_dialog_with_photoshop_defaults() {
    let mut app = app();
    let warnings = app.open_file("/tmp/pc-test/multi.pdf", &three_pages()).expect("open");
    assert!(warnings.is_empty());
    assert!(app.session.documents().is_empty(), "nothing opens before OK");
    let d = dialog(&app).expect("the Import PDF dialog");
    let f = &d.fields;
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
    let st = app.pdf_import.first().expect("the file is held");
    assert_eq!(st.info.as_ref().map(|i| i.pages.len()), Some(3));
    assert_eq!(st.path.as_deref(), Some("/tmp/pc-test/multi.pdf"));
}

#[test]
fn ok_opens_the_selected_pages_in_reverse_order_as_unsaved_documents() {
    let mut app = app();
    app.open_file("/tmp/pc-test/multi.pdf", &three_pages()).expect("open");
    let id = dialog(&app).unwrap().id;
    set(&mut app, "pages", json!([1, 2, 3]));
    set(&mut app, "resolution", json!(72.0));
    set(&mut app, "crop", json!("mediaBox"));
    let r = crate::dialogs::confirm(&mut app, id).expect("OK");
    assert_eq!(r.get("opened"), Some(&json!(3)));
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
    let id = dialog(&app).unwrap().id;
    set(&mut app, "name", json!("Foo"));
    set(&mut app, "pages", json!([2]));
    set(&mut app, "resolution", json!(72.0));
    set(&mut app, "mode", json!("gray"));
    set(&mut app, "depth", json!(16));
    set(&mut app, "crop", json!("boundingBox"));
    set(&mut app, "reverseOrder", json!(false));
    crate::dialogs::confirm(&mut app, id).expect("OK");
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.name, "Foo-2.pdf");
    assert_eq!((doc.size.width, doc.size.height), (535, 360), "the bounding box of the green rectangle");
    assert_eq!((doc.mode, doc.depth), (photocraft_color::ColorMode::Grayscale, photocraft_color::SampleType::U16));
    // The options the fields describe.
    let o = options(&dialog_fields_after(&app), 2, false, None);
    assert_eq!((o.page, o.crop, o.resolution, o.anti_alias, o.image), (2, CropTo::BoundingBox, 72.0, true, None));
}

/// The fields a closed dialog had, rebuilt from defaults plus the test's edits (the dialog is
/// gone after OK, so this stands in for them).
fn dialog_fields_after(app: &PhotocraftApp) -> Map<String, Value> {
    let mut f = Map::new();
    defaults(&mut f, app);
    f.insert("resolution".into(), json!(72.0));
    f.insert("mode".into(), json!("gray"));
    f
}

#[test]
fn cancel_opens_nothing_and_lets_the_file_go() {
    let mut app = app();
    app.open_bytes("multi.pdf", &three_pages()).expect("open");
    let id = dialog(&app).unwrap().id;
    crate::dialogs::cancel(&mut app, id).expect("cancel");
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
        let ids: Vec<u64> = app.ui.dialogs.iter().filter(|d| owns(&d.fields)).map(|d| d.id).collect();
        assert_eq!((ids.len(), app.pdf_import.len()), (2, 2), "a dialog and a held file each");
        assert_eq!(app.pdf_import.iter().map(|st| st.file_name.as_str()).collect::<Vec<_>>(), ["a.pdf", "b.pdf"]);
        for id in &ids {
            app.ui.dialog_mut(*id).unwrap().fields.insert("resolution".into(), json!(72.0));
        }
        (ids[0], ids[1])
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
    let r = crate::dialogs::confirm(&mut app, db).expect("OK on b");
    assert_eq!(r.get("opened"), Some(&json!(1)));
    assert_eq!(opened(&app), [("b".to_string(), 60, 40, vec![0.0, 0.0, 1.0, 1.0])]);
    assert_eq!(app.pdf_import.iter().map(|st| st.dialog).collect::<Vec<_>>(), [da], "a's file is still held, for a's dialog");
    assert!(app.ui.dialogs.iter().any(|d| d.id == da));
    crate::dialogs::confirm(&mut app, da).expect("OK on a");
    assert_eq!(opened(&app).last().map(|d| (d.0.as_str(), d.1, d.2, d.3.clone())), Some(("a", 100, 80, vec![1.0, 0.0, 0.0, 1.0])));
    assert!(app.pdf_import.is_empty() && dialog(&app).is_none());
    // Cancel on one lets only its file go.
    let mut app = self::app();
    let (da, db) = open_both(&mut app);
    crate::dialogs::cancel(&mut app, da).expect("cancel a");
    assert_eq!(app.pdf_import.iter().map(|st| st.dialog).collect::<Vec<_>>(), [db]);
    crate::dialogs::confirm(&mut app, db).expect("OK on b");
    assert_eq!(opened(&app), [("b".to_string(), 60, 40, vec![0.0, 0.0, 1.0, 1.0])]);
    assert!(app.pdf_import.is_empty());
}

#[test]
fn ok_with_nothing_selected_keeps_the_dialog_up() {
    let mut app = app();
    app.open_bytes("multi.pdf", &three_pages()).expect("open");
    let id = dialog(&app).unwrap().id;
    set(&mut app, "pages", json!([]));
    let r = crate::dialogs::confirm(&mut app, id).expect("OK");
    assert_eq!(r.get("pending"), Some(&json!(true)));
    assert!(app.session.documents().is_empty());
    let d = dialog(&app).expect("still up");
    assert!(d.fields.get("__error").is_some());
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
    let id = dialog(&app).unwrap().id;
    set(&mut app, "select", json!("images"));
    set(&mut app, "images", json!([1]));
    crate::dialogs::confirm(&mut app, id).expect("OK");
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
            let id = dialog(&app).expect("Import PDF asks every time").id;
            set(&mut app, "select", json!(select));
            set(&mut app, key, json!([if select == "images" { 1 } else { 2 }]));
            crate::dialogs::confirm(&mut app, id).expect("OK");
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

#[test]
fn the_dialog_draws_its_controls() {
    let mut app = app();
    app.open_bytes("multi.pdf", &three_pages()).expect("open");
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 800.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::dialogs::show(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(6);
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
        assert!(h.query_by_label(label).is_some(), "missing {label}");
    }
    // The thumbnails were rendered.
    let st = h.state().pdf_import.first().expect("held");
    assert!(st.thumbs.iter().all(Option::is_some), "every page has its thumbnail");
}
