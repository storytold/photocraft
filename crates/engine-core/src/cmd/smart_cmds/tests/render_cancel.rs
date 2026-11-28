use super::*;
use crate::cmd::smart_cmds::rendering::{place_in, refresh_or_fail_in, render_with_warnings_in};
use crate::cmd::smart_cmds::source::{cache_get, flattened_source, flattened_source_with, source_cache_key, source_decode_count, source_surface_from_buffer};
use photocraft_doc::StackMode;
use photocraft_raster::Interrupt;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// Makes each source's bytes unique, so no render finds another one's cached source.
static NONCE: AtomicU32 = AtomicU32::new(1);

/// An interrupt that cancels from its `n`th check on (the first is 0), counting the checks.
struct CancelAt {
    n: usize,
    checks: AtomicUsize,
}

impl CancelAt {
    fn new(n: usize) -> Self {
        Self { n, checks: AtomicUsize::new(0) }
    }

    fn cancelled(&self) -> bool {
        self.checks.fetch_add(1, Ordering::Relaxed) >= self.n
    }
}

fn target() -> Document {
    Document::new("target", Size::new(48, 40), ColorMode::Rgb, SampleType::U8)
}

fn block(fmt: &PixelFormat, nonce: u32, r: Rect) -> Surface {
    let mut s = Surface::new(*fmt);
    let mut data = Vec::new();
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let v = ((x * 7 + y * 13) % 23) as f32 / 22.0;
            let first = if x == r.x0 && y == r.y0 { (nonce % 251) as f32 / 250.0 } else { v };
            data.extend(photocraft_raster::from_rgba(fmt, [first, 1.0 - v, 0.25, 1.0]));
        }
    }
    s.write_region(r, &data);
    s
}

/// A source document with `count` raster layers, unique to `nonce`.
fn layered(nonce: u32, count: i32) -> Document {
    let mut doc = Document::new(format!("source {nonce}"), Size::new(40, 32), ColorMode::Rgb, SampleType::U8);
    let fmt = doc.pixel_format();
    doc.layers = (0..count).map(|i| Layer::new(format!("layer {i}"), LayerContent::Raster(block(&fmt, nonce, Rect::new(i, 2, 30 + i, 28))))).collect();
    doc
}

