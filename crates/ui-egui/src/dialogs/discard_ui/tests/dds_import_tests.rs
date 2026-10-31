//! DDS Import on File › Open (#8632): the prompt a texture's alpha channel and mip chain
//! park behind, the answers it opens with, and the scripted `ddsAlphaChannel`/`ddsMipmaps` that
//! skip it.
use super::*;
use crate::dds_import_ui;
use egui_kittest::kittest::Queryable;
use photocraft_io::DdsImport;
use std::{cell::RefCell, rc::Rc};

/// A 16×8 RGBA DDS with a mip chain and an alpha channel: the conditional questions to ask.
fn dds_bytes() -> Vec<u8> {
    let img = photocraft_codecs::Image::from_u8(16, 8, photocraft_codecs::ChannelLayout::Rgba, [10, 20, 30, 200].repeat(16 * 8)).unwrap();
    photocraft_codecs::encode(
        &img,
        photocraft_codecs::Format::Dds,
        &photocraft_codecs::EncodeOptions {
            dds: photocraft_codecs::DdsOptions { compression: photocraft_codecs::DdsCompression::None, mipmaps: true },
            ..Default::default()
        },
    )
    .unwrap()
}

/// An app that opens synchronously, whose importer records the DDS answers it was asked with.
fn app_importing() -> (Prompted, Rc<RefCell<Option<DdsImport>>>) {
    let mut app = app_with_docs(0);
    app.background_jobs = false;
    let chosen = Rc::new(RefCell::new(None));
    let seen = chosen.clone();
    app.services.import = Some(Box::new(move |name: &str, _b: &[u8], opts: &photocraft_io::ImportOptions| {
        *seen.borrow_mut() = Some(opts.dds);
        Ok((
            photocraft_doc::Document::new(name, photocraft_geom::Size::new(16, 8), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8),
            Vec::new(),
        ))
    }));
    let h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(
        |ui, app| {
            show(app, ui.ctx());
            dds_import_ui::show(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    (h, chosen)
}

/// Opening a mip-mapped DDS asks first; OK opens with what was answered, nothing before.
#[test]
fn opening_a_dds_texture_asks_about_alpha_and_mipmaps() {
    let (mut h, chosen) = app_importing();
    h.state_mut().open_file("textures/skin.dds", &dds_bytes()).unwrap();
    h.run_steps(2);
    let p = h.state().dds_imports.front().expect("parked");
    assert_eq!(p.name, "skin.dds");
    assert_eq!((p.info.width, p.info.height), (16, 8));
    assert!(p.info.levels > 1 && p.info.alpha);
    assert!(chosen.borrow().is_none(), "nothing is decoded before the answer");
    assert!(h.state_mut().open_file("textures/other.dds", &dds_bytes()).is_ok(), "later opens queue behind");

    // The first click on a just-opened modal only brings its layer to the front.
    h.get_by_label("DDS Import").click();
    h.run_steps(2);
    h.get_by_label("Mipmaps as separate layers").click();
    h.run_steps(2);
    h.get_by_label("Load transparency as alpha channel").click();
    h.run_steps(2);
    assert!(h.state().dds_imports.front().unwrap().options.alpha_as_channel, "alpha as a channel");
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(*chosen.borrow(), Some(DdsImport { alpha_as_channel: true, mips_as_layers: true, ..Default::default() }));
    assert!(h.state().dds_imports.front().is_some(), "the queued open asks next");
}

/// Cancel drops the open; the importer never runs.
#[test]
fn cancelling_the_dds_prompt_drops_the_open() {
    let (mut h, chosen) = app_importing();
    h.state_mut().open_file("t.dds", &dds_bytes()).unwrap();
    h.run_steps(2);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().dds_imports.is_empty());
    assert!(chosen.borrow().is_none());
    assert_eq!(h.state().ui.status, "Open cancelled");
}

#[derive(Debug)]
struct FakeDrop(std::path::PathBuf, Result<Vec<u8>, String>);

impl egui::DroppedFile for FakeDrop {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        self.1.clone()
    }
}

/// A DDS dropped on a tab slot keeps it through the prompt: the answered file opens there and
/// files dropped with it take the slots after.
#[test]
fn a_dds_dropped_on_a_tab_slot_opens_there_once_answered() {
    let (mut h, _chosen) = app_importing();
    let ctx = egui::Context::default();
    let app = h.state_mut();
    app.open_bytes("canvas.psd", b"x").unwrap();
    // One tab at (100, 20)–(200, 46): its left half is slot 0.
    let tab = egui::Rect::from_min_max(egui::pos2(100.0, 20.0), egui::pos2(200.0, 46.0));
    let strip = egui::Rect::from_min_max(egui::pos2(92.0, 14.0), egui::pos2(700.0, 50.0));
    app.tab_strip = Some(crate::canvas::TabStrip { rect: strip, tabs: vec![(0, tab)] });
    let dropped = |name: &str, bytes: Vec<u8>| -> egui::DroppedFileHandle { std::sync::Arc::new(FakeDrop(name.into(), Ok(bytes))) };
    app.open_dropped(&ctx, vec![dropped("mip.dds", dds_bytes()), dropped("second.png", b"x".to_vec())], Some(egui::pos2(120.0, 30.0)));
    let names = |app: &PhotocraftApp| app.session.documents().iter().map(|d| d.doc.name.clone()).collect::<Vec<_>>();
    assert_eq!(app.dds_imports.front().and_then(|p| p.slot), Some(0), "the parked open keeps the drop's slot");
    assert_eq!(names(app), ["canvas.psd", "second.png"], "the next file took the slot after");
    h.run_steps(2);
    h.get_by_label("DDS Import").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(names(h.state()), ["mip.dds", "canvas.psd", "second.png"], "the answered file moved to its slot");
}

/// A DDS open parked behind the prompt counts as open: the same path asked for again doesn't
/// queue a second prompt that would open a second copy once answered.
#[test]
fn a_parked_dds_open_counts_as_already_open() {
    let (mut h, _chosen) = app_importing();
    let dir = std::env::temp_dir().join(format!("photocraft-dds-parked-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("mip.dds");
    std::fs::write(&file, dds_bytes()).unwrap();
    let path = file.to_string_lossy().to_string();
    let app = h.state_mut();
    app.open_path(&path).unwrap();
    app.open_path(&path).unwrap();
    assert_eq!(app.dds_imports.len(), 1, "the repeat found the parked open");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A DDS parked behind the prompt keeps its place in a set opened together: the file after it
/// opens at once, and the answered DDS lands before it — in name order — not at the end.
#[test]
fn a_parked_dds_keeps_its_place_in_an_open_together_set() {
    let (mut h, _chosen) = app_importing();
    let dir = std::env::temp_dir().join(format!("photocraft-dds-order-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dds = dir.join("a.dds");
    let png = dir.join("b.png");
    std::fs::write(&dds, dds_bytes()).unwrap();
    std::fs::write(&png, b"x").unwrap();
    let paths = [dds.to_string_lossy().to_string(), png.to_string_lossy().to_string()];
    let app = h.state_mut();
    app.open_paths(&paths);
    let names = |app: &PhotocraftApp| app.session.documents().iter().map(|d| d.doc.name.clone()).collect::<Vec<_>>();
    assert_eq!(app.dds_imports.front().and_then(|p| p.order.map(|o| o.seq)), Some(0), "the parked member holds its place");
    assert_eq!(names(app), ["b.png"], "the member after it opened at once");
    h.run_steps(2);
    h.get_by_label("DDS Import").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(names(h.state()), ["a.dds", "b.png"], "the answered file moved to its place in the set");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `ddsAlphaChannel`/`ddsMipmaps` on `file.open`/`file.openAs` answer without asking; a texture with
/// no mip chain and no alpha never asks.
#[test]
fn scripted_dds_import_answers_and_plain_textures_skip_the_prompt() {
    let (mut h, chosen) = app_importing();
    h.state_mut().open_file_opts("t.dds", &dds_bytes(), Some(DdsImport { alpha_as_channel: true, mips_as_layers: true, ..Default::default() })).unwrap();
    assert!(h.state().dds_imports.is_empty(), "answered opens don't ask");
    assert_eq!(*chosen.borrow(), Some(DdsImport { alpha_as_channel: true, mips_as_layers: true, ..Default::default() }));

    // Params parse to the same answers, and bad values are refused.
    assert_eq!(
        dds_import_ui::params(&json!({"ddsAlphaChannel": true, "ddsMipmaps": true})).unwrap(),
        Some(DdsImport { alpha_as_channel: true, mips_as_layers: true, ..Default::default() })
    );
    assert!(dds_import_ui::params(&json!({})).unwrap().is_none());
    assert!(dds_import_ui::params(&json!({"ddsAlphaChannel": 1})).is_err());
    assert!(dds_import_ui::params(&json!({"ddsMipmaps": "yes"})).is_err());

    // A single-level opaque texture has no choices to make.
    let img = photocraft_codecs::Image::from_u8(4, 4, photocraft_codecs::ChannelLayout::Gray, [128].repeat(16)).unwrap();
    let flat = photocraft_codecs::encode(&img, photocraft_codecs::Format::Dds, &photocraft_codecs::EncodeOptions::default()).unwrap();
    h.state_mut().open_file("flat.dds", &flat).unwrap();
    assert!(h.state().dds_imports.is_empty(), "nothing to ask");
}
