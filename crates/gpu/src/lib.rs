//! GPU compositor (architecture §7.2, milestone M5).
//!
//! Renders a [`Document`] with wgpu, producing the same straight-alpha composite as the CPU
//! reference (`photocraft-compose`) within ~1/255:
//!
//! - **Residency.** Layer and mask surfaces live on the GPU in square *pages* (up to [`PAGE`]²,
//!   never above the device's texture limit) covering their allocated tiles. A page is uploaded
//!   only when a chunk inside it is drawn, and surfaces are copy-on-write `Arc` tiles, so a tile
//!   is re-uploaded only when its `Arc` changed (a brush stroke uploads just the touched 256²
//!   tiles; undo swaps pointers back and uploads only what differs). RGBA8 tiles upload with
//!   zero conversion. Pages over a byte budget are evicted least recently used first.
//! - **Planner** ([`plan`]). The layer tree becomes a linear list of passes over abstract
//!   chunk-sized buffers (blend with every Photoshop mode, opacity × fill, masks, clipping groups,
//!   pass-through vs isolated groups, adjustments, solid / gradient fills, layer effects).
//! - **Layer effects** (`fx.rs`). Each effect layer's maps (shadow, glow, satin, bevel, stroke
//!   bands) are built by fragment passes over its effect region and cached on the GPU per layer
//!   state; a brush stroke rebuilds only the damaged tiles grown by the effect reach. A region
//!   larger than the texture limit is built per page cell, over the cell grown by the effects'
//!   reach, so every map is exact inside its cell.
//! - **Execution.** The canvas is processed page cell by page cell, each in chunks (1024²
//!   RGBA32F accumulators, reused) that bind only that cell's pages, and each finished chunk is
//!   handed to a caller-supplied sink, e.g. to encode it straight into a display texture — no
//!   readback. Documents of any size composite on the GPU; a huge refresh submits as it goes, so
//!   staged uploads and evicted pages stay bounded.
//! - **Stack reuse.** A complete root stack unit can checkpoint its backdrop into bounded chunk
//!   textures. Exact COW snapshots invalidate the lower prefix; edits above it restore those
//!   pixels and execute only the remaining passes, capturing on the way the highest checkpoint
//!   below the edit, so repeating it (a duplicate or paste and its undo) skips that much more.
//!   Groups and clipping units stay indivisible.
//!
//! Vector masks (rasterised once per mask state into a combined mask texture), layers clipped to
//! pass-through groups, stroked shapes with clipped layers (fill and stroke split once per shape
//! state), pattern fills and artboards are planned like everything else. What remains
//! (Multichannel documents, patterns larger than the texture limit, uncached gradients with
//! coincident colour or opacity stops) returns [`Unsupported`];
//! callers fall back to the CPU compositor.
//!
//! [`CameraRawDevelop`] (`camera_raw.rs`) is a separate pass, not part of the compositor: Camera
//! Raw's per-pixel develop stages for the dialog's preview, built the first time the dialog opens.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod bounds;
mod camera_raw;
mod fx;
pub mod health;
pub mod plan;
mod prefix;
pub mod timing;

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::Arc;

use photocraft_color::{PixelFormat, SampleType};
use photocraft_compose::effects::FieldKind;
use photocraft_doc::{DocId, Document, LayerContent, LayerId, Pattern};
use photocraft_geom::{Rect, TILE_SIZE, TileCoord};
use photocraft_raster::{Surface, Tile};

pub use camera_raw::{CAMERA_RAW_SHADER, CameraRawDevelop};
pub use health::{DeviceHealth, Fault};
pub use plan::{Kernel, Plan, Role, Unsupported, plan};
pub use timing::{GpuTimer, GpuTiming};

