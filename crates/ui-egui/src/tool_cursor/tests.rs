use super::*;
#[cfg(target_os = "windows")]
use egui::{Event, Modifiers, PointerButton, pos2};
#[cfg(target_os = "windows")]
use egui_kittest::Harness;
#[cfg(target_os = "windows")]
use serde_json::json;

fn alpha(image: &CustomCursorImage, x: u16, y: u16) -> u8 {
    image.rgba[(usize::from(y) * usize::from(image.size[0]) + usize::from(x)) * 4 + 3]
}

fn frame(ctx: &egui::Context, draw: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
    let mut output = ctx.run_ui(Default::default(), draw);
    // These tests inspect cursor output without a renderer.
    output.textures_delta.clear();
    output
}

#[test]
fn bitmap_has_a_transparent_hotspot_and_the_correct_radius_at_each_dpi() {
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        let radius = 20.0;
        let image = rasterize(Shape::Circle { radius, centre: false }, scale).unwrap();
        let [x, y] = image.hotspot;
        assert_eq!(image.size, [2 * x + 1, 2 * y + 1]);
        assert_eq!(image.rgba.len(), usize::from(image.size[0]).pow(2) * 4);
        assert_eq!(alpha(&image, x, y), 0, "the document is visible through the tip");
        assert_eq!(alpha(&image, 0, 0), 0);
        let edge = x + (radius * scale) as u16;
        assert!(alpha(&image, edge, y) > 230);
        assert_eq!(alpha(&image, x - (radius * scale) as u16, y), alpha(&image, edge, y));
        let edge_byte = (usize::from(y) * usize::from(image.size[0]) + usize::from(edge)) * 4;
        assert_eq!(image.rgba[edge_byte], 242, "straight RGBA: 235/255 white over 160/255 black (premultiplied would be 235)");
        assert!(image.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0 && p[0] == 0), "dark outline stays visible on white");
    }
}

#[test]
fn precise_and_tip_centre_crosshairs_have_distinct_hotspots() {
    let ring = rasterize(Shape::Circle { radius: 20.0, centre: true }, 1.0).unwrap();
    assert!(alpha(&ring, ring.hotspot[0], ring.hotspot[1]) > 230);
    let precise = rasterize(Shape::Crosshair { length: 8.0, gap: 2.0 }, 1.0).unwrap();
    let [x, y] = precise.hotspot;
    assert_eq!(alpha(&precise, x, y), 0);
    assert!(alpha(&precise, x + 5, y) > 230);
    let solid = rasterize(Shape::Crosshair { length: 6.0, gap: 0.0 }, 1.0).unwrap();
    assert!(alpha(&solid, solid.hotspot[0], solid.hotspot[1]) > 230);
}

#[test]
fn hostile_sizes_are_rejected_before_allocating() {
    for radius in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, f32::MAX, 2048.0] {
        assert!(rasterize(Shape::Circle { radius, centre: false }, 1.0).is_none());
    }
    for scale in [f32::NAN, f32::INFINITY, -1.0, 0.0, f32::MAX] {
        assert!(rasterize(Shape::Circle { radius: 20.0, centre: false }, scale).is_none());
    }
    for (length, gap) in [(1.0, 2.0), (8.0, -1.0), (f32::NAN, 0.0), (8.0, f32::NAN)] {
        assert!(rasterize(Shape::Crosshair { length, gap }, 1.0).is_none());
    }
}

#[test]
fn hand_bitmaps_are_outlined_hands_around_a_centred_hotspot_at_each_dpi() {
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        let open = rasterize(Shape::Hand { closed: false }, scale).unwrap();
        let fist = rasterize(Shape::Hand { closed: true }, scale).unwrap();
        for image in [&open, &fist] {
            let [x, y] = image.hotspot;
            assert_eq!(image.size, [2 * x + 1, 2 * y + 1]);
            assert_eq!(image.rgba.len(), usize::from(image.size[0]).pow(2) * 4);
            assert!(image.size[0] <= (34.0 * scale) as u16, "no bigger than a stock cursor at {scale}: {:?}", image.size);
            assert!(alpha(image, x, y) > 230, "the hotspot is on the hand, not in a hole");
            assert_eq!(alpha(image, 0, 0), 0, "nothing at the corners");
            let pixels = image.rgba.as_chunks::<4>().0;
            assert!(pixels.iter().any(|p| p[3] > 230 && p[0] > 230), "a white hand");
            assert!(pixels.iter().any(|p| p[3] > 100 && p[0] < 25), "with a black outline, readable on white");
        }
        assert_ne!(open.rgba, fist.rgba, "the fist is a different picture");
    }
}

