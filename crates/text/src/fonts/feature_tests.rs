//! Face capability regressions use bundled OFL data and synthetic metadata; no font assets added.
use super::*;
use photocraft_doc::text::CharStyle;

/// Change only the GSUB directory tag (known absence), or its inner feature-list offset
/// (unreadable metadata). The outer SFNT remains readable and registerable.
fn changed_gsub(malformed: bool) -> Vec<u8> {
    let mut bytes = super::font_names_tests::numeric_font(&INTER_REGULAR, "NoFeatures", "NoFeaturesPS");
    let count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    for i in 0..count {
        let record = 12 + i * 16;
        if &bytes[record..record + 4] == b"GSUB" {
            if malformed {
                let offset = u32::from_be_bytes(bytes[record + 8..record + 12].try_into().unwrap()) as usize;
                bytes[offset + 6..offset + 8].copy_from_slice(&u16::MAX.to_be_bytes());
            } else {
                bytes[record..record + 4].copy_from_slice(b"ZZZZ");
            }
            return bytes;
        }
    }
    panic!("bundled Inter has GSUB");
}

/// Two named styles with identical CSS attributes must read their own face's feature table,
/// including exact PostScript selection and a PS name recovering an unrelated family name.
#[test]
fn features_follow_named_and_postscript_faces_in_one_family() {
    let mut db = FontDb::new();
    db.register_font_data(super::font_names_tests::numeric_font(&INTER_REGULAR, "20", "FeatureFace20"));
    db.register_font_data(super::font_names_tests::numeric_font(&JETBRAINS_MONO_REGULAR, "30", "FeatureFace30"));
    let mut style = CharStyle { font_family: "Numeric Test".into(), font_style: "20".into(), ..Default::default() };
    assert!(db.face_features(&style).unwrap().contains("dlig"));
    style.font_style = "30".into();
    assert!(!db.face_features(&style).unwrap().contains("dlig"));
    style.postscript_name = Some("FeatureFace20".into());
    assert!(db.face_features(&style).unwrap().contains("dlig"), "PS identity wins over named style");
    style.font_family = "Unavailable family".into();
    assert!(db.face_features(&style).unwrap().contains("dlig"), "exact PS face is known");
}

/// Inspect both faces of a collection in place. Named faces with identical CSS attributes
/// must retain distinct GSUB results without making shaping's per-face copies of the TTC.
#[test]
fn collection_capabilities_reuse_the_original_source_and_face_indices() {
    use super::font_names_tests::{collection, numeric_font};
    let mut db = FontDb::new();
    let ttc = collection(&[numeric_font(&INTER_REGULAR, "20", "CollectionPS20"), numeric_font(&JETBRAINS_MONO_REGULAR, "30", "CollectionPS30")]);
    assert_eq!(db.register_font_data(ttc), ["Numeric Test"]);
    let family = db.fcx.collection.family_by_name("Numeric Test").unwrap();
    let original: HashSet<_> = family.fonts().iter().map(|face| (face.source().id(), face.index())).collect();
    assert_eq!(original.len(), 2);
    let inter = CharStyle { font_family: "Numeric Test".into(), font_style: "20".into(), ..Default::default() };
    let mono = CharStyle { postscript_name: Some("CollectionPS30".into()), ..inter.clone() };
    assert!(db.face_features(&inter).unwrap().contains("dlig"));
    assert!(!db.face_features(&mono).unwrap().contains("dlig"));
    assert!(db.face_aliases.is_empty(), "inspection must not create shaping aliases");
    assert_eq!(db.feature_faces.keys().copied().collect::<HashSet<_>>(), original);
}

