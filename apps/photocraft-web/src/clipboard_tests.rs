use super::*;
use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image};
use photocraft_engine::Session;
use photocraft_ui_egui::{PhotocraftApp, Services};
use serde_json::json;

/// A `w`×`h` PNG of one colour, as a browser hands over a copied image.
fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    png_with(w, h, rgba, &EncodeOptions::default())
}

fn png_with(w: u32, h: u32, rgba: [u8; 4], options: &EncodeOptions) -> Vec<u8> {
    let image = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.repeat((w * h) as usize)).unwrap();
    photocraft_codecs::encode(&image, Format::Png, options).unwrap()
}

/// Ctrl+V with `file` on the clipboard: the keys, the paste event, then its image read.
fn paste_image(clipboard: &WebClipboard, file: Vec<u8>) {
    clipboard.key("ControlLeft", true);
    clipboard.key("KeyV", true);
    let number = clipboard.paste(true, false);
    clipboard.key("KeyV", false);
    clipboard.key("ControlLeft", false);
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
    let number = clipboard.paste(true, false);
    assert!(clipboard.reading());
    clipboard.read(number, Ok(png(3, 2, RED)));
    assert!(!clipboard.reading());
    assert_eq!(clipboard.image(), Ok(Some((3, 2, RED.repeat(6)))));
    // Text copied since: the clipboard holds no image any more.
    clipboard.paste(false, true);
    assert_eq!(clipboard.image(), Ok(None));
}

#[test]
fn a_slower_read_does_not_replace_a_newer_paste() {
    let clipboard = WebClipboard::default();
    let older = clipboard.paste(true, false);
    clipboard.paste(false, true);
    assert!(!clipboard.newest(older), "its bytes aren't worth copying");
    clipboard.read(older, Ok(png(3, 2, RED)));
    assert_eq!(clipboard.image(), Ok(None));
    assert!(!clipboard.reading());

    let older = clipboard.paste(true, false);
    let newer = clipboard.paste(true, false);
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
    let number = clipboard.paste(true, false);
    clipboard.read(number, Err("NotReadableError".into()));
    assert_eq!(clipboard.image(), Err("Couldn't read the pasted image: NotReadableError".into()));
}

#[test]
fn a_file_too_large_to_decode_is_not_read() {
    assert_eq!(too_large(max_file_bytes() + 1).as_deref(), Some("the file is too large"));
    assert_eq!(too_large(max_file_bytes()), None);
}

/// Edit › Paste from PhotoCraft's menu has no paste event: after a trip to another app the
/// image the last Ctrl+V showed may be gone, so it asks for Ctrl+V rather than paste it again.
#[test]
fn after_leaving_the_page_the_last_pasted_image_is_unknown() {
    let clipboard = WebClipboard::default();
    paste_image(&clipboard, png(3, 2, RED));
    clipboard.left();
    assert_eq!(clipboard.image(), Err("Press Ctrl+V (⌘V on a Mac) to paste from the system clipboard".into()));
    paste_image(&clipboard, png(4, 4, BLUE));
    assert_eq!(size(&clipboard), Some((4, 4)));
    // A copy made here is pasted as before.
    clipboard.copied();
    clipboard.left();
    assert_eq!(clipboard.image(), Ok(None));
}

/// The browser's Edit › Paste sends a paste event and no key press: once its image is read, the
/// shell presses ⌘V for it, once. Ctrl+V has a press of its own, and eframe makes one from text.
#[test]
fn a_paste_from_a_menu_is_pressed_once_its_image_is_read() {
    let clipboard = WebClipboard::default();
    let number = clipboard.paste(true, false);
    assert!(!clipboard.take_press(), "not before the image is read");
    clipboard.read(number, Ok(png(3, 2, RED)));
    assert!(clipboard.take_press());
    assert!(!clipboard.take_press(), "once");

    paste_image(&clipboard, png(3, 2, RED));
    assert!(!clipboard.take_press(), "Ctrl+V");
    let number = clipboard.paste(true, true);
    clipboard.read(number, Ok(png(3, 2, RED)));
    assert!(!clipboard.take_press(), "an image with text");

    // A key held when the page lost focus is let go elsewhere.
    clipboard.key("ShiftLeft", true);
    clipboard.left();
    let number = clipboard.paste(true, false);
    clipboard.read(number, Ok(png(3, 2, RED)));
    assert!(clipboard.take_press());
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
    clipboard.paste(false, true);
    paste_image(&clipboard, png(3, 2, RED));
    assert_eq!(size(&clipboard), Some((3, 2)), "text came in between: the image was copied again");
}

