//! File › Open on an animated GIF or WebP: with File Handling's preference at Ask (the default)
//! a prompt asks whether it opens as a frame animation, a video layer or its first frame, and
//! can remember the answer for that format.

use crate::{PhotocraftApp, Services};
use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_codecs::web::{GifFrame, WebpFrame, encode_gif_animated, encode_webp_animated};
use photocraft_engine::Session;
use photocraft_engine::prefs::AnimatedOpen;
use serde_json::Value;

/// An animated GIF of `n` solid 3×2 frames, 50 ms each.
fn gif(n: usize) -> Vec<u8> {
    let frames: Vec<GifFrame> =
        (0..n).map(|i| GifFrame { indices: vec![0; 6], palette: vec![[(i * 40) as u8, 0, 0]], transparent: None, delay_cs: 5 }).collect();
    encode_gif_animated(3, 2, &frames, true).unwrap()
}

fn webp() -> Vec<u8> {
    let frames: Vec<WebpFrame> = (0..2).map(|i| WebpFrame { rgba: [i * 100, 0, 0, 255].repeat(6), delay_ms: 100 }).collect();
    encode_webp_animated(3, 2, &frames, 0).unwrap()
}

fn harness(background: bool) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(
            Session::new(),
            Services {
                import: Some(Box::new(|name, bytes, opts| {
                    photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
                })),
                ..Default::default()
            },
        )
    });
    h.state_mut().background_jobs = background;
    h.run_steps(3);
    h
}

/// Run frames until no open is still reading.
fn settle(h: &mut Harness<'static, PhotocraftApp>) {
    for _ in 0..200 {
        h.run_steps(2);
        if h.state().jobs.opens.is_empty() && h.state().session.jobs().is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.run_steps(6);
}

fn open(h: &mut Harness<'static, PhotocraftApp>, name: &str, bytes: &[u8]) {
    h.state_mut().open_file(&format!("/pics/{name}"), bytes).unwrap();
    settle(h);
}

/// The one prompt up: its message.
fn prompt(h: &Harness<'static, PhotocraftApp>) -> Option<String> {
    let d = &h.state().ui.dialogs;
    assert!(d.len() <= 1, "one prompt at a time");
    d.first().map(|d| d.fields.get("message").and_then(Value::as_str).unwrap_or_default().to_string())
}

/// A real click (pointer events, hit-tested like a mouse).
fn click(h: &mut Harness<'static, PhotocraftApp>, label: &str) {
    let at: Pos2 = h.query_all_by_label(label).find(|n| !n.accesskit_node().is_disabled()).map(|n| n.rect().center()).unwrap_or_else(|| panic!("no `{label}`"));
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    settle(h);
}

/// What the active document opened as: (layers, frame-animation frames, video frames).
fn shape(h: &Harness<'static, PhotocraftApp>) -> (usize, Option<usize>, Option<usize>) {
    let doc = &h.state().session.active().unwrap().doc;
    (doc.layers.len(), doc.frame_animation.as_ref().map(|a| a.frames.len()), doc.layers.first().and_then(|l| l.video.as_ref()).map(|v| v.frames.len()))
}

#[test]
fn an_animated_gif_asks_how_to_open_and_each_answer_shapes_it() {
    for background in [false, true] {
        for (choice, want) in [("Frame animation", (3, Some(3), None)), ("Video layer", (1, None, Some(3))), ("First frame only", (1, None, None))] {
            let mut h = harness(background);
            open(&mut h, "a.gif", &gif(3));
            assert_eq!(prompt(&h).as_deref(), Some("“a.gif” is an animated GIF with 3 frames. Open it as:"));
            assert!(h.state().session.documents().is_empty(), "nothing opens before the answer");
            click(&mut h, choice);
            click(&mut h, "OK");
            assert_eq!(prompt(&h), None);
            assert_eq!(shape(&h), want, "{choice}, background {background}");
            assert_eq!(h.state().session.active().unwrap().path.as_deref(), Some("/pics/a.gif"));
        }
    }
}

#[test]
fn remembering_answers_later_files_of_that_format_only() {
    let mut h = harness(true);
    open(&mut h, "a.gif", &gif(2));
    click(&mut h, "Video layer");
    click(&mut h, "Remember this choice for GIF files");
    click(&mut h, "OK");
    assert_eq!(h.state().session.prefs().file_handling.animated_gif, AnimatedOpen::Video);
    open(&mut h, "b.gif", &gif(4));
    assert_eq!(prompt(&h), None, "a remembered choice doesn't ask");
    assert_eq!(shape(&h), (1, None, Some(4)));
    open(&mut h, "c.webp", &webp());
    assert_eq!(prompt(&h).as_deref(), Some("“c.webp” is an animated WebP with 2 frames. Open it as:"));
}

#[test]
fn a_remembered_answer_covers_the_files_already_waiting() {
    let mut h = harness(false);
    for name in ["a.gif", "b.gif"] {
        h.state_mut().open_file(&format!("/pics/{name}"), &gif(2)).unwrap();
    }
    settle(&mut h);
    click(&mut h, "First frame only");
    click(&mut h, "Remember this choice for GIF files");
    click(&mut h, "OK");
    assert_eq!(prompt(&h), None, "b.gif was waiting with the same question");
    let docs = h.state().session.documents();
    assert_eq!(docs.iter().map(|d| (d.doc.name.as_str(), d.doc.layers.len())).collect::<Vec<_>>(), [("a.gif", 1), ("b.gif", 1)]);
}

#[test]
fn cancel_opens_nothing_and_still_images_never_ask() {
    let mut h = harness(true);
    open(&mut h, "a.gif", &gif(2));
    click(&mut h, "Cancel");
    assert!(h.state().session.documents().is_empty());
    open(&mut h, "still.gif", &gif(1));
    assert_eq!(prompt(&h), None);
    assert_eq!(shape(&h), (1, None, None));
}
