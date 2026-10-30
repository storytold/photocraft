//! Cache regressions shared by every selected Info readout, including extra channels.

use super::*;
use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_color::SampleType;
use photocraft_doc::{AlphaChannel, Document, Layer, Size};
use photocraft_raster::{Surface, from_rgba};
use std::sync::Arc;

/// Three white pixels with increasing alpha and channels with the same increasing coverage.
fn document(mode: ColorMode, depth: SampleType) -> Document {
    let mut doc = Document::new("Cache fixture", Size::new(3, 1), mode, depth);
    let mut layer = Layer::raster("Pixels", doc.pixel_format());
    let surface = layer.surface_mut().unwrap();
    for (x, alpha) in [0.0, 0.5, 1.0].into_iter().enumerate() {
        surface.write_pixel(x as i32, 0, &from_rgba(&surface.format(), [1.0, 1.0, 1.0, alpha]));
    }
    doc.layers.push(layer);
    let gray = Document::new("Channel format", doc.size, ColorMode::Grayscale, depth).pixel_format();
    let mut surface = Surface::new(gray);
    for (x, value) in [0.0, 0.5, 1.0].into_iter().enumerate() {
        surface.write_pixel(x as i32, 0, &from_rgba(&surface.format(), [value, value, value, 1.0]));
    }
    let mut spot = AlphaChannel::new("Ink", surface.clone());
    spot.spot = Some((photocraft_color::Color::rgb(1.0, 0.0, 0.0), 1.0));
    doc.channels = vec![spot, AlphaChannel::new("Alpha", surface)];
    doc
}

fn app(doc: Document) -> PhotocraftApp {
    let mut session = photocraft_engine::Session::new();
    session.add_document(doc, None);
    PhotocraftApp::new(session, crate::Services::default())
}

/// Model an externally changed saved document: preserve its ID, but change its pixels/channels.
fn changed(doc: &Document) -> Document {
    let mut doc = doc.clone();
    let bounds = doc.bounds();
    let surface = doc.layers[0].surface_mut().unwrap();
    let black = from_rgba(&surface.format(), [0.0, 0.0, 0.0, 1.0]);
    surface.fill_rect(bounds, &black);
    for channel in &mut doc.channels {
        let white = from_rgba(&channel.surface.format(), [1.0, 1.0, 1.0, 1.0]);
        channel.surface.fill_rect(bounds, &white);
    }
    doc
}

fn reopen(app: &mut PhotocraftApp, doc: Document) {
    let old = app.session.active().unwrap();
    let (id, revision) = (old.doc.id, old.revision);
    app.session.close(0).unwrap();
    app.session.add_document(doc, None);
    assert_eq!(app.session.active().unwrap().doc.id, id, "the saved ID is retained");
    assert_eq!(app.session.active().unwrap().revision, revision, "the session starts at the same revision");
}

fn channel_percent(app: &mut PhotocraftApp, at: (i32, i32)) -> Vec<i32> {
    pointer_channels(app, at).iter().map(|c| c.percent).collect()
}

/// Matching revisions/coordinates across tabs must not reuse another document's colour/alpha.
#[test]
fn pointer_cache_follows_document_switches() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let doc = document(mode, depth);
            let mut app = app(doc.clone());
            let first = pointer_sample(&mut app, (1, 0)).unwrap();
            assert!((first[3] - 0.5).abs() < 0.01);
            app.session.add_document(changed(&doc), None);
            assert_eq!(app.session.documents()[0].revision, app.session.active().unwrap().revision);
            let second = pointer_sample(&mut app, (1, 0)).unwrap();
            assert_eq!(second[3], 1.0, "{mode:?}/{depth:?}: the new document's alpha");
            // Compare with an uncached read in the document's colour space: CMYK process black
            // need not convert to RGB zero through its profile.
            let fresh: [f32; 4] = serde_json::from_value(app.session.execute("document.sampleColor", json!({"x": 1.5, "y": 0.5, "size": 1})).unwrap()).unwrap();
            assert_eq!(second, fresh);
            assert!(app.session.set_active(0));
            assert_eq!(pointer_sample(&mut app, (1, 0)), Some(first));
        }
    }
}

