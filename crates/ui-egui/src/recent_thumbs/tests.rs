use std::path::{Path, PathBuf};

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
use photocraft_color::{ColorMode, SampleType as DocSample};
use photocraft_doc::{Document, Layer};
use photocraft_geom::Size;

use super::disk::{self, cache_key, file_name};
use super::*;

fn rgba(w: u32, h: u32, color: [u8; 4]) -> Rgba8Image {
    Rgba8Image { width: w, height: h, pixels: color.repeat((w * h) as usize) }
}

fn png(img: &Rgba8Image) -> Vec<u8> {
    let image = Image::from_raw(img.width, img.height, ChannelLayout::Rgba, SampleType::U8, img.pixels.clone()).unwrap();
    photocraft_codecs::encode(&image, Format::Png, &EncodeOptions::default()).unwrap()
}

fn probe(path: &Path, want_thumb: bool, known: Option<u64>, cache: Option<&PathBuf>) -> Done {
    let p = Probe { path: path.to_string_lossy().into_owned(), id: 0, want_thumb, known, ctx: egui::Context::default() };
    let shared = Shared::default();
    shared.allow(&p.path);
    disk::probe(&p, cache.map(PathBuf::as_path), &shared)
}

fn image_of(done: &Done) -> &Rgba8Image {
    match &done.outcome {
        Outcome::Image(img) => img,
        _ => panic!("expected a thumbnail"),
    }
}

fn is(done: &Done, what: fn(&Outcome) -> bool) -> bool {
    what(&done.outcome)
}

fn document(w: u32, h: u32) -> Document {
    let mut d = Document::new("t", Size::new(w, h), ColorMode::Rgb, DocSample::U8);
    let fmt = d.pixel_format();
    d.layers.push(Layer::raster("pixels", fmt));
    d
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn kinds_times_and_sizes_read_in_words() {
    assert_eq!(kind_label("/a/b.JPG"), "JPEG");
    assert_eq!(kind_label("/a/b.tif"), "TIFF");
    assert_eq!(kind_label("/a/b.psd"), "PSD");
    assert_eq!(kind_label("/a/b.pcraft"), "PCRAFT");
    assert_eq!(kind_label("/a/README"), "File");

    let opened = 1_000_000;
    assert_eq!(when_label(opened, opened), "Just now");
    assert_eq!(when_label(opened + 59, opened), "Just now");
    assert_eq!(when_label(opened + 60, opened), "1 minute ago");
    assert_eq!(when_label(opened + 6 * 60, opened), "6 minutes ago");
    assert_eq!(when_label(opened + 3 * 3600, opened), "3 hours ago");
    assert_eq!(when_label(opened + 2 * 86_400, opened), "2 days ago");
    assert_eq!(when_label(opened, opened + 500), "Just now", "a time in the future is now");

    assert_eq!(size_label(1), "1 KB");
    assert_eq!(size_label(3 * 1024), "3 KB");
    assert_eq!(size_label(184 << 20), "184.0 MB");
    assert_eq!(size_label(2 << 30), "2.0 GB");
}

#[test]
fn gray_keeps_alpha_and_removes_colour() {
    let mut px = vec![255, 0, 0, 200, 0, 0, 255, 10];
    gray_out(&mut px);
    assert_eq!(px[0], px[1]);
    assert_eq!(px[1], px[2]);
    assert_eq!(px[3], 200);
    assert_eq!(px[7], 10);
    gray_out(&mut []);
    gray_out(&mut [1, 2, 3]); // a stray partial pixel is left alone
}

#[test]
fn cache_keys_follow_the_path_size_and_modified_time() {
    let k = cache_key("/a.psd", 10, Some(5));
    assert_eq!(k, cache_key("/a.psd", 10, Some(5)));
    assert_ne!(k, cache_key("/b.psd", 10, Some(5)));
    assert_ne!(k, cache_key("/a.psd", 11, Some(5)));
    assert_ne!(k, cache_key("/a.psd", 10, Some(6)));
    assert_ne!(cache_key("/a.psd", u64::MAX, None), MISSING_KEY);
}

#[test]
fn the_cache_replaces_older_versions_and_finds_a_missing_files_thumbnail() {
    let dir = tempfile::tempdir().unwrap();
    let (red, blue) = (rgba(4, 2, [255, 0, 0, 255]), rgba(4, 2, [0, 0, 255, 255]));
    disk::store(dir.path(), "/pics/a.png", 100, Some(1), &red);
    assert_eq!(disk::load(dir.path(), "/pics/a.png", 100, Some(1)).as_ref(), Some(&red));
    assert!(disk::load(dir.path(), "/pics/a.png", 100, Some(2)).is_none(), "another version of the file");
    assert!(disk::load(dir.path(), "/pics/b.png", 100, Some(1)).is_none(), "another path");
    // A new version replaces the old one.
    disk::store(dir.path(), "/pics/a.png", 120, Some(2), &blue);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert!(disk::load(dir.path(), "/pics/a.png", 100, Some(1)).is_none());
    // The file is gone (no stat): the newest cached one is found by its path.
    assert_eq!(disk::load_any(dir.path(), "/pics/a.png").as_ref(), Some(&blue));
    assert!(disk::load_any(dir.path(), "/pics/other.png").is_none());
    assert!(disk::load_any(&dir.path().join("no such folder"), "/pics/a.png").is_none());
}

#[test]
fn the_cache_deletes_its_oldest_files_past_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let img = rgba(2, 2, [9, 9, 9, 255]);
    for i in 0..12 {
        std::fs::write(dir.path().join(file_name(&format!("/p/{i}.png"), 1, Some(i))), png(&img)).unwrap();
    }
    disk::evict(dir.path(), 5);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 5);
    disk::evict(dir.path(), 5);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 5, "nothing more to delete");
}

