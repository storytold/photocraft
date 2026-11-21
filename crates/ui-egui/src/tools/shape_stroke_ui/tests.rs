use super::*;

#[test]
fn options_serde_defaults_keep_old_tool_settings_and_full_stroke() {
    assert_eq!(serde_json::from_value::<StrokeOptions>(json!({})).unwrap(), StrokeOptions::default());
    let style = StrokeOptions { cap: photocraft_doc::LineCap::Round, dashes: vec![0.0, 2.0, 4.0, 1.0], dash_offset: -0.5, ..StrokeOptions::default() };
    let stroke = style.stroke(5.0, photocraft_doc::Fill::Solid(photocraft_doc::Color::BLACK));
    assert_eq!(StrokeOptions::from(&stroke), style);
    assert_eq!(serde_json::from_value::<StrokeOptions>(serde_json::to_value(&style).unwrap()).unwrap(), style);
}

/// Filtered fill edges need transparent texels on every side of the projected quad.
#[test]
fn pixel_align_preview_pads_fill_texture_before_fractional_projection() {
    use photocraft_geom::{Affine, Rect as PixelRect};
    for side in [40.0, 4096.0] {
        let shape = photocraft_doc::ShapeLayer {
            path: photocraft_vector::shapes::rect(20.0, 20.0, side, 30.0),
            fill: Some(photocraft_doc::Fill::Solid(photocraft_doc::Color::BLACK)),
            use_aligned_rendering: true,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let transform = Affine::scale(0.125).then(&Affine::translate(10.25, 10.25));
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::background()).with_clip_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 100.0)));
            paint_document_shape_preview(&painter, &shape, &transform, PixelRect::new(0, 0, 5000, 200));
        });
        let cache = ctx.data(|d| d.get_temp::<DocumentDragCache>(document_preview_id(egui::ViewportId::ROOT, egui::LayerId::background()))).unwrap();
        let delta = output.textures_delta.set.get(&cache.texture.id()).unwrap().last().unwrap();
        let egui::ImageData::Color(image) = &delta.image;
        let [w, h] = image.size;
        assert!(w <= 2048 && h <= 2048);
        assert!(image.pixels[..w].iter().all(|p| p.a() == 0), "top border");
        assert!(image.pixels[(h - 1) * w..].iter().all(|p| p.a() == 0), "bottom border");
        assert!((0..h).all(|y| image.pixels[y * w].a() == 0 && image.pixels[y * w + w - 1].a() == 0), "side borders");
        assert!(image.pixels.iter().any(|p| p.a() > 0));
        assert!(cache.bounds.x0 <= 20 - cache.factor as i32);
        assert!(cache.bounds.y0 <= 20 - cache.factor as i32);
        let mesh = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Mesh(mesh) if mesh.texture_id == cache.texture.id() => Some(mesh),
                _ => None,
            })
            .unwrap();
        let corner = transform.apply(photocraft_geom::Point::new(f64::from(cache.bounds.x0), f64::from(cache.bounds.y0)));
        assert_eq!(mesh.vertices[0].pos, egui::pos2(corner.x as f32, corner.y as f32));
        output.textures_delta.clear();
    }
}

