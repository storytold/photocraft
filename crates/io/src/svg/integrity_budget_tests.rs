use super::*;
use std::io::Write;

fn wrapped_options(pixels: u64) -> (usvg::Options<'static>, Arc<Mutex<Integrity>>, Arc<craft_svg::tree::ImageResolver>) {
    let state = Arc::new(Mutex::new(Integrity::new(pixels)));
    let resolver = Arc::new(craft_svg::tree::ImageResolver::new(tree_limits(MAX_XML_DEPTH, pixels)));
    let options =
        super::super::options_with_isolation(Arc::new(usvg::fontdb::Database::new()), MAX_XML_DEPTH, false, Arc::clone(&resolver), Arc::clone(&state));
    (options, state, resolver)
}

fn png() -> Vec<u8> {
    let image = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, [17, 83, 149].repeat(4)).unwrap();
    photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap()
}

#[test]
fn original_nested_xml_bytes_are_not_refunded_when_malformed_hrefs_shrink() {
    super::super::on_svg_stack(|| {
        let (options, state, resolver) = wrapped_options(MAX_RASTER_PIXELS);
        let prefix = "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><image width='1' height='1' href='data:image/png;base64,";
        let suffix = "!'/></svg>";
        let original_size = 3 << 20;
        let source = Arc::new([prefix.as_bytes(), &vec![b'A'; original_size - prefix.len() - suffix.len()], suffix.as_bytes()].concat());
        assert_eq!(source.len(), original_size);
        for accepted in 1..=5 {
            let kind = (options.image_href_resolver.resolve_data)("image/svg+xml", Arc::clone(&source), &options);
            assert!(matches!(kind, Some(usvg::ImageKind::SVG(_))), "source {accepted} fits the original-text allowance");
            drop(kind);
            let state = state.lock().unwrap_or_else(PoisonError::into_inner);
            assert_eq!(state.svg_bytes_left, MAX_BOUNDED_SVG_BYTES as usize - accepted * original_size);
            assert!(!state.svg_failure_exhausted);
            assert_eq!(state.classification_bytes_left(), MAX_BOUNDED_SVG_BYTES as usize);
            let usage = state.usage(resolver.usage()).unwrap();
            assert_eq!(usage.embedded_svg_bytes, accepted * original_size, "the short replacement cannot refund original XML work");
        }
        assert!((options.image_href_resolver.resolve_data)("image/svg+xml", Arc::clone(&source), &options).is_none());
        let state = state.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(state.svg_bytes_left, 0);
        assert_eq!((state.svg_limits, state.svg_resource_limits), (1, 1));
        assert!(!state.svg_failure_exhausted, "ordinary byte exhaustion cannot enter compatibility classification");
        assert_eq!(state.classification_bytes_left(), MAX_BOUNDED_SVG_BYTES as usize);
        assert_eq!(state.usage(resolver.usage()).unwrap().svg_resource_limit_images, 1);
        Ok(())
    })
    .unwrap();
}

#[test]
fn invalid_utf8_gzip_consumes_remaining_svg_work_before_the_next_child() {
    super::super::on_svg_stack(|| {
        let (options, state, resolver) = wrapped_options(MAX_RASTER_PIXELS);
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&[0xff; 4096]).unwrap();
        let invalid = Arc::new(encoder.finish().unwrap());
        assert!((options.image_href_resolver.resolve_data)("image/svg+xml", invalid, &options).is_none());
        {
            let state = state.lock().unwrap_or_else(PoisonError::into_inner);
            assert_eq!(state.svg_bytes_left, 0, "a failed decode cannot retry the full work allowance");
            assert_eq!(state.invalid_svg, 1);
            assert!(state.svg_failure_exhausted);
            assert_eq!(state.svg_resource_limits, 0);
            assert_eq!(state.classification_bytes_left(), MAX_BOUNDED_SVG_BYTES as usize);
        }
        let valid = Arc::new(b"<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><rect width='1' height='1'/></svg>".to_vec());
        let valid_size = valid.len();
        assert!((options.image_href_resolver.resolve_data)("image/svg+xml", valid, &options).is_none());
        let state = state.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!((state.invalid_svg, state.svg_limits, state.svg_resource_limits), (1, 1, 0));
        assert_eq!(state.classification_bytes_left(), MAX_BOUNDED_SVG_BYTES as usize - valid_size);
        let usage = state.usage(resolver.usage()).unwrap();
        assert_eq!(usage.embedded_svg_bytes, MAX_BOUNDED_SVG_BYTES as usize);
        assert_eq!(usage.svg_resource_limit_images, 0, "a later small child is omitted, not a resource error");
        let charged = 2 * MAX_BOUNDED_SVG_BYTES as usize - state.svg_bytes_left - state.classification_bytes_left();
        assert_eq!(charged, MAX_BOUNDED_SVG_BYTES as usize + valid_size);
        assert!(charged <= 2 * MAX_BOUNDED_SVG_BYTES as usize, "both work pools together have a fixed 32 MiB allowance");
        Ok(())
    })
    .unwrap();
}