#[test]
fn a_picture_is_decoded_cached_and_kept_after_the_file_is_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    let file = write(dir.path(), "pic.png", &png(&rgba(40, 20, [10, 200, 30, 255])));

    let first = probe(&file, true, None, Some(&cache));
    assert!(matches!(first.stat, Some(FileStat::Present { size, .. }) if size > 0));
    let img = image_of(&first);
    assert_eq!((img.width, img.height), (40, 20));
    // Through the import and the compositor: the same colour to within rounding.
    assert!(img.pixels[..4].iter().zip([10u8, 200, 30, 255]).all(|(a, b)| a.abs_diff(b) <= 2), "{:?}", &img.pixels[..4]);
    let key = first.key.unwrap();
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 1, "cached");

    // The same file again, and the thumbnail is already held.
    assert!(is(&probe(&file, true, Some(key), Some(&cache)), |o| matches!(o, Outcome::Unchanged)));
    // Only its status is wanted.
    let stat_only = probe(&file, false, None, Some(&cache));
    assert!(is(&stat_only, |o| matches!(o, Outcome::Skipped)) && stat_only.stat.is_some());
    // A changed file (another size) is looked at again.
    std::fs::write(&file, b"changed to something that is no picture").unwrap();
    let rewritten = probe(&file, true, None, Some(&cache));
    assert!(is(&rewritten, |o| matches!(o, Outcome::NoPreview)));

    // Back as it was: the cached thumbnail serves. Then deleted: missing, with the thumbnail it had.
    std::fs::write(&file, png(&rgba(40, 20, [10, 200, 30, 255]))).unwrap();
    let again = probe(&file, true, None, Some(&cache));
    assert!(is(&again, |o| matches!(o, Outcome::Image(_))));
    std::fs::remove_file(&file).unwrap();
    let gone = probe(&file, true, None, Some(&cache));
    assert_eq!(gone.stat, Some(FileStat::Missing));
    assert_eq!(gone.key, Some(MISSING_KEY));
    assert!(is(&gone, |o| matches!(o, Outcome::Image(_))), "the cached thumbnail is kept");
    assert!(is(&probe(&file, true, Some(MISSING_KEY), Some(&cache)), |o| matches!(o, Outcome::Unchanged)));
    // Never seen before and gone: nothing to show, and still just "missing".
    let never = probe(&dir.path().join("never.png"), true, None, Some(&cache));
    assert_eq!(never.stat, Some(FileStat::Missing));
    assert!(is(&never, |o| matches!(o, Outcome::NoPreview)));
    // Without a cache folder it all works in memory.
    let plain = write(dir.path(), "plain.png", &png(&rgba(3, 3, [1, 2, 3, 255])));
    assert!(is(&probe(&plain, true, None, None), |o| matches!(o, Outcome::Image(_))));
}