/// Reopening can reuse the native file's ID/revision without any intervening UI frame.
#[test]
fn pointer_cache_follows_reopened_instances() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let doc = document(ColorMode::Rgb, depth);
        let mut app = app(doc.clone());
        assert!(pointer_sample(&mut app, (1, 0)).unwrap()[3] < 0.51);
        reopen(&mut app, changed(&doc));
        assert_eq!(pointer_sample(&mut app, (1, 0)), Some([0.0, 0.0, 0.0, 1.0]));
    }
}

/// A preserved document ID is also insufficient for the new spot/alpha-channel cache.
#[test]
fn channel_cache_follows_reopened_instances() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let doc = document(ColorMode::Rgb, depth);
        let mut app = app(doc.clone());
        assert_eq!(channel_percent(&mut app, (0, 0)), vec![0, 100]);
        reopen(&mut app, changed(&doc));
        assert_eq!(channel_percent(&mut app, (0, 0)), vec![100, 0]);
    }
}

/// An inactive tab can close before its saved ID is reopened; no intermediate read sees the switches.
#[test]
fn caches_follow_reopened_inactive_tabs_and_unobserved_switches() {
    let saved = document(ColorMode::Rgb, SampleType::U8);
    let mut app = app(saved.clone());
    assert!(pointer_sample(&mut app, (1, 0)).unwrap()[3] < 0.51);
    assert_eq!(channel_percent(&mut app, (0, 0)), vec![0, 100]);
    app.session.add_document(document(ColorMode::Rgb, SampleType::U8), None);
    app.session.close(0).unwrap();
    app.session.add_document(changed(&saved), None);
    assert_eq!(app.session.active().unwrap().doc.id, saved.id);
    assert!(app.session.set_active(0));
    assert!(app.session.set_active(1));
    assert_eq!(pointer_sample(&mut app, (1, 0)), Some([0.0, 0.0, 0.0, 1.0]));
    assert_eq!(channel_percent(&mut app, (0, 0)), vec![100, 0]);
}

/// Recreated sample points can reach the old revision while belonging to a new document instance.
#[test]
fn sampler_cache_follows_reopened_instances() {
    let doc = document(ColorMode::Rgb, SampleType::U8);
    let mut app = app(doc.clone());
    app.run("view.colorSamplers.add", json!({"x": 0, "y": 0})).unwrap();
    assert_eq!(sampler_readouts(&mut app)[0]["opacity"], 0);
    let revision = app.session.active().unwrap().revision;
    app.session.close(0).unwrap();
    app.session.add_document(changed(&doc), None);
    app.run("view.colorSamplers.add", json!({"x": 2, "y": 0})).unwrap();
    assert_eq!(app.session.active().unwrap().revision, revision, "the old cached revision");
    let readouts = sampler_readouts(&mut app);
    assert_eq!(readouts[0]["position"], json!([2.0, 0.0]));
    assert_eq!(readouts[0]["rgb"], json!([0, 0, 0]));
    assert_eq!(readouts[0]["opacity"], 100);
    assert_eq!(readouts[0]["channels"][0]["percent"], 100);
}

/// Stable frames reuse channel/point allocations; pixel, size and revision changes still refresh.
#[test]
fn caches_keep_hits_and_refresh_for_existing_inputs() {
    let mut app = app(document(ColorMode::Rgb, SampleType::U8));
    assert_eq!(pointer_sample(&mut app, (0, 0)).unwrap()[3], 0.0);
    assert!((pointer_sample(&mut app, (1, 0)).unwrap()[3] - 0.5).abs() < 0.01);
    app.ui.tool_options.eyedropper_size = 5;
    assert!((pointer_sample(&mut app, (0, 0)).unwrap()[3] - 0.5).abs() < 0.01);
    assert_eq!(channel_percent(&mut app, (0, 0)), vec![0, 100], "channels stay one-pixel samples");
    let channels = pointer_channels(&mut app, (0, 0));
    assert!(Arc::ptr_eq(&channels, &pointer_channels(&mut app, (0, 0))));
    assert_eq!(channel_percent(&mut app, (2, 0)), vec![100, 0]);
    app.run("view.colorSamplers.add", json!({"x": 0, "y": 0})).unwrap();
    let points = sampler_readouts(&mut app);
    assert!(Arc::ptr_eq(&points, &sampler_readouts(&mut app)));
    app.session
        .edit("Replace pixels", |doc, _| {
            *doc = changed(doc);
            Ok(())
        })
        .unwrap();
    assert_eq!(pointer_sample(&mut app, (0, 0)), Some([0.0, 0.0, 0.0, 1.0]));
    assert_eq!(channel_percent(&mut app, (0, 0)), vec![100, 0]);
    assert_eq!(sampler_readouts(&mut app)[0]["opacity"], 100);
    assert!(!Arc::ptr_eq(&points, &sampler_readouts(&mut app)));
}

