use super::*;
#[cfg(not(target_arch = "wasm32"))]
use egui::{Event, Modifiers, PointerButton, pos2};
#[cfg(not(target_arch = "wasm32"))]
use egui_kittest::Harness;
#[cfg(not(target_arch = "wasm32"))]
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
        let image = rasterize(&Shape::Circle { radius, centre: false }, scale).unwrap();
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
    let ring = rasterize(&Shape::Circle { radius: 20.0, centre: true }, 1.0).unwrap();
    assert!(alpha(&ring, ring.hotspot[0], ring.hotspot[1]) > 230);
    let precise = rasterize(&Shape::Crosshair { length: 8.0, gap: 2.0 }, 1.0).unwrap();
    let [x, y] = precise.hotspot;
    assert_eq!(alpha(&precise, x, y), 0);
    assert!(alpha(&precise, x + 5, y) > 230);
    let solid = rasterize(&Shape::Crosshair { length: 6.0, gap: 0.0 }, 1.0).unwrap();
    assert!(alpha(&solid, solid.hotspot[0], solid.hotspot[1]) > 230);
}

/// On Linux a tip stays within what X servers and hardware cursor planes take (256 px); a larger
/// one falls back to the painted outline. Windows and macOS take far larger cursors.
#[test]
fn cursor_bitmaps_stay_within_the_platform_limit() {
    let fits = |radius: f32| rasterize(&Shape::Circle { radius, centre: false }, 1.0).map(|i| i.size[0]);
    let large = cfg!(any(target_os = "windows", target_os = "macos"));
    assert!(fits(100.0).is_some_and(|side| side <= 255));
    assert_eq!(fits(200.0).is_some(), large);
    assert_eq!(fits(1000.0).is_some(), large, "a 2000 px brush at 100 % still has a native outline");
    assert!(fits(200.0).is_none_or(|side| side <= 2048));
    assert!(fits(1100.0).is_none(), "nothing past 2048 px");
}

#[test]
fn hostile_sizes_are_rejected_before_allocating() {
    for radius in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, f32::MAX, 2048.0] {
        assert!(rasterize(&Shape::Circle { radius, centre: false }, 1.0).is_none());
    }
    for scale in [f32::NAN, f32::INFINITY, -1.0, 0.0, f32::MAX] {
        assert!(rasterize(&Shape::Circle { radius: 20.0, centre: false }, scale).is_none());
    }
    for (length, gap) in [(1.0, 2.0), (8.0, -1.0), (f32::NAN, 0.0), (8.0, f32::NAN)] {
        assert!(rasterize(&Shape::Crosshair { length, gap }, 1.0).is_none());
    }
}

#[test]
fn hand_bitmaps_are_outlined_hands_around_a_centred_hotspot_at_each_dpi() {
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        let open = rasterize(&Shape::Hand { closed: false }, scale).unwrap();
        let fist = rasterize(&Shape::Hand { closed: true }, scale).unwrap();
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
    let image = rasterize(&Shape::Hand { closed: false }, 1.0).unwrap();
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
        assert!(rasterize(&Shape::Hand { closed: false }, scale).is_none(), "{scale}");
        assert!(rasterize(&Shape::Hand { closed: true }, scale).is_none(), "{scale}");
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
        assert_eq!(image.rgba, rasterize(&Shape::Hand { closed }, 1.0).unwrap().rgba);
        assert_eq!(bitmap.platform_output.cursor_icon, CursorIcon::Crosshair, "visible fallback if the OS rejects a bitmap");
    }
    let open = hand_with(&ctx, false, true);
    let fist = hand_with(&ctx, true, true);
    assert_eq!((open, fist), (CursorIcon::None, CursorIcon::None));
}