/// GPU compositor. Create once per device; call [`Compositor::render`] per refresh.
pub struct Compositor {
    kit: Kit,
    pool: Vec<(wgpu::Texture, wgpu::TextureView)>,
    residents: HashMap<ResKey, Resident>,
    fx: HashMap<FxKey, FxEntry>,
    prefix: prefix::Prefix,
    /// Effect temporaries (R32F, region sized) and the frame they were last used.
    temps: Vec<(Tex, u64)>,
    patterns: HashMap<String, CachedPattern>,
    /// The colour mode of the document being encoded: patterns paint in it
    /// (`photocraft_compose::pattern::pattern_rgba`).
    pattern_mode: photocraft_color::ColorMode,
    /// The document being encoded, which owns the patterns it paints with.
    pattern_doc: DocId,
    /// GPU memory pattern textures may hold ([`PATTERN_BUDGET`] by default).
    pattern_budget: u64,
    /// Texture side limit the paging is planned for (the device's, or smaller in tests).
    max_dim: u32,
    /// The device's real texture side limit.
    device_max: u32,
    /// Page side (a power of two, multiple of the tile size, at most `max_dim`).
    page: u32,
    /// Chunk side (divides `page`).
    chunk: u32,
    resident_budget: u64,
    /// GPU memory the effect-map cache may hold ([`FX_BUDGET`] or less).
    fx_budget: u64,
    /// Layer-page bytes the last full fit check asked for ([`Compositor::fit_layers`]).
    layer_need: u64,
    /// Bytes of `fx_budget` lent to `resident_budget` so those pages fit.
    lent: u64,
    /// `resident_budget` was set exactly ([`Compositor::set_resident_budget`]): nothing is lent.
    exact_resident: bool,
    /// What the view shows: a full refresh draws these cells last, so their pages are the
    /// ones still resident for the edits that follow.
    focus: Option<Rect>,
    frame: u64,
    /// Cell visits so far (LRU clock of pages).
    stamp: u64,
    /// Bytes staged by uploads since the last submit.
    staged: u64,
    /// Copies and render passes recorded since the last submit.
    recorded: u32,
    /// Why a layer page texture couldn't be created this refresh (it then ends on the CPU).
    layer_alloc: Option<String>,
    /// Bytes of pages dropped to admit new ones, not yet counted towards an intermediate submit.
    admit_freed: u64,
    /// Test hook ([`Compositor::fail_layer_allocs`]): how many more layer page textures fail as
    /// out of memory.
    fail_allocs: u32,
    acc_format: wgpu::TextureFormat,
    /// Whether the effect-map pipelines could be built (else documents with layer effects are
    /// [`Unsupported`] and use the CPU compositor).
    effect_maps: bool,
    /// `CmykSpace::id` of the document being encoded (0: built-in coated CMYK).
    cmyk: u64,
    /// The device's health (see [`Compositor::set_health`]); `None` = assumed healthy.
    health: Option<DeviceHealth>,
    /// Timestamp queries measuring how long the GPU spends on a refresh (see
    /// [`Compositor::measure_gpu_time`]); `None` = not measured.
    timer: Option<timing::GpuTimer>,
}
use W::{F, I, U};

mod batch;
mod cells;
mod consts;
mod convert;
mod encode;
mod fx_passes;
mod kit;
mod paging;
#[cfg(not(target_arch = "wasm32"))]
mod readback;
mod setup;
mod textures;