/// A floating viewport can have a different pass count while a root frame owns cleanup.
#[test]
fn pixel_align_floating_preview_survives_root_cleanup_and_completes() {
    use photocraft_geom::{Affine, Rect as PixelRect};
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    egui::Context::set_immediate_viewport_renderer(|ctx, mut viewport| {
        let id = viewport.ids.this;
        let mut input = egui::RawInput { viewport_id: id, ..Default::default() };
        input.viewports.insert(id, egui::ViewportInfo { parent: Some(egui::ViewportId::ROOT), ..Default::default() });
        let mut output = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
        output.textures_delta.clear();
    });
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    for _ in 0..3 {
        let mut output = ctx.run_ui(Default::default(), |_| {});
        output.textures_delta.clear();
    }
    let shape = photocraft_doc::ShapeLayer {
        path: photocraft_vector::shapes::rect(0.0, 0.0, 2050.0, 2050.0),
        fill: Some(photocraft_doc::Fill::Solid(photocraft_doc::Color::BLACK)),
        use_aligned_rendering: true,
        ..Default::default()
    };
    let id = document_preview_id(egui::ViewportId::from_hash_of("floating-preview"), egui::LayerId::background());
    let mut accumulator = None;
    for frame in 0..2 {
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.ctx().show_viewport_immediate(egui::ViewportId::from_hash_of("floating-preview"), Default::default(), |ui, _| {
                assert_ne!(ui.ctx().cumulative_pass_nr(), ui.ctx().cumulative_pass_nr_for(egui::ViewportId::ROOT));
                let painter =
                    ui.ctx().layer_painter(egui::LayerId::background()).with_clip_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0)));
                paint_document_shape_preview(&painter, &shape, &Affine::scale(0.125).then(&Affine::translate(10.0, 10.0)), PixelRect::new(0, 0, 2050, 2050));
            });
            finish_frame(&mut app, ui.ctx());
        });
        output.textures_delta.clear();
        let cache = ctx.data(|d| d.get_temp::<DocumentDragCache>(id)).expect("root cleanup must retain the floating preview");
        if frame == 0 {
            accumulator = Some(Arc::downgrade(cache.reduction.as_ref().expect("unfinished first pass")));
        } else {
            assert!(cache.reduction.is_none(), "the second pass must finish rather than restart");
        }
    }
    assert!(accumulator.unwrap().upgrade().is_none(), "completion releases the accumulator");
    let mut output = ctx.run_ui(Default::default(), |ui| finish_frame(&mut app, ui.ctx()));
    output.textures_delta.clear();
    assert!(ctx.data(|d| d.get_temp::<DocumentDragCache>(id)).is_none(), "ending the drag releases its texture");
}

/// Canvas-aligned gradient paint must not use the clipped preview coverage as its frame.
#[test]
fn pixel_align_preview_preserves_canvas_gradient_frame() {
    use photocraft_doc::{Color, Fill, GradientStyle, ShapeLayer};
    use photocraft_geom::{Affine, Rect as PixelRect};
    let canvas = PixelRect::new(0, 0, 200, 120);
    let gradient = Fill::Gradient {
        stops: vec![(0.0, Color::rgb(1.0, 0.0, 0.0)), (1.0, Color::rgb(0.0, 0.0, 1.0))],
        angle: 0.0,
        scale: 1.0,
        style: GradientStyle::Linear,
        reverse: false,
        opacity_stops: vec![],
        midpoints: vec![],
        offset: (0.0, 0.0),
        dither: false,
        align: false,
    };
    let shape = ShapeLayer {
        path: photocraft_vector::shapes::rect(20.0, 20.0, 140.0, 80.0),
        fill: Some(gradient.clone()),
        stroke: Some(ShapeStroke { width: 1.0, paint: gradient, ..Default::default() }),
        use_aligned_rendering: true,
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |ui| {
        let painter =
            ui.ctx().layer_painter(egui::LayerId::background()).with_clip_rect(egui::Rect::from_min_max(egui::pos2(30.0, 25.0), egui::pos2(90.0, 65.0)));
        paint_document_shape_preview(&painter, &shape, &Affine::IDENTITY, canvas);
    });
    let cache = ctx.data(|d| d.get_temp::<DocumentDragCache>(document_preview_id(egui::ViewportId::ROOT, egui::LayerId::background()))).unwrap();
    assert_ne!(cache.visible, canvas, "test requires distinct coverage and paint frames");
    let image = &output.textures_delta.set.get(&cache.texture.id()).unwrap()[0].image;
    let egui::ImageData::Color(image) = image;
    let rgba = photocraft_vector::CompiledShape::with_frames(&shape, photocraft_vector::DEFAULT_TOLERANCE, cache.visible, canvas).render_rgba(cache.bounds);
    let wrong = photocraft_vector::CompiledShape::new(&shape, photocraft_vector::DEFAULT_TOLERANCE, cache.visible).render_rgba(cache.bounds);
    assert_ne!(rgba, wrong, "test must detect a gradient laid out in the wrong frame");
    let expected: Vec<_> = rgba
        .iter()
        .map(|p| {
            egui::Color32::from_rgba_unmultiplied(
                (p[0] * 255.0).round() as u8,
                (p[1] * 255.0).round() as u8,
                (p[2] * 255.0).round() as u8,
                (p[3] * 255.0).round() as u8,
            )
        })
        .collect();
    assert_eq!(image.pixels, expected);
    assert_eq!(cache.shape.path, shape.path);
    output.textures_delta.clear();
}

