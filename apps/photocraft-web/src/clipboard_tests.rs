use super::*;
use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image};
use photocraft_engine::Session;
use photocraft_ui_egui::{PhotocraftApp, Services};
use serde_json::json;

/// A `w`×`h` PNG of one colour, as a browser hands over a copied image.
fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let image = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.repeat((w * h) as usize)).unwrap();
    photocraft_codecs::encode(&image, Format::Png, &EncodeOptions::default()).unwrap()
}

/// One browser paste of `file`: the event, then its image read.
fn paste_image(clipboard: &WebClipboard, file: Vec<u8>) {
    let number = clipboard.paste(true);
    clipboard.read(number, Ok(file));
}

fn size(clipboard: &WebClipboard) -> Option<(u32, u32)> {
    clipboard.image().unwrap().map(|(w, h, _)| (w, h))
}

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

#[test]
fn a_pasted_image_is_what_the_clipboard_holds_once_it_has_been_read() {
    let clipboard = WebClipboard::default();
    assert_eq!(clipboard.image(), Ok(None), "nothing is known before the first paste");
    let number = clipboard.paste(true);
    assert!(clipboard.reading());
    clipboard.read(number, Ok(png(3, 2, RED)));
    assert!(!clipboard.reading());
    assert_eq!(clipboard.image(), Ok(Some((3, 2, RED.repeat(6)))));
    // Text copied since: the clipboard holds no image any more.
    clipboard.paste(false);
    assert_eq!(clipboard.image(), Ok(None));
}

#[test]
fn a_slower_read_does_not_replace_a_newer_paste() {
    let clipboard = WebClipboard::default();
    let older = clipboard.paste(true);
    clipboard.paste(false);
    clipboard.read(older, Ok(png(3, 2, RED)));
    assert_eq!(clipboard.image(), Ok(None));
    assert!(!clipboard.reading());

    let older = clipboard.paste(true);
    let newer = clipboard.paste(true);
    clipboard.read(newer, Ok(png(4, 4, BLUE)));
    assert!(clipboard.reading(), "the older read is still under way");
    clipboard.read(older, Ok(png(3, 2, RED)));
    assert!(!clipboard.reading());
    assert_eq!(size(&clipboard), Some((4, 4)));
}

#[test]
fn an_image_that_cannot_be_read_says_why() {
    let clipboard = WebClipboard::default();
    paste_image(&clipboard, b"not an image".to_vec());
    let error = clipboard.image().unwrap_err();
    assert!(error.starts_with("Couldn't read the pasted image: "), "{error}");
    let number = clipboard.paste(true);
    clipboard.read(number, Err("NotReadableError".into()));
    assert_eq!(clipboard.image(), Err("Couldn't read the pasted image: NotReadableError".into()));
}

#[test]
fn a_copy_made_here_is_newer_than_the_image_the_clipboard_already_held() {
    let clipboard = WebClipboard::default();
    paste_image(&clipboard, png(3, 2, RED));
    clipboard.copied();
    assert_eq!(clipboard.image(), Ok(None), "the app pastes its own copy");
    // Still the same image after a trip to another app that copied nothing.
    clipboard.left();
    paste_image(&clipboard, png(3, 2, RED));
    assert_eq!(clipboard.image(), Ok(None));
    // Another app copied a new image: it is the newer one now, and stays so.
    paste_image(&clipboard, png(4, 4, BLUE));
    assert_eq!(size(&clipboard), Some((4, 4)));
    paste_image(&clipboard, png(3, 2, RED));
    assert_eq!(size(&clipboard), Some((3, 2)), "copied again in the other app");
}

#[test]
fn an_image_first_seen_after_the_copy_is_older_while_the_page_kept_focus() {
    let clipboard = WebClipboard::default();
    clipboard.copied();
    // The screenshot was on the clipboard before the copy, never pasted until now.
    paste_image(&clipboard, png(3, 2, RED));
    assert_eq!(clipboard.image(), Ok(None));
    clipboard.left();
    paste_image(&clipboard, png(3, 2, RED));
    assert_eq!(clipboard.image(), Ok(None), "still the image that was under the copy");
    clipboard.paste(false);
    paste_image(&clipboard, png(3, 2, RED));
    assert_eq!(size(&clipboard), Some((3, 2)), "text came in between: the image was copied again");
}