#[test]
fn hand_bitmap_is_a_straight_rgba_silhouette_outlined_outside_the_hand() {
    let image = rasterize(Shape::Hand { closed: false }, 1.0).unwrap();
    let side = usize::from(image.size[0]);
    let px = |x: usize, y: usize| <[u8; 4]>::try_from(&image.rgba[(y * side + x) * 4..][..4]).unwrap();
    let mid = side / 2;
    // Walking left from the hotspot along the middle finger: white fill, then the dark outline,
    // then nothing, never the other way round.
    let mut seen = Vec::new();
    for x in (0..=mid).rev() {
        let [r, _, _, a] = px(x, mid - 4);
        let kind = if a < 20 {
            'n'
        } else if r > 200 {
            'w'
        } else {
            'd'
        };
        if seen.last() != Some(&kind) {
            seen.push(kind);
        }
    }
    assert_eq!(seen.first(), Some(&'w'));
    assert_eq!(seen.last(), Some(&'n'));
    let first_dark = seen.iter().position(|k| *k == 'd').expect("an outline");
    let first_none = seen.iter().position(|k| *k == 'n').unwrap();
    assert!(first_dark < first_none, "outline between fill and background: {seen:?}");
}

#[test]
fn hostile_scales_get_no_hand_bitmap() {
    for scale in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, f32::MAX, 500.0] {
        assert!(rasterize(Shape::Hand { closed: false }, scale).is_none(), "{scale}");
        assert!(rasterize(Shape::Hand { closed: true }, scale).is_none(), "{scale}");
    }
}

#[test]
fn zoom_and_pipette_bitmaps_are_readable_at_each_dpi_and_bound_allocations() {
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        let plus = rasterize(Shape::Zoom { out: false }, scale).unwrap();
        let minus = rasterize(Shape::Zoom { out: true }, scale).unwrap();
        assert_eq!((plus.size, plus.hotspot), (minus.size, minus.hotspot));
        assert_ne!(plus.rgba, minus.rgba);
        let [x, y] = plus.hotspot;
        let sign_y = y - (2.5 * scale).round() as u16;
        assert!(alpha(&plus, x, sign_y) > alpha(&minus, x, sign_y), "+ has a vertical arm at {scale}x");
        for image in [&plus, &minus, &rasterize(Shape::Pipette, scale).unwrap()] {
            assert!(image.rgba.as_chunks::<4>().0.iter().any(|p| p[0] > 230 && p[3] > 230));
            assert!(image.rgba.as_chunks::<4>().0.iter().any(|p| p[0] < 25 && p[3] > 100));
            assert_eq!(alpha(image, 0, 0), 0);
            assert!(alpha(image, image.hotspot[0], image.hotspot[1]) > 230);
        }
    }
    for shape in [Shape::Zoom { out: false }, Shape::Zoom { out: true }, Shape::Pipette] {
        for scale in [f32::NAN, f32::INFINITY, -1.0, 0.0, f32::MAX, 500.0] {
            assert!(rasterize(shape, scale).is_none());
        }
    }
}