/// Crossing the preview texture limit must retain box-filtered aligned document coverage.
#[test]
fn pixel_align_oversized_preview_preserves_document_coverage() {
    use photocraft_doc::{Fill, ShapeLayer};
    use photocraft_geom::{Affine, Rect as PixelRect};
    for width in [2044.0, 2046.0, 2048.0, 4096.0] {
        for dashed in [false, true] {
            let shape = ShapeLayer {
                path: photocraft_vector::shapes::rect(0.0, 1.0, width, 6.0),
                fill: None,
                stroke: Some(ShapeStroke {
                    width: 1.0,
                    paint: Fill::Solid(photocraft_doc::Color::BLACK),
                    dashes: if dashed { vec![3.0, 2.0] } else { vec![] },
                    dash_offset: -0.5,
                    ..Default::default()
                }),
                use_aligned_rendering: true,
                ..Default::default()
            };
            let ctx = egui::Context::default();
            let transform = Affine::scale(0.125).then(&Affine::translate(10.0, 10.0));
            let canvas = PixelRect::new(0, 0, 5000, 100);
            let mut output = ctx.run_ui(Default::default(), |ui| {
                let painter =
                    ui.ctx().layer_painter(egui::LayerId::background()).with_clip_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 60.0)));
                paint_document_shape_preview(&painter, &shape, &transform, canvas);
            });
            let cache = ctx.data(|d| d.get_temp::<DocumentDragCache>(document_preview_id(egui::ViewportId::ROOT, egui::LayerId::background())));
            let image = cache.as_ref().and_then(|c| output.textures_delta.set.get(&c.texture.id())).and_then(|d| d.last()).map(|d| d.image.clone());
            output.textures_delta.clear();
            let cache = cache.expect("large previews must keep aligned document coverage instead of using the legacy proxy");
            let egui::ImageData::Color(image) = image.expect("uploaded document preview");
            assert!(image.size.into_iter().all(|n| n <= 2048));
            let factor = cache.bounds.width().max(cache.bounds.height()).div_ceil(2048).max(1) as usize;
            let compiled = photocraft_vector::CompiledShape::with_frames(&shape, photocraft_vector::DEFAULT_TOLERANCE, cache.visible, canvas);
            let native = compiled.render_rgba(cache.bounds);
            let mut alpha = vec![0.0f64; image.pixels.len()];
            for (i, pixel) in native.iter().enumerate() {
                let x = i % cache.bounds.width() as usize;
                let y = i / cache.bounds.width() as usize;
                alpha[(y / factor) * image.size[0] + x / factor] += f64::from(pixel[3]);
            }
            let expected: Vec<u8> = alpha.iter().map(|a| (a / (factor * factor) as f64 * 255.0).round() as u8).collect();
            let actual: Vec<u8> = image.pixels.iter().map(|p| p.a()).collect();
            assert_eq!(actual, expected, "width={width}, dashed={dashed}");
            assert!(actual.iter().any(|a| *a > 0));
            assert_eq!(cache.shape.path, shape.path);
        }
    }
}