#[test]
fn zoom_bitmaps_are_outlined_magnifiers_centred_on_the_lens_at_each_dpi() {
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        let plus = rasterize(&Shape::Zoom { out: false }, scale).unwrap();
        let minus = rasterize(&Shape::Zoom { out: true }, scale).unwrap();
        for image in [&plus, &minus] {
            let [x, y] = image.hotspot;
            assert_eq!(image.size, [2 * x + 1, 2 * y + 1]);
            assert_eq!(image.rgba.len(), usize::from(image.size[0]).pow(2) * 4);
            assert!(image.size[0] <= (34.0 * scale) as u16, "no bigger than a stock cursor at {scale}: {:?}", image.size);
            assert_eq!(alpha(image, 0, 0), 0, "nothing at the corners");
            let handle = (9.0 * scale) as u16;
            assert!(alpha(image, x + handle, y + handle) > 230, "the handle reaches down and right");
            assert_eq!(alpha(image, x - handle, y - handle), 0, "and only that way");
            let pixels = image.rgba.as_chunks::<4>().0;
            assert!(pixels.iter().any(|p| p[3] > 230 && p[0] > 230), "a white lens");
            assert!(pixels.iter().any(|p| p[3] > 100 && p[0] < 25), "with a black outline, readable on white");
        }
        let side = usize::from(plus.size[0]);
        let red = |image: &CustomCursorImage, x: u16, y: u16| image.rgba[(usize::from(y) * side + usize::from(x)) * 4];
        let [x, y] = plus.hotspot;
        let up = y - (2.5 * scale) as u16;
        assert!(red(&plus, x, y) < 60 && red(&minus, x, y) < 60, "both glyphs cross the hotspot");
        assert!(red(&plus, x, up) < 60, "the plus has a vertical arm");
        assert!(red(&minus, x, up) > 200, "the minus does not");
    }
}

#[test]
fn hostile_scales_get_no_zoom_bitmap() {
    for scale in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, f32::MAX, 500.0] {
        assert!(rasterize(&Shape::Zoom { out: false }, scale).is_none(), "{scale}");
        assert!(rasterize(&Shape::Zoom { out: true }, scale).is_none(), "{scale}");
    }
}

/// winit shows the plain arrow for `ZoomIn` and `ZoomOut` on Windows (#3735).
#[test]
fn the_magnifier_is_a_bitmap_only_where_the_os_has_none() {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    for out in [false, true] {
        let native = frame(&ctx, |ui| {
            let icon = zoom_with(ui.ctx(), out, false);
            ui.ctx().set_cursor_icon(icon);
        });
        assert_eq!(native.platform_output.cursor_icon, if out { CursorIcon::ZoomOut } else { CursorIcon::ZoomIn });
        assert!(native.platform_output.cursor_image.is_none(), "macOS and the Linux themes draw their own magnifiers");

        let bitmap = frame(&ctx, |ui| {
            let icon = zoom_with(ui.ctx(), out, true);
            ui.ctx().set_cursor_icon(icon);
        });
        let image = bitmap.platform_output.cursor_image.expect("Windows has no stock magnifier");
        assert_eq!(image.rgba, rasterize(&Shape::Zoom { out }, 1.0).unwrap().rgba);
        assert_eq!(bitmap.platform_output.cursor_icon, CursorIcon::Crosshair, "visible fallback if the OS rejects a bitmap");
    }
}

#[test]
fn unchanged_geometry_reuses_the_os_upload_and_dpi_invalidates_it() {
    let ctx = egui::Context::default();
    let shape = Shape::Circle { radius: 20.0, centre: false };
    let a = cached_image(&ctx, &shape, 1.0).unwrap();
    let b = cached_image(&ctx, &shape, 1.0).unwrap();
    assert!(Arc::ptr_eq(&a.rgba, &b.rgba));
    let scaled = cached_image(&ctx, &shape, 2.0).unwrap();
    assert!(!Arc::ptr_eq(&a.rgba, &scaled.rgba));
    assert!(scaled.size[0] > a.size[0]);
    let larger = cached_image(&ctx, &Shape::Circle { radius: 40.0, centre: false }, 2.0).unwrap();
    assert!(larger.size[0] > scaled.size[0]);
}