#[test]
fn the_hand_is_a_bitmap_only_where_the_os_has_none() {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    for closed in [false, true] {
        let native = frame(&ctx, |ui| {
            let icon = hand_with(ui.ctx(), closed, false);
            ui.ctx().set_cursor_icon(icon);
        });
        assert_eq!(native.platform_output.cursor_icon, if closed { CursorIcon::Grabbing } else { CursorIcon::Grab });
        assert!(native.platform_output.cursor_image.is_none(), "macOS and the Linux themes draw their own hands");

        let bitmap = frame(&ctx, |ui| {
            let icon = hand_with(ui.ctx(), closed, true);
            ui.ctx().set_cursor_icon(icon);
        });
        let image = bitmap.platform_output.cursor_image.expect("Windows has no stock hand");
        assert_eq!(image.rgba, rasterize(Shape::Hand { closed }, 1.0).unwrap().rgba);
        assert_eq!(bitmap.platform_output.cursor_icon, CursorIcon::Crosshair, "visible fallback if the OS rejects a bitmap");
    }
    let open = hand_with(&ctx, false, true);
    let fist = hand_with(&ctx, true, true);
    assert_eq!((open, fist), (CursorIcon::None, CursorIcon::None));
}

#[test]
fn unchanged_geometry_reuses_the_os_upload_and_dpi_invalidates_it() {
    let ctx = egui::Context::default();
    let shape = Shape::Circle { radius: 20.0, centre: false };
    let a = cached_image(&ctx, shape, 1.0).unwrap();
    let b = cached_image(&ctx, shape, 1.0).unwrap();
    assert!(Arc::ptr_eq(&a.rgba, &b.rgba));
    let scaled = cached_image(&ctx, shape, 2.0).unwrap();
    assert!(!Arc::ptr_eq(&a.rgba, &scaled.rgba));
    assert!(scaled.size[0] > a.size[0]);
    let larger = cached_image(&ctx, Shape::Circle { radius: 40.0, centre: false }, 2.0).unwrap();
    assert!(larger.size[0] > scaled.size[0]);
}

#[test]
fn lifecycle_clears_sticky_images_and_respects_later_widgets() {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    let image = rasterize(Shape::Circle { radius: 20.0, centre: false }, 1.0).unwrap();
    let output = frame(&ctx, |ui| {
        ui.ctx().set_cursor_image(Some(image.clone()));
        ui.ctx().set_cursor_icon(CursorIcon::None);
    });
    assert!(output.platform_output.cursor_image.is_some());
    assert_eq!(output.platform_output.cursor_icon, CursorIcon::Crosshair, "visible fallback if the OS rejects a bitmap");
    let output = frame(&ctx, |_| {});
    assert!(output.platform_output.cursor_image.is_none(), "leaving the canvas cannot keep its bitmap");
    let output = frame(&ctx, |ui| {
        ui.ctx().set_cursor_image(Some(image.clone()));
        ui.ctx().set_cursor_icon(CursorIcon::None);
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    });
    assert!(output.platform_output.cursor_image.is_none());
    assert_eq!(output.platform_output.cursor_icon, CursorIcon::ResizeHorizontal);
}

#[test]
#[cfg(target_os = "windows")]
fn ordinary_brush_cursor_is_not_part_of_the_canvas_draw_list() {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    let output = frame(&ctx, |ui| {
        let icon = circle(ui.painter(), pos2(100.0, 100.0), 20.0, true);
        ui.ctx().set_cursor_icon(icon);
    });
    assert!(output.platform_output.cursor_image.is_some());
    assert!(output.shapes.iter().all(|s| matches!(s.shape, egui::Shape::Noop)));

    let output = frame(&ctx, |ui| {
        let icon = circle(ui.painter(), pos2(100.0, 100.0), 3000.0, false);
        ui.ctx().set_cursor_icon(icon);
    });
    let image = output.platform_output.cursor_image.unwrap();
    assert!(image.size[0] < 32, "oversized tips retain a small native hotspot");
    assert!(output.shapes.iter().any(|s| matches!(s.shape, egui::Shape::Circle(_))), "the full tip is not silently shrunk");
}

