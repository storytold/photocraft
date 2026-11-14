use super::*;
use crate::{Services, color_picker_ui};
use photocraft_geom::Rect as DocRect;

const INKS: [[f32; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
const MIXED_INKS: [[f32; 4]; 2] = [[0.95, 0.10, 0.0, 0.0], [0.95, 0.20, 0.0, 0.0]];

fn chart(depth: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 32, "height": 8, "mode": "cmyk", "depth": depth, "background": "white"})).unwrap();
    app.session
        .edit("CMYK chart", |doc, _| {
            for (i, ink) in INKS.iter().enumerate() {
                doc.layers[0].surface_mut().unwrap().fill_rect(DocRect::new(i as i32 * 8, 0, (i as i32 + 1) * 8, 8), &[ink[0], ink[1], ink[2], ink[3], 1.0]);
            }
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::Eyedropper;
    app
}

fn mixed_chart(depth: u32) -> PhotocraftApp {
    let mut app = chart(depth);
    app.session
        .edit("Mixed CMYK chart", |doc, _| {
            for (i, ink) in MIXED_INKS.iter().enumerate() {
                doc.layers[0]
                    .surface_mut()
                    .unwrap()
                    .fill_rect(DocRect::new((i * 8) as i32, 0, ((i + 1) * 8) as i32, 8), &[ink[0], ink[1], ink[2], ink[3], 1.0]);
            }
            Ok(())
        })
        .unwrap();
    app
}

#[test]
fn eyedropper_cmyk_mixed_inks_preserve_quantized_channels() {
    for depth in [8, 16] {
        let mut app = mixed_chart(depth);
        let tolerance = if depth == 8 { 1.0 / 255.0 + 1e-6 } else { 1.0 / 65535.0 + 1e-6 };
        let mut previous = None;

        for (i, nominal) in MIXED_INKS.iter().enumerate() {
            let x = (i * 8 + 3) as i32;
            let stored = app.session.active().unwrap().doc.layers[0].surface().unwrap().pixel(x, 3);

            for ch in 0..4 {
                assert!((stored[ch] - nominal[ch]).abs() <= tolerance, "depth {depth}, sample {i}, channel {ch}: stored {:?}, expected {:?}", stored, nominal);
            }

            sample_eyedropper(&mut app, x as f64, 3.0, egui::Modifiers::NONE);
            let sampled = app.session.tools.picked_role_cmyk(0).unwrap();

            for ch in 0..4 {
                assert!((sampled[ch] - stored[ch]).abs() <= 1e-6, "depth {depth}, sample {i}, channel {ch}: sampled {:?}, stored {:?}", sampled, stored);
            }

            let id = color_picker_ui::open(&mut app, "foreground");
            let reopened = color_picker_ui::current(&app.ui.dialog_mut(id).unwrap().fields).cmyk;
            for ch in 0..4 {
                assert!((reopened[ch] - sampled[ch]).abs() <= 1e-6);
            }
            app.ui.close_dialog(id);

            if let Some(prev) = previous {
                assert_ne!(sampled, prev, "distinct mixed CMYK inks must remain distinct");
            }
            previous = Some(sampled);
        }
    }
}

#[test]
fn eyedropper_cmyk_foreground_and_background_reopen_with_exact_inks() {
    for depth in [8, 16] {
        let mut app = chart(depth);
        let revision = app.session.active().unwrap().revision;
        for (i, ink) in INKS.iter().enumerate() {
            for (target, mods) in [("foreground", egui::Modifiers::NONE), ("background", egui::Modifiers::ALT)] {
                sample_eyedropper(&mut app, (i * 8 + 3) as f64, 3.0, mods);
                let role = if target == "foreground" { 0 } else { 1 };
                assert_eq!(app.session.tools.picked_role_cmyk(role), Some(*ink));
                let id = color_picker_ui::open(&mut app, target);
                assert_eq!(color_picker_ui::current(&app.ui.dialog_mut(id).unwrap().fields).cmyk, *ink, "{target}, depth {depth}, ink {ink:?}");
                app.ui.close_dialog(id);
            }
        }
        assert_eq!(app.session.active().unwrap().revision, revision, "sampling must not edit the document");
    }
}

#[test]
fn eyedropper_cmyk_pick_fills_with_the_original_inks() {
    for (ink_index, ink) in INKS.iter().enumerate() {
        let mut app = chart(8);
        sample_eyedropper(&mut app, (ink_index * 8 + 3) as f64, 3.0, egui::Modifiers::NONE);
        app.run("edit.fillForeground", json!({})).unwrap();
        let st = app.session.active().unwrap();
        let pixel = st.doc.layers[0].surface().unwrap().pixel(1, 1);
        assert_eq!(pixel.get(..4), Some(ink.as_slice()));
    }
}

#[test]
fn eyedropper_cmyk_sampling_respects_current_layer_and_transparency() {
    let mut app = chart(8);
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool_options.eyedropper_sample = "current".into();
    let fg = app.session.tools.foreground;
    sample_eyedropper(&mut app, 3.0, 3.0, egui::Modifiers::NONE);
    assert_eq!(app.session.tools.foreground, fg, "an empty current layer picks nothing");
    app.ui.tool_options.eyedropper_sample = "all".into();
    sample_eyedropper(&mut app, 3.0, 3.0, egui::Modifiers::NONE);
    assert_eq!(app.session.tools.picked_role_cmyk(0), Some(INKS[0]));
}

#[test]
fn eyedropper_cmyk_mask_keeps_grayscale_sampling() {
    let mut app = chart(8);
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    app.ui.tool_options.eyedropper_sample = "current".into();
    sample_eyedropper(&mut app, 3.0, 3.0, egui::Modifiers::NONE);
    assert_eq!(app.session.tools.foreground, [1.0; 4]);
}

#[test]
fn eyedropper_cmyk_dialog_sampling_confirms_original_inks_and_reset_restores_opening_color() {
    for (ink_index, ink) in INKS.iter().enumerate() {
        let mut app = chart(16);
        let original = app.session.tools.foreground;
        let id = color_picker_ui::open(&mut app, "foreground");
        color_picker_ui::sample_at(&mut app, (ink_index * 8 + 3) as f64, 3.0);
        let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        assert_eq!(color_picker_ui::current(&fields).cmyk, *ink);
        assert_eq!(app.session.tools.foreground, original, "sampling inside a dialog does not commit the colour");
        color_picker_ui::reset(&mut app.ui.dialog_mut(id).unwrap().fields);
        assert_eq!(app.ui.dialog_mut(id).unwrap().fields["color"], color_picker_ui::hex([original[0], original[1], original[2]]));
        color_picker_ui::confirm(&mut app, &fields).unwrap();
        assert_eq!(app.session.tools.picked_role_cmyk(0), Some(*ink));
        app.ui.close_dialog(id);
        let id = color_picker_ui::open(&mut app, "foreground");
        assert_eq!(color_picker_ui::current(&app.ui.dialog_mut(id).unwrap().fields).cmyk, *ink);
    }
}

#[test]
fn eyedropper_cmyk_dialog_web_only_still_snaps_and_drops_native_inks() {
    let mut app = mixed_chart(8);
    let original_rgb = crate::canvas::sampled_cmyk_color(&app, 3.0, 3.0, "all").unwrap().0;
    let original_hex = color_picker_ui::hex(original_rgb);
    app.ui.color_panel.web_only = true;
    let id = color_picker_ui::open(&mut app, "foreground");
    color_picker_ui::sample_at(&mut app, 3.0, 3.0);
    let fields = &app.ui.dialog_mut(id).unwrap().fields;
    assert_ne!(fields["color"], json!(original_hex), "web-only must actually snap the sampled color");
    assert!(fields.get("__cmyk").is_none());
    assert!(color_picker_ui::current(fields).rgb.iter().all(|v| (v * 5.0 - (v * 5.0).round()).abs() < 1e-6));
}

#[test]
fn eyedropper_cmyk_mixed_3x3_ring_matches_picked_foreground() {
    for depth in [8, 16] {
        let mut app = mixed_chart(depth);
        app.ui.tool_options.eyedropper_size = 3;

        // The sample crosses the boundary between two CMYK colors.
        let (x, y) = (7.0, 3.0);
        let (ring_sample, _) = crate::canvas::eyedropper_ring_colors(&mut app, x, y).expect("mixed CMYK sample should be available");

        sample_eyedropper(&mut app, x, y, egui::Modifiers::NONE);

        let foreground = app.session.tools.foreground;
        let to_byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let picked = egui::Color32::from_rgb(to_byte(foreground[0]), to_byte(foreground[1]), to_byte(foreground[2]));

        assert_eq!(ring_sample, picked, "depth {depth}: ring must match the picked foreground");
        assert!(app.session.tools.picked_role_cmyk(0).is_some(), "depth {depth}: native CMYK inks must be preserved");
    }
}