#[test]
fn a_file_that_is_no_picture_has_no_preview_and_is_not_missing() {
    let dir = tempfile::tempdir().unwrap();
    let text = write(dir.path(), "notes.txt", b"hello");
    let done = probe(&text, true, None, None);
    assert!(matches!(done.stat, Some(FileStat::Present { size: 5, .. })));
    assert!(is(&done, |o| matches!(o, Outcome::NoPreview)));
    // A folder is not a picture either.
    let folder = probe(dir.path(), true, None, None);
    assert!(matches!(folder.stat, Some(FileStat::Present { size: 0, .. })));
    assert!(is(&folder, |o| matches!(o, Outcome::NoPreview)));
}

#[test]
fn a_file_over_the_size_cap_is_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("huge.png");
    let f = std::fs::File::create(&big).unwrap();
    f.set_len(MAX_DECODE_BYTES + 1).unwrap(); // a hole on most file systems: no bytes to write
    drop(f);
    let done = probe(&big, true, None, None);
    assert!(matches!(done.stat, Some(FileStat::Present { size, .. }) if size == MAX_DECODE_BYTES + 1));
    assert!(is(&done, |o| matches!(o, Outcome::NoPreview)));
}

#[test]
fn a_psd_shows_the_preview_it_embeds_without_being_read_whole() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = photocraft_io::export(&document(64, 32), "t.psd", &photocraft_io::ExportOptions::default()).unwrap().bytes;
    let file = write(dir.path(), "t.psd", &bytes);
    let done = probe(&file, true, None, None);
    let img = image_of(&done);
    assert_eq!((img.width, img.height), (64, 32));
    // Damaged and truncated PSDs are "No preview", not a crash.
    let garbage = write(dir.path(), "garbage.psd", &[0x55; 300]);
    assert!(is(&probe(&garbage, true, None, None), |o| matches!(o, Outcome::NoPreview)));
    for cut in [0, 10, 26, 40, 60] {
        let short = write(dir.path(), "short.psd", &bytes[..cut]);
        assert!(is(&probe(&short, true, None, None), |o| matches!(o, Outcome::NoPreview)), "cut at {cut}");
    }
}

#[test]
fn a_pcraft_shows_its_embedded_thumbnail_and_damaged_ones_show_none() {
    let dir = tempfile::tempdir().unwrap();
    let opts = photocraft_format::SaveOptions { thumbnail: Some(rgba(8, 6, [255, 0, 0, 255])), ..Default::default() };
    let bytes = photocraft_format::save_to_bytes(&document(16, 12), &opts).unwrap();
    let file = write(dir.path(), "a.pcraft", &bytes);
    let done = probe(&file, true, None, None);
    let img = image_of(&done);
    assert_eq!((img.width, img.height), (8, 6));
    assert_eq!(&img.pixels[..4], &[255, 0, 0, 255]);
    let garbage = write(dir.path(), "garbage.pcraft", &[7; 500]);
    assert!(is(&probe(&garbage, true, None, None), |o| matches!(o, Outcome::NoPreview)));
    for cut in [0, 4, bytes.len() / 3, bytes.len() / 2, bytes.len() - 1] {
        let short = write(dir.path(), "short.pcraft", &bytes[..cut]);
        assert!(is(&probe(&short, true, None, None), |o| matches!(o, Outcome::NoPreview)), "cut at {cut}");
    }
}

#[test]
fn opening_a_document_seeds_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    let file = write(dir.path(), "doc.psd", b"not read: the cache is seeded from the open document");
    let path = file.to_string_lossy().into_owned();
    let shared = Shared::default();
    shared.allow(&path);
    disk::seed(&path, &document(30, 20), Some(cache.as_path()), &shared);
    let (stat, _) = disk::stat(&path);
    let FileStat::Present { size, modified } = stat else { panic!("the file exists") };
    let img = disk::load(&cache, &path, size, modified).expect("cached");
    assert_eq!((img.width, img.height), (30, 20));
    // No cache folder, or a file that is not there: nothing to do.
    disk::seed(&path, &document(4, 4), None, &shared);
    shared.allow("/no/such/file.psd");
    disk::seed("/no/such/file.psd", &document(4, 4), Some(cache.as_path()), &shared);
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 1);
}