/// Run explicitly for visual QA: native cursors are absent from GPU screenshots.
#[test]
#[ignore = "writes target/tool-cursor-preview.png for visual inspection"]
fn write_cursor_preview() {
    let cases = [
        (Shape::Circle { radius: 20.0, centre: false }, 1.0),
        (Shape::Circle { radius: 20.0, centre: false }, 1.5),
        (Shape::Circle { radius: 20.0, centre: true }, 2.0),
        (Shape::Crosshair { length: 8.0, gap: 2.0 }, 2.0),
        (Shape::Crosshair { length: 6.0, gap: 0.0 }, 2.0),
        (Shape::Hand { closed: false }, 1.0),
        (Shape::Hand { closed: true }, 1.0),
        (Shape::Hand { closed: false }, 1.5),
        (Shape::Hand { closed: true }, 2.0),
        (Shape::Zoom { out: false }, 1.0),
        (Shape::Zoom { out: true }, 1.0),
        (Shape::Pipette, 1.0),
        (Shape::Zoom { out: false }, 2.0),
        (Shape::Zoom { out: true }, 2.0),
        (Shape::Pipette, 2.0),
    ];
    // The 2x pipette bitmap is 101 px high, including transparent padding.
    let row_height = 128;
    let (width, height) = (600_usize, cases.len() * row_height);
    let mut rgba = vec![255; width * height * 4];
    for (col, background) in [0_u8, 128, 255].into_iter().enumerate() {
        for y in 0..height {
            for x in col * 200..(col + 1) * 200 {
                rgba[(y * width + x) * 4..(y * width + x) * 4 + 3].fill(background);
            }
        }
        for (row, &(shape, scale)) in cases.iter().enumerate() {
            let image = rasterize(shape, scale).unwrap();
            let side = usize::from(image.size[0]);
            let x0 = col * 200 + 100 - usize::from(image.hotspot[0]);
            let y0 = row * row_height + row_height / 2 - usize::from(image.hotspot[1]);
            for (i, pixel) in image.rgba.as_chunks::<4>().0.iter().enumerate() {
                let at = ((y0 + i / side) * width + x0 + i % side) * 4;
                let a = f32::from(pixel[3]) / 255.0;
                let value = (f32::from(pixel[0]) * a + f32::from(background) * (1.0 - a)).round() as u8;
                rgba[at..at + 3].fill(value);
            }
        }
    }
    let image = photocraft_codecs::Image::from_u8(width as u32, height as u32, photocraft_codecs::ChannelLayout::Rgba, rgba).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tool-cursor-preview.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, png).unwrap();
}

#[cfg(target_os = "windows")]
fn harness() -> Harness<'static, crate::PhotocraftApp> {
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 40, "hardness": 1.0}})).unwrap();
    app.ui.tool = crate::Tool::Brush;
    app.sync_views();
    let mut h = Harness::builder().with_size(vec2(600.0, 500.0)).build_ui_state(
        |ui, app: &mut crate::PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::Panel::top("cursor-test-toolbar").show(ui, |ui| {
                ui.label("Toolbar");
            });
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            crate::dialogs::show(app, &ctx);
        },
        app,
    );
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    h.event(Event::PointerMoved(h.state().last_canvas_rect.center()));
    h.run_steps(3);
    h
}

#[test]
#[cfg(target_os = "windows")]
fn brush_hover_uses_an_os_bitmap_that_survives_motion_but_not_tool_or_panel_changes() {
    let mut h = harness();
    let initial = h.output().platform_output.cursor_image.clone().expect("brush hover must reach the OS, not the painter");
    let p = h.state().last_canvas_rect.center();
    h.event(Event::PointerMoved(p + vec2(25.0, 12.0)));
    h.run_steps(2);
    let moved = h.output().platform_output.cursor_image.clone().unwrap();
    assert!(Arc::ptr_eq(&initial.rgba, &moved.rgba), "moving does not rasterize or re-upload the cursor");

    h.event(Event::PointerMoved(pos2(20.0, 12.0)));
    h.run_steps(2);
    assert!(h.output().platform_output.cursor_image.is_none(), "toolbar restores the normal pointer");
    h.event(Event::PointerMoved(p));
    h.run_steps(2);
    assert!(h.output().platform_output.cursor_image.is_some());

    // Windows has no stock hand (winit maps `Grab` to the move cursor), so the Hand gets the open
    // hand as a bitmap too (#2196).
    h.state_mut().ui.tool = crate::Tool::Hand;
    h.run_steps(2);
    let hand = h.output().platform_output.cursor_image.clone().expect("the Hand's open hand reaches the OS as a bitmap");
    assert_eq!(hand.rgba, rasterize(Shape::Hand { closed: false }, h.ctx.pixels_per_point()).unwrap().rgba);

    h.state_mut().ui.tool = crate::Tool::Brush;
    h.state_mut().run("prefs.set", json!({"path": "cursors.painting", "value": "standard"})).unwrap();
    h.run_steps(2);
    assert!(h.output().platform_output.cursor_image.is_none());
    assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::Default);
}