/// Zoom and rotation project committed document coverage, including ineligible half-pixel strokes.
#[test]
fn pixel_align_preview_texture_matches_document_coverage_before_zoom() {
    use photocraft_doc::{Fill, ShapeLayer};
    use photocraft_geom::{Affine, Point, Rect as PixelRect};
    for (curved, path) in [(false, photocraft_vector::shapes::rect(20.0, 20.0, 40.0, 30.0)), (true, photocraft_vector::shapes::ellipse(20.0, 20.0, 40.0, 30.0))]
    {
        for width in [0.5, 1.0, 2.0] {
            for dashed in [false, true] {
                for zoom in [0.5, 1.0, 2.0, 3.0] {
                    for angle in [0.0, 0.3] {
                        let shape = ShapeLayer {
                            path: path.clone(),
                            fill: None,
                            stroke: Some(ShapeStroke {
                                width,
                                paint: Fill::Solid(photocraft_doc::Color::BLACK),
                                dashes: if dashed { vec![3.0, 2.0] } else { vec![] },
                                dash_offset: 0.5,
                                ..Default::default()
                            }),
                            use_aligned_rendering: true,
                            ..Default::default()
                        };
                        let ctx = egui::Context::default();
                        let transform = Affine::scale(zoom).then(&Affine::rotate(angle)).then(&Affine::translate(40.0, 40.0));
                        let mut output = ctx.run_ui(Default::default(), |ui| {
                            let ctx = ui.ctx();
                            let painter =
                                ctx.layer_painter(egui::LayerId::background()).with_clip_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, 300.0)));
                            paint_document_shape_preview(&painter, &shape, &transform, PixelRect::new(0, 0, 200, 200));
                        });
                        let cache = ctx
                            .data(|d| d.get_temp::<DocumentDragCache>(document_preview_id(egui::ViewportId::ROOT, egui::LayerId::background())))
                            .expect("document preview cache");
                        let image = output.textures_delta.set.get(&cache.texture.id()).expect("uploaded preview");
                        let egui::ImageData::Color(image) = &image[0].image;
                        let compiled = photocraft_vector::CompiledShape::new(&shape, photocraft_vector::DEFAULT_TOLERANCE, cache.visible);
                        let rgba = compiled.render_rgba(cache.bounds);
                        let expected: Vec<_> = rgba
                            .iter()
                            .map(|p| {
                                egui::Color32::from_rgba_unmultiplied(
                                    (p[0] * 255.0).round() as u8,
                                    (p[1] * 255.0).round() as u8,
                                    (p[2] * 255.0).round() as u8,
                                    (p[3] * 255.0).round() as u8,
                                )
                            })
                            .collect();
                        assert_eq!(image.pixels, expected, "width={width}, angle={angle}");
                        if curved {
                            assert!(rgba.iter().any(|p| p[3] > 0.0 && p[3] < 1.0), "curve antialiasing");
                        } else if !dashed {
                            let edge = compiled.render_rgba(PixelRect::new(35, 20, 36, 21))[0][3];
                            assert_eq!(edge, if width == 0.5 { 0.25 } else { 1.0 });
                        }
                        let mesh = output
                            .shapes
                            .iter()
                            .find_map(|s| match &s.shape {
                                egui::Shape::Mesh(mesh) if mesh.texture_id == cache.texture.id() => Some(mesh),
                                _ => None,
                            })
                            .expect("projected quad");
                        let corner = transform.apply(Point::new(f64::from(cache.bounds.x0), f64::from(cache.bounds.y0)));
                        assert_eq!(mesh.vertices[0].pos, egui::pos2(corner.x as f32, corner.y as f32));
                        output.textures_delta.clear();
                    }
                }
            }
        }
    }
}

/// Huge previews retain a bounded allocation instead of rasterizing an unbounded document.
#[test]
fn pixel_align_document_preview_rejects_oversized_raster_allocations() {
    let shape = photocraft_doc::ShapeLayer {
        path: photocraft_vector::shapes::rect(0.0, 0.0, 100_000.0, 100_000.0),
        fill: Some(photocraft_doc::Fill::Solid(photocraft_doc::Color::BLACK)),
        use_aligned_rendering: true,
        ..Default::default()
    };
    let visible = photocraft_geom::Rect::new(0, 0, 100_000, 100_000);
    let compiled = photocraft_vector::CompiledShape::new(&shape, photocraft_vector::DEFAULT_TOLERANCE, visible);
    for source in [visible, photocraft_geom::Rect::new(0, 0, 10_000, 1), photocraft_geom::Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX)] {
        let layout = PreviewLayout::new(source).unwrap();
        assert!(layout.size.into_iter().all(|n| n <= 2048));
        assert!(layout.size[0] * layout.size[1] <= 4_194_304);
    }
    assert_eq!(document_preview_bounds(&compiled, visible), Some(visible));
}

