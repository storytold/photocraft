use photocraft_color::PixelFormat;
use photocraft_doc::mask_paint::{EffectKind, Paint, Stack};
use photocraft_geom::Rect;
use photocraft_gpu::{MaskBrushSettings, MaskRoundDab, MaskStackRenderer};
use photocraft_raster::Surface;

use crate::GPU_LOCK;

#[test]
fn mask_stack_preflight_accepts_spatial_fill_without_gpu_setup() {
    let stack = Stack {
        base: Surface::with_default(PixelFormat::GRAY8, &[1.0]),
        layers: vec![Paint {
            name: "Spatial Fill".into(),
            visible: true,
            opacity: 1.0,
            blend: photocraft_color::BlendMode::Normal,
            surface: Surface::new(PixelFormat::RGBA8),
            fill_map: Some(Surface::new(PixelFormat::RGBA8)),
            fill_map_size: [8, 8],
            fill_bounds: Some([0, 0, 8, 8]),
            kind: EffectKind::Fill { value: 0.5, rotation: 0.0, repeat_u: false, repeat_v: false, mirror_u: false, mirror_v: false },
        }],
        active: 0,
    };
    assert!(MaskStackRenderer::validate(&stack).is_ok());
}

#[test]
fn gpu_spatial_and_legacy_image_fill_match_cpu_stack() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("skipping GPU image-Fill test: no adapter");
        return;
    };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {
        eprintln!("skipping GPU image-Fill test: no device");
        return;
    };
    let size = [8, 8];
    let rect = Rect::new(0, 0, size[0] as i32, size[1] as i32);
    let shell_ids = vec![1; (size[0] * size[1]) as usize];
    let mut map = Surface::with_default(PixelFormat::GRAY8, &[0.25]);
    map.write_region(Rect::new(0, 0, 2, 2), &[0.0, 0.25, 0.75, 1.0]);
    let cases =
        [(Some([1, 1, 7, 7]), 0.0, true, false, true, false), (Some([1, 1, 7, 7]), 31.0, true, true, true, true), (None, 0.0, true, false, true, false)];
    for (index, (bounds, rotation, repeat_u, repeat_v, mirror_u, mirror_v)) in cases.into_iter().enumerate() {
        let layer = Paint {
            name: "Image Fill".into(),
            visible: true,
            opacity: 0.73,
            blend: photocraft_color::BlendMode::Multiply,
            surface: Surface::new(PixelFormat::RGBA8),
            fill_map: Some(map.clone()),
            fill_map_size: [2, 2],
            fill_bounds: bounds,
            kind: EffectKind::Fill { value: 0.9, rotation, repeat_u, repeat_v, mirror_u, mirror_v },
        };
        let stack = Stack { base: Surface::with_default(PixelFormat::GRAY8, &[0.6]), layers: vec![layer], active: 0 };
        let target = create_test_target(&device, size);
        let mut renderer = MaskStackRenderer::new(&device);
        renderer.render_cached(&device, &queue, &stack, size, &shell_ids, 100 + index as u64, 1, &target).unwrap();
        let actual = read_gray(&device, &queue, &target, size);
        let shell_map = photocraft_compose::mask_paint::ShellIdMap::new(rect, &shell_ids).unwrap();
        let expected = photocraft_compose::mask_paint::compose_stack_region_with_shell_ids(&stack, stack.layers.len(), rect, shell_map).unwrap();
        for (pixel, (actual, expected)) in actual.iter().zip(expected.iter()).enumerate() {
            assert!((actual - expected).abs() < 0.004, "case {index}, pixel {pixel}: GPU={actual}, CPU={expected}");
        }
    }
}