#[test]
#[cfg(target_os = "windows")]
fn zoom_and_eyedropper_have_native_cursors_over_the_canvas() {
    let mut h = harness();
    for tool in [crate::Tool::Zoom, crate::Tool::Eyedropper] {
        h.state_mut().ui.tool = tool;
        h.run_steps(3);
        assert!(h.output().platform_output.cursor_image.is_some(), "{tool:?} must have an OS cursor image on Windows");
        let p = h.state().last_canvas_rect.center();
        h.event(Event::PointerMoved(pos2(20.0, 12.0)));
        h.run_steps(2);
        assert!(h.output().platform_output.cursor_image.is_none(), "{tool:?} must release the pointer outside the canvas");
        h.event(Event::PointerMoved(p));
        h.run_steps(2);
    }
}

#[test]
#[cfg(target_os = "windows")]
fn zoom_alt_changes_the_symbol_without_motion_and_matches_click_direction() {
    let mut h = harness();
    h.state_mut().ui.tool = crate::Tool::Zoom;
    h.run_steps(2);
    let p = h.state().last_canvas_rect.center();
    let plus = h.output().platform_output.cursor_image.clone().unwrap();
    for (mods, out) in [(Modifiers::ALT, true), (Modifiers::NONE, false)] {
        h.event(Event::ModifiersChanged(mods));
        h.run_steps(2);
        let image = h.output().platform_output.cursor_image.as_ref().unwrap();
        assert_eq!(image.rgba, rasterize(Shape::Zoom { out }, h.ctx.pixels_per_point()).unwrap().rgba);
        let z = h.state().current_zoom();
        let expected = crate::zoom_levels::step(z, if out { -1 } else { 1 }, h.state().ui.views[0].doc_size);
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: mods });
            h.step();
        }
        h.run_steps(2);
        assert_eq!(h.state().current_zoom(), expected);
    }
    assert_eq!(h.output().platform_output.cursor_image.as_ref().unwrap().rgba, plus.rgba);
    h.state_mut().ui.shell.sticky_alt = true;
    h.run_steps(2);
    assert_eq!(h.output().platform_output.cursor_image.as_ref().unwrap().rgba, rasterize(Shape::Zoom { out: true }, 1.0).unwrap().rgba);
    let z = h.state().current_zoom();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    assert!(h.state().current_zoom() < z);
}

