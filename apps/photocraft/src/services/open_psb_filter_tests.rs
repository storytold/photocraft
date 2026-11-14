//! The Open dialog lists `.psb` as its own file type, directly after the layered document format (#14055).

use super::{OPEN_EXTS, OPEN_FILTERS, open_filter_extensions, open_filters};

#[test]
fn first_entries_are_layered_document_psb_then_photocraft() {
    assert_eq!(OPEN_FILTERS[0].1, &["psd", "psdt"]);
    assert_eq!(OPEN_FILTERS[1], ("Large Document Format", &["psb"][..]));
    assert_eq!(OPEN_FILTERS[2], ("PhotoCraft", &["pcraft"][..]));
}

#[test]
fn every_open_extension_is_covered_by_some_entry() {
    for ext in OPEN_EXTS {
        assert!(OPEN_FILTERS.iter().any(|(_, exts)| exts.contains(ext)), "no Open filter entry covers .{ext}");
    }
}

#[test]
fn all_formats_still_matches_psb() {
    let filters = open_filters();
    assert_eq!(filters[0].0, "All Formats");
    assert!(filters[0].1.contains(&open_filter_extensions(&["psb"])[0]));
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn psb_entry_gets_the_case_insensitive_glob_on_linux() {
    assert_eq!(open_filter_extensions(&["psb"]), vec!["[pP][sS][bB]".to_string()]);
    let filters = open_filters();
    assert_eq!(filters[2].0, "Large Document Format");
    assert_eq!(filters[2].1, vec!["[pP][sS][bB]".to_string()]);
}

#[test]
fn psb_entry_in_the_built_list_follows_the_layered_entry() {
    let filters = open_filters();
    assert_eq!(filters[2].0, "Large Document Format");
    assert_eq!(filters[2].1, open_filter_extensions(&["psb"]));
}