#[test]
fn gpu_mask_fill_blend_modes_match_cpu_semantics() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("skipping mask blend-mode test: no adapter");
        return;
    };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {
        eprintln!("skipping mask blend-mode test: no device");
        return;
    };
    let size = [8, 8];
    let rect = Rect::new(0, 0, size[0] as i32, size[1] as i32);
    let shell_ids = vec![1; (size[0] * size[1]) as usize];
    for (index, blend) in photocraft_doc::mask_paint::MASK_BLEND_MODES.into_iter().enumerate() {
        let stack = Stack {
            base: Surface::with_default(PixelFormat::GRAY8, &[0.37]),
            layers: vec![Paint {
                name: "Fill".into(),
                visible: true,
                opacity: 0.61,
                blend,
                surface: Surface::new(PixelFormat::RGBA8),
                fill_map: None,
                fill_map_size: [0, 0],
                fill_bounds: None,
                kind: EffectKind::Fill { value: 0.73, rotation: 0.0, repeat_u: false, repeat_v: false, mirror_u: false, mirror_v: false },
            }],
            active: 0,
        };
        let target = create_test_target(&device, size);
        let mut renderer = MaskStackRenderer::new(&device);
        renderer.render_cached(&device, &queue, &stack, size, &shell_ids, 700 + index as u64, 1, &target).unwrap();
        let actual = read_gray(&device, &queue, &target, size);
        let shell_map = photocraft_compose::mask_paint::ShellIdMap::new(rect, &shell_ids).unwrap();
        let expected = photocraft_compose::mask_paint::compose_stack_region_with_shell_ids(&stack, stack.layers.len(), rect, shell_map).unwrap();
        for (pixel, (actual, expected)) in actual.iter().zip(expected.iter()).enumerate() {
            assert!((actual - expected).abs() < 0.004, "mode {blend:?}, pixel {pixel}: GPU={actual}, CPU={expected}");
        }
    }
}

#[test]
fn gpu_round_dab_preview_and_release_patches_match_cpu_stroke() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("skipping GPU brush preview test: no adapter");
        return;
    };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {
        eprintln!("skipping GPU brush preview test: no device");
        return;
    };
    let size = [40, 32];
    let base = Surface::with_default(PixelFormat::RGBA8, &[0.25, 0.5, 0.75, 1.0]);
    let brush = photocraft_paint::BrushSettings {
        size: 8.0,
        hardness: 0.45,
        pressure_size: false,
        pressure_opacity: false,
        opacity: 0.73,
        flow: 0.64,
        color: [0.1, 0.2, 0.3, 1.0],
        ..Default::default()
    };
    let points = vec![photocraft_paint::StrokePoint::new(10.25, 11.75, 1.0), photocraft_paint::StrokePoint::new(18.0, 14.0, 1.0)];
    let stroke = photocraft_paint::Stroke { brush: brush.clone(), points: points.clone() };
    let dabs = photocraft_paint::dabs(&stroke);
    assert!(dabs.len() > 1);
    let gpu_dabs: Vec<_> = dabs
        .iter()
        .map(|dab| MaskRoundDab {
            center: [dab.center.x as f32, dab.center.y as f32],
            radius: dab.radius * dab.roundness,
            alpha: dab.alpha,
            opacity: dab.opacity,
        })
        .collect();
    let mut renderer = MaskStackRenderer::new(&device);
    renderer
        .begin_round_mask_brush_preview(
            &device,
            &queue,
            991,
            size,
            &base,
            MaskBrushSettings { hardness: brush.hardness, stroke_opacity: brush.opacity, color: brush.color },
        )
        .unwrap();
    let split = gpu_dabs.len() / 2;
    let changed_first = renderer.push_round_mask_brush_dabs(&device, &queue, &gpu_dabs[..split]).unwrap();
    assert!(!changed_first.is_empty());
    assert!(!renderer.take_mask_brush_dirty_regions().is_empty());
    let changed = renderer.push_round_mask_brush_dabs(&device, &queue, &gpu_dabs[split..]).unwrap();
    assert!(!changed.is_empty());
    let preview_texture = renderer.mask_brush_preview_texture().unwrap();
    let preview = read_gray(&device, &queue, preview_texture, size);
    let patches = renderer.read_mask_brush_patches(&device, &queue).unwrap();
    assert!(!patches.is_empty());
    let mut canonical = base.clone();
    photocraft_paint::render_stroke(&mut canonical, &brush, &points, None, false, 1.0);
    for patch in patches {
        let expected = canonical.try_read_region(patch.rect).unwrap();
        for (pixel_index, (actual, expected)) in patch.pixels_rgba8.chunks_exact(4).zip(expected.chunks_exact(4)).enumerate() {
            for channel in 0..4 {
                let actual = f32::from(actual[channel]) / 255.0;
                assert!(
                    (actual - expected[channel]).abs() < 0.012,
                    "patch {:?}, pixel {pixel_index} ({},{}), channel {channel}: GPU={actual}, CPU={}",
                    patch.rect,
                    patch.rect.x0 + pixel_index as i32 % patch.rect.width() as i32,
                    patch.rect.y0 + pixel_index as i32 / patch.rect.width() as i32,
                    expected[channel]
                );
            }
        }
    }
    assert_eq!(preview.len(), (size[0] * size[1]) as usize);
    renderer.cancel_mask_brush_preview();
    assert!(renderer.mask_brush_preview_texture().is_none());
}

