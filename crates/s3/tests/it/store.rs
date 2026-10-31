//! `S3Store` against the in-process fake: the requests a save makes, previews, failures, reads and
//! configuration errors.

use photocraft_format::{BundleStore, ByteSource, FormatError, PcraftWriter, SaveOptions};
use photocraft_s3::{S3Store, is_conflict};

use crate::fake::FakeS3;
use crate::{bundle_lifecycle, doc, load, with_previews};

#[test]
fn a_bundle_saves_reopens_and_refuses_lost_updates() {
    let s3 = FakeS3::start();
    bundle_lifecycle(&s3.config(), "pool/", "lifecycle/");
}

#[test]
fn a_save_uploads_new_objects_once_then_moves_the_head_conditionally() {
    let s3 = FakeS3::start();
    let mut store = S3Store::new(s3.config(), "pool/", "docs/a/").unwrap();
    let mut writer = PcraftWriter::new();
    writer.save_to(&doc(1), &mut store, &SaveOptions::default()).unwrap();
    let log = s3.take_log();
    let (objects, rest): (Vec<_>, Vec<_>) = log.iter().partition(|r| r.key.starts_with("pool/"));
    assert!(!objects.is_empty());
    let absent = Some(("if-none-match".to_string(), "*".to_string()));
    assert!(objects.iter().all(|r| r.method == "PUT" && r.condition == absent), "{objects:?}");
    // The manifest is created only if absent, after every object; previews it has none of are cleared.
    assert_eq!(
        rest.iter().map(|r| (r.method.as_str(), r.key.as_str())).collect::<Vec<_>>(),
        [("PUT", "docs/a/manifest.json"), ("DELETE", "docs/a/thumb.png"), ("DELETE", "docs/a/composite/preview.png")]
    );
    assert_eq!(rest[0].condition, absent);
    assert!(log.iter().all(|r| r.authorization.starts_with("AWS4-HMAC-SHA256 Credential=test-key/")), "{log:?}");
    let head = store.head().unwrap();

    // Nothing changed: only the manifest goes up, replacing exactly the version this store wrote.
    writer.save_to(&doc(1), &mut store, &SaveOptions::default()).unwrap();
    let log = s3.take_log();
    assert!(log.iter().all(|r| !r.key.starts_with("pool/")), "{log:?}");
    assert_eq!(log[0].condition, Some(("if-match".to_string(), head)));
}

#[test]
fn previews_follow_the_head() {
    let s3 = FakeS3::start();
    let mut store = S3Store::new(s3.config(), "pool/", "docs/p/").unwrap();
    PcraftWriter::new().save_to(&doc(1), &mut store, &with_previews()).unwrap();
    let thumb = s3.object("docs/p/thumb.png").unwrap();
    assert!(thumb.starts_with(b"\x89PNG"));
    assert!(s3.object("docs/p/composite/preview.png").is_some());
    assert_eq!(store.get("thumb.png", usize::MAX).unwrap(), thumb);

    PcraftWriter::new().save_to(&doc(1), &mut store, &SaveOptions::default()).unwrap();
    assert!(s3.keys().iter().all(|k| !k.ends_with(".png")), "{:?}", s3.keys());
}

#[test]
fn a_failed_upload_leaves_the_document_where_it_was() {
    let s3 = FakeS3::start();
    let mut store = S3Store::new(s3.config(), "pool/", "docs/f/").unwrap();
    PcraftWriter::new().save_to(&doc(1), &mut store, &SaveOptions::default()).unwrap();
    let before = s3.object("docs/f/manifest.json").unwrap();

    let edited = doc(2);
    s3.state.lock().unwrap().fail_puts_under = Some("pool/".into());
    let err = PcraftWriter::new().save_to(&edited, &mut store, &SaveOptions::default()).unwrap_err();
    assert!(!is_conflict(&err));
    assert!(err.to_string().contains("HTTP 500: InternalError"), "{err}");
    assert_eq!(s3.object("docs/f/manifest.json").unwrap(), before);

    // Once storage recovers the same store saves, uploading what it couldn't.
    s3.state.lock().unwrap().fail_puts_under = None;
    PcraftWriter::new().save_to(&edited, &mut store, &SaveOptions::default()).unwrap();
    assert_eq!(load(&S3Store::new(s3.config(), "pool/", "docs/f/").unwrap()).unwrap(), edited);
}

#[test]
fn reads_report_missing_keys_and_enforce_limits() {
    let s3 = FakeS3::start();
    let mut store = S3Store::new(s3.config(), "pool/", "docs/r/").unwrap();
    assert!(matches!(store.get("manifest.json", usize::MAX), Err(FormatError::Corrupt(m)) if m.contains("manifest.json")));
    assert!(matches!(load(&store), Err(FormatError::Corrupt(_))));
    assert_eq!(store.head(), None);

    PcraftWriter::new().save_to(&doc(1), &mut store, &SaveOptions::default()).unwrap();
    let manifest = s3.object("docs/r/manifest.json").unwrap();
    assert_eq!(store.get("manifest.json", manifest.len()).unwrap(), manifest);
    assert!(matches!(store.get("manifest.json", manifest.len() - 1), Err(FormatError::LimitExceeded(_))));
}

#[test]
fn store_steps_outside_a_save_are_refused() {
    let s3 = FakeS3::start();
    let mut store = S3Store::new(s3.config(), "pool/", "docs/x/").unwrap();
    assert!(store.preview("thumb.png", Vec::new()).is_err());
    assert!(store.commit(b"{}").is_err());
    assert!(s3.take_log().is_empty());
}

#[test]
fn endpoints_and_buckets_that_could_misroute_requests_are_refused() {
    let s3 = FakeS3::start();
    for (endpoint, bucket) in [
        ("127.0.0.1:9000", "designs"),
        ("ftp://127.0.0.1", "designs"),
        ("http://127.0.0.1:9000/extra", "designs"),
        ("http://user@127.0.0.1:9000", "designs"),
        ("http://127.0.0.1:9000", "Designs"),
        ("http://127.0.0.1:9000", "a/b"),
        ("http://127.0.0.1:9000", ""),
    ] {
        let config = photocraft_s3::S3Config { endpoint: endpoint.into(), bucket: bucket.into(), ..s3.config() };
        assert!(matches!(S3Store::new(config, "pool/", "d/"), Err(FormatError::Unsupported(_))), "{endpoint} {bucket}");
    }
    // A trailing slash and the scheme's default port are fine.
    let config = photocraft_s3::S3Config { endpoint: "https://s3.example.com:443/".into(), ..s3.config() };
    assert!(S3Store::new(config, "pool/", "d/").is_ok());
}

#[test]
fn credentials_never_show_in_debug_output() {
    let shown = format!("{:?}", FakeS3::start().config());
    assert!(shown.contains("test-key") && !shown.contains("test-secret"), "{shown}");
}