/// Empty sessions never expose a previously cached value; switching tabs refreshes extra channels.
#[test]
fn caches_follow_channel_tabs_and_empty_sessions() {
    let doc = document(ColorMode::Rgb, SampleType::U8);
    let mut app = app(doc.clone());
    assert_eq!(channel_percent(&mut app, (0, 0)), vec![0, 100]);
    app.session.add_document(changed(&doc), None);
    assert_eq!(channel_percent(&mut app, (0, 0)), vec![100, 0]);
    assert!(app.session.set_active(0));
    assert_eq!(channel_percent(&mut app, (0, 0)), vec![0, 100]);
    app.session.close(1).unwrap();
    pointer_sample(&mut app, (1, 0)).unwrap();
    sampler_readouts(&mut app);
    app.session.close(0).unwrap();
    assert_eq!(pointer_sample(&mut app, (1, 0)), None);
    assert!(pointer_channels(&mut app, (0, 0)).is_empty());
    assert!(sampler_readouts(&mut app).is_empty());
}

/// Check the percentage paired with the requested label, rather than an unrelated colour row.
fn assert_row(h: &Harness<'_, PhotocraftApp>, label: &str, value: &str) {
    let label = h.get_by_label(label).rect();
    assert!(
        h.query_all_by_label(value).any(|n| {
            let rect = n.rect();
            rect.left() >= label.right() && (rect.center().y - label.center().y).abs() < 1.0
        }),
        "expected {value} beside {label:?}"
    );
}

/// Exercise the displayed values too; opt in to matching native-wgpu captures with this variable.
#[test]
fn cached_readouts_refresh_after_reopening() {
    let directory = std::env::var_os("PHOTOCRAFT_INFO_CACHE_SNAPSHOTS").map(std::path::PathBuf::from);
    if let Some(dir) = &directory {
        std::fs::create_dir_all(dir).unwrap();
    }
    let _gpu = directory.as_ref().map(|_| photocraft_testkit::gpu_lock());
    let doc = document(ColorMode::Rgb, SampleType::U8);
    let mut app = app(doc.clone());
    app.ui.info_panel.readouts = vec![Readout::Rgb, Readout::Opacity, Readout::SpotChannels, Readout::AlphaChannels];
    app.hover_doc = Some([0.5, 0.5]);
    let builder = Harness::builder().with_size(vec2(340.0, 320.0)).with_pixels_per_point(2.0);
    let builder = if directory.is_some() { builder.wgpu() } else { builder };
    let mut h = builder.build_ui_state(|ui, app| info_panel(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.run_steps(3);
    assert_row(&h, "Composite opacity:", "0%");
    assert_row(&h, "Ink:", "0%");
    assert_row(&h, "Alpha:", "100%");
    if let Some(dir) = &directory {
        h.render().unwrap().save(dir.join("initial.png")).unwrap();
    }
    reopen(h.state_mut(), changed(&doc));
    h.run_steps(3);
    if let Some(dir) = &directory {
        h.render().unwrap().save(dir.join("reopened.png")).unwrap();
    }
    assert_row(&h, "Composite opacity:", "100%");
    assert_row(&h, "Ink:", "100%");
    assert_row(&h, "Alpha:", "0%");
    assert_row(&h, "R:", "0");
}