#[test]
fn gpu_brush_preview_recomposes_downstream_blur_and_matches_full_stack() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("skipping GPU brush+Blur test: no adapter");
        return;
    };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {
        eprintln!("skipping GPU brush+Blur test: no device");
        return;
    };
    let size = [48, 40];
    let shell_ids = vec![1; (size[0] * size[1]) as usize];
    let base = Surface::new(PixelFormat::RGBA8);
    let mut stack = Stack {
        base: Surface::with_default(PixelFormat::GRAY8, &[0.0]),
        layers: vec![
            Paint {
                name: "Paint".into(),
                visible: true,
                opacity: 1.0,
                blend: photocraft_color::BlendMode::Normal,
                surface: base.clone(),
                fill_map: None,
                fill_map_size: [0, 0],
                fill_bounds: None,
                kind: EffectKind::Paint,
            },
            Paint {
                name: "Blur".into(),
                visible: true,
                opacity: 1.0,
                blend: photocraft_color::BlendMode::Normal,
                surface: Surface::new(PixelFormat::RGBA8),
                fill_map: None,
                fill_map_size: [0, 0],
                fill_bounds: None,
                kind: EffectKind::Blur { radius: 5 },
            },
        ],
        active: 0,
    };
    let target = create_test_target(&device, size);
    let mut renderer = MaskStackRenderer::new(&device);
    renderer.render_cached(&device, &queue, &stack, size, &shell_ids, 818, 14, &target).unwrap();
    renderer
        .begin_round_mask_brush_preview(
            &device,
            &queue,
            818,
            size,
            &base,
            MaskBrushSettings { hardness: 0.7, stroke_opacity: 0.8, color: [1.0, 1.0, 1.0, 1.0] },
        )
        .unwrap();
    renderer
        .push_round_mask_brush_dabs(
            &device,
            &queue,
            &[
                MaskRoundDab { center: [21.0, 18.0], radius: 8.0, alpha: 1.0, opacity: 0.8 },
                MaskRoundDab { center: [28.0, 22.0], radius: 8.0, alpha: 1.0, opacity: 0.8 },
            ],
        )
        .unwrap();
    let dirty = renderer.take_mask_brush_dirty_regions();
    assert!(!dirty.is_empty());
    renderer.render_cached_with_brush_preview(&device, &queue, &stack, size, &shell_ids, 14, 0, &dirty, &target).unwrap();
    let actual = read_gray(&device, &queue, &target, size);

    for patch in renderer.read_mask_brush_patches(&device, &queue).unwrap() {
        let values: Vec<f32> = patch.pixels_rgba8.iter().map(|value| f32::from(*value) / 255.0).collect();
        stack.layers[0].surface.write_region(patch.rect, &values);
    }
    let rect = Rect::new(0, 0, size[0] as i32, size[1] as i32);
    let shell_map = photocraft_compose::mask_paint::ShellIdMap::new(rect, &shell_ids).unwrap();
    let expected = photocraft_compose::mask_paint::compose_stack_region_with_shell_ids(&stack, stack.layers.len(), rect, shell_map).unwrap();
    assert_eq!(actual.len(), expected.len());
    for (index, (actual, expected)) in actual.iter().zip(expected.iter()).enumerate() {
        assert!((actual - expected).abs() < 0.006, "pixel {index}: transient GPU={actual}, full CPU stack={expected}");
    }
    renderer.cancel_mask_brush_preview();
}