fn svg(nonce: u32, shapes: usize) -> Vec<u8> {
    let rects: String = (0..shapes).map(|i| format!(r##"<rect x="{}" y="4" width="20" height="20" fill="#3080c0"/>"##, 2 + 6 * i)).collect();
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="32"><!-- {nonce} -->{rects}</svg>"#).into_bytes()
}

/// One smart object of each kind of source: a native document, PNG, PSB, SVG, Stack Mode over a
/// document and over an SVG, a document holding a smart object, and a scaled placement.
fn sources() -> Vec<(&'static str, String, Vec<u8>, SmartObject)> {
    let nonce = NONCE.fetch_add(1, Ordering::Relaxed);
    let doc = layered(nonce, 1);
    let export = |ext: &str| photocraft_io::export(&doc, &format!("source.{ext}"), &Default::default()).unwrap().bytes;
    let inner_bytes = encode_source(&layered(nonce, 1)).unwrap();
    let inner = SmartObject::new(
        SmartSource::Embedded { file_name: "inner.pcraft".into(), bytes: Arc::new(inner_bytes) },
        Affine::IDENTITY,
        Some(block(&doc.pixel_format(), nonce, Rect::new(0, 2, 30, 28))),
    );
    let mut nested = layered(nonce, 0);
    nested.layers = vec![Layer::new("inner", LayerContent::Smart(inner))].into();
    let scale = Affine { m: [1.5, 0.0, 0.0, 1.5, 3.25, 1.5] };
    type Kind<'a> = (&'static str, &'a str, Vec<u8>, Option<StackMode>, Affine);
    let kinds: Vec<Kind> = vec![
        ("native", "source.pcraft", encode_source(&doc).unwrap(), None, Affine::IDENTITY),
        ("png", "source.png", export("png"), None, Affine::IDENTITY),
        ("psb", "source.psb", export("psb"), None, Affine::IDENTITY),
        ("svg", "source.svg", svg(nonce, 1), None, Affine::IDENTITY),
        ("stack", "source.pcraft", encode_source(&layered(nonce, 3)).unwrap(), Some(StackMode::Median), Affine::IDENTITY),
        ("svg stack", "source.svg", svg(nonce, 3), Some(StackMode::Maximum), Affine::IDENTITY),
        ("nested", "source.pcraft", encode_source(&nested).unwrap(), None, Affine::IDENTITY),
        ("scaled", "source.pcraft", encode_source(&layered(nonce, 2)).unwrap(), None, scale),
    ];
    kinds
        .into_iter()
        .map(|(label, name, bytes, stack_mode, transform)| {
            let mut sm = SmartObject::new(SmartSource::Embedded { file_name: name.into(), bytes: Arc::new(bytes.clone()) }, transform, None);
            sm.stack_mode = stack_mode;
            (label, name.to_string(), bytes, sm)
        })
        .collect()
}

/// A cancel at any check while the source is decoded, drawn or placed stops the placement with
/// `Cancelled` and caches no partly decoded source; past the last check the placement finishes.
#[test]
fn a_cancel_at_any_check_stops_the_placement_of_every_source_type() {
    let doc = target();
    let fmt = doc.pixel_format();
    for kind in 0..sources().len() {
        let mut n = 0;
        let label = loop {
            let (label, name, bytes, sm) = sources().swap_remove(kind);
            let at = CancelAt::new(n);
            let cancel = || at.cancelled();
            let result = place_in(&name, &bytes, &doc, &sm, &Interrupt::cancel_only(&cancel));
            if at.checks.load(Ordering::Relaxed) <= n {
                let placed = result.unwrap_or_else(|e| panic!("{label}: {e}"));
                assert!(!placed.content_bounds().is_empty(), "{label}: nothing was placed");
                break label;
            }
            assert!(matches!(result, Err(EngineError::Cancelled)), "{label}: a cancel at check {n} gave {:?}", result.err());
            // A cancel after the source was decoded may keep the decoded source, never a part of it.
            // Read the entry the placement wrote, under its own key, without decoding on a miss.
            let key = source_cache_key(&name, &bytes, fmt, Some(photocraft_io::svg::is_svg(&bytes)));
            let entry = cache_get(&key);
            assert_eq!(entry.is_some(), source_decode_count(&bytes, fmt) > 0, "{label}: a cancel at check {n} cached the source under another key");
            if let Some(cached) = entry {
                let full = flattened_source(&decode_source(&name, &bytes).unwrap(), fmt).unwrap();
                assert_eq!(
                    cached.surface.read_region(cached.bounds),
                    full.read_region(cached.bounds),
                    "{label}: a cancel at check {n} cached a partial source"
                );
            }
            n += 1;
        };
        assert!(n >= 3, "{label}: the placement checks for a cancel only {n} times");
    }
}

/// A CMYK or Lab source, or one rendered for a CMYK or Lab document, is composited in bands, and a
/// cancel is seen between them; the bands give the same pixels as one composite.
#[test]
fn a_cmyk_or_lab_source_checks_for_a_cancel_between_bands() {
    for (mode, fmt) in [
        (ColorMode::Cmyk, PixelFormat::new(ColorMode::Cmyk, SampleType::U8, true)),
        (ColorMode::Lab, PixelFormat::new(ColorMode::Lab, SampleType::U8, true)),
        (ColorMode::Rgb, PixelFormat::new(ColorMode::Cmyk, SampleType::U8, true)),
    ] {
        // Three bands of 1 Mpx at most: 256, 256 and 8 rows.
        let mut doc = Document::new("bands", Size::new(4096, 520), mode, SampleType::U8);
        let layer_fmt = doc.pixel_format();
        doc.layers = vec![Layer::new("block", LayerContent::Raster(block(&layer_fmt, 3, Rect::new(10, 200, 4000, 300))))].into();
        let at = CancelAt::new(usize::MAX);
        let cancel = || at.cancelled();
        let banded = flattened_source_with(&doc, fmt, &Interrupt::cancel_only(&cancel)).unwrap();
        assert!(at.checks.load(Ordering::Relaxed) >= 4, "{mode:?}: {} checks", at.checks.load(Ordering::Relaxed));
        let whole = source_surface_from_buffer(&doc, &photocraft_compose::flatten(&doc), fmt).unwrap();
        assert!(banded.read_region(doc.bounds()) == whole.read_region(doc.bounds()), "{mode:?}: the bands differ from one composite");
        let at = CancelAt::new(2);
        let cancel = || at.cancelled();
        let cancelled = flattened_source_with(&doc, fmt, &Interrupt::cancel_only(&cancel));
        assert!(matches!(cancelled, Err(EngineError::Cancelled)), "{mode:?}: {:?}", cancelled.err());
    }
}

/// A job cancelled before its render starts gets `Cancelled` from the render the Place job and
/// the smart filter jobs use, decodes nothing, and leaves the smart object's layer as it was.
#[test]
fn a_cancelled_job_renders_nothing_and_leaves_the_layer_unchanged() {
    for (label, _, bytes, mut sm) in sources() {
        let mut doc = target();
        let fmt = doc.pixel_format();
        sm.cache = Some(block(&fmt, 0, Rect::new(0, 0, 8, 8)));
        let layer = Layer::new("smart", LayerContent::Smart(sm.clone()));
        let id = layer.id;
        doc.layers.push(layer);
        let ctx = JobCtx::new();
        ctx.cancel();
        let rendered = render_with_warnings_in(&doc, &sm, &ctx);
        assert!(matches!(rendered, Err(EngineError::Cancelled)), "{label}: {:?}", rendered.err());
        let refreshed = refresh_or_fail_in(&mut doc, id, &ctx);
        assert!(matches!(refreshed, Err(EngineError::Cancelled)), "{label}: {:?}", refreshed.err());
        let LayerContent::Smart(after) = &doc.layer(id).unwrap().content else { panic!("{label}: not smart") };
        let cache = after.cache.as_ref().unwrap();
        assert_eq!(cache.read_region(Rect::new(0, 0, 8, 8)), block(&fmt, 0, Rect::new(0, 0, 8, 8)).read_region(Rect::new(0, 0, 8, 8)), "{label}");
        assert_eq!(source_decode_count(&bytes, fmt), 0, "{label}: a cancelled job decoded its source");
        // Once it is not cancelled, the same render finishes.
        assert!(render_with_warnings_in(&doc, &sm, &JobCtx::new()).unwrap().0.is_some(), "{label}");
    }
}