#[test]
fn keyboard_events_wait_for_the_pasted_image_and_keep_their_order() {
    let key = |pressed| egui::Event::Key { key: egui::Key::V, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::COMMAND };
    let moved = |x| egui::Event::PointerMoved(egui::pos2(x, 0.0));
    let mut held = Held::default();
    let mut events = vec![moved(1.0), key(true), key(false)];
    held.hold_or_release(true, &mut events);
    assert_eq!(events, vec![moved(1.0)], "the pointer goes on");
    let mut events = vec![egui::Event::Text("a".into())];
    held.hold_or_release(true, &mut events);
    assert!(events.is_empty());
    let mut events = vec![moved(2.0)];
    held.hold_or_release(false, &mut events);
    assert_eq!(events, vec![key(true), key(false), egui::Event::Text("a".into()), moved(2.0)]);
    let mut events = vec![moved(3.0)];
    held.hold_or_release(false, &mut events);
    assert_eq!(events, vec![moved(3.0)], "released once");
}

#[test]
fn only_raster_images_are_read() {
    assert!(is_raster("image/png") && is_raster("image/jpeg") && is_raster("image/webp"));
    assert!(!is_raster("image/svg+xml") && !is_raster("text/plain") && !is_raster("text/html"));
}

/// The browser shell's clipboard services ([`crate::web`]) as a test can build them natively.
fn app(clipboard: &WebClipboard) -> PhotocraftApp {
    let (get, copied) = (clipboard.clone(), clipboard.clone());
    let services =
        Services { clipboard_get_image: Some(Box::new(move || get.image())), clipboard_copied: Some(Box::new(move || copied.copied())), ..Default::default() };
    let mut app = PhotocraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app
}

fn pasted_size(app: &PhotocraftApp) -> (u32, u32) {
    let st = app.session.active().unwrap();
    let bounds = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
    (bounds.width(), bounds.height())
}

/// #7842: Ctrl+V with a screenshot on the clipboard said "Paste is not available: the clipboard
/// is empty". It pastes the screenshot now, and a copy made in PhotoCraft afterwards still pastes
/// that copy rather than the screenshot.
#[test]
fn ctrl_v_pastes_an_image_from_another_app_and_copies_made_here_still_win() {
    let clipboard = WebClipboard::default();
    let mut app = app(&clipboard);
    paste_image(&clipboard, png(3, 2, RED));
    app.run("edit.paste", json!({})).unwrap();
    assert_eq!(pasted_size(&app), (3, 2));

    // The pasted layer is empty up here: copy what the document shows.
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 4})).unwrap();
    app.run("edit.copyMerged", json!({})).unwrap();
    // Ctrl+V: the paste event still shows the screenshot.
    paste_image(&clipboard, png(3, 2, RED));
    app.run("edit.paste", json!({})).unwrap();
    assert_eq!(pasted_size(&app), (8, 4));

    // Back from another app with a new image on the clipboard.
    clipboard.left();
    paste_image(&clipboard, png(5, 5, BLUE));
    app.run("edit.paste", json!({})).unwrap();
    assert_eq!(pasted_size(&app), (5, 5));
}

/// A screenshot that was on the clipboard before PhotoCraft copied, never pasted before, does not
/// replace the copy.
#[test]
fn an_older_screenshot_does_not_replace_a_copy_made_here() {
    let clipboard = WebClipboard::default();
    let mut app = app(&clipboard);
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 4})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    paste_image(&clipboard, png(3, 2, RED));
    app.run("edit.paste", json!({})).unwrap();
    assert_eq!(pasted_size(&app), (8, 4));
}

/// Text on the clipboard and nothing copied in PhotoCraft: Paste says so quietly, not as an
/// error.
#[test]
fn ctrl_v_with_text_on_the_clipboard_has_nothing_to_paste() {
    let clipboard = WebClipboard::default();
    let mut app = app(&clipboard);
    clipboard.paste(false);
    assert_eq!(app.run("edit.paste", json!({})), Ok(json!({"pasted": false})));
    assert_eq!(app.ui.status, "Nothing to paste: the clipboard holds no image");
    assert!(!app.ui.status_error);
}