#[test]
#[cfg(target_os = "windows")]
fn eyedropper_keeps_the_pipette_while_sampling_and_respects_precise_cursors() {
    let mut h = harness();
    h.state_mut().ui.tool = crate::Tool::Eyedropper;
    h.run_steps(2);
    let p = h.state().last_canvas_rect.center();
    let before = h.output().platform_output.cursor_image.clone().unwrap();
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerMoved(p + vec2(16.0, 0.0)));
    h.run_steps(2);
    let held = h.output().platform_output.cursor_image.as_ref().unwrap();
    assert!(Arc::ptr_eq(&before.rgba, &held.rgba), "rings accompany the native pipette instead of hiding it");
    h.event(Event::PointerButton { pos: p + vec2(16.0, 0.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.state().sampling_comparison.is_none());
    h.state_mut().run("prefs.set", json!({"path": "cursors.other", "value": "precise"})).unwrap();
    h.run_steps(2);
    let image = h.output().platform_output.cursor_image.as_ref().unwrap();
    assert_eq!(image.rgba, rasterize(Shape::Crosshair { length: 8.0, gap: 2.0 }, 1.0).unwrap().rgba);
}

#[test]
#[cfg(target_os = "windows")]
#[ignore = "writes synthetic comparison-ring screenshots for visual inspection"]
fn write_sampling_ring_preview() {
    use crate::control::{ControlRequest, Outcome, handle};
    use crate::theme::ThemeKind;
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plan/evidence/cursors");
    std::fs::create_dir_all(&output).unwrap();
    for (width, height) in [(1000.0, 750.0), (800.0, 600.0)] {
        for scale in [1.0, 2.0] {
            let mut h = Harness::builder().with_size(vec2(width, height)).with_pixels_per_point(scale).wgpu().build_eframe(|cc| {
                crate::PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::default());
                let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                app.run("file.new", json!({"width": 200, "height": 200, "background": "white"})).unwrap();
                app.run("shape.create", json!({"kind": "rect", "rect": [0, 0, 100, 200], "fill": "#ef5b45"})).unwrap();
                app.run("shape.create", json!({"kind": "rect", "rect": [100, 0, 100, 200], "fill": "#f0cc51"})).unwrap();
                app.ui.tool = crate::Tool::Eyedropper;
                app
            });
            h.run_steps(4);
            for theme in ThemeKind::ALL {
                let ctx = h.ctx.clone();
                let (request, _) = ControlRequest::new("ui.set", json!({"theme": theme.id()}));
                assert!(matches!(handle(h.state_mut(), &ctx, &request), Outcome::Done(v) if v["ok"] == true));
                h.state_mut().run("tools.setColors", json!({"foreground": [0.1, 0.35, 0.9, 1.0]})).unwrap();
                h.run_steps(3);
                let p = h.state().last_canvas_rect.center() - vec2(32.0, 0.0);
                h.event(Event::PointerMoved(p));
                h.step();
                h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
                h.step();
                h.event(Event::PointerMoved(p + vec2(8.0, 0.0)));
                h.run_steps(3);
                assert!(h.output().platform_output.cursor_image.is_some());
                let (_, old) = crate::canvas::eyedropper_ring_colors(h.state_mut(), 50.0, 50.0).unwrap();
                assert_eq!(old, egui::Color32::from_rgb(26, 89, 230));
                // Harness::render draws a synthetic arrow even when the OS cursor is a bitmap.
                // A viewport screenshot captures only the real UI; native cursor bitmaps have
                // their separate visual atlas above.
                h.ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
                h.step();
                h.step();
                let image = h
                    .ctx
                    .input(|i| i.events.iter().find_map(|e| if let Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }))
                    .expect("viewport screenshot delivered by the harness");
                let rgba = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                let image =
                    photocraft_codecs::Image::from_u8(image.size[0] as u32, image.size[1] as u32, photocraft_codecs::ChannelLayout::Rgba, rgba).unwrap();
                let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
                std::fs::write(output.join(format!("ring-{}-{width:.0}x{height:.0}-{scale:.0}x.png", theme.id())), png).unwrap();
                h.event(Event::PointerButton { pos: p + vec2(8.0, 0.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
                h.run_steps(2);
                assert!(h.state().sampling_comparison.is_none());
            }
        }
    }
}

#[test]
#[cfg(target_os = "windows")]
fn painting_preference_and_zoom_change_the_native_cursor_without_changing_pointer_input() {
    let mut h = harness();
    let image = h.output().platform_output.cursor_image.clone().unwrap();
    h.state_mut().ui.views[0].zoom *= 2.0;
    h.run_steps(2);
    assert!(h.output().platform_output.cursor_image.as_ref().unwrap().size[0] > image.size[0]);
    h.state_mut().run("prefs.set", json!({"path": "cursors.showOnlyCrosshairWhilePainting", "value": true})).unwrap();
    let pos = h.state().last_canvas_rect.center();
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerMoved(pos + vec2(20.0, 0.0)));
    h.run_steps(2);
    let cross_size = h.output().platform_output.cursor_image.as_ref().unwrap().size[0];
    assert!(cross_size < image.size[0]);
    assert_eq!(h.ctx.pointer_latest_pos(), Some(pos + vec2(20.0, 0.0)));
    h.event(Event::PointerButton { pos: pos + vec2(20.0, 0.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.output().platform_output.cursor_image.as_ref().unwrap().size[0] > cross_size);
}