#[test]
fn gpu_mask_blur_matches_shell_aware_cpu_stack_without_crossing_islands() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("skipping GPU mask-stack test: no adapter");
        return;
    };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {
        eprintln!("skipping GPU mask-stack test: no device");
        return;
    };

    let size = [16, 8];
    let rect = Rect::new(0, 0, size[0] as i32, size[1] as i32);
    let mut shell_ids = vec![0; (size[0] * size[1]) as usize];
    for y in 0..size[1] {
        for x in 1..7 {
            shell_ids[(y * size[0] + x) as usize] = 1;
        }
        for x in 7..14 {
            shell_ids[(y * size[0] + x) as usize] = 2;
        }
    }
    let mut base = Surface::with_default(PixelFormat::GRAY8, &[0.5]);
    base.fill_rect(rect, &[0.5]);
    let mut paint_surface = Surface::new(PixelFormat::RGBA8);
    paint_surface.write_region(Rect::new(5, 2, 6, 6), &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    let stack = Stack {
        base,
        layers: vec![
            Paint {
                name: "Paint".into(),
                visible: true,
                opacity: 1.0,
                blend: photocraft_color::BlendMode::Normal,
                surface: paint_surface,
                fill_map: None,
                fill_map_size: [0, 0],
                fill_bounds: None,
                kind: EffectKind::Paint,
            },
            Paint {
                name: "Blur".into(),
                visible: true,
                opacity: 1.0,
                blend: photocraft_color::BlendMode::Normal,
                surface: Surface::new(PixelFormat::RGBA8),
                fill_map: None,
                fill_map_size: [0, 0],
                fill_bounds: None,
                kind: EffectKind::Blur { radius: 2 },
            },
        ],
        active: 1,
    };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mask-stack-test-output"),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    MaskStackRenderer::new(&device).render(&device, &queue, &stack, size, &shell_ids, &target).unwrap();

    let row = (size[0] * 8).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mask-stack-test-readback"),
        size: u64::from(row * size[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("mask-stack-test-readback") });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: &target, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(size[1]) },
        },
        wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
        let _ = send.send(result);
    });
    let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    receive.recv().unwrap().unwrap();
    let mapped = buffer.slice(..).get_mapped_range().unwrap();
    let actual: Vec<f32> = (0..(size[0] * size[1]) as usize)
        .map(|index| {
            let x = index % size[0] as usize;
            let y = index / size[0] as usize;
            let offset = y * row as usize + x * 8;
            half::f16::from_le_bytes([mapped[offset], mapped[offset + 1]]).to_f32()
        })
        .collect();
    drop(mapped);
    buffer.unmap();

    let shell_map = photocraft_compose::mask_paint::ShellIdMap::new(rect, &shell_ids).unwrap();
    let expected = photocraft_compose::mask_paint::compose_stack_region_with_shell_ids(&stack, stack.layers.len(), rect, shell_map).unwrap();
    for (index, (actual, expected)) in actual.iter().zip(expected.iter()).enumerate() {
        assert!((actual - expected).abs() < 0.008, "pixel {index}: GPU={actual}, CPU={expected}");
    }
    assert!(
        actual
            .iter()
            .enumerate()
            .filter(|(index, _)| index % (size[0] as usize) >= 7 && index % (size[0] as usize) < 14)
            .all(|(_, value)| (*value - 0.5).abs() < 0.002)
    );
}

#[test]
fn incremental_paint_tile_upload_matches_fresh_render_through_multiple_blurs() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("skipping incremental mask-stack test: no adapter");
        return;
    };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {
        eprintln!("skipping incremental mask-stack test: no device");
        return;
    };
    let size = [300, 12];
    let rect = Rect::new(0, 0, size[0] as i32, size[1] as i32);
    let shell_ids = vec![1; (size[0] * size[1]) as usize];
    let mut base = Surface::with_default(PixelFormat::GRAY8, &[0.5]);
    base.fill_rect(rect, &[0.5]);
    let mut paint = Surface::new(PixelFormat::RGBA8);
    paint.write_region(Rect::new(8, 3, 10, 5), &[0.0, 0.0, 0.0, 1.0].repeat(4).as_slice());
    let make_layer = |name: &str, kind| Paint {
        name: name.into(),
        visible: true,
        opacity: 1.0,
        blend: photocraft_color::BlendMode::Normal,
        surface: Surface::new(PixelFormat::RGBA8),
        fill_map: None,
        fill_map_size: [0, 0],
        fill_bounds: None,
        kind,
    };
    let mut stack = Stack {
        base,
        layers: vec![
            Paint {
                name: "Paint".into(),
                visible: true,
                opacity: 1.0,
                blend: photocraft_color::BlendMode::Normal,
                surface: paint,
                fill_map: None,
                fill_map_size: [0, 0],
                fill_bounds: None,
                kind: EffectKind::Paint,
            },
            make_layer("Blur 1", EffectKind::Blur { radius: 2 }),
            make_layer("Blur 2", EffectKind::Blur { radius: 1 }),
        ],
        active: 2,
    };
    let target = create_test_target(&device, size);
    let mut renderer = MaskStackRenderer::new(&device);
    renderer.render_cached(&device, &queue, &stack, size, &shell_ids, 77, 9, &target).unwrap();

    // This edit allocates/changes one surface tile in the adjacent tile column.
    stack.layers[0].surface.write_region(Rect::new(270, 4, 273, 7), &[0.0, 0.0, 0.0, 1.0].repeat(9).as_slice());
    renderer.render_cached(&device, &queue, &stack, size, &shell_ids, 77, 9, &target).unwrap();
    let incremental = read_gray(&device, &queue, &target, size);

    let fresh = create_test_target(&device, size);
    let mut fresh_renderer = MaskStackRenderer::new(&device);
    fresh_renderer.render_cached(&device, &queue, &stack, size, &shell_ids, 77, 9, &fresh).unwrap();
    let full = read_gray(&device, &queue, &fresh, size);
    assert_eq!(incremental.len(), full.len());
    for (index, (incremental, full)) in incremental.iter().zip(full.iter()).enumerate() {
        assert!((incremental - full).abs() < 0.001, "pixel {index}: incremental={incremental}, fresh={full}");
    }
}

