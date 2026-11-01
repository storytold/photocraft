use super::*;

const ICONS: &[(&str, &[u8])] = &[
    ("box", br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="4" y="4" width="16" height="16"/></svg>"#),
    ("special", br##"<svg stroke-width="1" fill="#123456" stroke-width='2'/>"##),
];

fn set(color: &'static str, width: Option<&'static str>, prefix: &'static str) -> SvgIconSet {
    SvgIconSet::new(ICONS, SvgStyle { current_color: color, stroke_width: width }, prefix, "box")
}

fn parts(source: ImageSource<'static>) -> (String, Arc<[u8]>) {
    match source {
        ImageSource::Bytes { uri, bytes: Bytes::Shared(bytes) } => (uri.to_string(), bytes),
        _ => panic!("expected shared SVG bytes"),
    }
}

#[test]
fn app_styles_preserve_their_own_strokes_and_white_spelling() {
    let photo = set("#ffffff", Some("1.75"), "bytes://icons/");
    let vector = set("white", Some("1.6"), "bytes://icon/");
    for (set, white, width) in [(&photo, "#ffffff", "1.75"), (&vector, "white", "1.6")] {
        let (_, bytes) = parts(set.source("box"));
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.contains(&format!("stroke=\"{white}\"")));
        assert!(text.contains(&format!("stroke-width=\"{width}\"")));
        assert!(!text.contains("currentColor"));
        assert_eq!(&*parts(set.source("special")).1, ICONS[1].1);
    }
    assert_eq!(parts(photo.source("box")).0, "bytes://icons/box.svg");
    assert_eq!(parts(vector.source("box")).0, "bytes://icon/box.svg");
}

#[test]
fn stroke_replacement_is_optional() {
    let (_, bytes) = parts(set("white", None, "bytes://plain/").source("box"));
    assert!(std::str::from_utf8(&bytes).unwrap().contains("stroke-width=\"2\""));
}

#[test]
fn fallback_shares_bytes_but_preserves_requested_cache_key() {
    let icons = set("white", None, "bytes://test/");
    let (uri, missing) = parts(icons.source("missing"));
    let (_, known) = parts(icons.source("box"));
    assert_eq!(uri, "bytes://test/missing.svg");
    assert!(Arc::ptr_eq(&known, &missing));
    assert!(Arc::ptr_eq(&known, &parts(icons.source("box")).1));
    assert!(!icons.contains("missing"));
    assert_eq!(icons.canonical_name("missing"), None);
    assert_eq!(icons.canonical_name("box"), Some("box"));
}

#[test]
fn absent_fallback_is_an_empty_source_not_a_panic() {
    let icons = SvgIconSet::new(&[], SvgStyle { current_color: "white", stroke_width: None }, "bytes://empty/", "absent");
    let (uri, bytes) = parts(icons.source("unknown"));
    assert_eq!(uri, "bytes://empty/unknown.svg");
    assert!(bytes.is_empty());
}

#[test]
fn duplicate_names_keep_the_last_entry_and_invalid_utf8_is_lossy() {
    let icons =
        SvgIconSet::new(&[("a", b"first"), ("a", b"last\xff")], SvgStyle { current_color: "white", stroke_width: None }, "bytes://duplicates/", "a");
    assert_eq!(std::str::from_utf8(&parts(icons.source("a")).1).unwrap(), "last\u{fffd}");
}

#[test]
fn egui_svg_loader_rasterizes_both_styles_at_multiple_pixel_sizes() {
    let ctx = egui::Context::default();
    egui_extras::install_image_loaders(&ctx);
    for (prefix, width) in [("bytes://photo/", "1.75"), ("bytes://vector/", "1.6")] {
        let icons = set("white", Some(width), prefix);
        for size in [16, 24, 48] {
            let (uri, bytes) = parts(icons.source("box"));
            ctx.include_bytes(uri.clone(), Bytes::Shared(bytes));
            let result = ctx.try_load_image(&uri, egui::load::SizeHint::Size { width: size, height: size, maintain_aspect_ratio: true }).unwrap();
            let egui::load::ImagePoll::Ready { image } = result else { panic!("embedded SVG did not load synchronously") };
            assert_eq!(image.size, [size as usize, size as usize]);
            assert!(image.pixels.iter().any(|pixel| pixel.a() != 0));
        }
    }
}