#[cfg(test)]
use batch::{BATCH, batch_kernel, batch_op_words, batch_runs, op_words, run_starts, tile_grid};
#[cfg(test)]
use consts::OP_UNIFORM;
pub use consts::{ACC_FORMAT, CHUNK, FX_BUDGET, PAGE, PATTERN_BUDGET, PATTERN_IDLE_FRAMES, RESIDENT_BUDGET};
use convert::W;
#[cfg(test)]
use convert::{CONVERT_BATCH, colour_managed, convert_tile, convert_tiles, rgb_gray16_to_f16, rgb_gray32_to_f16};
pub use convert::{f16_to_f32, f32_to_f16};
pub use kit::SHADER;
use kit::{Kit, tex_entry};
pub use paging::OVER_BUDGET;
use paging::first_error;
#[cfg(test)]
use paging::{lru_victims, uniform_buffer_len};
#[cfg(not(target_arch = "wasm32"))]
pub use readback::{render_to_vec, render_to_vec_stats};
#[cfg(test)]
use textures::TexKind;
use textures::{CachedPattern, FxEntry, FxKey, ResKey, Resident, Tex};
pub use textures::{ChunkOut, Stats};

    #[test]
    fn uniform_buffer_sizes_are_checked_before_allocation() {
        assert_eq!(uniform_buffer_len(1, 255, 65_536).unwrap(), 65_536);
        assert!(uniform_buffer_len(1, 255, 65_535).is_err());
        assert!(uniform_buffer_len(usize::MAX, 1, u64::MAX).is_err());
        assert!(uniform_buffer_len(usize::MAX, 0, u64::MAX).is_err());
        assert!(uniform_buffer_len(1, 16_777_216, u64::MAX).is_err(), "the last record offset cannot be represented by u32");
        #[cfg(target_pointer_width = "64")]
        assert_eq!(uniform_buffer_len(1, 16_777_215, u64::MAX).unwrap(), 4_294_967_296, "the last valid u32 dynamic offset is accepted");
    }

    #[test]
    fn half_roundtrip() {
        for v in [0.0f32, 1.0, 0.5, 0.25, 1.0 / 255.0, 0.333, 65504.0, -2.5, 1e-6] {
            let r = f16_to_f32(f32_to_f16(v));
            assert!((r - v).abs() <= v.abs() / 1024.0 + 1e-7, "{v} -> {r}");
        }
    }

    #[test]
    fn shader_validates() {
        let module = wgpu::naga::front::wgsl::parse_str(SHADER).unwrap_or_else(|e| panic!("{}", e.emit_to_string(SHADER)));
        wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::empty())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{e:?}"));
        for k in Kernel::DRAWN {
            let e = k.entry().unwrap();
            assert!(module.entry_points.iter().any(|ep| ep.name == e), "missing entry point {e}");
        }
    }

    /// The decimal float literals in `src` (comments skipped; hex literals are exact, so skipped),
    /// with their line numbers.
    fn decimal_float_literals(src: &str) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        for (n, line) in src.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("").as_bytes();
            let mut i = 0;
            while i < code.len() {
                let digit_or_dot = code[i].is_ascii_digit() || (code[i] == b'.' && code.get(i + 1).is_some_and(u8::is_ascii_digit));
                let word_before = i > 0 && (code[i - 1].is_ascii_alphanumeric() || code[i - 1] == b'_' || code[i - 1] == b'.');
                if !digit_or_dot || word_before {
                    i += 1;
                    continue;
                }
                if code[i] == b'0' && matches!(code.get(i + 1), Some(b'x' | b'X')) {
                    i += 2;
                    while i < code.len() && (code[i].is_ascii_hexdigit() || matches!(code[i], b'.' | b'p' | b'P' | b'+' | b'-')) {
                        i += 1;
                    }
                    continue;
                }
                let start = i;
                while i < code.len() && (code[i].is_ascii_digit() || code[i] == b'.') {
                    i += 1;
                }
                if i < code.len() && matches!(code[i], b'e' | b'E') {
                    let mut j = i + 1;
                    if j < code.len() && matches!(code[j], b'+' | b'-') {
                        j += 1;
                    }
                    if j < code.len() && code[j].is_ascii_digit() {
                        i = j;
                        while i < code.len() && code[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                out.push((n + 1, String::from_utf8_lossy(&code[start..i]).into_owned()));
            }
        }
        out
    }

    /// Browsers' WebGPU shader compiler (Tint) rejects a float literal beyond f32's finite range
    /// even when it would round to `f32::MAX` (naga, used natively, rounds it), and then no
    /// compositor pipeline builds and the web app falls back to the CPU renderer. `3.40282347e38`
    /// was such a literal.
    #[test]
    fn shader_float_literals_fit_f32() {
        let lits = decimal_float_literals(SHADER);
        assert!(lits.iter().any(|(_, l)| l == "1.0"), "the scanner finds literals: {lits:?}");
        let bad: Vec<_> = lits.iter().filter(|(_, l)| l.parse::<f64>().map_or(true, |v| !v.is_finite() || v.abs() > f64::from(f32::MAX))).collect();
        assert!(bad.is_empty(), "float literals outside f32's finite range (line, literal): {bad:?}");
    }

    /// FXC (the D3D12 shader compiler Windows uses) mis-analyses a function whose `switch` case
    /// returns from inside an `if`: when it inlines the function it concludes that a path can
    /// reach the end of the function without a value and rejects it with X3507 ("not all control
    /// paths return a value"). The pipeline then fails to build, the compositor falls back to the
    /// CPU renderer and redrawing a large document takes seconds. Every function keeps one exit:
    /// the cases assign, and the function returns once at the end.
    #[test]
    fn shader_switch_cases_do_not_return_early() {
        let module = wgpu::naga::front::wgsl::parse_str(SHADER).unwrap_or_else(|e| panic!("{}", e.emit_to_string(SHADER)));
        let mut bad = Vec::new();
        for (_handle, fun) in module.functions.iter() {
            let mut risky = Vec::new();
            early_returns_in_cases(&fun.body, &mut risky);
            if !risky.is_empty() {
                bad.push(format!("{}: {risky:?}", fun.name.clone().unwrap_or_default()));
            }
        }
        assert!(bad.is_empty(), "switch cases that return from inside an `if`: {bad:?}");
    }

    /// The cases of every switch in `block` that return from inside an `if` or a loop, with the
    /// function's name for the message.
    fn early_returns_in_cases(block: &[wgpu::naga::Statement], out: &mut Vec<String>) {
        for stmt in block {
            match stmt {
                wgpu::naga::Statement::Switch { cases, .. } => {
                    for case in cases {
                        if returns_in(&case.body).1 > 0 {
                            out.push(format!("case {:?}", case.value));
                        }
                        let mut nested = Vec::new();
                        early_returns_in_cases(&case.body, &mut nested);
                        out.append(&mut nested);
                    }
                }
                wgpu::naga::Statement::If { accept, reject, .. } => {
                    early_returns_in_cases(accept, out);
                    early_returns_in_cases(reject, out);
                }
                wgpu::naga::Statement::Block(body) => early_returns_in_cases(body, out),
                wgpu::naga::Statement::Loop { body, .. } => early_returns_in_cases(body, out),
                _ => {}
            }
        }
    }

    /// The returns a block holds: the ones it returns directly, and the ones nested inside an
    /// `if` or a loop (the ones FXC mis-analyses when it inlines the function).
    fn returns_in(block: &[wgpu::naga::Statement]) -> (usize, usize) {
        let mut direct = 0;
        let mut nested = 0;
        for stmt in block {
            match stmt {
                wgpu::naga::Statement::Return { .. } => direct += 1,
                wgpu::naga::Statement::If { accept, reject, .. } => {
            for part in [accept, reject] {
                let (d, n) = returns_in(part);
                nested += d + n;
            }
        }
        wgpu::naga::Statement::Block(body) => {
            let (d, n) = returns_in(body);
            direct += d;
            nested += n;
        }
        wgpu::naga::Statement::Loop { body, .. } => {
            let (d, n) = returns_in(body);
            nested += d + n;
        }
                _ => {}
            }
        }
        (direct, nested)
    }

    #[test]
    fn op_record_fits_the_uniform() {
        let p = plan::Pass::new(Kernel::FxPaint, 0);
        assert_eq!(op_words(&p, None, None, None).len() as u64, OP_UNIFORM);
    }

    /// #6534: a 16-bit mask (16-bit documents, feathered or vector combined masks) costs two
    /// bytes per pixel on the GPU, twice an 8-bit mask, not four; 32-bit masks stay R32Float.
    #[test]
    fn sixteen_bit_masks_upload_at_two_bytes_per_pixel() {
        let gray = |sample| PixelFormat { sample, ..PixelFormat::GRAY8 };
        assert_eq!(TexKind::for_surface(Role::Mask, PixelFormat::GRAY8).bytes_per_pixel(), 1);
        let k16 = TexKind::for_surface(Role::Mask, gray(SampleType::U16));
        assert_eq!(k16.bytes_per_pixel(), 2, "{k16:?}");
        assert_eq!(k16.format().block_copy_size(None), Some(2), "{k16:?}");
        let k32 = TexKind::for_surface(Role::Mask, gray(SampleType::F32));
        assert_eq!(k32.format(), wgpu::TextureFormat::R32Float);
        // The texels hold the mask values (a half each), the default pixel included.
        let mut s = Surface::with_default(gray(SampleType::U16), &[1.0]);
        s.fill_rect(Rect::new(0, 0, 4, 1), &[0.25]);
        let c = TileCoord::new(0, 0);
        let bytes = convert_tile(&s, s.tile(c), k16, c);
        assert_eq!(bytes.len(), (TILE_SIZE * TILE_SIZE) as usize * 2);
        assert_eq!(bytes.get(..2), Some(f32_to_f16(0.25).to_le_bytes().as_slice()));
        assert_eq!(bytes.get(8..10), Some(f32_to_f16(1.0).to_le_bytes().as_slice()));
        assert_eq!(convert_tile(&s, None, k16, c).get(..2), Some(f32_to_f16(1.0).to_le_bytes().as_slice()));
    }

    /// #1774: a solid fill shares one tile across every coordinate; it is converted once, and
    /// every coordinate (shared or not) still gets exactly the bytes `convert_tile` gives it, at
    /// every depth and texture kind.
    #[test]
    fn convert_tiles_converts_each_distinct_tile_once() {
        let area = Rect::new(0, 0, TILE_SIZE * 9, TILE_SIZE * 9);
        for (fmt, kind) in [
            (PixelFormat::RGBA8, TexKind::Rgba8Direct),
            (PixelFormat { alpha: false, ..PixelFormat::RGBA8 }, TexKind::Rgba8),
            (PixelFormat::RGBA16, TexKind::Rgba16F),
            (PixelFormat::RGBA32F, TexKind::Rgba16F),
            (PixelFormat::GRAY8, TexKind::R8Direct),
            (PixelFormat { sample: SampleType::U16, ..PixelFormat::GRAY8 }, TexKind::R16F),
            (PixelFormat { sample: SampleType::F32, ..PixelFormat::GRAY8 }, TexKind::R32F),
        ] {
            let mut s = Surface::new(fmt);
            let px = [0.2, 0.4, 0.6, 1.0];
            let n = fmt.channels();
            // Fully covered tiles share one `Arc`; then two tiles get their own pixels.
            s.fill_rect(area, &px[..n]);
            s.fill_rect(Rect::new(3, 5, 40, 60), &[0.9, 0.1, 0.3, 0.5][..n]);
            s.fill_rect(Rect::new(TILE_SIZE * 4 + 7, TILE_SIZE * 2, TILE_SIZE * 4 + 9, TILE_SIZE * 2 + 3), &[0.0; 4][..n]);
            let tiles: Vec<_> = area.tiles().filter_map(|c| s.tile(c).map(|t| (c, t.clone()))).collect();
            assert_eq!(tiles.len(), 81, "{fmt:?}");
            let mut seen = Vec::new();
            let converted = convert_tiles(&s, tiles, kind, |t, cs, bytes| {
                for &c in cs {
                    assert!(s.tile(c).is_some_and(|own| Arc::ptr_eq(own, t)), "{fmt:?} {c:?}");
                    assert_eq!(bytes, convert_tile(&s, Some(t), kind, c).as_slice(), "{fmt:?} {c:?}");
                    seen.push(c);
                }
            });
            assert_eq!(converted, 3, "{fmt:?}: the shared tile once, plus the two edited ones");
            seen.sort_by_key(|c| (c.ty, c.tx));
            let mut want: Vec<_> = area.tiles().collect();
            want.sort_by_key(|c| (c.ty, c.tx));
            assert_eq!(seen, want, "{fmt:?}: every coordinate once");
        }
        // More distinct tiles than one batch: all converted, each coordinate once.
        let mut s = Surface::new(PixelFormat::RGBA16);
        let wide = Rect::new(0, 0, TILE_SIZE * (CONVERT_BATCH as i32 + 5), TILE_SIZE);
        for c in wide.tiles() {
            s.fill_rect(Rect::new(c.rect().x0, 0, c.rect().x0 + 1, 1), &[c.tx as f32 / 100.0, 0.0, 0.0, 1.0]);
        }
        let tiles: Vec<_> = wide.tiles().filter_map(|c| s.tile(c).map(|t| (c, t.clone()))).collect();
        let mut n = 0;
        let converted = convert_tiles(&s, tiles, TexKind::Rgba16F, |t, cs, bytes| {
            for &c in cs {
                assert_eq!(bytes, convert_tile(&s, Some(t), TexKind::Rgba16F, c).as_slice());
                n += 1;
            }
        });
        assert_eq!((converted, n), (CONVERT_BATCH + 5, CONVERT_BATCH + 5));
    }

    /// The 16-bit and 32-bit float RGB/gray fast paths give exactly the general path's half
    /// floats (32-bit samples above 1 and below 0 included).
    #[test]
    fn sixteen_and_thirty_two_bit_fast_paths_match_the_general_one() {
        for fmt in [
            PixelFormat::RGBA16,
            PixelFormat { alpha: false, ..PixelFormat::RGBA16 },
            PixelFormat { sample: SampleType::U16, ..PixelFormat::GRAY8 },
            PixelFormat { sample: SampleType::U16, alpha: true, ..PixelFormat::GRAY8 },
            PixelFormat::RGBA32F,
            PixelFormat { alpha: false, ..PixelFormat::RGBA32F },
            PixelFormat { sample: SampleType::F32, ..PixelFormat::GRAY8 },
            PixelFormat { sample: SampleType::F32, alpha: true, ..PixelFormat::GRAY8 },
        ] {
            let mut s = Surface::new(fmt);
            let r = Rect::new(0, 0, TILE_SIZE, TILE_SIZE);
            let n = (TILE_SIZE * TILE_SIZE) as usize * fmt.channels();
            let hdr = if fmt.sample == SampleType::F32 { 4.0 } else { 1.0 };
            let data: Vec<f32> = (0..n).map(|i| (i as u32).wrapping_mul(2_654_435_761) as f32 / u32::MAX as f32 * hdr - (hdr - 1.0) / 4.0).collect();
            s.write_region(r, &data);
            let t = s.tile(TileCoord::new(0, 0)).expect("written").clone();
            let fast = rgb_gray16_to_f16(&fmt, t.bytes()).or_else(|| rgb_gray32_to_f16(&fmt, t.bytes())).expect("fast path");
            let raw = s.read_region(r);
            let general: Vec<u8> =
                raw.chunks_exact(fmt.channels()).flat_map(|px| photocraft_raster::to_rgba(&fmt, px).map(|x| f32_to_f16(x).to_le_bytes())).flatten().collect();
            assert_eq!(fast, general, "{fmt:?}");
            assert_eq!(convert_tile(&s, Some(&t), TexKind::Rgba16F, TileCoord::new(0, 0)), general, "{fmt:?}");
        }
        assert!(rgb_gray16_to_f16(&PixelFormat::RGBA8, &[]).is_none());
        assert!(rgb_gray16_to_f16(&PixelFormat { sample: SampleType::U16, ..PixelFormat::CMYKA8 }, &[]).is_none());
        assert!(rgb_gray32_to_f16(&PixelFormat::RGBA16, &[]).is_none());
        assert!(rgb_gray32_to_f16(&PixelFormat { sample: SampleType::F32, ..PixelFormat::CMYKA8 }, &[]).is_none());
    }

    /// CMYK and Lab tiles convert in parallel bands of rows, each run of equal pixels once
    /// (#5108): the texels are still exactly the per-pixel conversion's, at 8 and 16 bits, with
    /// the built-in CMYK profile and with a document's own (which the worker threads enter).
    #[test]
    fn colour_managed_tiles_match_the_per_pixel_conversion() {
        use photocraft_cms::synth::{CmykParams, cmyk_profile};
        use photocraft_color::ColorMode;
        use photocraft_color::convert::{CmykSpace, with_cmyk_space};
        let icc = Arc::new(
            cmyk_profile(&CmykParams { description: "GPU test CMYK".into(), tvi: [0.26, 0.26, 0.26, 0.3], grid_a2b: 5, grid_b2a: 9, ..Default::default() })
                .to_bytes(),
        );
        let own = CmykSpace::for_profile(Some(&icc));
        assert!(own.is_some(), "the synthetic profile is not the built-in one");
        let area = Rect::new(0, 0, TILE_SIZE * 3, TILE_SIZE);
        for (mode, sample, space) in [
            (ColorMode::Cmyk, SampleType::U8, None),
            (ColorMode::Cmyk, SampleType::U16, None),
            (ColorMode::Cmyk, SampleType::U8, own.clone()),
            (ColorMode::Cmyk, SampleType::U16, own.clone()),
            (ColorMode::Lab, SampleType::U8, None),
            (ColorMode::Lab, SampleType::U16, None),
        ] {
            let fmt = PixelFormat { mode, sample, alpha: true };
            let kind = TexKind::for_surface(Role::Content, fmt);
            assert!(colour_managed(&fmt, kind), "{fmt:?}");
            let ch = fmt.channels();
            let mut s = Surface::new(fmt);
            // Tile 0: noise. Tile 1: transparent with a soft dab (runs of equal pixels). Tile 2:
            // a flat colour with a few lines of noise.
            let n = (TILE_SIZE * TILE_SIZE) as usize * ch;
            let noise: Vec<f32> = (0..n).map(|i| (i as u32).wrapping_mul(2_654_435_761) as f32 / u32::MAX as f32).collect();
            s.write_region(Rect::new(0, 0, TILE_SIZE, TILE_SIZE), &noise);
            let mut dab = [0.8, 0.6, 0.1, 0.05, 0.0];
            for r in (1..30).rev() {
                dab[ch - 1] = 1.0 - r as f32 / 30.0;
                s.fill_rect(Rect::new(TILE_SIZE + 100 - r, 90 - r, TILE_SIZE + 100 + r, 90 + r), &dab[..ch]);
            }
            s.fill_rect(Rect::new(TILE_SIZE * 2, 0, TILE_SIZE * 3, TILE_SIZE), &[0.3, 0.2, 0.7, 0.1, 1.0][..ch]);
            s.write_region(Rect::new(TILE_SIZE * 2, 40, TILE_SIZE * 3, 43), &noise[..TILE_SIZE as usize * 3 * ch]);
            let tiles: Vec<_> = area.tiles().filter_map(|c| s.tile(c).map(|t| (c, t.clone()))).collect();
            assert_eq!(tiles.len(), 3, "{fmt:?}");
            let mut seen = 0;
            with_cmyk_space(space.as_ref(), || {
                convert_tiles(&s, tiles, kind, |_, cs, bytes| {
                    for &c in cs {
                        // The old path: every pixel through `to_rgba` on this thread, in this scope.
                        let want: Vec<u8> = s
                            .read_region(c.rect())
                            .chunks_exact(ch)
                            .flat_map(|px| {
                                let v = photocraft_raster::to_rgba(&fmt, px);
                                match kind {
                                    TexKind::Rgba16F => v.iter().flat_map(|x| f32_to_f16(*x).to_le_bytes()).collect::<Vec<u8>>(),
                                    _ => v.map(|x| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).to_vec(),
                                }
                            })
                            .collect();
                        assert!(
                            bytes == want.as_slice(),
                            "{fmt:?} {c:?} with {}",
                            if space.is_some() { "the document's profile" } else { "the built-in profile" }
                        );
                        assert_eq!(convert_tile(&s, s.tile(c), kind, c), want, "{fmt:?} {c:?}");
                        seen += 1;
                    }
                });
            });
            assert_eq!(seen, 3, "{fmt:?}");
        }
    }

    #[test]
    fn grid_rounds_out() {
        assert_eq!(tile_grid(Rect::new(0, 0, 300, 256)), Rect::new(0, 0, 512, 256));
    }
}