#[test]
fn paint_only_change_recomposites_dirty_region_and_matches_full_render() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("skipping dirty mask-stack test: no adapter");
        return;
    };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {
        eprintln!("skipping dirty mask-stack test: no device");
        return;
    };
    let size = [300, 12];
    let mut stack = Stack {
        base: Surface::with_default(PixelFormat::GRAY8, &[0.5]),
        layers: vec![Paint {
            name: "Paint".into(),
            visible: true,
            opacity: 1.0,
            blend: photocraft_color::BlendMode::Normal,
            surface: Surface::new(PixelFormat::RGBA8),
            fill_map: None,
            fill_map_size: [0, 0],
            fill_bounds: None,
            kind: EffectKind::Paint,
        }],
        active: 0,
    };
    let shell_ids = vec![1; (size[0] * size[1]) as usize];
    let output = create_test_target(&device, size);
    let mut renderer = MaskStackRenderer::new(&device);
    renderer.render_cached(&device, &queue, &stack, size, &shell_ids, 78, 9, &output).unwrap();
    stack.layers[0].surface.write_region(Rect::new(270, 4, 273, 7), &[0.0, 0.0, 0.0, 1.0].repeat(9).as_slice());
    renderer.render_cached(&device, &queue, &stack, size, &shell_ids, 78, 9, &output).unwrap();
    let incremental = read_gray(&device, &queue, &output, size);

    let expected = create_test_target(&device, size);
    let mut full_renderer = MaskStackRenderer::new(&device);
    full_renderer.render_cached(&device, &queue, &stack, size, &shell_ids, 78, 9, &expected).unwrap();
    let full = read_gray(&device, &queue, &expected, size);
    for (index, (incremental, full)) in incremental.iter().zip(full.iter()).enumerate() {
        assert!((incremental - full).abs() < 0.001, "pixel {index}: partial={incremental}, full={full}");
    }
}

#[test]
fn unchanged_stack_reuses_the_caller_target_without_full_replay() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("skipping unchanged mask-stack test: no adapter");
        return;
    };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {
        eprintln!("skipping unchanged mask-stack test: no device");
        return;
    };
    let size = [8, 8];
    let stack = Stack { base: Surface::with_default(PixelFormat::GRAY8, &[0.4]), layers: Vec::new(), active: 0 };
    let shells = vec![1; (size[0] * size[1]) as usize];
    let output = create_test_target(&device, size);
    let mut renderer = MaskStackRenderer::new(&device);
    renderer.render_cached(&device, &queue, &stack, size, &shells, 901, 1, &output).unwrap();
    let sentinel = half::f16::from_f32(0.9375).to_le_bytes();
    let alpha = half::f16::from_f32(1.0).to_le_bytes();
    let pixel = [sentinel[0], sentinel[1], 0, 0, 0, 0, alpha[0], alpha[1]];
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &output, mip_level: 0, origin: wgpu::Origin3d { x: 3, y: 2, z: 0 }, aspect: wgpu::TextureAspect::All },
        &pixel,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(8), rows_per_image: Some(1) },
        wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
    );
    renderer.render_cached(&device, &queue, &stack, size, &shells, 901, 1, &output).unwrap();
    let actual = read_gray(&device, &queue, &output, size);
    assert!((actual[2 * size[0] as usize + 3] - 0.9375).abs() < 0.001);
}

