use super::*;
use photocraft_codecs::{ChannelLayout, EncodeOptions, Image};
use photocraft_engine::Session;
use photocraft_format::list_recovery;
use photocraft_ui_egui::{PhotocraftApp, prefs_ui};
use serde_json::json;

/// #6607: a PSD/PSB save streams the file model into the temporary file, which then holds
/// exactly the exported bytes; a serializer error halfway leaves the old file and no
/// temporary file behind.
#[test]
fn a_psd_save_streams_the_exported_bytes_and_keeps_the_old_file_on_error() {
    let dir = temp("stream");
    let mut doc = Document::new("streamed", photocraft_geom::Size::new(64, 48), photocraft_doc::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let fmt = doc.pixel_format();
    let px: Vec<u8> = (0..64 * 48 * fmt.bytes_per_pixel()).map(|i| (i * 7 % 251) as u8).collect();
    doc.layers
        .push(photocraft_doc::Layer::new("A", photocraft_doc::LayerContent::Raster(photocraft_raster::Surface::from_interleaved(fmt, doc.bounds(), &px))));
    let opts = photocraft_io::ExportOptions::default();
    for name in ["a.psd", "a.psb"] {
        let path = dir.join(name);
        std::fs::write(&path, b"old").unwrap();
        let want = photocraft_io::export(&doc, name, &opts).unwrap().bytes;
        let r = photocraft_io::encode_with(&doc, name, &opts, &photocraft_raster::Interrupt::NONE).unwrap();
        let photocraft_io::Encoded::Psd(mut file) = r.encoded else { panic!("{name}: not streamed") };
        let seen = std::cell::RefCell::new(Vec::new());
        write_encoded_atomic(&path, &photocraft_io::Encoded::Psd(file.clone()), &|f| seen.borrow_mut().push(f)).unwrap();
        assert!(std::fs::read(&path).unwrap() == want, "{name}: the saved file differs from the export");
        assert_eq!(seen.borrow().last().copied(), Some(1.0), "{name}");

        let info = file.layer_info.as_mut().unwrap();
        let layer = info.layers[0].clone();
        info.layers = vec![layer; 32768];
        let e = write_encoded_atomic(&path, &photocraft_io::Encoded::Psd(file), &|_| {}).unwrap_err();
        assert!(e.contains("32767") && e.contains("not changed"), "{name}: {e}");
        assert!(std::fs::read(&path).unwrap() == want, "{name}: a failed save replaced the file");
        let temps: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).filter(|n| n.to_string_lossy().ends_with(".tmp")).collect();
        assert!(temps.is_empty(), "{name}: left behind {temps:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Regression using the real clipboard on an isolated Windows CI runner.
#[cfg(target_os = "windows")]
#[test]
#[ignore = "requires exclusive use of the Windows clipboard"]
fn busy_native_clipboard_preserves_copied_shape() {
    for id in ["edit.paste", "edit.pasteSpecial.pasteInPlace"] {
        let mut native = native(None, craft::host::Wake::none());
        let services =
            Services { clipboard_set_image: native.clipboard_set_image.take(), clipboard_get_image: native.clipboard_get_image.take(), ..Default::default() };
        let mut app = PhotocraftApp::new(Session::new(), services);
        app.run("file.new", json!({"width": 80, "height": 60})).unwrap();
        app.run("shape.create", json!({"kind": "rect", "rect": [10, 10, 40, 20], "fill": "#ff0000"})).unwrap();
        app.run("edit.copy", json!({})).unwrap();
        assert!(app.session.clipboard.as_ref().unwrap().layers.is_some());
        assert!(app.services.clipboard_get_image.as_mut().unwrap()().unwrap().is_some(), "native export succeeded");
        app.run("file.new", json!({"width": 80, "height": 60})).unwrap();

        // Another thread holds the OS clipboard open beyond arboard's bounded retry window.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            // A non-null window distinguishes this owner from arboard's null-window opens.
            let guard = winsafe::HWND::GetDesktopWindow().OpenClipboard().unwrap();
            ready_tx.send(()).unwrap();
            release_rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
            drop(guard);
        });
        ready_rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        let contention = arboard::Clipboard::new().unwrap().get_image().err();
        let first = app.run(id, json!({}));
        release_tx.send(()).unwrap();
        holder.join().unwrap();
        assert!(matches!(contention, Some(arboard::Error::ClipboardOccupied)), "diagnostic must actually occupy the clipboard: {contention:?}");
        eprintln!("paste while clipboard occupied: {first:?}");
        assert!(first.is_err(), "report contention rather than a successful no-op");
        assert!(
            app.session.clipboard.as_ref().is_some_and(|clip| clip.layers.is_some()),
            "a temporary OS read failure must not discard the copied editable shape"
        );
        app.run(id, json!({})).unwrap();
        let state = app.session.active().unwrap();
        assert!(
            matches!(state.doc.layer(state.active_layer.unwrap()).unwrap().content, photocraft_doc::LayerContent::Shape(_)),
            "retry keeps the shape editable"
        );
    }
}

#[test]
fn xwayland_command_matches_how_photocraft_was_installed() {
    // Flatpak: WAYLAND_DISPLAY doesn't reach the sandbox's socket choice; flatpak's flags do.
    let flatpak = xwayland_command(Some("ai.storyteller.photocraft"), None, false);
    assert_eq!(flatpak.as_deref(), Some("flatpak run --nosocket=wayland --socket=x11 ai.storyteller.photocraft"));
    // AppImage: its own path, quoted for the shell (spaces, quotes).
    let appimage = xwayland_command(None, Some("/home/me/My Apps/it's.AppImage"), true);
    assert_eq!(appimage.as_deref(), Some(r"WAYLAND_DISPLAY= '/home/me/My Apps/it'\''s.AppImage'"));
    let bare = xwayland_command(None, Some("/opt/photocraft-0.3.0-linux-x86_64.AppImage"), true);
    assert_eq!(bare.as_deref(), Some("WAYLAND_DISPLAY= /opt/photocraft-0.3.0-linux-x86_64.AppImage"));
    // deb, rpm, AUR, tarball: the binary on the PATH.
    assert_eq!(xwayland_command(None, None, true).as_deref(), Some("WAYLAND_DISPLAY= photocraft"));
    assert_eq!(xwayland_command(Some(""), Some(""), true).as_deref(), Some("WAYLAND_DISPLAY= photocraft"));
    // No X server (XWayland disabled): nothing to suggest.
    assert_eq!(xwayland_command(None, Some("/opt/p.AppImage"), false), None);
}

/// A name in another letter case reaches the same file where the file system ignores case
/// (Windows, and macOS by default), and only there. Two files with the same contents are two.
#[test]
fn same_file_follows_the_file_system() {
    let dir = std::env::temp_dir().join(format!("photocraft-same-file-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (cat, dog, upper) = (dir.join("CAT.psd"), dir.join("dog.psd"), dir.join("CAT.PSD"));
    std::fs::write(&cat, b"same").unwrap();
    std::fs::write(&dog, b"same").unwrap();
    assert!(same_file(&cat, &cat));
    assert!(!same_file(&cat, &dog));
    assert_eq!(same_file(&upper, &cat), upper.exists(), "the same file only where letter case is ignored");
    assert!(!same_file(&dir.join("missing.psd"), &cat));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A hard link is another entry in the folder, though it reaches the same file: a save to it
/// replaces that entry and leaves the other name on the old file. Another spelling of a name,
/// where letter case is ignored, is the same entry.
#[test]
fn a_hard_link_is_another_entry() {
    let dir = std::env::temp_dir().join(format!("photocraft-same-entry-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("other")).unwrap();
    let (cat, bare, upper, other) = (dir.join("CAT.psd"), dir.join("CAT"), dir.join("CAT.PSD"), dir.join("other").join("CAT.psd"));
    std::fs::write(&cat, b"cat").unwrap();
    std::fs::hard_link(&cat, &bare).unwrap();
    std::fs::hard_link(&cat, &other).unwrap();
    // Where both names reach one file (one inode on Unix), the folder still lists both.
    let one_file = |_: &Path, _: &Path| true;
    assert!(!same_entry(&bare, &cat, one_file));
    assert!(!same_entry(&bare, &cat, same_file));
    assert!(!same_entry(&other, &cat, same_file), "another folder");
    assert!(same_entry(&cat, &cat, one_file));
    assert!(same_entry(&cat, &cat, same_file));
    if upper.exists() {
        assert!(same_entry(&upper, &cat, same_file), "one entry where letter case is ignored");
    } else {
        std::fs::hard_link(&cat, &upper).unwrap();
        assert!(!same_entry(&upper, &cat, same_file), "two entries where letter case tells names apart");
    }
    assert!(!same_entry(&dir.join("missing.psd"), &cat, same_file));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn block_on_waits_for_a_wake_from_another_thread() {
    let (tx, rx) = std::sync::mpsc::channel::<std::task::Waker>();
    let mut woken = false;
    let ready = std::future::poll_fn(move |cx| {
        if woken {
            return std::task::Poll::Ready(7);
        }
        woken = true;
        let _ = tx.send(cx.waker().clone());
        std::task::Poll::Pending
    });
    let waker = std::thread::spawn(move || rx.recv().unwrap().wake());
    assert_eq!(block_on(ready), 7);
    waker.join().unwrap();
}

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

/// One app launch with crash recovery in `dir` (dropping it is the crash: background writes
/// already queued finish, nothing else runs).
fn launch(dir: &Path) -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(Session::new(), recovery_services(Some(dir.to_path_buf())));
    let ctx = app.ctx().clone();
    prefs_ui::tick(&mut app, &ctx, std::time::Instant::now());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.session.has_jobs() && std::time::Instant::now() < deadline {
        photocraft_ui_egui::jobs_ui::tick(&mut app, &ctx);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(!app.session.has_jobs(), "recovery finished");
    app
}

/// The first pixel of each open document, and whether it's unsaved.
fn open_docs(app: &PhotocraftApp) -> Vec<([f32; 4], bool)> {
    app.session.documents().iter().map(|d| (photocraft_compose::flatten(&d.doc).px.first().copied().unwrap_or_default(), d.is_dirty())).collect()
}

fn index_of(app: &PhotocraftApp, color: [f32; 4]) -> usize {
    open_docs(app).iter().position(|(c, _)| *c == color).unwrap()
}

fn new_doc(app: &mut PhotocraftApp, color: &str) {
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    app.run("edit.fill", json!({"color": color})).unwrap();
}

fn autosave(app: &mut PhotocraftApp, ctx: &egui::Context) {
    prefs_ui::autosave_now(app);
    prefs_ui::tick(app, ctx, std::time::Instant::now());
}

#[test]
fn recovered_documents_survive_a_second_crash_until_saved_or_closed() {
    let dir = std::env::temp_dir().join(format!("photocraft-recovery-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // Launch 1: two unsaved documents are autosaved, then the app crashes.
    {
        let mut app = launch(&dir);
        let ctx = app.ctx().clone();
        new_doc(&mut app, "#ff0000");
        new_doc(&mut app, "#0000ff");
        autosave(&mut app, &ctx);
    }
    assert_eq!(list_recovery(&dir).len(), 2);
    // Launch 2 recovers both and crashes again before its first autosave (#444).
    {
        let mut app = launch(&dir);
        let ctx = app.ctx().clone();
        let mut docs = open_docs(&app);
        docs.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
        assert_eq!(docs, [(BLUE, true), (RED, true)], "recovered and still unsaved");
        prefs_ui::tick(&mut app, &ctx, std::time::Instant::now());
    }
    assert_eq!(list_recovery(&dir).len(), 2, "recovering keeps the entries on disk");
    // Launch 3 has both again; autosaving unchanged documents adds nothing, a new document
    // (whose id may repeat one from an earlier launch) gets an entry of its own.
    {
        let mut app = launch(&dir);
        let ctx = app.ctx().clone();
        assert_eq!(app.session.documents().len(), 2);
        autosave(&mut app, &ctx);
        new_doc(&mut app, "#00ff00");
        autosave(&mut app, &ctx);
    }
    assert_eq!(list_recovery(&dir).len(), 3);
    // Launch 4: closing and saving recovered documents removes their entries, no duplicates.
    let mut app = launch(&dir);
    let ctx = app.ctx().clone();
    assert_eq!(app.session.documents().len(), 3);
    let red = index_of(&app, RED);
    app.run("file.close", json!({"document": red})).unwrap();
    // Saving marks the revision saved (the file write itself is the save service's job).
    let blue = index_of(&app, BLUE);
    assert!(app.session.set_active(blue));
    let st = app.session.active_mut().unwrap();
    st.saved_revision = st.revision;
    prefs_ui::tick(&mut app, &ctx, std::time::Instant::now());
    let left = list_recovery(&dir);
    assert_eq!(left.len(), 1);
    assert_eq!(photocraft_compose::flatten(&photocraft_format::recover(&left[0]).unwrap()).px.first().copied(), Some(GREEN));
    let green = index_of(&app, GREEN);
    app.run("file.close", json!({"document": green})).unwrap();
    prefs_ui::tick(&mut app, &ctx, std::time::Instant::now());
    assert!(list_recovery(&dir).is_empty());
    drop(app);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A background autosave that fails reaches the status line, and the same revision is written at
/// the next interval with no edit in between (not before it). Time moves only through the `now`
/// each tick hands the autosave timer; ticks repeated at one `now` queue nothing new.
#[test]
fn failed_background_autosave_is_reported_and_retried_without_another_edit() {
    let dir = temp("autosave-retry");
    let recovery = dir.join("Recovery");
    // A regular file prevents creation of the recovery bundle directory.
    std::fs::write(&recovery, b"blocked").unwrap();
    let mut app = launch(&recovery);
    let ctx = app.ctx().clone();
    app.run("prefs.set", json!({"path": "fileHandling.autosaveMinutes", "value": 5})).unwrap();
    new_doc(&mut app, "#ff0000");
    let revision = app.session.active().unwrap().revision;
    assert_ne!(revision, 0, "a sidecar-less bundle lists as revision 0");
    let interval = std::time::Duration::from_secs(5 * 60);
    // The changed interval restarts the clock at this tick; the next one at the interval writes.
    let start = std::time::Instant::now();
    prefs_ui::tick(&mut app, &ctx, start);
    let first = start + interval;
    prefs_ui::tick(&mut app, &ctx, first);

    let mut failed = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        prefs_ui::tick(&mut app, &ctx, first);
        if app.ui.status.starts_with("Autosave failed:") {
            failed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(failed, "background write failures must reach the UI");
    assert!(app.ui.status_error);
    assert!(list_recovery(&recovery).is_empty());

    // No edit occurs. Before the next interval nothing is written, even with the folder back.
    std::fs::remove_file(&recovery).unwrap();
    std::fs::create_dir(&recovery).unwrap();
    prefs_ui::tick(&mut app, &ctx, first + interval / 2);
    let queued = app.services.flush_autosaves.as_mut().unwrap()();
    assert!(queued.is_empty() && list_recovery(&recovery).is_empty(), "retried before the interval: {queued:?}");
    // At the next interval the very same revision is written.
    let second = first + interval;
    prefs_ui::tick(&mut app, &ctx, second);
    // The worker commits the bundle's manifest before it publishes the sidecar, and
    // `list_recovery` lists a committed sidecar-less bundle (revision 0) on its own, so
    // the first non-empty listing can come from the middle of the write. Wait for the write
    // to finish: the entry then carries the sidecar's revision.
    let mut recovered = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        prefs_ui::tick(&mut app, &ctx, second);
        recovered = list_recovery(&recovery);
        if recovered.iter().any(|e| e.info.revision == revision) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(recovered.len(), 1, "unchanged revision should retry after failure");
    assert_eq!(recovered[0].info.revision, revision);
    let restored = photocraft_format::recover(&recovered[0]).unwrap();
    assert_eq!(photocraft_compose::flatten(&restored).px.first().copied(), Some(RED));
    drop(app);
    std::fs::remove_dir_all(dir).unwrap();
}

use std::sync::atomic::{AtomicUsize, Ordering};

fn temp(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("photocraft-services-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn red_png(w: u32, h: u32) -> Vec<u8> {
    let img = Image::from_u8(w, h, ChannelLayout::Rgba, [255u8, 0, 0, 255].repeat((w * h) as usize)).unwrap();
    photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &EncodeOptions::default()).unwrap()
}

/// A 1×1 uncompressed 32-bit TGA (TGA has no magic number).
fn tga_1x1() -> Vec<u8> {
    let mut b = vec![0u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 32, 8];
    b.extend_from_slice(&[0, 0, 255, 255]); // BGRA red
    b
}

#[test]
fn copied_image_file_pastes_its_pixels() {
    let dir = temp("png");
    let png = dir.join("red.png");
    std::fs::write(&png, red_png(3, 2)).unwrap();
    let (w, h, px) = image_from_files(&[png]).unwrap();
    assert_eq!((w, h), (3, 2));
    assert_eq!(px, [255u8, 0, 0, 255].repeat(6));
}

#[test]
fn crlf_from_a_uri_list_is_ignored() {
    // arboard splits text/uri-list on LF, so GNOME Files' CRLF list leaves a '\r' behind.
    let dir = temp("crlf");
    let png = dir.join("red image.png");
    std::fs::write(&png, red_png(2, 2)).unwrap();
    let with_cr = PathBuf::from(format!("{}\r", png.display()));
    assert_eq!(image_from_files(&[with_cr]).map(|(w, h, _)| (w, h)), Some((2, 2)));
}

#[test]
fn non_images_are_skipped_for_the_first_image() {
    let dir = temp("mixed");
    let (txt, png) = (dir.join("notes.txt"), dir.join("red.png"));
    std::fs::write(&txt, b"not an image").unwrap();
    std::fs::write(&png, red_png(4, 1)).unwrap();
    let missing = dir.join("gone.png");
    assert_eq!(image_from_files(&[missing.clone(), txt.clone(), dir.clone(), png]).map(|(w, h, _)| (w, h)), Some((4, 1)));
    assert!(image_from_files(&[missing, txt, dir]).is_none());
    assert!(image_from_files(&[]).is_none());
}

#[test]
fn a_damaged_image_is_skipped_not_a_panic() {
    let dir = temp("damaged");
    let bad = dir.join("cut.png");
    std::fs::write(&bad, &red_png(8, 8)[..20]).unwrap();
    assert!(image_from_files(&[bad]).is_none());
}

fn encoded(format: photocraft_codecs::Format, w: u32, h: u32) -> Vec<u8> {
    let img = Image::from_u8(w, h, ChannelLayout::Rgba, [255u8, 0, 0, 255].repeat((w * h) as usize)).unwrap();
    photocraft_codecs::encode(&img, format, &EncodeOptions::default()).unwrap()
}

#[test]
fn non_png_clipboard_images_paste() {
    // #2546: a screenshot tool that offers only image/bmp or image/jpeg pasted nothing.
    let bmp = encoded(photocraft_codecs::Format::Bmp, 3, 2);
    let offers = vec!["text/plain".to_string(), "image/bmp".to_string()];
    let mut asked = Vec::new();
    let got = image_from_offers(&offers, |mime| {
        asked.push(mime.to_string());
        Some(bmp.clone())
    });
    assert_eq!(got, Some((3, 2, [255u8, 0, 0, 255].repeat(6))));
    assert_eq!(asked, ["image/bmp"], "text offers are never fetched");

    let jpeg = encoded(photocraft_codecs::Format::Jpeg, 16, 8);
    let offers = vec!["image/jpeg".to_string()];
    assert_eq!(image_from_offers(&offers, |_| Some(jpeg.clone())).map(|(w, h, _)| (w, h)), Some((16, 8)));
}

#[test]
fn undecodable_clipboard_offers_fall_through_to_the_next() {
    let bmp = encoded(photocraft_codecs::Format::Bmp, 2, 2);
    let cut = encoded(photocraft_codecs::Format::Png, 8, 8)[..20].to_vec();
    let offers: Vec<String> = ["image/webp", "image/png", "image/x-tga", "image/bmp"].map(String::from).to_vec();
    let got = image_from_offers(&offers, |mime| match mime {
        "image/webp" => None,
        "image/png" => Some(cut.clone()),
        "image/x-tga" => Some(b"not an image".to_vec()),
        _ => Some(bmp.clone()),
    });
    assert_eq!(got.map(|(w, h, _)| (w, h)), Some((2, 2)));
    assert!(image_from_offers(&offers[..3], |_| Some(cut.clone())).is_none());
    assert!(image_from_offers(&[], |_| Some(bmp.clone())).is_none());
}

#[test]
fn clipboard_tga_is_trusted_only_by_its_type() {
    assert_eq!(image_from_offers(&["image/x-tga".to_string()], |_| Some(tga_1x1())), Some((1, 1, vec![255, 0, 0, 255])));
    assert!(image_from_offers(&["image/x-unknown".to_string()], |_| Some(tga_1x1())).is_none());
}

#[test]
fn tga_is_trusted_only_with_its_extension() {
    let dir = temp("tga");
    let (tga, bin) = (dir.join("red.tga"), dir.join("red.bin"));
    std::fs::write(&tga, tga_1x1()).unwrap();
    std::fs::write(&bin, tga_1x1()).unwrap();
    assert_eq!(image_from_files(&[tga]), Some((1, 1, vec![255, 0, 0, 255])));
    assert!(image_from_files(&[bin]).is_none());
}

/// A 121 × 81 white rectangle with a 1-unit black stroke, written as markup the way a vector
/// app's Copy puts it on the clipboard as text.
const COPIED_RECTANGLE: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="121" height="81" viewBox="0 0 121 81">
  <g><path d="M0.5 0.5 L120.5 0.5 L120.5 80.5 L0.5 80.5 Z" fill="#ffffff" stroke="#000000"/></g>
</svg>
"##;

/// A 40 × 20 drawing whose left half is a red square; the right half has no art.
const HALF_RED: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="20" height="20" fill="#ff0000"/></svg>"##;

/// The clipboard as `clipboard_image` reads it when it holds only `text`.
fn from_text(text: &str) -> Option<(u32, u32, Vec<u8>)> {
    clipboard_image(&mut (), |_| Ok(None), |_| Ok(None), |_| Ok(Some(text.to_string()))).expect("no read fails")
}

#[test]
fn svg_clipboard_source_preserves_markup_and_bounds_its_size() {
    let bytes = svg_clipboard_source(Some(HALF_RED.into())).unwrap().unwrap();
    assert_eq!(bytes, HALF_RED.as_bytes());
    for text in [None, Some("plain text".into()), Some("<p>text</p>".into())] {
        assert_eq!(svg_clipboard_source(text), Ok(None));
    }
    let mut oversized = HALF_RED.to_string();
    oversized.extend(std::iter::repeat_n(' ', (16 << 20) + 1));
    assert!(svg_clipboard_source(Some(oversized)).is_err());
}

#[test]
fn a_panicking_svg_clipboard_read_is_reported_and_can_be_retried() {
    let panics = Rc::new(Cell::new(true));
    let fail = panics.clone();
    let mut get = guarded_clipboard_svg(move || {
        if fail.get() {
            panic!("SVG clipboard reader bug");
        }
        Ok(Some(HALF_RED.as_bytes().to_vec()))
    });
    assert!(get().unwrap_err().contains("Paste failed"));
    panics.set(false);
    assert_eq!(get().unwrap().unwrap(), HALF_RED.as_bytes());
}

fn pixel(img: &(u32, u32, Vec<u8>), x: u32, y: u32) -> [u8; 4] {
    let at = ((y * img.0 + x) * 4) as usize;
    img.2[at..at + 4].try_into().unwrap()
}

#[test]
fn svg_markup_copied_as_text_pastes_its_pixels() {
    let img = from_text(COPIED_RECTANGLE).expect("the drawing's pixels");
    assert_eq!((img.0, img.1, img.2.len()), (121, 81, 121 * 81 * 4), "one pixel per SVG unit");
    assert_eq!(pixel(&img, 60, 40), [255, 255, 255, 255], "the white fill");
    for (x, y) in [(0, 40), (120, 40), (60, 0), (60, 80)] {
        let [r, g, b, a] = pixel(&img, x, y);
        assert!(a == 255 && r < 8 && g < 8 && b < 8, "the black stroke at {x},{y}: {:?}", [r, g, b, a]);
    }
    let img = from_text(HALF_RED).expect("the drawing's pixels");
    assert_eq!((img.0, img.1), (40, 20));
    assert_eq!(pixel(&img, 5, 10), [255, 0, 0, 255]);
    assert_eq!(pixel(&img, 30, 10), [0, 0, 0, 0], "transparent where there is no art");
}

#[test]
fn text_that_is_not_a_drawing_pastes_nothing() {
    for text in ["", "hello", "<p>not a drawing</p>", "<?xml version=\"1.0\"?><x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>"] {
        assert!(from_text(text).is_none(), "{text:?}");
    }
    // Markup that doesn't parse, and markup past the read limit, paste nothing either.
    assert!(from_text("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"8\"><rect").is_none());
    assert!(from_text("<svg width=\"8\" height=\"8\"><g></svg>").is_none());
    let mut huge = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"8\">");
    huge.push_str(&" ".repeat(usize::try_from(MAX_CLIPBOARD_SVG_BYTES).unwrap()));
    huge.push_str("</svg>");
    assert!(from_text(&huge).is_none(), "markup past the limit isn't parsed");
}

#[test]
fn svg_clipboard_text_accepts_a_long_xml_preamble() {
    let padding = "x".repeat(8192);
    for prefix in [format!("<!--{padding}-->"), format!("<?xml version=\"1.0\"?>\n<!--{padding}-->"), format!("<?drawing {padding}?>")] {
        let text = format!("{prefix}{HALF_RED}");
        let img = from_text(&text).expect("SVG after the XML preamble");
        assert_eq!((img.0, img.1), (40, 20));
        assert_eq!(pixel(&img, 5, 10), [255, 0, 0, 255]);
        assert_eq!(pixel(&img, 30, 10), [0, 0, 0, 0]);
    }
    assert!(from_text(&format!("<!--{padding}<svg/>--><not-svg/>")).is_none());
    assert!(from_text(&format!("<!--{padding}--><svg><g></svg>")).is_none());
}

#[test]
fn a_drawing_past_the_pixel_limit_pastes_scaled_down() {
    let wide = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20000" height="10000"><rect width="20000" height="10000" fill="#0000ff"/></svg>"##;
    let img = from_text(wide).expect("scaled down, not dropped");
    let pixels = u64::from(img.0) * u64::from(img.1);
    assert!(pixels <= 24_000_000 && pixels > 12_000_000, "{} × {}", img.0, img.1);
    assert!(img.0.abs_diff(img.1 * 2) <= 2, "the proportions are kept: {} × {}", img.0, img.1);
    assert_eq!(pixel(&img, img.0 / 2, img.1 / 2), [0, 0, 255, 255]);
}

/// Copied files and an image come before SVG text, which is read only when neither is there.
#[test]
fn copied_files_and_images_come_before_svg_text() {
    let dir = temp("svg-order");
    let png = dir.join("red.png");
    std::fs::write(&png, red_png(3, 2)).unwrap();
    let blue = (2, 2, [0u8, 0, 255, 255].repeat(4));
    let mut text_read = false;
    let svg_text = |read: &mut bool| {
        *read = true;
        Ok(Some(COPIED_RECTANGLE.to_string()))
    };
    let got = clipboard_image(&mut text_read, |_| Ok(Some(vec![png.clone()])), |_| Ok(Some(blue.clone())), svg_text);
    assert_eq!(got.unwrap().map(|(w, h, _)| (w, h)), Some((3, 2)), "the copied file");
    let got = clipboard_image(&mut text_read, |_| Ok(None), |_| Ok(Some(blue.clone())), svg_text);
    assert_eq!(got.unwrap().map(|(w, h, _)| (w, h)), Some((2, 2)), "the image, not the SVG text");
    assert!(!text_read, "the text isn't read while there is a file or an image");
    // A copied file that isn't an image (an .svg) falls through to the image, then the text.
    let svg = dir.join("drawing.svg");
    std::fs::write(&svg, COPIED_RECTANGLE).unwrap();
    let got = clipboard_image(&mut (), |_| Ok(Some(vec![svg.clone()])), |_| Ok(None), |_| Ok(Some(COPIED_RECTANGLE.to_string())));
    assert_eq!(got.unwrap().map(|(w, h, _)| (w, h)), Some((121, 81)));
}

/// A failed read, such as one of a busy clipboard, is the paste's error: the reads after it,
/// the SVG text included, don't run.
#[test]
fn a_failed_clipboard_read_is_an_error_and_the_text_is_not_read() {
    fn busy<T>() -> Result<T, String> {
        Err("the clipboard is busy".to_string())
    }
    let mut text_read = false;
    let svg_text = |read: &mut bool| {
        *read = true;
        Ok(Some(COPIED_RECTANGLE.to_string()))
    };
    let got = clipboard_image(&mut text_read, |_| busy(), |_| Ok(None), svg_text);
    assert_eq!(got, Err("the clipboard is busy".to_string()), "the file list's error");
    let got = clipboard_image(&mut text_read, |_| Ok(None), |_| busy(), svg_text);
    assert_eq!(got, Err("the clipboard is busy".to_string()), "the image's error");
    assert!(!text_read, "the text isn't read after a failed read");
    let got = clipboard_image(&mut (), |_| Ok(None), |_| Ok(None), |_| busy());
    assert_eq!(got, Err("the clipboard is busy".to_string()), "the text's error");
}

/// Only a clipboard with nothing in the format asked for reads as nothing to paste. A read
/// that fails in any other way, such as a broken pipe on Wayland, is the paste's error, for
/// the copied files and the text alike. Text that isn't UTF-8 reads as no SVG markup.
#[test]
fn only_missing_content_reads_as_nothing_to_paste() {
    use arboard::Error;
    let failures = || [Error::ClipboardOccupied, Error::ClipboardNotSupported, Error::Unknown { description: "broken pipe".to_string() }];
    assert_eq!(clipboard_read(Ok::<_, Error>(vec![PathBuf::from("a.png")])), Ok(Some(vec![PathBuf::from("a.png")])));
    assert_eq!(clipboard_read::<Vec<PathBuf>>(Err(Error::ContentNotAvailable)), Ok(None));
    for error in failures().into_iter().chain([Error::ConversionFailure]) {
        let message = error.to_string();
        assert_eq!(clipboard_read::<Vec<PathBuf>>(Err(error)), Err(message.clone()), "{message}");
    }
    assert_eq!(clipboard_text(Ok("words".to_string())), Ok(Some("words".to_string())));
    assert_eq!(clipboard_text(Err(Error::ContentNotAvailable)), Ok(None));
    assert_eq!(clipboard_text(Err(Error::ConversionFailure)), Ok(None), "text that isn't UTF-8");
    for error in failures() {
        let message = error.to_string();
        assert_eq!(clipboard_text(Err(error)), Err(message.clone()), "{message}");
    }
}

/// Edit › Paste with SVG markup as the clipboard's only content adds a new layer that holds the
/// drawing's art; other text is still a quiet "nothing to paste".
#[test]
fn paste_turns_svg_text_on_the_clipboard_into_a_layer() {
    let text: Rc<RefCell<String>> = Rc::new(RefCell::new(HALF_RED.to_string()));
    let clip = text.clone();
    let services = Services {
        clipboard_get_image: Some(Box::new(move || clipboard_image(&mut (), |_| Ok(None), |_| Ok(None), |_| Ok(Some(clip.borrow().clone()))))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(Session::new(), services);
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let layers = app.session.active().unwrap().doc.layers.len();
    let r = photocraft_ui_egui::menus::invoke(&mut app, &ctx, "edit.paste", json!({})).unwrap();
    assert_ne!(r["pasted"], json!(false), "{r}");
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers.len(), layers + 1);
    let surface = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    let bounds = surface.content_bounds();
    assert_eq!((bounds.width(), bounds.height()), (20, 20), "the red half; the transparent half adds nothing");
    *text.borrow_mut() = "just words".to_string();
    let r = photocraft_ui_egui::menus::invoke(&mut app, &ctx, "edit.paste", json!({})).unwrap();
    assert_eq!(r["pasted"], json!(false));
    assert_eq!(app.ui.status, "Nothing to paste: the clipboard holds no image");
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1);
}

/// #7582: a panic while reading the OS clipboard (in arboard, or decoding a copied file) is
/// the paste's error in the status bar, not a crash: the document stays and the next paste
/// works. The same for a panic while writing it.
#[test]
fn a_panicking_clipboard_read_is_reported_not_a_crash() {
    let panics = Rc::new(Cell::new(true));
    let p = panics.clone();
    let get = guarded_clipboard_get(move || if p.get() { panic!("clipboard decoder bug") } else { Ok(Some((2, 2, [0u8, 0, 255, 255].repeat(4)))) });
    let mut app = PhotocraftApp::new(Session::new(), Services { clipboard_get_image: Some(get), ..Default::default() });
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    let layers = app.session.active().unwrap().doc.layers.len();

    let error = app.run("edit.paste", json!({})).unwrap_err();
    assert!(error.contains("Paste failed"), "{error}");
    assert!(app.ui.status_error && app.ui.status.contains("Paste failed"), "{}", app.ui.status);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "the document is unchanged");

    panics.set(false);
    app.run("edit.paste", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1, "the next paste works");

    let mut set = guarded_clipboard_set(|_: u32, _: u32, _: &[u8]| panic!("clipboard writer bug"));
    assert!(set(1, 1, &[0; 4]).unwrap_err().contains("Copy failed"));
}

/// #8827: what TIFF Options sets in `ExportSettings` reaches the file the app writes: the
/// compression, Save Transparency and Save Image Pyramid.
#[test]
fn tiff_options_reach_the_written_file() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100, "background": "transparent"})).unwrap();
    let doc = s.active().unwrap().doc.clone();
    let settings = photocraft_ui_egui::ExportSettings {
        tiff_compression: photocraft_codecs::TiffCompression::Lzw,
        tiff_transparency: Some(false),
        tiff_pyramid: true,
        ..Default::default()
    };
    let bytes = photocraft_io::export(&doc, "t.tif", &export_options(&settings)).unwrap().bytes;
    let info = photocraft_codecs::tiff_info(&bytes).unwrap();
    assert_eq!(info.pages.iter().map(|p| (p.width, p.compression)).collect::<Vec<_>>(), [(200, 5), (100, 5), (50, 5)]);
    assert!(!photocraft_codecs::decode(&bytes).unwrap().layout().has_alpha());
}