#[test]
fn lifecycle_clears_sticky_images_and_respects_later_widgets() {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    let image = rasterize(&Shape::Circle { radius: 20.0, centre: false }, 1.0).unwrap();
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
#[cfg(not(target_arch = "wasm32"))]
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

#[test]
#[cfg(target_os = "linux")]
fn scaled_wayland_paints_the_brush_outline_instead_of_an_os_bitmap() {
    // Hyprland ignores the viewport winit sizes cursor bitmaps with: at 1.25 the outline was
    // 1.25× too large and its centre 0.2 × its half-width right of and below where the brush
    // painted. Off scale 1 on Wayland the outline is painted at the pointer, at its true size.
    let run = |ctx: &egui::Context, native_ppp: f32| {
        let mut input = egui::RawInput::default();
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(native_ppp);
        let mut output = ctx.run_ui(input, |ui| {
            let icon = circle(ui.painter(), pos2(100.0, 100.0), 20.0, false);
            ui.ctx().set_cursor_icon(icon);
        });
        output.textures_delta.clear();
        let circles: Vec<_> =
            output.shapes.iter().filter_map(|s| if let egui::Shape::Circle(c) = &s.shape { Some((c.center, c.radius)) } else { None }).collect();
        (output.platform_output.cursor_image.is_some(), output.platform_output.cursor_icon, circles)
    };
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    set_wayland(&ctx, true);
    for ppp in [1.25, 1.5, 2.0] {
        let (bitmap, icon, circles) = run(&ctx, ppp);
        assert!(!bitmap, "{ppp}: no OS bitmap");
        assert_eq!(icon, CursorIcon::None, "{ppp}: the painted outline is the pointer");
        assert!(!circles.is_empty() && circles.iter().all(|&(c, r)| c == pos2(100.0, 100.0) && r == 20.0), "{ppp}: {circles:?}");
    }
    let (bitmap, _, circles) = run(&ctx, 1.0);
    assert!(bitmap && circles.is_empty(), "Wayland at scale 1 keeps the low-latency bitmap");
    set_wayland(&ctx, false);
    let (bitmap, _, circles) = run(&ctx, 1.25);
    assert!(bitmap && circles.is_empty(), "X11 at 1.25 keeps the bitmap");
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
        (Shape::Zoom { out: false }, 2.0),
    ];
    let (width, height) = (600_usize, cases.len() * 100);
    let mut rgba = vec![255; width * height * 4];
    for (col, background) in [0_u8, 128, 255].into_iter().enumerate() {
        for y in 0..height {
            for x in col * 200..(col + 1) * 200 {
                rgba[(y * width + x) * 4..(y * width + x) * 4 + 3].fill(background);
            }
        }
        for (row, (shape, scale)) in cases.iter().enumerate() {
            let image = rasterize(shape, *scale).unwrap();
            let side = usize::from(image.size[0]);
            let x0 = col * 200 + 100 - usize::from(image.hotspot[0]);
            let y0 = row * 100 + 50 - usize::from(image.hotspot[1]);
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

#[cfg(not(target_arch = "wasm32"))]
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
#[cfg(not(target_arch = "wasm32"))]
fn brush_cursor_follows_pen_orientation_6350() {
    for control in ["penTilt", "rotation"] {
        let mut h = harness();
        h.state_mut()
            .run(
                "tools.setBrush",
                json!({"brush": {
                    "size": 60, "hardness": 1.0, "roundness": 0.25,
                    "shapeDynamics": {"enabled": true, "angle": {"control": control}}
                }}),
            )
            .unwrap();
        h.state_mut().stylus.feed.set(Some(crate::stylus::PenSample { tilt_x: 45.0, ..Default::default() }));
        h.run_steps(3);
        let initial = h.output().platform_output.cursor_image.clone().expect("initial brush cursor");
        h.state_mut().stylus.feed.set(Some(crate::stylus::PenSample { tilt_y: -45.0, rotation: 90.0, ..Default::default() }));
        h.run_steps(3);
        let turned = h.output().platform_output.cursor_image.clone().expect("turned brush cursor");
        save_pen_cursor_image(&format!("{control}-initial"), &initial);
        save_pen_cursor_image(&format!("{control}-turned"), &turned);
        let pen = h.state().stylus.sample().expect("simulated pen reaches the canvas");
        assert_eq!((pen.tilt_y, pen.rotation), (-45.0, 90.0));
        let point = photocraft_paint::StrokePoint { tilt_y: pen.tilt_y, rotation: pen.rotation, ..Default::default() };
        let step = photocraft_paint::dynamics::StepInput { point, direction: 0.0, initial_direction: 0.0, step: 0 };
        let mut dabs = Vec::new();
        photocraft_paint::dynamics::DabBuilder::new(&h.state().session.tools.brush).build(&step, &mut dabs);
        assert_eq!(dabs.len(), 1);
        assert!((dabs[0].angle.to_degrees() - 90.0).abs() < 0.001, "painted dab turns: {control}");
        assert_ne!(initial.rgba, turned.rgba, "cursor must follow pen orientation: {control}");
        // The dynamically rotated cursor must match the same tip set to that angle directly.
        h.state_mut().session.tools.brush.shape_dynamics.enabled = false;
        h.state_mut().session.tools.brush.angle = 90.0;
        h.run_steps(3);
        let reference = h.output().platform_output.cursor_image.clone().expect("static rotated reference");
        assert_eq!(turned.size, reference.size);
        assert_eq!(turned.hotspot, reference.hotspot);
        assert_eq!(turned.rgba, reference.rgba, "dynamic cursor matches the dab angle: {control}");
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn round_cursor_shows_brush_projection_6350() {
    let mut h = harness();
    h.state_mut().session.tools.brush.shape_dynamics.enabled = true;
    h.state_mut().session.tools.brush.shape_dynamics.brush_projection = true;
    h.state_mut().stylus.feed.set(Some(crate::stylus::PenSample::default()));
    h.run_steps(3);
    let upright = h.output().platform_output.cursor_image.clone().unwrap();
    h.state_mut().stylus.feed.set(Some(crate::stylus::PenSample { tilt_x: 60.0, ..Default::default() }));
    h.run_steps(3);
    let tilted = h.output().platform_output.cursor_image.clone().unwrap();
    save_pen_cursor_image("projection-upright", &upright);
    save_pen_cursor_image("projection-tilted", &tilted);
    assert_ne!(upright.rgba, tilted.rgba, "a projected round brush must not keep a circular cursor");
    h.state_mut().stylus.feed.set(None);
    h.run_steps(3);
    assert_eq!(upright.rgba, h.output().platform_output.cursor_image.clone().unwrap().rgba, "mouse restores the unprojected tip");
}

#[cfg(not(target_arch = "wasm32"))]
fn save_pen_cursor_image(name: &str, cursor: &egui::CustomCursorImage) {
    let Some(dir) = std::env::var_os("PHOTOCRAFT_CURSOR_OUTPUT") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let image =
        photocraft_codecs::Image::from_u8(u32::from(cursor.size[0]), u32::from(cursor.size[1]), photocraft_codecs::ChannelLayout::Rgba, cursor.rgba.to_vec())
            .unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    std::fs::write(dir.join(format!("{name}.png")), png).unwrap();
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
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
    // hand as a bitmap too (#2196); macOS and the Linux themes draw their own.
    h.state_mut().ui.tool = crate::Tool::Hand;
    h.run_steps(2);
    let hand = h.output().platform_output.cursor_image.clone();
    if cfg!(target_os = "windows") {
        let hand = hand.expect("the Hand's open hand reaches the OS as a bitmap");
        assert_eq!(hand.rgba, rasterize(&Shape::Hand { closed: false }, h.ctx.pixels_per_point()).unwrap().rgba);
    } else {
        assert!(hand.is_none(), "the OS draws the hand");
        assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::Grab);
    }

    // Likewise the Zoom tool's magnifier, a minus while ⌥ is held (#3735).
    h.state_mut().ui.tool = crate::Tool::Zoom;
    for out in [false, true] {
        let modifiers = if out { Modifiers::ALT } else { Modifiers::NONE };
        h.event(Event::ModifiersChanged(modifiers));
        h.event(Event::PointerMoved(p + vec2(1.0, 0.0)));
        h.run_steps(2);
        let image = h.output().platform_output.cursor_image.clone();
        if cfg!(target_os = "windows") {
            let image = image.expect("the magnifier reaches the OS as a bitmap");
            assert_eq!(image.rgba, rasterize(&Shape::Zoom { out }, h.ctx.pixels_per_point()).unwrap().rgba, "out: {out}");
        } else {
            assert!(image.is_none(), "the OS draws the magnifier");
            assert_eq!(h.output().platform_output.cursor_icon, if out { CursorIcon::ZoomOut } else { CursorIcon::ZoomIn });
        }
    }
    h.event(Event::ModifiersChanged(Modifiers::NONE));

    h.state_mut().ui.tool = crate::Tool::Brush;
    h.state_mut().run("prefs.set", json!({"path": "cursors.painting", "value": "standard"})).unwrap();
    h.run_steps(2);
    assert!(h.output().platform_output.cursor_image.is_none());
    assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::Default);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
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

/// #4596: the close-path pointer is the centre cross with a ring below right.
#[test]
fn close_path_bitmap_has_the_centre_cross_and_a_ring_beside_it() {
    for scale in [1.0, 2.0] {
        let image = rasterize(&Shape::Outline { rings: vec![close_ring()].into(), centre: true }, scale).unwrap();
        let [x, y] = image.hotspot;
        let at = |dx: f32, dy: f32| alpha(&image, x + (dx * scale) as u16, y + (dy * scale) as u16);
        assert!(at(0.0, 0.0) > 230 && at(2.0, 0.0) > 0, "the centre cross");
        assert!(at(12.5, 9.0) > 0 && at(9.0, 12.5) > 0, "the ring around (9, 9)");
        assert_eq!(at(9.0, 9.0), 0, "open in the middle");
        assert_eq!(at(0.0, 9.0), 0, "and apart from the cross");
    }
}

/// A big tip's outline is a thin ring in a large square: only the ring (and the centre mark) is
/// evaluated, and the bitmap is exactly what evaluating every pixel gives.
#[test]
fn circle_bitmaps_evaluate_only_the_ring_and_match_every_pixel() {
    let [dark, light] = crate::theme::Tokens::cursor_outline();
    for (radius, scale, centre) in [(0.4, 1.0, true), (3.0, 1.0, false), (20.0, 1.25, true), (57.3, 2.0, false), (90.0, 1.0, true)] {
        let shape = Shape::Circle { radius, centre };
        let image = rasterize(&shape, scale).unwrap();
        let [hot, _] = image.hotspot;
        let side = usize::from(image.size[0]);
        for row in 0..side {
            for col in 0..side {
                let (dx, dy) = (col as f32 - f32::from(hot), row as f32 - f32::from(hot));
                let want = stroke_pixel(coverage(&shape, dx, dy, scale), dark, light);
                let i = (row * side + col) * 4;
                assert_eq!(image.rgba[i..i + 4], want, "radius {radius} at {scale}×, pixel ({col}, {row})");
            }
        }
    }
}

/// The arrow is the handle's direction as the view shows it: turned with a rotated view and
/// mirrored with a flipped one.
#[test]
fn resize_follows_the_view() {
    use CursorIcon::{ResizeHorizontal as H, ResizeNeSw as NeSw, ResizeNwSe as NwSe, ResizeVertical as V};
    let view = |rotation: f32, flip: bool| ViewXform {
        rect: egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)),
        zoom: 1.0,
        center: [0.0, 0.0],
        flip,
        rotation,
        aspect: 1.0,
    };
    // Right, down-right, down and down-left in the document.
    let directions = [[1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [-1.0, 1.0]];
    for (rotation, flip, want) in
        [(0.0, false, [H, NwSe, V, NeSw]), (90.0, false, [V, NeSw, H, NwSe]), (45.0, false, [NwSe, V, NeSw, H]), (0.0, true, [H, NeSw, V, NwSe])]
    {
        let got = directions.map(|d| resize(d, &view(rotation, flip)));
        assert_eq!(got, want, "view turned {rotation}°, flip {flip}");
    }
}

#[test]
fn type_path_cursors_are_small_and_centred() {
    for rings in [super::area_type_rings(), super::path_type_rings()] {
        assert!(rings.len() > 3);
        for v in rings.iter().flatten() {
            assert!(v.x.abs() <= 12.0 && v.y.abs() <= 12.0, "{v:?}");
        }
    }
}

#[test]
fn the_pointer_belongs_to_another_window_from_its_report_until_this_one_hears_it_again() {
    // The main window, then a floating panel's window over it, the desktop, and back.
    let mut state = false;
    let mut frame = |here, there| {
        state = elsewhere(state, here, there);
        state
    };
    assert!(!frame(true, false), "over the main window");
    assert!(frame(true, true), "crossing into the floating window");
    assert!(frame(false, true), "moving in the floating window");
    assert!(frame(false, false), "on the desktop: the main window still heard nothing");
    assert!(!frame(true, false), "back over the main window");
    let float = Some(egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(200.0, 300.0)));
    assert!(covers(float, Some(egui::pos2(150.0, 150.0))) && !covers(float, Some(egui::pos2(98.0, 150.0))), "just outside its edge stays here");
}

#[test]
fn pen_bitmaps_render_all_six_variants_with_exact_outline_and_hotspots_at_each_scale() {
    for scale in [1.0, 1.25, 1.5, 2.0] {
        for variant in super::PenVariant::all() {
            let image = super::pen::rasterize(variant, scale).expect("pen variant rasterizes");
            let [w, h] = image.size;
            assert!(w > 0 && h > 0, "{variant:?} side at {scale}");
            assert!(w <= (38.0 * scale) as u16, "{variant:?} side within cursor bounds at {scale}");
            assert_eq!(image.rgba.len(), (usize::from(w) * usize::from(h) * 4));

            let [hx, hy] = image.hotspot;
            assert!(hx < w && hy < h, "{variant:?} hotspot inside bitmap at {scale}");

            // Verify straight RGBA: white foreground has RGB=255, dark outline has RGB=0 and alpha>0
            let pixels = image.rgba.as_chunks::<4>().0;
            let has_white = pixels.iter().any(|p| p[0] == 255 && p[1] == 255 && p[2] == 255 && p[3] > 180);
            let has_dark_outline = pixels.iter().any(|p| p[0] == 0 && p[1] == 0 && p[2] == 0 && p[3] > 50);
            assert!(has_white, "{variant:?} at {scale} has solid white foreground");
            assert!(has_dark_outline, "{variant:?} at {scale} has dark outline");

            // Verify hotspot matches expected formula: pad + (hot_box / 24.0) * 22.0 * scale
            let offset_px = 1.0 * scale;
            let pad = (offset_px + 2.0).ceil() as f32;
            let hot_box = variant.hotspot_in_box();
            let ideal_hx = pad + (hot_box.x / 24.0) * 22.0 * scale;
            let ideal_hy = pad + (hot_box.y / 24.0) * 22.0 * scale;
            let expected_hx = ideal_hx.round() as u16;
            let expected_hy = ideal_hy.round() as u16;
            assert_eq!(hx, expected_hx, "{variant:?} hotspot x at {scale}");
            assert_eq!(hy, expected_hy, "{variant:?} hotspot y at {scale}");
        }
    }
}

#[test]
fn pen_variants_have_distinct_hotspots_and_geometry() {
    let standard = super::pen::rasterize(super::PenVariant::Standard, 1.0).unwrap();
    let convert = super::pen::rasterize(super::PenVariant::ConvertPoint, 1.0).unwrap();
    let freeform = super::pen::rasterize(super::PenVariant::Freeform, 1.0).unwrap();

    // Standard nib vs ConvertPoint apex vs Freeform nib
    assert_ne!(standard.hotspot, convert.hotspot);
    assert_ne!(standard.hotspot, freeform.hotspot);
    assert_ne!(convert.hotspot, freeform.hotspot);
    assert_ne!(standard.rgba, convert.rgba);
    assert_ne!(standard.rgba, freeform.rgba);
}

#[test]
fn hostile_scales_get_no_pen_bitmap() {
    for scale in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, f32::MAX, 500.0] {
        for variant in super::PenVariant::all() {
            assert!(super::pen::rasterize(variant, scale).is_none(), "{variant:?} at {scale}");
        }
    }
}

#[test]
fn pen_cursor_bounded_cache_reuses_uploads_and_bounds_memory() {
    let ctx = egui::Context::default();
    let shape_a = super::Shape::Pen(super::PenVariant::Standard);
    let a1 = super::cached_image(&ctx, &shape_a, 1.0).unwrap();
    let a2 = super::cached_image(&ctx, &shape_a, 1.0).unwrap();
    assert!(Arc::ptr_eq(&a1.rgba, &a2.rgba), "pointer movement reuses cached bitmap");

    // All six variants can be cached concurrently without thrashing
    let mut images = Vec::new();
    for v in super::PenVariant::all() {
        let img = super::cached_image(&ctx, &super::Shape::Pen(v), 1.0).unwrap();
        images.push(img);
    }
    for (i, v) in super::PenVariant::all().into_iter().enumerate() {
        let lookup = super::cached_image(&ctx, &super::Shape::Pen(v), 1.0).unwrap();
        assert!(Arc::ptr_eq(&lookup.rgba, &images[i].rgba), "switching tools retains cached image");
    }

    // Non-Pen shapes use single-entry cache (memory bounded to 1 bitmap)
    for i in 0..25 {
        let _ = super::cached_image(&ctx, &super::Shape::Circle { radius: (10 + i) as f32, centre: false }, 1.0);
    }
    let id = egui::Id::unique("photocraft-tool-cursor");
    let cached = ctx.data(|d| d.get_temp::<super::Cached>(id)).unwrap();
    assert_eq!(cached.shape, super::Shape::Circle { radius: 34.0, centre: false }, "non-Pen cache holds only single most recent shape");
}

#[test]
fn pen_cursor_lifecycle_and_fallback_marks_visible_cursor() {
    let ctx = egui::Context::default();
    ctx.add_plugin(super::CursorLifecycle);

    // Simulated native pass
    let output = frame(&ctx, |ui| {
        let icon = super::pen(ui.ctx(), Tool::Pen);
        if let Some(i) = icon {
            ui.ctx().set_cursor_icon(i);
        }
    });
    assert!(output.platform_output.cursor_image.is_some(), "desktop root sets native cursor");
    assert_eq!(output.platform_output.cursor_icon, CursorIcon::Crosshair, "fallback if OS rejects upload");

    // Clear on pass end / leaving canvas
    let empty = frame(&ctx, |_| {});
    assert!(empty.platform_output.cursor_image.is_none(), "lifecycle clears cursor image");
}

#[test]
fn pen_bitmap_independent_comparison_versus_painted_path_on_light_dark_midgrey() {
    for scale in [1.0, 1.25, 1.5, 2.0] {
        for bg_lum in [240u8, 15u8, 128u8] {
            for variant in super::PenVariant::all() {
                // 1. Native bitmap path
                let native_img = super::pen::rasterize(variant, scale).expect("rasterize succeeds");
                let [nw, nh] = native_img.size;

                let mut native_comp = vec![bg_lum; usize::from(nw) * usize::from(nh)];
                for (i, px) in native_img.rgba.as_chunks::<4>().0.iter().enumerate() {
                    let fg_val = px[0] as f32;
                    let alpha = px[3] as f32 / 255.0;
                    let comp = fg_val * alpha + (bg_lum as f32) * (1.0 - alpha);
                    native_comp[i] = comp.round().clamp(0.0, 255.0) as u8;
                }

                // 2. Independent painted path representation
                let icon_px = (22.0 * scale).round() as u32;
                let svg_image = crate::icons::rasterize_svg(variant.icon_name(), icon_px, icon_px).expect("svg rasterize succeeds");
                let [sw, sh] = svg_image.size;

                let pad = ((1.0 * scale) + 2.0).ceil() as usize;
                let pw = sw + pad * 2;
                let ph = sh + pad * 2;
                assert_eq!(usize::from(nw), pw);
                assert_eq!(usize::from(nh), ph);

                let mut painted_comp = vec![bg_lum; pw * ph];

                let outline_alpha = 200.0 / 255.0;
                let offset = 1.0 * scale;
                for (dx, dy) in [(-1.0f32, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (1.0, -1.0)] {
                    let ox = (pad as f32) + dx * offset;
                    let oy = (pad as f32) + dy * offset;
                    for y in 0..sh {
                        for x in 0..sw {
                            let src_a = f32::from(svg_image.pixels[y * sw + x].a()) / 255.0;
                            if src_a <= 0.0 {
                                continue;
                            }
                            let layer_a = src_a * outline_alpha;
                            let dst_x = (ox + x as f32).round() as usize;
                            let dst_y = (oy + y as f32).round() as usize;
                            if dst_x < pw && dst_y < ph {
                                let idx = dst_y * pw + dst_x;
                                let cur = painted_comp[idx] as f32;
                                painted_comp[idx] = (cur * (1.0 - layer_a)).round().clamp(0.0, 255.0) as u8;
                            }
                        }
                    }
                }

                for y in 0..sh {
                    for x in 0..sw {
                        let src_a = f32::from(svg_image.pixels[y * sw + x].a()) / 255.0;
                        if src_a <= 0.0 {
                            continue;
                        }
                        let dst_x = pad + x;
                        let dst_y = pad + y;
                        if dst_x < pw && dst_y < ph {
                            let idx = dst_y * pw + dst_x;
                            let cur = painted_comp[idx] as f32;
                            painted_comp[idx] = (255.0 * src_a + cur * (1.0 - src_a)).round().clamp(0.0, 255.0) as u8;
                        }
                    }
                }

                // 3. Comparison metrics
                let mut sum_abs_diff = 0.0f64;
                let total_px = pw * ph;
                for i in 0..total_px {
                    let d = (native_comp[i] as i16 - painted_comp[i] as i16).unsigned_abs() as u8;
                    sum_abs_diff += d as f64;
                }
                let mean_abs_diff = sum_abs_diff / (total_px as f64);

                // Both must have visible white foreground and dark outline
                assert!(native_comp.iter().any(|&p| p > bg_lum.saturating_add(30) || (bg_lum == 240 && p >= 250)));
                assert!(native_comp.iter().any(|&p| p < bg_lum.saturating_sub(30) || (bg_lum == 15 && p <= 5)));

                // Perceptual similarity: Mean Absolute Error should remain within tolerance (< 20 / 255)
                // Note: we do not assert pixel-exact equality as subpixel resampling differs from discrete integer offsets
                assert!(mean_abs_diff < 20.0, "{variant:?} at scale {scale} on bg {bg_lum}: MAE {mean_abs_diff:.2} exceeds tolerance");
            }
        }
    }
}