#[test]
fn paint_above_blur_reuses_blur_prefix_and_matches_full_render() {
    let _lock = GPU_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("skipping Blur-prefix mask-stack test: no adapter");
        return;
    };
    let Ok((device, queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else {
        eprintln!("skipping Blur-prefix mask-stack test: no device");
        return;
    };
    let size = [64, 32];
    let mut seed = Surface::new(PixelFormat::RGBA8);
    seed.write_region(Rect::new(24, 10, 32, 18), &[0.0, 0.0, 0.0, 1.0].repeat(64).as_slice());
    let layer = |name: &str, surface, kind| Paint {
        name: name.into(),
        visible: true,
        opacity: 1.0,
        blend: photocraft_color::BlendMode::Normal,
        surface,
        fill_map: None,
        fill_map_size: [0, 0],
        fill_bounds: None,
        kind,
    };
    let mut stack = Stack {
        base: Surface::with_default(PixelFormat::GRAY8, &[0.25]),
        layers: vec![
            layer("Lower Blur", Surface::new(PixelFormat::RGBA8), EffectKind::Blur { radius: 3 }),
            layer("Paint", seed, EffectKind::Paint),
            layer("Upper Blur", Surface::new(PixelFormat::RGBA8), EffectKind::Blur { radius: 2 }),
        ],
        active: 1,
    };
    let shell_ids = vec![1; (size[0] * size[1]) as usize];
    let output = create_test_target(&device, size);
    let mut renderer = MaskStackRenderer::new(&device);
    renderer.render_cached(&device, &queue, &stack, size, &shell_ids, 79, 9, &output).unwrap();
    stack.layers[1].surface.write_region(Rect::new(42, 8, 45, 11), &[0.0, 0.0, 0.0, 1.0].repeat(9).as_slice());
    renderer.render_cached(&device, &queue, &stack, size, &shell_ids, 79, 9, &output).unwrap();
    let incremental = read_gray(&device, &queue, &output, size);

    let expected = create_test_target(&device, size);
    let mut full_renderer = MaskStackRenderer::new(&device);
    full_renderer.render_cached(&device, &queue, &stack, size, &shell_ids, 79, 9, &expected).unwrap();
    let full = read_gray(&device, &queue, &expected, size);
    for (index, (incremental, full)) in incremental.iter().zip(full.iter()).enumerate() {
        assert!((incremental - full).abs() < 0.001, "pixel {index}: resumed={incremental}, full={full}");
    }
}

#[test]
fn mask_stack_configuration_key_ignores_paint_pixels_but_tracks_effect_settings() {
    let mut stack = Stack {
        base: Surface::with_default(PixelFormat::GRAY8, &[1.0]),
        layers: vec![Paint {
            name: "Paint".into(),
            visible: true,
            opacity: 1.0,
            blend: photocraft_color::BlendMode::Normal,
            surface: Surface::new(PixelFormat::RGBA8),
            fill_map: None,
            fill_map_size: [0, 0],
            fill_bounds: None,
            kind: EffectKind::Paint,
        }],
        active: 0,
    };
    let initial = MaskStackRenderer::configuration_key(&stack);
    stack.layers[0].surface.write_region(Rect::new(0, 0, 1, 1), &[0.0, 0.0, 0.0, 1.0]);
    assert_eq!(MaskStackRenderer::configuration_key(&stack), initial, "unsupported GPU stacks should not be retried for each changed paint tile");
    stack.layers[0].opacity = 0.5;
    assert_ne!(MaskStackRenderer::configuration_key(&stack), initial, "layer setting edits must invalidate the negative cache");
}

fn create_test_target(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("mask-stack-incremental-test-output"),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

fn read_gray(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture, size: [u32; 2]) -> Vec<f32> {
    let row = (size[0] * 8).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mask-stack-incremental-test-readback"),
        size: u64::from(row * size[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("mask-stack-incremental-test-readback") });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(size[1]) },
        },
        wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
        let _ = send.send(result);
    });
    let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    receive.recv().unwrap().unwrap();
    let mapped = buffer.slice(..).get_mapped_range().unwrap();
    let values = (0..(size[0] * size[1]) as usize)
        .map(|index| {
            let x = index % size[0] as usize;
            let y = index / size[0] as usize;
            let offset = y * row as usize + x * 8;
            half::f16::from_le_bytes([mapped[offset], mapped[offset + 1]]).to_f32()
        })
        .collect();
    drop(mapped);
    buffer.unmap();
    values
}
