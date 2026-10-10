//! AVIF Export As uses the existing service seam, with visible controls and honest limits.
#![cfg(feature = "avif")]

use std::sync::{Arc, Mutex};

use egui::vec2;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_ui_egui::{ExportSettings, FileDialogAnswer, FileDialogRequest, PhotocraftApp, Services, export_dialog, i18n, theme::ThemeKind};
use serde_json::json;

const LIMITS: &str = "AVIF uses 8-bit sRGB. ICC, EXIF and XMP metadata are not preserved.";
const SIZE_NOTE: &str = "File size is shown after export.";

fn fixture(theme: ThemeKind, size: egui::Vec2, gpu: bool) -> Harness<'static, PhotocraftApp> {
    let builder = Harness::builder().with_step_dt(1.0 / 60.0).with_size(size);
    let builder = if gpu { builder.wgpu() } else { builder };
    let mut h = builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
        app.run("file.new", json!({"width":64,"height":48,"background":"transparent"})).unwrap();
        app.session.prefs.edit(|p| {
            p.interface.language = "en".into();
            p.export.jpeg_quality = 67;
        });
        let id = export_dialog::open(&mut app).unwrap();
        let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
        fields.insert("format".into(), json!("avif"));
        fields.insert("metadata".into(), json!("all"));
        app
    });
    h.run_steps(4);
    let ctx = h.ctx.clone();
    h.state_mut().set_theme(&ctx, theme);
    h.run_steps(4);
    h
}

#[test]
fn avif_controls_render_in_every_theme_and_fit_the_dialog() {
    let snapshots = std::env::var_os("PHOTOCRAFT_AVIF_SNAPSHOTS");
    for theme in ThemeKind::ALL {
        let name = theme.id();
        for size in [vec2(1024.0, 768.0), vec2(1440.0, 900.0)] {
            let mut h = fixture(theme, size, snapshots.is_some());
            for label in ["Quality", "Transparency", LIMITS, SIZE_NOTE, "Scale"] {
                let node = h.get_by_label(label);
                let rect = node.rect();
                assert!(rect.left() >= 0.0 && rect.right() <= size.x && rect.top() >= 0.0 && rect.bottom() <= size.y, "{name} {size:?}: {label} overflows");
            }
            assert!(h.get_by_value("None").accesskit_node().is_disabled(), "AVIF cannot preserve metadata");
            assert_eq!(h.state().ui.dialogs[0].fields["metadata"], "none");
            if let Some(dir) = &snapshots {
                let dir = std::path::Path::new(dir);
                std::fs::create_dir_all(dir).unwrap();
                h.render().unwrap().save(dir.join(format!("avif-{name}-{}x{}.png", size.x as u32, size.y as u32))).unwrap();
                if theme == ThemeKind::Pro && size == vec2(1440.0, 900.0) {
                    // Existing PNG controls for visual context, rendered by this same build.
                    // This is not a screenshot of a previous revision or a feature-disabled build.
                    h.state_mut().ui.dialogs[0].fields.insert("format".into(), json!("png"));
                    h.run_steps(4);
                    h.render().unwrap().save(dir.join("export-png-default-context-pro-1440x900.png")).unwrap();
                    h.state_mut().ui.dialogs[0].fields.insert("format".into(), json!("avif"));
                    h.run_steps(4);
                }
            }
            h.get_by_label("Transparency").click();
            h.run_steps(3);
            assert_eq!(h.state().ui.dialogs[0].fields["transparency"], false);
        }
    }
}

#[test]
fn avif_limits_and_size_note_are_localized_in_every_language() {
    let mut h = fixture(ThemeKind::Pro, vec2(1440.0, 900.0), false);
    for lang in i18n::Lang::all() {
        h.state_mut().session.prefs.edit(|p| p.interface.language = lang.code().into());
        h.run_steps(4);
        for label in ["Quality", "Transparency", LIMITS, SIZE_NOTE] {
            assert!(h.query_by_label(i18n::tr(lang, label)).is_some(), "{}: {label}", lang.code());
        }
    }
}

#[test]
fn confirming_avif_passes_quality_and_writes_the_selected_extension() {
    let encoded = Arc::new(Mutex::new(Vec::new()));
    let written = Arc::new(Mutex::new(Vec::new()));
    let receives_export = encoded.clone();
    let receives_write = written.clone();
    let services = Services {
        file_dialog: Some(Box::new(|request, _parent, reply| {
            let FileDialogRequest::Save { suggested } = request else { panic!("expected a Save dialog") };
            assert_eq!(suggested, "sample.avif");
            reply.send(Some(FileDialogAnswer::SaveTo(suggested)));
        })),
        export: Some(Box::new(move |doc, path, settings: &ExportSettings| {
            receives_export.lock().unwrap().push((path.to_string(), doc.size, settings.clone()));
            Ok((vec![1; 2400], vec!["lossy compression".into()]))
        })),
        write: Some(Box::new(move |path, bytes| {
            receives_write.lock().unwrap().push((path.to_string(), bytes.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"name":"sample","width":64,"height":48})).unwrap();
    let original = app.session.active().unwrap().doc.clone();
    let id = export_dialog::open(&mut app).unwrap();
    let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
    fields.insert("format".into(), json!("avif"));
    fields.insert("quality".into(), json!(43));
    fields.insert("scale".into(), json!(50));
    fields.insert("transparency".into(), json!(false));
    fields.insert("metadata".into(), json!("all"));
    let fields = fields.clone();
    let result = export_dialog::confirm(&mut app, &fields).unwrap();
    assert_eq!(result, json!({"fileDialog": "save"}));
    assert!(app.file_dialog_open());
    assert!(encoded.lock().unwrap().is_empty(), "encoding waits for the chosen save path");
    assert!(written.lock().unwrap().is_empty());
    app.poll_file_dialog(&egui::Context::default(), None);
    assert!(!app.file_dialog_open());
    let received = encoded.lock().unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].0, "sample.avif");
    assert_eq!(received[0].1, photocraft_doc::Size::new(32, 24));
    assert_eq!(received[0].2.jpeg_quality, Some(43));
    assert_eq!(received[0].2.webp_quality, None);
    assert!(!received[0].2.xmp_all);
    assert_eq!(written.lock().unwrap().as_slice(), &[("sample.avif".into(), vec![1; 2400])]);
    assert!(app.ui.status.contains("sample.avif"));
    assert!(app.ui.status.contains("(2K)"), "warnings preserve the exported file size in the status");
    let notice = app.ui.notices.last().expect("lossy export notice");
    assert_eq!(notice.lines, ["lossy compression"]);
    assert!(notice.title.contains("sample.avif"));
    assert!(notice.title.contains("(2K)"), "warnings preserve the exported file size in the notice title");
    assert!(Arc::ptr_eq(&original, &app.session.active().unwrap().doc), "export preserves the source document");
}