#[test]
fn big_pictures_are_reduced_by_averaging() {
    let img = disk::fit(rgba(1000, 500, [100, 150, 200, 255]), 100);
    assert_eq!((img.width, img.height), (100, 50));
    assert_eq!(img.pixels.len(), 100 * 50 * 4);
    assert_eq!(&img.pixels[..4], &[100, 150, 200, 255]);
    let small = disk::fit(rgba(10, 5, [1, 2, 3, 4]), 100);
    assert_eq!((small.width, small.height), (10, 5));
    let odd = disk::fit(rgba(7, 1000, [0; 4]), 3);
    assert_eq!((odd.width, odd.height), (1, 3));
}

/// Frames of the screen: the worker finds the status and a thumbnail of each listed file.
#[test]
fn the_view_asks_the_workers_and_shows_what_they_find() {
    let dir = tempfile::tempdir().unwrap();
    let pic = write(dir.path(), "pic.png", &png(&rgba(30, 10, [1, 2, 3, 255]))).to_string_lossy().into_owned();
    let text = write(dir.path(), "t.txt", b"x").to_string_lossy().into_owned();
    let gone = dir.path().join("gone.png").to_string_lossy().into_owned();
    let paths = vec![pic.clone(), text.clone(), gone.clone()];
    let ctx = egui::Context::default();
    let mut view = RecentView::default();
    assert!(view.status(&pic).is_none(), "not statted yet is not missing");
    let cache = dir.path().join("cache");
    for _ in 0..500 {
        // The pass's texture uploads are not drawn anywhere: drop them on purpose.
        ctx.run_ui(Default::default(), |ui| {
            view.begin(ui.ctx(), &paths);
            for p in &paths {
                view.want(p, 1);
            }
            view.finish(ui.ctx(), Some(cache.as_path()));
        })
        .textures_delta
        .clear();
        let settled = paths.iter().all(|p| view.status(p).is_some() && !matches!(view.preview(p), Preview::Loading));
        if settled {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(matches!(view.status(&pic), Some(FileStat::Present { .. })));
    assert!(matches!(view.preview(&pic), Preview::Image { size: [30.0, 10.0], gray: false, .. }));
    assert!(matches!(view.status(&text), Some(FileStat::Present { .. })));
    assert!(matches!(view.preview(&text), Preview::None));
    assert_eq!(view.status(&gone), Some(FileStat::Missing));
    assert!(matches!(view.preview(&gone), Preview::None));
    assert_eq!(view.textures(), 1);
    // A file taken off the list is forgotten.
    ctx.run_ui(Default::default(), |ui| view.begin(ui.ctx(), std::slice::from_ref(&pic))).textures_delta.clear();
    assert!(view.status(&text).is_none());
}

#[test]
fn a_thumbnail_of_a_file_that_goes_missing_turns_gray() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    let file = write(dir.path(), "pic.png", &png(&rgba(30, 10, [200, 20, 20, 255])));
    let path = file.to_string_lossy().into_owned();
    let ctx = egui::Context::default();
    let mut view = RecentView::default();
    let paths = vec![path.clone()];
    let frame = |view: &mut RecentView| {
        ctx.run_ui(Default::default(), |ui| {
            view.begin(ui.ctx(), &paths);
            view.want(&path, 1);
            view.finish(ui.ctx(), Some(cache.as_path()));
        })
        .textures_delta
        .clear();
    };
    for _ in 0..500 {
        frame(&mut view);
        if matches!(view.preview(&path), Preview::Image { .. }) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(matches!(view.preview(&path), Preview::Image { gray: false, .. }));
    std::fs::remove_file(&file).unwrap();
    // The screen is up for a while: its statuses are looked at again.
    view.generation += 1;
    for _ in 0..500 {
        frame(&mut view);
        if matches!(view.preview(&path), Preview::Image { gray: true, .. }) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(view.status(&path), Some(FileStat::Missing));
    assert!(matches!(view.preview(&path), Preview::Image { gray: true, .. }), "dimmed, with its thumbnail");
}

#[test]
fn only_so_many_textures_are_kept() {
    let ctx = egui::Context::default();
    let mut view = RecentView::default();
    let mut answers = Vec::new();
    for i in 0..MAX_TEXTURES + 5 {
        let path = format!("/p/{i}.png");
        view.entries.insert(path.clone(), Entry::new());
        answers.push(Done {
            path,
            id: 0,
            stat: Some(FileStat::Present { size: 1, modified: Some(1) }),
            key: Some(i as u64),
            outcome: Outcome::Image(rgba(2, 2, [1, 1, 1, 255])),
        });
    }
    for (n, done) in answers.into_iter().enumerate() {
        view.tick = n as u64 + 1;
        view.apply(&ctx, done);
    }
    view.drop_unused_textures();
    assert_eq!(view.textures(), MAX_TEXTURES);
    // The ones shown longest ago went; the newest stay.
    assert!(matches!(view.preview("/p/0.png"), Preview::Loading));
    assert!(matches!(view.preview(&format!("/p/{}.png", MAX_TEXTURES + 4)), Preview::Image { .. }));
    // A thumbnail with the wrong number of pixels is no preview.
    view.entries.insert("/bad.png".into(), Entry::new());
    view.apply(
        &ctx,
        Done { path: "/bad.png".into(), id: 0, stat: None, key: None, outcome: Outcome::Image(Rgba8Image { width: 4, height: 4, pixels: vec![0; 5] }) },
    );
    assert!(matches!(view.preview("/bad.png"), Preview::None));
}

/// A PNG that is a few dozen bytes but declares 60 000 × 60 000 pixels (3.6 gigapixels).
fn huge_png() -> Vec<u8> {
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&60_000u32.to_be_bytes());
    ihdr.extend_from_slice(&60_000u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    for (kind, data) in [(b"IHDR", ihdr.as_slice()), (b"IEND", &[][..])] {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(data);
        out.extend_from_slice(&crc.sum().to_be_bytes());
    }
    out
}

/// A small file that declares an enormous picture is refused by the codec's limits, before any
/// of it is allocated, instead of being imported.
#[test]
fn a_picture_that_declares_a_huge_size_is_not_decoded() {
    let bytes = huge_png();
    assert!(bytes.len() < 100);
    let opts = photocraft_codecs::DecodeOptions { limits: disk::decode_limits(), ..Default::default() };
    assert!(
        matches!(photocraft_codecs::decode_with(&bytes, &opts), Err(photocraft_codecs::CodecError::LimitExceeded(_))),
        "the limits, not a missing image, refuse it"
    );
    let dir = tempfile::tempdir().unwrap();
    let file = write(dir.path(), "bomb.png", &bytes);
    assert!(disk::make(&file.to_string_lossy(), bytes.len() as u64, false).is_none());
    let done = probe(&file, true, None, None);
    assert!(matches!(done.stat, Some(FileStat::Present { .. })));
    assert!(is(&done, |o| matches!(o, Outcome::NoPreview)));
    // The limits are about 100 megapixels and 512 MiB.
    let l = disk::decode_limits();
    assert!(l.max_pixels <= 100_000_000 && l.max_alloc <= 512 << 20);
}

/// Bytes that merely look like nothing known, and a text file named like a picture, have no preview.
#[test]
fn unknown_bytes_named_like_pictures_have_no_preview() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.png", "b.dng", "c.cr2", "d.svg", "e.tif"] {
        let file = write(dir.path(), name, b"definitely not a picture");
        assert!(is(&probe(&file, true, None, None), |o| matches!(o, Outcome::NoPreview)), "{name}");
    }
}

/// Forgetting deletes cache-named thumbnails (one path's, or all) and nothing else.
#[test]
fn forgetting_deletes_only_the_caches_own_files() {
    let dir = tempfile::tempdir().unwrap();
    let img = rgba(2, 2, [5, 6, 7, 255]);
    disk::store(dir.path(), "/a/one.png", 1, Some(1), &img);
    disk::store(dir.path(), "/a/two.png", 1, Some(1), &img);
    let unrelated = [write(dir.path(), "notes.txt", b"keep"), write(dir.path(), "photo.png", b"keep"), write(dir.path(), "0123456789abcdef.png", b"keep")];
    std::fs::create_dir(dir.path().join("0123456789abcdef_1_1.png.d")).unwrap();
    disk::forget(dir.path(), Some("/a/one.png"));
    assert!(disk::load_any(dir.path(), "/a/one.png").is_none());
    assert!(disk::load_any(dir.path(), "/a/two.png").is_some(), "another path's thumbnail stays");
    disk::forget(dir.path(), None);
    assert!(disk::load_any(dir.path(), "/a/two.png").is_none());
    assert!(unrelated.iter().all(|f| f.is_file()), "files with other names stay");
    assert!(dir.path().join("0123456789abcdef_1_1.png.d").is_dir());
    assert!(dir.path().is_dir(), "the folder stays");
    disk::forget(&dir.path().join("missing"), None); // no folder: nothing to do
}

/// A file taken off the list is no longer held, and nothing more is cached for it.
#[test]
fn a_forgotten_file_is_dropped_from_the_view() {
    let mut view = RecentView::default();
    for p in ["/a/one.png", "/a/two.png"] {
        view.entries.insert(p.into(), Entry::new());
        view.shared.allow(p);
    }
    view.forget(None, Some("/a/one.png"));
    assert!(!view.entries.contains_key("/a/one.png"));
    let dir = tempfile::tempdir().unwrap();
    let img = rgba(2, 2, [5, 6, 7, 255]);
    let shared = view.shared.clone();
    let write_to = |path: &str| shared.store(path, || disk::store(dir.path(), path, 1, Some(1), &img));
    write_to("/a/one.png");
    assert!(disk::load_any(dir.path(), "/a/one.png").is_none(), "a late write for a forgotten file is not made");
    write_to("/a/two.png");
    assert!(disk::load_any(dir.path(), "/a/two.png").is_some());
    view.forget(None, None);
    assert!(view.entries.is_empty());
    write_to("/a/two.png");
    assert!(disk::load_any(dir.path(), "/a/two.png").is_some(), "the earlier file stays; only new writes were stopped");
}

/// A seed queued before the file left the list is cancelled by `forget`, and listing the file
/// again allows it once more.
#[test]
fn forgetting_cancels_a_queued_seed_and_listing_again_allows_it() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    let file = write(dir.path(), "doc.psd", b"x");
    let path = file.to_string_lossy().into_owned();
    let mut view = RecentView::default();
    view.shared.allow(&path);
    view.forget(None, Some(&path));
    disk::seed(&path, &document(8, 8), Some(cache.as_path()), &view.shared);
    assert!(!cache.exists() || std::fs::read_dir(&cache).unwrap().count() == 0, "nothing cached after the removal");
    view.shared.allow(&path);
    disk::seed(&path, &document(8, 8), Some(cache.as_path()), &view.shared);
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 1);
}

