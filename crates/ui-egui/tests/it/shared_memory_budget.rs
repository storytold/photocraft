//! Where the GPU shares system memory (Apple silicon, integrated graphics), resident layer pages
//! are a second copy of the layers in the memory the document already holds (#9007). The same
//! canvas is driven as both kinds of adapter (`set_shares_system_memory` stands in for the
//! detection): a lowered budget frees the pages it no longer covers at once only where memory is
//! shared, and the CPU redraw a large document gets there draws what the GPU would. Skips when
//! there is no GPU adapter.

use eframe::egui_wgpu::RenderState;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, Size};
use photocraft_geom::Rect;
use photocraft_testkit::gpu_lock;
use photocraft_ui_egui::gpu_canvas::{self, GpuCanvas};

fn render_state() -> Option<RenderState> {
    let rs =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| egui_kittest::wgpu::create_render_state(gpu_canvas::wgpu_setup(), Default::default()))).ok();
    if rs.is_none() {
        eprintln!("skipping: no GPU adapter");
    }
    rs
}

/// `n` full-canvas layers of seeded noise, partly transparent so every layer shows.
fn noise_doc(w: u32, h: u32, n: u32) -> Document {
    let mut d = Document::new("t", Size::new(w, h), ColorMode::Rgb, SampleType::U8);
    for k in 0..n {
        let mut l = Layer::raster(format!("noise {k}"), PixelFormat::RGBA8);
        let mut state = 0x9e37_79b9_u32.wrapping_mul(k + 1);
        let bytes: Vec<u8> = (0..w as usize * h as usize * 4)
            .map(|i| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                // Alpha between 96 and 255: no layer covers the ones below it.
                if i % 4 == 3 { 96 + (state % 160) as u8 } else { (state >> 8) as u8 }
            })
            .collect();
        l.surface_mut().unwrap().write_interleaved(Rect::from_xywh(0, 0, w, h), &bytes);
        d.layers.push(l);
    }
    d
}

fn resident(g: &GpuCanvas) -> u64 {
    g.compositor_bytes().map_or(0, |(pages, _)| pages)
}

fn texels(g: &GpuCanvas, d: &Document) -> Vec<[f32; 4]> {
    g.read_texels(d.id.0).unwrap().2
}

fn max_diff(a: &[[f32; 4]], b: &[[f32; 4]]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).flat_map(|(p, q)| p.iter().zip(q).map(|(x, y)| (x - y).abs())).fold(0.0, f32::max)
}

#[test]
fn a_lowered_budget_frees_pages_at_once_only_where_the_gpu_shares_memory() {
    let _gpu = gpu_lock();
    let Some(rs) = render_state() else { return };
    let g = GpuCanvas::new(&rs);
    let small = noise_doc(600, 400, 4);
    let large = noise_doc(900, 700, 6);
    // One page cell holds a layer of either document whole, so a megabyte holds none of them.
    let (roomy, tight) = (1u64 << 30, 1u64 << 20);

    // Pricing before the compositor exists matches what it then keeps resident.
    let priced = g.layer_working_set(&small);
    g.set_shares_system_memory(false);
    g.set_memory_budget(roomy);
    assert_eq!(g.refresh(small.id.0, &small, None, None).kind, "gpu-full");
    assert_eq!(resident(&g), priced, "every page of the small document is resident");
    assert_eq!(g.measure_fit(&small).unwrap().needed, priced);

    // A graphics card's own memory (today's behaviour): the large document's full refresh goes
    // to the CPU, and the small one's pages stay until a render needs the room.
    g.set_memory_budget(tight);
    let r = g.refresh(large.id.0, &large, None, None);
    assert_eq!((r.kind, r.fallback.as_ref().map(photocraft_ui_egui::render_path::CpuReason::kind)), ("full", Some("overBudget")));
    assert_eq!(resident(&g), priced, "a discrete card keeps its pages");
    let cpu = texels(&g, &large);

    // Shared memory: the same lowered budget frees them at once.
    g.set_shares_system_memory(true);
    assert!(g.shares_system_memory());
    g.set_memory_budget(roomy);
    assert_eq!(g.refresh(small.id.0, &small, None, None).kind, "gpu-full");
    assert_eq!(resident(&g), priced);
    g.set_memory_budget(tight);
    let r = g.refresh(large.id.0, &large, None, None);
    assert_eq!((r.kind, r.fallback.as_ref().map(photocraft_ui_egui::render_path::CpuReason::kind)), ("full", Some("overBudget")));
    assert!(resident(&g) <= tight, "{} bytes of pages left over a {tight}-byte budget", resident(&g));
    assert_eq!(texels(&g, &large), cpu, "the CPU redraw is the same either way");

    // An edit in the view stays on the GPU under the small budget, and matches the CPU.
    let mut edited = large.clone();
    let dab = Rect::from_xywh(400, 300, 60, 40);
    edited.layers[2].surface_mut().unwrap().fill_rect(dab, &[0.9, 0.2, 0.1, 1.0]);
    assert_eq!(g.refresh(edited.id.0, &edited, Some(dab), None).kind, "gpu-rect");
    let gpu_edit = texels(&g, &edited);

    // With room for every page the whole document draws on the GPU, within 1/255 of the CPU.
    g.set_memory_budget(roomy);
    assert_eq!(g.refresh(edited.id.0, &edited, None, None).kind, "gpu-full");
    let gpu = texels(&g, &edited);
    g.set_memory_budget(tight);
    assert_eq!(g.refresh(edited.id.0, &edited, None, None).kind, "full");
    let cpu_edit = texels(&g, &edited);
    assert!(max_diff(&gpu, &cpu_edit) <= 1.0 / 255.0 + 1e-6, "GPU vs CPU {}", max_diff(&gpu, &cpu_edit));
    assert!(max_diff(&gpu_edit, &cpu_edit) <= 1.0 / 255.0 + 1e-6, "GPU edit vs CPU {}", max_diff(&gpu_edit, &cpu_edit));
    g.release();
}