/// Ctrl+V waits for its image, and so does a click after it (on another document's tab, say),
/// so the image goes to the document that was active when Ctrl+V was pressed.
#[test]
fn input_waits_for_the_pasted_image_and_keeps_its_order() {
    let key = |pressed| egui::Event::Key { key: egui::Key::V, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::COMMAND };
    let click =
        |pressed| egui::Event::PointerButton { pos: egui::pos2(1.0, 0.0), button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let mut held = Held::default();
    let mut events = vec![key(true), key(false), click(true)];
    held.hold_or_release(true, false, 10.0, &mut events);
    assert!(events.is_empty());
    let mut events = vec![click(false)];
    held.hold_or_release(true, false, 10.1, &mut events);
    assert!(events.is_empty());
    let mut events = vec![egui::Event::Text("a".into())];
    held.hold_or_release(false, false, 10.2, &mut events);
    assert_eq!(events, vec![key(true), key(false), click(true), click(false), egui::Event::Text("a".into())]);
    let mut events = vec![click(true)];
    held.hold_or_release(false, false, 10.3, &mut events);
    assert_eq!(events, vec![click(true)], "released once");
    // A paste from a menu: its ⌘V goes ahead of what came after it.
    let mut events = vec![click(false)];
    held.hold_or_release(false, true, 10.4, &mut events);
    assert_eq!(events, vec![key(true), key(false), click(false)]);
}

/// A read that never finishes gives the input back after [`HOLD_SECONDS`], and input goes on
/// until that read is done; the next one waits again.
#[test]
fn input_waits_no_longer_than_the_hold_limit() {
    let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let mut held = Held::default();
    let mut events = vec![key(egui::Key::A)];
    held.hold_or_release(true, false, 1.0, &mut events);
    assert!(events.is_empty());
    let mut events = vec![key(egui::Key::B)];
    held.hold_or_release(true, false, 1.0 + HOLD_SECONDS, &mut events);
    assert_eq!(events, vec![key(egui::Key::A), key(egui::Key::B)], "time is up");
    let mut events = vec![key(egui::Key::C)];
    held.hold_or_release(true, false, 9.0, &mut events);
    assert_eq!(events, vec![key(egui::Key::C)], "the same read: input goes on");
    let mut events = vec![];
    held.hold_or_release(false, false, 9.5, &mut events);
    let mut events = vec![key(egui::Key::D)];
    held.hold_or_release(true, false, 10.0, &mut events);
    assert!(events.is_empty(), "a new read waits again");
}

#[test]
fn only_raster_images_are_read() {
    assert!(is_raster("image/png") && is_raster("image/jpeg") && is_raster("image/webp"));
    assert!(!is_raster("image/svg+xml") && !is_raster("text/plain") && !is_raster("text/html"));
}

/// Which of a paste's items is read: a raster image file, PNG first; an SVG alone is an image
/// that can't be pasted here; text and image strings (not files) offer nothing.
#[test]
fn a_paste_offers_its_png_first_and_names_an_unsupported_image() {
    let png = ("file", "image/png");
    let jpeg = ("file", "image/jpeg");
    let svg = ("file", "image/svg+xml");
    let text = ("string", "text/plain");
    assert_eq!(offer([text, jpeg, png]), Offer::Image(2));
    assert_eq!(offer([svg, jpeg]), Offer::Image(1));
    assert_eq!(offer([text, svg]), Offer::Unsupported("image/svg+xml".into()));
    assert_eq!(offer([text, ("string", "text/html")]), Offer::Nothing);
    assert_eq!(offer([("string", "image/png")]), Offer::Nothing);
    assert_eq!(offer([]), Offer::Nothing);
}

#[test]
fn an_unsupported_image_says_so_and_a_menu_paste_of_it_is_pressed() {
    let clipboard = WebClipboard::default();
    paste_image(&clipboard, png(3, 2, RED));
    clipboard.unsupported("image/svg+xml", false);
    assert_eq!(clipboard.image(), Err("Couldn't read the pasted image: image/svg+xml isn't supported".into()));
    assert!(clipboard.take_press(), "no key held, no text: the paste is pressed so its message shows");
}

/// The browser may encode the same clipboard image differently each time: the pixels decide.
#[test]
fn the_same_pixels_in_another_encoding_are_the_same_image() {
    let clipboard = WebClipboard::default();
    paste_image(&clipboard, png(3, 2, RED));
    clipboard.copied();
    clipboard.left();
    let interlaced = png_with(3, 2, RED, &EncodeOptions { png_interlaced: true, ..Default::default() });
    assert_ne!(interlaced, png(3, 2, RED));
    paste_image(&clipboard, interlaced);
    assert_eq!(clipboard.image(), Ok(None), "still the image under the copy: the copy wins");
}

/// A second copy after a trip away is newer again: an image the clipboard held before it
/// doesn't replace it.
#[test]
fn a_copy_after_coming_back_is_newer_again() {
    let clipboard = WebClipboard::default();
    paste_image(&clipboard, png(3, 2, RED));
    clipboard.copied();
    clipboard.left();
    paste_image(&clipboard, png(3, 2, RED));
    clipboard.copied();
    paste_image(&clipboard, png(3, 2, RED));
    assert_eq!(clipboard.image(), Ok(None));
}

/// The app with the browser shell's clipboard services ([`wire`], as `crate::web` wires them).
fn app(clipboard: &WebClipboard) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(Session::new(), wire(clipboard, Services::default()));
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

/// An image that can't be read: Paste says why, as an error.
#[test]
fn ctrl_v_with_an_unreadable_image_says_why() {
    let clipboard = WebClipboard::default();
    let mut app = app(&clipboard);
    paste_image(&clipboard, b"not an image".to_vec());
    assert!(app.run("edit.paste", json!({})).is_err());
    assert!(app.ui.status.starts_with("Couldn't read the pasted image: "), "{}", app.ui.status);
    assert!(app.ui.status_error);
}

/// Text on the clipboard and nothing copied in PhotoCraft: Paste says so quietly, not as an
/// error.
#[test]
fn ctrl_v_with_text_on_the_clipboard_has_nothing_to_paste() {
    let clipboard = WebClipboard::default();
    let mut app = app(&clipboard);
    clipboard.paste(false, true);
    assert_eq!(app.run("edit.paste", json!({})), Ok(json!({"pasted": false})));
    assert_eq!(app.ui.status, "Nothing to paste: the clipboard holds no image");
    assert!(!app.ui.status_error);
}