/// The answer to a probe of an entry that was removed and listed again is not for the new entry.
#[test]
fn a_stale_probe_answer_is_ignored() {
    let ctx = egui::Context::default();
    let mut view = RecentView::default();
    view.want("/a/one.png", 0);
    let old = view.entries["/a/one.png"].id;
    view.forget(None, Some("/a/one.png"));
    view.want("/a/one.png", 0);
    let new = view.entries["/a/one.png"].id;
    assert_ne!(old, new);
    view.entries.get_mut("/a/one.png").unwrap().in_flight = true;
    view.apply(&ctx, Done { path: "/a/one.png".into(), id: old, stat: Some(FileStat::Missing), key: None, outcome: Outcome::Skipped });
    assert_eq!(view.status("/a/one.png"), None, "the old answer changes nothing");
    assert!(view.entries["/a/one.png"].in_flight);
    view.apply(&ctx, Done { path: "/a/one.png".into(), id: new, stat: Some(FileStat::Missing), key: None, outcome: Outcome::Skipped });
    assert_eq!(view.status("/a/one.png"), Some(FileStat::Missing));
}

/// A status-only refresh that finds the file changed drops the thumbnail shown for it.
#[test]
fn a_changed_status_drops_the_thumbnail() {
    let ctx = egui::Context::default();
    let mut view = RecentView::default();
    let present = |size| Some(FileStat::Present { size, modified: Some(1) });
    view.entries.insert("/p.png".into(), Entry::new());
    view.apply(&ctx, Done { path: "/p.png".into(), id: 0, stat: present(1), key: Some(1), outcome: Outcome::Image(rgba(2, 2, [1, 1, 1, 255])) });
    assert!(matches!(view.preview("/p.png"), Preview::Image { .. }));
    view.apply(&ctx, Done { path: "/p.png".into(), id: 0, stat: present(1), key: None, outcome: Outcome::Skipped });
    assert!(matches!(view.preview("/p.png"), Preview::Image { .. }), "same status: kept");
    view.apply(&ctx, Done { path: "/p.png".into(), id: 0, stat: present(2), key: None, outcome: Outcome::Skipped });
    assert!(matches!(view.preview("/p.png"), Preview::Loading), "changed: made again when shown");
}

