use super::*;
use std::fmt::Write;

fn parameters(count: usize) -> String {
    let mut text = String::new();
    for index in 0..count {
        write!(&mut text, ";p{index}=x").unwrap();
    }
    text
}

#[test]
fn a_hundred_thousand_unique_mime_parameters_are_refused_before_the_original_parser() {
    let params = parameters(100_000);
    for (essence, expected) in [
        ("image/png", INVALID_IMAGE),
        ("text/plain", INVALID_IMAGE),
        ("", INVALID_IMAGE),
        ("not-a-mime", INVALID_IMAGE),
        ("application/octet-stream", IGNORED_DATA),
        ("audio/ogg", IGNORED_DATA),
    ] {
        for delimiter in [",AA==", "#fragment,AA==", ""] {
            let value = format!("data:{essence}{params};base64{delimiter}");
            let mut work = value.len() * 2;
            assert!(!mime_header_admitted(&value));
            let result =
                malformed_data_url_replacement_with(&value, &mut work, |_| panic!("the original parameter flood must not reach DataUrl::process")).unwrap();
            assert_eq!(result, Some(expected), "{essence} {delimiter}");
        }
    }
}

#[test]
fn mime_parameter_admission_preserves_the_boundary_and_ignores_body_semicolons() {
    let accepted = format!("data:image/png{};base64,AA==", parameters(MAX_MIME_SEPARATORS - 1));
    assert!(mime_header_admitted(&accepted));
    let called = std::cell::Cell::new(false);
    let mut work = accepted.len() * 2;
    assert_eq!(
        malformed_data_url_replacement_with(&accepted, &mut work, |value| {
            called.set(true);
            data_url::DataUrl::process(value)
        })
        .unwrap(),
        None
    );
    assert!(called.get(), "an admitted original URL still reaches the real parser");
    let refused = format!("data:image/png{};base64,AA==", parameters(MAX_MIME_SEPARATORS));
    assert!(!mime_header_admitted(&refused));
    let mut work = refused.len() * 2;
    assert_eq!(malformed_data_url_replacement(&refused, &mut work).unwrap(), Some(INVALID_IMAGE));

    for value in [format!("data:text/plain,{}", ";".repeat(100_000)), format!("external.png?{}", parameters(100_000))] {
        assert!(mime_header_admitted(&value), "payloads/external URLs are not MIME parameters");
        let mut work = value.len() * 2;
        assert_eq!(malformed_data_url_replacement(&value, &mut work).unwrap(), None);
    }
    let value = format!(" \td\ta\nt\ra:image/png{};base64,A", parameters(MAX_MIME_SEPARATORS));
    assert!(!mime_header_admitted(&value), "scheme normalization matches the pinned parser");
    let mut work = value.len() * 2;
    assert_eq!(malformed_data_url_replacement(&value, &mut work).unwrap(), Some(INVALID_IMAGE));
}

#[test]
fn refused_parameter_lists_normalize_both_hrefs_and_foreign_resources_without_preflight_warnings() {
    let value = format!("data:image/png{};base64,AA==", parameters(MAX_MIME_SEPARATORS));
    let text = format!(
        "<svg xmlns='{SVG_NS}' xmlns:x='{XLINK_NS}' xmlns:f='urn:foreign'><defs><image id='unused' href='{value}' x:href='{value}'/></defs><image display='none' href='{value}' x:href='{value}'/><f:image href='{value}' x:href='{value}'/></svg>"
    );
    let limits = Limits { bytes: 1 << 20, ..Default::default() };
    let repaired = repair(&text, &limits).unwrap();
    assert_eq!(repaired.matches(INVALID_IMAGE).count(), 6);
    assert!(!repaired.contains(";p0="));
    let resolver = craft_svg::tree::ImageResolver::new(limits);
    resolver.check_document(&repaired).unwrap();
    let usage = resolver.usage();
    assert_eq!((usage.invalid_raster_images, usage.invalid_svg_images), (0, 0));
    assert!(matches!(repair(&repaired, &Limits { bytes: 1 << 20, ..Default::default() }).unwrap(), Cow::Borrowed(_)));
}

#[test]
fn duplicate_and_quoted_parameters_share_the_same_conservative_boundary() {
    for (accepted, refused) in [
        (format!("data:image/png{},%00", ";p=x".repeat(MAX_MIME_SEPARATORS)), format!("data:image/png{},%00", ";p=x".repeat(MAX_MIME_SEPARATORS + 1))),
        (format!("data:image/png;p=\"{}\",%00", ";".repeat(MAX_MIME_SEPARATORS - 1)), format!("data:image/png;p=\"{}\",%00", ";".repeat(MAX_MIME_SEPARATORS))),
    ] {
        assert!(mime_header_admitted(&accepted));
        let mut work = accepted.len() * 2;
        assert_eq!(malformed_data_url_replacement(&accepted, &mut work).unwrap(), None);
        assert!(!mime_header_admitted(&refused));
        let mut work = refused.len() * 2;
        assert_eq!(malformed_data_url_replacement(&refused, &mut work).unwrap(), Some(INVALID_IMAGE));
    }
    for value in ["external.png", "#local", "datx:image/png;param=x,body"] {
        let mut work = value.len() * 2;
        assert_eq!(malformed_data_url_replacement(value, &mut work).unwrap(), None);
    }
    let value = "data:image/png;base64";
    let mut work = value.len() * 2;
    assert_eq!(malformed_data_url_replacement(value, &mut work).unwrap(), Some(INVALID_IMAGE));
}

#[test]
fn parameter_flood_scans_and_essence_classification_spend_one_shared_work_ledger() {
    let value = format!("data:application/octet-stream{};base64,AA==", parameters(MAX_MIME_SEPARATORS));
    let essence_len = "data:application/octet-stream".len();
    let charge = value.len() + essence_len;
    let mut work = charge * 2;
    for remaining in [charge, 0] {
        assert_eq!(malformed_data_url_replacement_with(&value, &mut work, |_| panic!("not admitted")).unwrap(), Some(IGNORED_DATA));
        assert_eq!(work, remaining, "the original header scan and parameter-free fallback each retain their charge");
    }
    assert!(malformed_data_url_replacement_with(&value, &mut work, |_| panic!("no work remains")).is_err());
    assert_eq!(work, 0);
    let mut not_enough_for_essence = value.len();
    assert!(malformed_data_url_replacement_with(&value, &mut not_enough_for_essence, |_| panic!("not admitted")).is_err());
    assert_eq!(not_enough_for_essence, 0);
}