#[test]
fn post_failure_classification_is_cumulative_and_never_admits_later_children() {
    super::super::on_svg_stack(|| {
        let (options, state, resolver) = wrapped_options(MAX_RASTER_PIXELS);
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&[0xff; 256]).unwrap();
        assert!((options.image_href_resolver.resolve_data)("image/svg+xml", Arc::new(encoder.finish().unwrap()), &options).is_none());
        let valid = Arc::new(b"<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'/>".to_vec());
        // Exercise the cumulative boundary with a small injected second pool;
        // public policy tests separately cover the full 16 MiB allowance.
        let classification_limit = valid.len() * 2 - 1;
        state.lock().unwrap_or_else(PoisonError::into_inner).classification_bytes_left = classification_limit;
        assert!((options.image_href_resolver.resolve_data)("image/svg+xml", Arc::clone(&valid), &options).is_none());
        {
            let state = state.lock().unwrap_or_else(PoisonError::into_inner);
            assert_eq!(state.classification_bytes_left(), valid.len() - 1);
            assert_eq!((state.svg_limits, state.svg_resource_limits), (1, 0));
        }
        assert!((options.image_href_resolver.resolve_data)("image/svg+xml", Arc::clone(&valid), &options).is_none());
        let state = state.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(state.classification_bytes_left(), 0);
        assert_eq!((state.svg_limits, state.svg_resource_limits), (2, 1));
        assert_eq!(state.svg_bytes_left, 0, "classification never refunds original failed work");
        assert!(state.svg_failure_exhausted);
        let charged = 2 * MAX_BOUNDED_SVG_BYTES as usize - state.svg_bytes_left - state.classification_bytes_left();
        assert_eq!(charged, 2 * MAX_BOUNDED_SVG_BYTES as usize, "exhausted pools cannot authorize more than their combined 32 MiB allowance");
        assert_eq!(resolver.usage().embedded_svg_count, 0, "classified children must never reach the shared parser");
        Ok(())
    })
    .unwrap();
}

#[test]
fn failed_raster_arcs_are_retained_and_cached_failures_do_not_recharge_work() {
    let mut broken = png();
    let idat = broken.windows(4).position(|bytes| bytes == b"IDAT").unwrap();
    broken[idat + 4] ^= 1;
    assert_eq!(photocraft_codecs::probe_allocation_size(&broken).unwrap(), (2, 2), "failure must occur after four-pixel admission");
    let encoded = Arc::new(broken);
    let key = Arc::as_ptr(&encoded).addr();
    let weak = Arc::downgrade(&encoded);
    let kind = usvg::ImageKind::PNG(Arc::clone(&encoded));
    let mut state = Integrity::new(4);
    let encoded_before = state.encoded_left;
    assert!(!state.raster(&kind));
    assert_eq!(state.pixels_left, 0);
    assert_eq!(state.encoded_left, encoded_before - encoded.len());
    assert_eq!(state.raster.get(&key).unwrap().result, Err(raster_validation::Failure::Invalid));
    let charged = state.encoded_left;
    assert!(!state.raster(&kind));
    assert_eq!((state.pixels_left, state.encoded_left, state.raster.len(), state.invalid_raster), (0, charged, 1, 2));
    drop(kind);
    drop(encoded);
    let retained = weak.upgrade().expect("cache must retain failed encoded sources to prevent pointer reuse");
    assert_eq!(Arc::as_ptr(&retained).addr(), key);
    assert_eq!(Arc::as_ptr(&state.raster.get(&key).unwrap()._encoded).addr(), key);
    let other = Arc::new(png());
    assert_ne!(Arc::as_ptr(&other).addr(), key, "a live failed source's identity cannot be reused");
    assert!(!state.raster(&usvg::ImageKind::PNG(other)), "a distinct source cannot reuse pixels spent by the failed decode");
    assert_eq!(state.raster_limits, 1);
}

#[test]
fn cached_valid_png_with_exactly_four_pixels_is_not_charged_twice() {
    let encoded = Arc::new(png());
    let kind = usvg::ImageKind::PNG(Arc::clone(&encoded));
    let mut state = Integrity::new(4);
    let encoded_before = state.encoded_left;
    assert!(state.raster(&kind));
    assert_eq!(state.pixels_left, 0);
    assert_eq!(state.encoded_left, encoded_before - encoded.len());
    let charged = state.encoded_left;
    assert!(state.raster(&kind), "cached validation remains valid after it spent the last four pixels");
    assert_eq!((state.pixels_left, state.encoded_left, state.raster.len()), (0, charged, 1));
    assert_eq!((state.invalid_raster, state.raster_limits), (0, 0));
}

#[test]
fn raster_callbacks_own_pixel_charges_and_keep_mime_aliases_separate() {
    let image = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, [0, 255, 0].repeat(4)).unwrap();
    let png = Arc::new(photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap());
    let shared = craft_svg::tree::ImageResolver::new(tree_limits(MAX_XML_DEPTH, 4));
    let state = Arc::new(Mutex::new(Integrity::new(4)));
    let resolver = wrap(shared.resolver(), Arc::clone(&state));
    for _ in 0..2 {
        assert!((resolver.resolve_data)("image/png", Arc::clone(&png), &usvg::Options::default()).is_some());
        let usage = state.lock().unwrap().usage(shared.usage()).unwrap();
        assert_eq!(usage.raster_pixels, 4, "a retained alias does not spend decoding work again");
        assert_eq!(usage.invalid_raster_images, 0);
        assert_eq!(shared.usage().raster_pixels, 0, "generic decoding must not duplicate exact renderer validation");
    }
    assert!((resolver.resolve_data)("image/jpeg", png, &usvg::Options::default()).is_none());
    let usage = state.lock().unwrap().usage(shared.usage()).unwrap();
    assert_eq!(usage.raster_pixels, 4);
    assert_eq!(usage.invalid_raster_images, 1, "a mismatched MIME cannot borrow the successful PNG cache entry");
}