/// A file that goes from missing to unreadable (not missing) gets its colour thumbnail back.
#[test]
fn an_unavailable_file_does_not_keep_its_gray_thumbnail() {
    let ctx = egui::Context::default();
    let mut view = RecentView::default();
    view.entries.insert("/p.png".into(), Entry::new());
    let image = || Outcome::Image(rgba(2, 2, [1, 1, 1, 255]));
    view.apply(&ctx, Done { path: "/p.png".into(), id: 0, stat: Some(FileStat::Missing), key: Some(1), outcome: image() });
    assert!(matches!(view.entries.get("/p.png").map(|e| &e.thumb), Some(Thumb::Ready { gray: true, .. })));
    view.apply(&ctx, Done { path: "/p.png".into(), id: 0, stat: Some(FileStat::Unavailable), key: Some(1), outcome: Outcome::Unchanged });
    assert!(matches!(view.preview("/p.png"), Preview::Loading), "made again, in colour, when shown");
}

/// Zero bytes read as 0 KB, not 1 KB.
#[test]
fn an_empty_file_is_zero_kb() {
    assert_eq!(size_label(0), "0 KB");
    assert_eq!(size_label(1), "1 KB");
    assert_eq!(size_label(1024), "1 KB");
}

/// Eviction deletes only the cache's own files, whatever their age.
#[test]
fn eviction_leaves_other_files_alone() {
    let dir = tempfile::tempdir().unwrap();
    let keep = write(dir.path(), "notes.txt", b"keep");
    let img = rgba(2, 2, [9, 9, 9, 255]);
    for i in 0..6 {
        std::fs::write(dir.path().join(file_name(&format!("/p/{i}.png"), 1, Some(i))), png(&img)).unwrap();
    }
    disk::evict(dir.path(), 2);
    assert!(keep.is_file());
    assert!(std::fs::read_dir(dir.path()).unwrap().count() <= 3);
}