/// GPU Memory set by hand keeps today's behaviour where memory is shared too: a lowered budget
/// leaves the pages resident until a render needs the room, as on a card with its own memory.
#[test]
fn gpu_memory_set_by_hand_keeps_todays_behaviour_on_shared_memory() {
    let _gpu = gpu_lock();
    let Some(rs) = render_state() else { return };
    let g = GpuCanvas::new(&rs);
    let small = noise_doc(600, 400, 4);
    let large = noise_doc(900, 700, 6);
    let (roomy, tight) = (1u64 << 30, 1u64 << 20);
    let priced = g.layer_working_set(&small);
    g.set_shares_system_memory(true);
    g.set_gpu_memory_by_hand(true);
    g.set_memory_budget(roomy);
    assert_eq!(g.refresh(small.id.0, &small, None, None).kind, "gpu-full");
    assert_eq!(resident(&g), priced);
    g.set_memory_budget(tight);
    let r = g.refresh(large.id.0, &large, None, None);
    assert_eq!((r.kind, r.fallback.as_ref().map(photocraft_ui_egui::render_path::CpuReason::kind)), ("full", Some("overBudget")));
    assert_eq!(resident(&g), priced, "GPU Memory set by hand keeps its pages until a render needs the room");
    // Automatic again: the next lowered budget frees them at once.
    g.set_gpu_memory_by_hand(false);
    g.set_memory_budget(roomy);
    assert_eq!(g.refresh(small.id.0, &small, None, None).kind, "gpu-full");
    g.set_memory_budget(tight);
    assert_eq!(g.refresh(large.id.0, &large, None, None).kind, "full");
    assert!(resident(&g) <= tight, "{} bytes of pages left over a {tight}-byte budget", resident(&g));
    g.release();
}

/// The app's budget (`budget_for`) is today's on a card with its own memory, and where memory is
/// shared, a small document keeps it too; only one too large to copy gets the edit budget
/// (`compositor_budget` covers the thresholds).
#[test]
fn the_apps_budget_follows_the_adapter_kind() {
    let _gpu = gpu_lock();
    let Some(rs) = render_state() else { return };
    let g = GpuCanvas::new(&rs);
    let d = noise_doc(300, 200, 2);
    let (allowance, pixels) = (8u64 << 30, 1u64 << 20);
    let today = gpu_canvas::memory_budget(allowance, pixels, gpu_canvas::physical_memory(), g.video_memory(), 0);
    for shared in [false, true] {
        g.set_shares_system_memory(shared);
        assert_eq!(g.budget_for(&d, allowance, pixels, 0), today, "shared {shared}: a small document");
        assert_eq!(
            g.budget_for(&d, allowance, pixels, 3 << 30),
            gpu_canvas::memory_budget(allowance, pixels, gpu_canvas::physical_memory(), g.video_memory(), 3 << 30)
        );
    }
    // What it prices on a shared adapter: the display set and the layer pages.
    let expect = gpu_canvas::compositor_budget(true, allowance, u64::MAX / 8, gpu_canvas::physical_memory(), g.video_memory(), 0, || gpu_canvas::DocGpuBytes {
        display: g.display_bytes(&d),
        layers: g.layer_working_set(&d),
        cell: g.cell_working_set(&d),
    });
    assert_eq!(g.budget_for(&d, allowance, u64::MAX / 8, 0), expect);
    assert_eq!(expect, gpu_canvas::SHARED_EDIT_BUDGET, "pixels beyond any machine's sixth");
    g.set_shares_system_memory(false);
    assert_eq!(
        g.budget_for(&d, allowance, u64::MAX / 8, 0),
        gpu_canvas::memory_budget(allowance, u64::MAX / 8, gpu_canvas::physical_memory(), g.video_memory(), 0)
    );
    g.release();
}
