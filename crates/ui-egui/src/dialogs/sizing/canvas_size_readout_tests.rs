//! #14582: the Canvas Size dialog reports the flat image's size (width × height × channels
//! × bytes per channel, no alpha) with one decimal, not the layer-backed `__bytesPerPixel`.

use super::*;

#[test]
fn canvas_readout_matches_the_measured_flat_sizes() {
    for (w, h, want) in [
        (400.0, 300.0, "351.6K"),
        (1.0, 300.0, "900 bytes"),
        (133.0, 300.0, "116.9K"),
        (300000.0, 300.0, "257.5M"),
        (500.0, 400.0, "585.9K"),
        (567.0, 300.0, "498.3K"),
    ] {
        assert_eq!(canvas_size_text(w * h * 3.0), want, "{w}x{h} RGB 8");
    }
}

#[test]
fn flat_format_counts_channels_without_alpha() {
    use photocraft_color::{ColorMode, SampleType};
    for (mode, depth, want) in [
        (ColorMode::Rgb, SampleType::U8, (3, 1)),
        (ColorMode::Grayscale, SampleType::U8, (1, 1)),
        (ColorMode::Cmyk, SampleType::U8, (4, 1)),
        (ColorMode::Rgb, SampleType::U16, (3, 2)),
        (ColorMode::Rgb, SampleType::F32, (3, 4)),
    ] {
        let (channels, per_channel) = flat_format(mode, depth);
        assert_eq!((channels, per_channel), want, "{mode:?} {depth:?}");
        assert_eq!(channels * per_channel, want.0 * want.1, "{mode:?} {depth:?} bytes per pixel");
    }
}

#[test]
fn malformed_fields_fall_back_instead_of_nan() {
    let mut f = Map::new();
    assert_eq!(canvas_flat_bpp(&f), 3.0, "missing fields fall back to RGB 8");
    assert_eq!(canvas_size_text(400.0 * 300.0 * canvas_flat_bpp(&f)), "351.6K");
    for bad in [json!(f64::NAN), json!(f64::INFINITY), json!(0.0), json!(-2.0), json!("x"), json!(null)] {
        f.insert("__channels".into(), bad.clone());
        f.insert("__bytesPerChannel".into(), bad.clone());
        let bpp = canvas_flat_bpp(&f);
        assert!(bpp.is_finite() && bpp > 0.0, "{bad:?} falls back, got {bpp}");
        let text = canvas_size_text(400.0 * 300.0 * bpp);
        assert!(!text.contains("NaN") && !text.contains("inf"), "{bad:?} reads {text:?}");
    }
    // One bad field still falls back for that field only.
    f.insert("__channels".into(), json!(4.0));
    f.insert("__bytesPerChannel".into(), json!("x"));
    assert_eq!(canvas_flat_bpp(&f), 4.0);
}

#[test]
fn canvas_readout_uses_the_system_decimal_mark() {
    crate::i18n::with_decimal_separator(Some(','), || {
        assert_eq!(canvas_size_text(400.0 * 300.0 * 3.0), "351,6K");
        assert_eq!(canvas_size_text(500.0 * 400.0 * 3.0), "585,9K");
    });
    assert_eq!(canvas_size_text(400.0 * 300.0 * 3.0), "351.6K");
}

/// The open dialog on a 400x300 RGB 8 document shows the flat 351.6K, not the layer-backed 469K.
#[test]
fn canvas_size_dialog_shows_the_flat_size() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    open(h.state_mut(), "image.canvasSize").unwrap();
    h.run_steps(3);
    // Current Size and New Size both read 351.6K on an unchanged dialog.
    assert_eq!(h.get_all_by_label_contains("351.6K").count(), 2, "the dialog shows the flat 400x300 RGB 8 size");
    assert_eq!(h.get_all_by_label_contains("469K").count(), 0, "the layer-backed size is gone");
}