/// Only "not found" is missing; a modified time keeps its sub-second part.
#[test]
fn stat_tells_not_found_from_other_failures() {
    let dir = tempfile::tempdir().unwrap();
    let file = write(dir.path(), "a.png", b"x");
    let (stat, _) = disk::stat(&file.to_string_lossy());
    let FileStat::Present { modified: Some(nanos), .. } = stat else { panic!("present with a time") };
    assert!(nanos > 1_000_000_000_000_000_000 / 1000, "nanoseconds since the epoch, not seconds");
    assert_eq!(disk::stat(&dir.path().join("none.png").to_string_lossy()).0, FileStat::Missing);
    assert_eq!(disk::stat(&file.join("below.png").to_string_lossy()).0, FileStat::Missing, "below a file: not there");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        let inner = write(&locked, "b.png", b"x");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let denied = matches!(std::fs::metadata(&inner), Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied);
        let got = disk::stat(&inner.to_string_lossy()).0;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        if denied {
            assert_eq!(got, FileStat::Unavailable, "no permission is not missing");
        }
    }
}

/// Two versions of a file a moment apart do not share a cache file.
#[test]
fn modified_times_keep_nanoseconds_in_the_cache_name() {
    assert_ne!(file_name("/a.psd", 10, Some(5_000_000_001)), file_name("/a.psd", 10, Some(5_000_000_002)));
    assert_ne!(cache_key("/a.psd", 10, Some(5_000_000_001)), cache_key("/a.psd", 10, Some(5_000_000_002)));
}

/// An older thumbnail goes only after the new one is in place.
#[test]
fn a_failed_store_keeps_the_older_thumbnail() {
    let dir = tempfile::tempdir().unwrap();
    let red = rgba(4, 2, [255, 0, 0, 255]);
    disk::store(dir.path(), "/pics/a.png", 100, Some(1), &red);
    // A folder is where the new file would go: the rename fails.
    std::fs::create_dir(dir.path().join(file_name("/pics/a.png", 120, Some(2)))).unwrap();
    disk::store(dir.path(), "/pics/a.png", 120, Some(2), &rgba(4, 2, [0, 0, 255, 255]));
    assert_eq!(disk::load(dir.path(), "/pics/a.png", 100, Some(1)).as_ref(), Some(&red));
}