/// Large reductions make bounded progress over frames and release their working buffers at completion.
#[test]
fn pixel_align_large_preview_completes_in_bounded_passes() {
    use photocraft_geom::{Affine, Rect as PixelRect};
    let bounds = PixelRect::new(0, 0, 2050, 2050);
    let shape = photocraft_doc::ShapeLayer {
        path: photocraft_vector::shapes::rect(0.0, 0.0, 2050.0, 2050.0),
        fill: Some(photocraft_doc::Fill::Solid(photocraft_doc::Color::BLACK)),
        use_aligned_rendering: true,
        ..Default::default()
    };
    let compiled = photocraft_vector::CompiledShape::new(&shape, photocraft_vector::DEFAULT_TOLERANCE, bounds);
    let mut work = PreviewReduction::new(compiled, PreviewLayout::new(bounds).unwrap());
    assert!(work.advance() <= document_preview::PIXELS_PER_PASS);
    assert!(!work.complete(), "the source must not be rasterized in one unbounded pass");
    assert!(work.advance() <= document_preview::PIXELS_PER_PASS);
    assert!(work.complete());
    assert!(work.image().pixels.iter().all(|p| p.a() == 255));
    drop(work);
    let ctx = egui::Context::default();
    let transform = Affine::scale(0.125).then(&Affine::translate(10.0, 10.0));
    for frame in 0..3 {
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::background()).with_clip_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0)));
            paint_document_shape_preview(&painter, &shape, &transform, bounds);
        });
        let cache = ctx.data(|d| d.get_temp::<DocumentDragCache>(document_preview_id(egui::ViewportId::ROOT, egui::LayerId::background()))).unwrap();
        let image = output.textures_delta.set.get(&cache.texture.id()).and_then(|d| d.last()).map(|d| d.image.clone());
        output.textures_delta.clear();
        if frame == 0 {
            assert!(cache.reduction.is_some());
        } else {
            assert!(cache.reduction.is_none(), "completed previews must release native buffers");
            if let Some(egui::ImageData::Color(image)) = image {
                let oracle = photocraft_vector::CompiledShape::with_frames(&shape, photocraft_vector::DEFAULT_TOLERANCE, cache.visible, bounds);
                for y in [0, 1, image.size[1] / 2, image.size[1] - 1] {
                    for x in [0, 1, image.size[0] / 2, image.size[0] - 1] {
                        let source = PixelRect::from_xywh(
                            cache.bounds.x0 + (x as u32 * cache.factor) as i32,
                            cache.bounds.y0 + (y as u32 * cache.factor) as i32,
                            cache.factor,
                            cache.factor,
                        )
                        .intersect(&cache.bounds);
                        let alpha: f64 = oracle.render_rgba(source).iter().map(|p| f64::from(p[3])).sum();
                        let expected = (alpha / f64::from(cache.factor).powi(2) * 255.0).round() as u8;
                        assert_eq!(image.pixels[y * image.size[0] + x].a(), expected, "source padding at {x},{y}");
                    }
                }
            }
        }
    }
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut changed = shape.clone();
    changed.path = photocraft_vector::shapes::rect(1.0, 0.0, 2050.0, 2050.0);
    let mut output = ctx.run_ui(Default::default(), |ui| {
        let painter = ui.ctx().layer_painter(egui::LayerId::background()).with_clip_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0)));
        paint_document_shape_preview(&painter, &changed, &transform, bounds);
        finish_frame(&mut app, ui.ctx());
    });
    output.textures_delta.clear();
    let cache = ctx.data(|d| d.get_temp::<DocumentDragCache>(document_preview_id(egui::ViewportId::ROOT, egui::LayerId::background()))).unwrap();
    let unfinished = Arc::downgrade(cache.reduction.as_ref().expect("pending replacement preview"));
    drop(cache);
    let mut output = ctx.run_ui(Default::default(), |ui| finish_frame(&mut app, ui.ctx()));
    output.textures_delta.clear();
    assert!(unfinished.upgrade().is_none(), "ending a drag must release the unfinished buffers");
}