/// A file can disappear after its face name has been cached. The named face stays unknown
/// even though CSS matching could still pick a readable sibling with identical attributes.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn unavailable_named_source_does_not_borrow_css_sibling_features() {
    use super::font_names_tests::numeric_font;
    let directory = std::env::temp_dir().join(format!("photocraft-feature-sources-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let inter = directory.join("inter.ttf");
    let mono = directory.join("mono.ttf");
    std::fs::write(&inter, numeric_font(&INTER_REGULAR, "20", "InstalledPS20")).unwrap();
    std::fs::write(&mono, numeric_font(&JETBRAINS_MONO_REGULAR, "30", "InstalledPS30")).unwrap();
    let mut db = FontDb::new();
    db.fcx.collection.load_fonts_from_paths([&inter, &mono]);
    let faces = db.faces("Numeric Test");
    let family = db.fcx.collection.family_by_name("Numeric Test").unwrap();
    let css_face = family.match_font(FontWidth::default(), FontStyle::Normal, FontWeight::new(400.0), true).unwrap();
    let (named, source) = faces.iter().zip(family.fonts()).find(|(_, font)| font.source().id() != css_face.source().id()).unwrap();
    let SourceKind::Path(removed) = source.source().kind() else { panic!("expected a path-backed face") };
    let request = CharStyle { font_family: "Numeric Test".into(), postscript_name: named.postscript_name.clone(), ..Default::default() };
    assert!(request.postscript_name.is_some());
    db.fcx.source_cache = SourceCache::default();
    std::fs::remove_file(removed).unwrap();
    let actual = db.face_features(&request);
    let sibling = db.face_features(&CharStyle { postscript_name: None, font_style: String::new(), ..request });
    drop(db);
    std::fs::remove_dir_all(directory).unwrap();
    assert!(sibling.is_some(), "the CSS sibling must remain readable");
    assert!(actual.is_none(), "a missing named source must remain unknown");
}

/// An unknown request cannot borrow Inter's features; a later font registration invalidates
/// both negative and positive results. Cached Arc identity proves repeated queries don't reparse.
#[test]
fn feature_cache_is_local_to_catalog_and_refreshes_after_registration() {
    let mut db = FontDb::new();
    let style = CharStyle { font_family: "Numeric Test".into(), font_style: "20".into(), ..Default::default() };
    assert!(db.face_features(&style).is_none());
    assert!(db.face_features(&style).is_none());
    db.register_font_data(super::font_names_tests::numeric_font(&INTER_REGULAR, "20", "FeatureFace20"));
    let first = db.face_features(&style).unwrap();
    assert!(first.contains("dlig"));
    assert!(Arc::ptr_eq(&first, &db.face_features(&style).unwrap()));
    assert!(FontDb::new().face_features(&style).is_none(), "another catalog cannot share request results");
    db.register_font_data(super::font_names_tests::numeric_font(&JETBRAINS_MONO_REGULAR, "30", "FeatureFace30"));
    assert!(!Arc::ptr_eq(&first, &db.face_features(&style).unwrap()), "registration invalidates metadata");
}

/// Editable PDF imports register embedded faces under separate family aliases. A cached miss
/// must refresh after registration, and two aliases sharing a PostScript name must stay distinct.
#[test]
fn embedded_font_aliases_keep_their_own_feature_capabilities() {
    let mut db = FontDb::new();
    let first_alias = "SharedEmbeddedPS 0123456789abcdef";
    let second_alias = "SharedEmbeddedPS fedcba98765432100123456789abcdef";
    let mut style = CharStyle { font_family: first_alias.into(), postscript_name: Some("SharedEmbeddedPS".into()), ..Default::default() };
    assert!(db.face_features(&style).is_none());
    let first = super::font_names_tests::numeric_font(&INTER_REGULAR, "20", "SharedEmbeddedPS");
    assert!(!db.register_font_data_as(first, &style.font_family).is_empty());
    assert!(db.face_features(&style).unwrap().contains("dlig"));

    let second = super::font_names_tests::numeric_font(&JETBRAINS_MONO_REGULAR, "30", "SharedEmbeddedPS");
    style.font_family = second_alias.into();
    assert!(db.face_features(&style).is_none(), "a different subset cannot stand in for a missing alias");
    assert!(
        db.face_features(&CharStyle { font_family: "Shared Embedded".into(), ..style.clone() }).is_none(),
        "a subset cannot stand in for a missing full font"
    );
    assert!(!db.register_font_data_as(second, second_alias).is_empty());
    assert!(!db.face_features(&style).unwrap().contains("dlig"));
    style.font_family = first_alias.into();
    assert!(db.face_features(&style).unwrap().contains("dlig"));
}

/// Reading capabilities for a missing PostScript name must leave shaping's cached choice
/// intact when an installed full face and an embedded subset share a PostScript name. If a
/// later shaping search changes the match, a warm capability query must observe the new choice.
#[test]
fn feature_requests_follow_shapings_postscript_match_without_overwriting_it() {
    let mut db = FontDb::new();
    let ps = "SharedEmbeddedPS";
    let full_family = "Shared Embedded PS";
    let full = super::font_names_tests::numeric_font(&INTER_REGULAR, "20", ps);
    assert!(!db.register_font_data_as(full, full_family).is_empty());
    let subset = super::font_names_tests::numeric_font(&JETBRAINS_MONO_REGULAR, "30", ps);
    assert!(!db.register_font_data_as(subset, "SharedEmbeddedPS 0123456789abcdef").is_empty());
    assert_eq!(db.resolve_postscript(ps).family, full_family);
    let request = CharStyle { font_family: "Unavailable family".into(), postscript_name: Some(ps.into()), ..Default::default() };
    assert!(db.face_features(&request).unwrap().contains("dlig"));

    let missing_name = "Unregistered-PostScript";
    let missing = CharStyle { postscript_name: Some(missing_name.into()), ..Default::default() };
    assert!(db.face_features(&missing).is_none());
    assert_eq!(db.resolve_postscript(ps).family, full_family);
    assert!(db.face_features(&request).unwrap().contains("dlig"));

    assert!(!db.resolve_postscript(missing_name).exact);
    assert_eq!(db.resolve_postscript(ps).family, "SharedEmbeddedPS 0123456789abcdef");
    assert!(db.face_features(&request).is_none(), "a cached full face must not stand in for the newly chosen subset");
}

/// Fonts with no GSUB have a known empty capability set, unlike malformed/unreadable GSUB.
#[test]
fn absent_and_malformed_tables_are_distinct_from_missing_fonts() {
    let style = CharStyle { font_family: "Numeric Test".into(), font_style: "NoFeatures".into(), ..Default::default() };
    let mut no_table = FontDb::new();
    assert!(!no_table.register_font_data(changed_gsub(false)).is_empty());
    let features = no_table.face_features(&style).unwrap();
    assert!(!features.contains("liga") && !features.contains("dlig"));
    let mut malformed = FontDb::new();
    assert!(!malformed.register_font_data(changed_gsub(true)).is_empty());
    assert!(malformed.face_features(&style).is_none());
    assert!(malformed.face_features(&CharStyle { font_family: "Missing".into(), ..style }).is_none());
}

/// A target larger than 256 distinct readable faces must retain its working set across all
/// buttons and frames. Each source shares bundled bytes, so the counterexample needs no large
/// fixture allocation. Arc identity detects a reparsed table, rather than timing the machine.
#[test]
fn large_targets_reuse_face_tables_across_repeated_queries() {
    let mut db = FontDb::new();
    let styles: Vec<_> = (0..257)
        .map(|index| {
            let family = format!("OpenType Cache Face {index}");
            db.register_blob(Blob::new(Arc::new(INTER_REGULAR.as_slice())), Some(&family));
            CharStyle { font_family: family, ..Default::default() }
        })
        .collect();
    let first: Vec<_> = styles.iter().map(|style| db.face_features(style).unwrap()).collect();
    for _ in 0..3 {
        for (style, previous) in styles.iter().zip(&first) {
            assert!(Arc::ptr_eq(previous, &db.face_features(style).unwrap()), "{} was reparsed", style.font_family);
        }
    }
}

/// An outer SFNT directory can advertise GSUB while its bytes are unavailable. Unlike an
/// absent table, zero/out-of-bounds offsets and excessive lengths must report unknown support.
#[test]
fn malformed_gsub_directory_is_unknown_instead_of_known_absence() {
    for (field, value) in [(8, 0), (8, u32::MAX), (12, u32::MAX)] {
        let mut bytes = super::font_names_tests::numeric_font(&INTER_REGULAR, "BrokenDirectory", "BrokenDirectoryPS");
        let count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
        let record = (0..count).map(|i| 12 + i * 16).find(|&r| &bytes[r..r + 4] == b"GSUB").unwrap();
        bytes[record + field..record + field + 4].copy_from_slice(&value.to_be_bytes());
        let mut db = FontDb::new();
        assert!(!db.register_font_data(bytes).is_empty());
        let style = CharStyle { font_family: "Numeric Test".into(), font_style: "BrokenDirectory".into(), ..Default::default() };
        assert!(db.face_features(&style).is_none(), "directory field {field} = {value}");
    }
}

/// Empty family names explicitly mean the default font in the document model; only unknown
/// named families must remain unknown instead of borrowing that font's features.
#[test]
fn unnamed_styles_use_the_default_but_missing_named_fonts_stay_unknown() {
    let mut db = FontDb::new();
    let style = CharStyle::default();
    let implicit = db.face_features(&style).unwrap();
    let explicit = db.face_features(&CharStyle { font_family: DEFAULT_FAMILY.into(), ..style.clone() }).unwrap();
    assert!(Arc::ptr_eq(&implicit, &explicit));
    assert!(implicit.contains("dlig"));
    assert!(db.face_features(&CharStyle { font_family: "Definitely Missing Font".into(), ..style }).is_none());
}

/// Arbitrary missing-font identities must not accumulate for the lifetime of the shared engine.
/// The bound includes PS aliases that the general import resolver would otherwise retain;
/// actual catalog names and source-face tables stay reusable without forcing an OOM.
#[test]
fn request_cache_bounds_negative_and_resolved_request_churn() {
    let mut db = FontDb::new();
    for index in 0..5000 {
        let missing = CharStyle { font_family: format!("Missing Font {index}"), ..Default::default() };
        assert!(db.face_features(&missing).is_none());
    }
    assert!(db.feature_cache.len() <= 4096, "negative requests accumulated: {}", db.feature_cache.len());
    for index in 0..5000 {
        let missing = CharStyle { font_family: DEFAULT_FAMILY.into(), postscript_name: Some(format!("Inter-MissingFace{index}")), ..Default::default() };
        assert!(db.face_features(&missing).is_none());
    }
    assert!(db.ps_cache.len() <= BUNDLED.len(), "unknown PS requests escaped into the resolver cache: {}", db.ps_cache.len());
    assert!(db.feature_cache.len() <= 4096);
    let mono = CharStyle { font_family: MONO_FAMILY.into(), ..Default::default() };
    let first = db.face_features(&mono).unwrap();
    for index in 0..5000 {
        let style = CharStyle { font_style: format!("Unknown Style {index}"), ..mono.clone() };
        assert!(db.face_features(&style).is_some());
    }
    assert!(db.feature_cache.len() <= 4096, "resolved requests accumulated: {}", db.feature_cache.len());
    assert!(Arc::ptr_eq(&first, &db.face_features(&mono).unwrap()), "raw-key eviction must retain real-face tables");
}

/// A single oversized imported name must not bypass the cache's memory budget. Rejecting optional
/// metadata is distinct from rejecting engine formatting, which is not changed by this query.
#[test]
fn oversized_font_requests_do_not_remain_in_the_metadata_cache() {
    let mut db = FontDb::new();
    let style = CharStyle { font_family: "X".repeat(2 * 1024 * 1024 + 1), ..Default::default() };
    assert!(db.face_features(&style).is_none());
    let stored: usize = db.feature_cache.keys().map(|key| key.family.len() + key.style.len() + key.postscript.as_ref().map_or(0, String::len)).sum();
    assert!(stored <= 2 * 1024 * 1024, "an oversized negative request retained {stored} bytes");
}

/// Reserve the current target before evicting historical requests. Named-instance resolution
/// reads fvar on a cold request, so a partial eviction must not repeat that work next frame.
#[test]
fn batch_admission_retains_named_instance_working_sets_after_request_churn() {
    let mut db = FontDb::new();
    db.register_font_data(super::font_names_tests::instance_font(&INTER_REGULAR));
    for index in 0..3900 {
        let missing = CharStyle { font_family: format!("Historic Missing {index}"), ..Default::default() };
        let _ = db.face_features(&missing);
    }
    let styles: Vec<_> = (0..257)
        .map(|index| CharStyle {
            font_family: "Instance Test".into(),
            font_style: format!("Request Style {index}"),
            postscript_name: Some("InstanceTest-LightCondensed".into()),
            ..Default::default()
        })
        .collect();
    let first = db.face_features_for_styles(&styles).unwrap();
    assert!(first.iter().all(|features| features.contains("dlig")));
    FEATURE_RESOLUTIONS.with(|count| count.set(0));
    for _ in 0..3 {
        let next = db.face_features_for_styles(&styles).unwrap();
        assert!(first.iter().zip(&next).all(|(a, b)| Arc::ptr_eq(a, b)));
    }
    assert_eq!(FEATURE_RESOLUTIONS.with(|count| count.get()), 0, "a stable target must not repeat face/table resolution");
}

/// Resource-limited metadata must report unknown before loading faces, rather than repeatedly
/// evicting a target larger than the request budget. Real formatting commands remain independent.
#[test]
fn oversized_working_sets_are_unknown_without_face_resolution() {
    let mut db = FontDb::new();
    let styles: Vec<_> =
        (0..4097).map(|index| CharStyle { font_family: DEFAULT_FAMILY.into(), font_style: format!("Style {index}"), ..Default::default() }).collect();
    FEATURE_RESOLUTIONS.with(|count| count.set(0));
    assert!(db.face_features_for_styles(&styles).is_none());
    assert_eq!(FEATURE_RESOLUTIONS.with(|count| count.get()), 0);
}

/// Many runs with the same font need one capability record, rather than a result allocation
/// proportional to the document's style-run count.
#[test]
fn repeated_style_spans_share_one_capability_record() {
    let mut db = FontDb::new();
    let style = CharStyle { font_family: DEFAULT_FAMILY.into(), ..Default::default() };
    let styles = vec![style; 10_000];
    FEATURE_RESOLUTIONS.with(|count| count.set(0));
    let features = db.face_features_for_styles(&styles).unwrap();
    assert_eq!(features.len(), 1);
    assert!(features[0].contains("dlig"));
    assert_eq!(FEATURE_RESOLUTIONS.with(|count| count.get()), 1);
}

/// A missing explicit PostScript face cannot inherit a nearby family's capabilities, including
/// PS-only styles with no family. Exact installed PS identities and ordinary CSS requests remain known.
#[test]
fn unresolved_postscript_faces_do_not_advertise_fallback_capabilities() {
    let mut db = FontDb::new();
    for family in ["", DEFAULT_FAMILY] {
        let style = CharStyle { font_family: family.into(), postscript_name: Some("Inter-DefinitelyMissingFace".into()), ..Default::default() };
        assert!(db.face_features(&style).is_none(), "missing PS identity in {family:?}");
    }
    let known = CharStyle { postscript_name: Some("Inter-Regular".into()), ..Default::default() };
    assert!(db.face_features(&known).unwrap().contains("dlig"), "an exact PS face recovers its family");
    assert!(db.face_features(&CharStyle { font_family: DEFAULT_FAMILY.into(), ..Default::default() }).unwrap().contains("dlig"));
}

/// Shaping may use a matching instance suffix for compatibility. Capability inspection needs
/// the instance's own PostScript name or the full name derived from the font's metadata.
#[test]
fn variable_instance_suffix_alone_does_not_verify_a_face() {
    let mut db = FontDb::new();
    db.register_font_data(super::font_names_tests::instance_font(&INTER_REGULAR));
    let mut style = CharStyle { font_family: "Instance Test".into(), postscript_name: Some("UnrelatedFont-LightCondensed".into()), ..Default::default() };
    assert!(db.face_features(&style).is_none());
    style.postscript_name = Some("InstanceTest-LightCondensed".into());
    assert!(db.face_features(&style).unwrap().contains("dlig"));
}

/// An earlier sibling with the same instance-style suffix cannot shadow a later exact
/// PostScript match. Shaping and capability inspection must choose the same source face.
#[test]
fn variable_instance_exact_match_precedes_an_earlier_sibling_suffix() {
    use super::font_names_tests::variable_sibling;
    let mut db = FontDb::new();
    db.register_font_data(variable_sibling(&INTER_REGULAR, "First", "First-Regular"));
    db.register_font_data(variable_sibling(&JETBRAINS_MONO_REGULAR, "Second", "Second-Regular"));
    let faces = db.faces("Instance Test");
    assert_eq!(faces.iter().map(|face| face.postscript_name.as_deref()).collect::<Vec<_>>(), [Some("First-Regular"), Some("Second-Regular")]);

    let mut style = CharStyle { font_family: "Instance Test".into(), postscript_name: Some("Second-LightCondensed".into()), ..Default::default() };
    assert!(!db.face_features(&style).unwrap().contains("dlig"), "the exact Second instance has Mono's features");
    db.select_named_face(&mut style);
    assert_eq!(db.faces(&style.font_family)[0].postscript_name.as_deref(), Some("Second-Regular"));
    let mut fallback = CharStyle { font_family: "Instance Test".into(), postscript_name: Some("Unrelated-LightCondensed".into()), ..Default::default() };
    assert!(db.face_features(&fallback).is_none(), "the suffix cannot identify a font for capability inspection");
    db.select_named_face(&mut fallback);
    assert_eq!(db.faces(&fallback.font_family)[0].postscript_name.as_deref(), Some("First-Regular"), "shaping still accepts the first suffix fallback");
}

/// Borrowed metadata admission consumes only a bounded prefix of an oversized iterator.
/// A duplicate-font stream retains one result while ignoring irrelevant feature payloads.
#[test]
fn borrowed_requests_bound_consumption_before_collecting_a_whole_target() {
    let mut db = FontDb::new();
    let styles: Vec<_> =
        (0..5000).map(|index| CharStyle { font_family: DEFAULT_FAMILY.into(), font_style: format!("Style {index}"), ..Default::default() }).collect();
    let consumed = std::cell::Cell::new(0usize);
    let requests = styles.iter().map(FaceRequest::from).inspect(|_| consumed.set(consumed.get() + 1));
    assert!(db.face_features_for_requests(requests).is_none());
    assert!(consumed.get() <= 4097, "the target must be rejected before collecting its remaining identities");
    let style = CharStyle {
        font_family: DEFAULT_FAMILY.into(),
        features: vec![photocraft_doc::text::FontFeature { tag: "x".repeat(1024), value: 1 }; 32],
        ..Default::default()
    };
    let known = db.face_features_for_requests(std::iter::repeat_n(FaceRequest::from(&style), 10_000)).unwrap();
    assert_eq!(known.len(), 1);
    assert!(known[0].contains("dlig"));
}
