//! Generative Fill after review (#10903): what a feathered edge sends, colour matching made
//! visible, the framing check and the diagnostic capture.

use super::*;

/// The decoded source PNG of the first request, as RGBA, with the area's top-left corner.
fn sent_source(f: &Fake, r: &Value) -> (Image, i32, i32) {
    let sent = f.sent.lock().unwrap();
    let img = codecs::decode(&sent[0].source_png).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
    (img, r["bounds"][0].as_i64().unwrap() as i32, r["bounds"][1].as_i64().unwrap() as i32)
}

#[test]
fn a_feathered_edge_sends_none_of_the_replaced_colour() {
    // A red square on white, selected with a wide feather: even where the selection only just
    // touches a pixel, the picture sent carries no red, and the mask keeps the feather.
    let mut s = session(160, 160);
    s.execute("select.rect", json!({"x": 50, "y": 50, "width": 60, "height": 60})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.rect", json!({"x": 50, "y": 50, "width": 60, "height": 60, "feather": 8})).unwrap();
    let f = install(&mut s, fake([0, 0, 255]));
    let r = s.execute(FILL, json!({"variations": 1})).unwrap();
    let (img, x0, y0) = sent_source(&f, &r);
    let mask = codecs::decode(&f.sent.lock().unwrap()[0].mask_png).unwrap().convert(ChannelLayout::Rgba, CodecSample::U8);
    let mut partial = 0;
    for (i, (px, m)) in img.data().as_chunks::<4>().0.iter().zip(mask.data().as_chunks::<4>().0).enumerate() {
        let (x, y) = (x0 + (i % img.width() as usize) as i32, y0 + (i / img.width() as usize) as i32);
        let [r, g, b, _] = *px;
        assert!(i16::from(r) - i16::from(g) <= 2 && i16::from(r) - i16::from(b) <= 2, "({x},{y}) {px:?}: red was sent");
        if (1..255).contains(&m[3]) {
            partial += 1;
        }
    }
    assert!(partial > 0, "the mask keeps the feather");
}

#[test]
fn whether_colour_matching_ran_is_kept_for_each_variation() {
    // A solid colour where white was: too different around the selection to match.
    let (s, _, r) = filled(Some(1));
    assert_eq!(r["matching"][0]["colour"]["state"], "tooDifferent", "{r}");
    assert_eq!(r["matching"][0]["framing"]["state"], "same");
    let id = photocraft_doc::LayerId(r["variations"][0].as_u64().unwrap());
    let note = s.generative_variation_note(id).unwrap();
    assert!(note.colour.skipped());
    assert!(matches!(note.colour, ColourMatch::TooDifferent { limit, .. } if limit == 0.25));
    // A relit copy of the picture is matched.
    let mut s = session(160, 160);
    s.execute("select.rect", json!({"x": 50, "y": 50, "width": 60, "height": 60})).unwrap();
    install(&mut s, Fake { echo: Some(-40), ..fake([0, 0, 0]) });
    let r = s.execute(FILL, json!({"variations": 1})).unwrap();
    assert_eq!(r["matching"][0]["colour"]["state"], "matched", "{r}");
    let id = photocraft_doc::LayerId(r["variations"][0].as_u64().unwrap());
    assert!(!s.generative_variation_note(id).unwrap().colour.skipped());
}

/// A 160² canvas, white with a black band down the left and a red one down the right, the
/// middle selected: the area sent is the whole canvas, square.
fn banded() -> Session {
    let mut s = session(160, 160);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 40, "height": 160})).unwrap();
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    s.execute("select.rect", json!({"x": 124, "y": 0, "width": 36, "height": 160})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.rect", json!({"x": 50, "y": 50, "width": 60, "height": 60})).unwrap();
    s
}

#[test]
fn a_result_with_added_sides_is_cut_to_its_centre_not_squeezed() {
    let mut s = banded();
    install(&mut s, Fake { echo: Some(0), reframe: Some(Reframe::Pad(40)), ..fake([0, 0, 0]) });
    let r = s.execute(FILL, json!({"variations": 1})).unwrap();
    assert_eq!(r["matching"][0]["framing"], json!({"state": "centre", "returned": [240, 160], "sent": [160, 160]}), "{r}");
    // Lined up as sent, the picture around the selection matches, so its colours were matched.
    assert_eq!(r["matching"][0]["colour"]["state"], "matched", "{r}");
}

#[test]
fn a_stretched_result_is_scaled_back_when_the_whole_of_it_lines_up() {
    let mut s = banded();
    install(&mut s, Fake { echo: Some(0), reframe: Some(Reframe::Stretch(240, 160)), ..fake([0, 0, 0]) });
    let r = s.execute(FILL, json!({"variations": 1})).unwrap();
    assert_eq!(r["matching"][0]["framing"]["state"], "scaled", "{r}");
}

#[test]
fn a_result_framed_differently_is_refused_not_stretched() {
    let mut s = banded();
    let layers = doc(&s).layers.len();
    install(&mut s, Fake { size: Some((1536, 1024)), ..fake([0, 0, 255]) });
    let e = s.execute(FILL, json!({"variations": 2})).unwrap_err().to_string();
    assert!(e.contains("1536×1024") && e.contains("160×160") && e.contains("not placed"), "{e}");
    assert_eq!(doc(&s).layers.len(), layers, "nothing landed");
    // One of the same shape is placed and the other dropped, like a failed variation.
    let mut s = banded();
    install(&mut s, Fake { size: Some((1024, 1024)), ..fake([0, 0, 255]) });
    let r = s.execute(FILL, json!({"variations": 1})).unwrap();
    assert_eq!(r["matching"][0]["framing"]["state"], "same");
}

#[test]
fn save_fill_diagnostics_writes_what_was_sent_and_returned_only_when_asked() {
    let mut s = session(160, 160);
    s.execute("select.rect", json!({"x": 50, "y": 50, "width": 60, "height": 60})).unwrap();
    let f = install(&mut s, Fake { echo: Some(-40), ..fake([0, 0, 0]) });
    let dir = tempfile::tempdir().unwrap();
    assert!(s.execute(SAVE_DIAGNOSTICS, json!({"folder": dir.path().to_string_lossy()})).is_err(), "no generative layer yet");
    let r = s.execute(FILL, json!({"prompt": "a lake", "variations": 2})).unwrap();
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "nothing is written on its own");
    assert!(s.is_enabled(SAVE_DIAGNOSTICS));
    let group = photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
    assert!(s.generative_has_diagnostics(group));
    assert!(s.execute(SAVE_DIAGNOSTICS, json!({})).is_err(), "needs a folder");
    assert!(s.execute(SAVE_DIAGNOSTICS, json!({"folder": dir.path().join("missing").to_string_lossy()})).is_err());
    let saved = s.execute(SAVE_DIAGNOSTICS, json!({"folder": dir.path().to_string_lossy()})).unwrap();
    let out = std::path::PathBuf::from(saved["folder"].as_str().unwrap());
    assert_eq!(out.parent(), Some(dir.path()));
    let mut names: Vec<String> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(
        names,
        ["composite-1.png", "composite-2.png", "crop.png", "mask.png", "prompt.txt", "returned-1.png", "returned-2.png", "source.png", "summary.json"]
    );
    // Exactly what was sent.
    let sent = f.sent.lock().unwrap()[0].clone();
    assert_eq!(std::fs::read(out.join("source.png")).unwrap(), sent.source_png);
    assert_eq!(std::fs::read(out.join("mask.png")).unwrap(), sent.mask_png);
    assert_eq!(std::fs::read_to_string(out.join("prompt.txt")).unwrap(), "a lake");
    let summary: Value = serde_json::from_slice(&std::fs::read(out.join("summary.json")).unwrap()).unwrap();
    let fill = &summary["fills"][0];
    assert_eq!(fill["model"], "Fake 1");
    assert_eq!(fill["connection"], "Fake");
    assert_eq!(fill["requestSize"], json!([160, 160]));
    assert_eq!(fill["variations"][0]["colourMatching"], "ran");
    assert_eq!(fill["variations"][0]["placed"]["framing"]["state"], "same");
    assert_eq!(fill["variations"][0]["returnedSize"], json!([160, 160]));
    assert!(fill["instructionNote"].as_str().unwrap().contains("doesn't see"));
    // A second save never replaces the first.
    let again = s.execute(SAVE_DIAGNOSTICS, json!({"folder": dir.path().to_string_lossy(), "layer": group.0})).unwrap();
    assert_ne!(again["folder"], saved["folder"]);
    // Undone, the layer is gone and so is the command.
    assert!(s.undo());
    assert!(!s.generative_has_diagnostics(group));
    assert!(s.execute(SAVE_DIAGNOSTICS, json!({"folder": dir.path().to_string_lossy(), "layer": group.0})).is_err());
}

#[test]
fn a_failed_fill_keeps_its_capture_until_retried_or_discarded() {
    let mut s = banded();
    install(&mut s, Fake { size: Some((1536, 1024)), ..fake([0, 0, 255]) });
    let job = started(&mut s, FILL, json!({"prompt": "x", "variations": 1}));
    assert!(s.wait_job(job).is_err());
    let fills = s.generative_fills();
    assert_eq!((fills[0].state, fills[0].diagnostics), ("failed", true));
    let dir = tempfile::tempdir().unwrap();
    let saved = s.execute(SAVE_DIAGNOSTICS, json!({"folder": dir.path().to_string_lossy(), "fill": fills[0].id})).unwrap();
    let out = std::path::PathBuf::from(saved["folder"].as_str().unwrap());
    let summary: Value = serde_json::from_slice(&std::fs::read(out.join("summary.json")).unwrap()).unwrap();
    let v = &summary["fills"][0]["variations"][0];
    assert!(v["refused"].as_str().unwrap().contains("1536×1024"), "{v}");
    assert_eq!(v["composite"], Value::Null);
    assert!(out.join("returned-1.png").is_file(), "the picture as returned, to look at");
    assert!(summary["fills"][0]["failure"].is_string());
    s.execute(DISCARD, json!({"fill": fills[0].id})).unwrap();
    assert!(s.execute(SAVE_DIAGNOSTICS, json!({"folder": dir.path().to_string_lossy(), "fill": fills[0].id})).is_err());
}

#[test]
fn only_the_last_fills_keep_their_capture() {
    let mut s = session(64, 64);
    install(&mut s, fake([0, 0, 255]));
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let groups: Vec<photocraft_doc::LayerId> =
        (0..=diag::KEEP).map(|_| photocraft_doc::LayerId(s.execute(FILL, json!({"variations": 1})).unwrap()["layer"].as_u64().unwrap())).collect();
    assert!(!s.generative_has_diagnostics(groups[0]), "the oldest was let go");
    assert!(groups[1..].iter().all(|g| s.generative_has_diagnostics(*g)));
}

#[test]
fn generate_more_takes_the_layer_asked_for_whatever_is_active() {
    let (mut s, f, r) = filled(Some(1));
    let id = r["layer"].as_u64().unwrap();
    // An ordinary layer is active; the generative layer is named.
    let background = doc(&s).layers[0].id;
    s.execute("layer.select", json!({"layer": background.0})).unwrap();
    assert_eq!(s.generative_layer(), None);
    let more = s.execute(MORE, json!({"layer": id, "count": 1})).unwrap();
    assert_eq!(more["layer"], id);
    assert_eq!(group(&s, id).children().unwrap().len(), 2);
    assert_eq!(f.sent.lock().unwrap().len(), 2);
    // Without `layer`, the active one decides, and it isn't one.
    assert!(s.execute(MORE, json!({"count": 1})).is_err());
}

#[test]
fn what_a_generative_layer_was_made_from_is_let_go_once_nothing_can_bring_it_back() {
    let mut s = session(64, 64);
    install(&mut s, fake([0, 0, 255]));
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let layer = |r: &Value| photocraft_doc::LayerId(r["layer"].as_u64().unwrap());
    let variation = |r: &Value| photocraft_doc::LayerId(r["variations"][0].as_u64().unwrap());
    let first = s.execute(FILL, json!({"prompt": "first", "variations": 1})).unwrap();
    // Undone, redo can bring it back.
    assert!(s.undo());
    assert_eq!(s.generative_prompt(layer(&first)), Some("first"));
    // A new step drops the redo state: nothing holds the first layer now.
    let second = s.execute(FILL, json!({"prompt": "second", "variations": 1})).unwrap();
    assert_eq!(s.generative_prompt(layer(&first)), None);
    assert!(s.generative_variation_note(variation(&first)).is_none());
    // Deleted, undo can bring it back: kept.
    s.execute("layer.delete", json!({"layer": layer(&second).0})).unwrap();
    s.execute(FILL, json!({"prompt": "third", "variations": 1})).unwrap();
    assert_eq!(s.generative_prompt(layer(&second)), Some("second"));
    assert!(s.generative_variation_note(variation(&second)).is_some());
    // Closed, nothing is kept.
    s.close(0);
    assert_eq!(s.generative_prompt(layer(&second)), None);
    assert!(s.generative_variation_note(variation(&second)).is_none());
}

#[test]
fn an_inline_fill_that_failed_stays_for_retry_discard_and_its_diagnostics() {
    let mut s = banded();
    install(&mut s, Fake { size: Some((1536, 1024)), ..fake([0, 0, 255]) });
    assert!(s.execute(FILL, json!({"prompt": "x", "variations": 1})).is_err());
    let fills = s.generative_fills();
    assert_eq!(fills.len(), 1, "kept as in the background");
    assert_eq!((fills[0].state, fills[0].diagnostics), ("failed", true));
    let dir = tempfile::tempdir().unwrap();
    s.execute(SAVE_DIAGNOSTICS, json!({"folder": dir.path().to_string_lossy(), "fill": fills[0].id})).unwrap();
    // Retry, now answered with the area's shape, lands it.
    install(&mut s, fake([0, 0, 255]));
    s.execute(RETRY, json!({"fill": fills[0].id})).unwrap();
    assert!(s.generative_fills().is_empty());
}

#[test]
fn a_failed_fills_diagnostics_can_be_saved_whatever_generative_layer_is_active() {
    let mut s = session(64, 64);
    install(&mut s, fake([0, 0, 255]));
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let groups: Vec<u64> = (0..=diag::KEEP).map(|_| s.execute(FILL, json!({"variations": 1})).unwrap()["layer"].as_u64().unwrap()).collect();
    // The oldest generative layer, whose capture was let go, is active.
    s.execute("layer.select", json!({"layer": groups[0]})).unwrap();
    assert!(!s.is_enabled(SAVE_DIAGNOSTICS));
    install(&mut s, Fake { size: Some((1536, 1024)), ..fake([0, 0, 255]) });
    assert!(s.execute(FILL, json!({"variations": 1})).is_err());
    let id = s.generative_fills()[0].id;
    assert!(s.is_enabled(SAVE_DIAGNOSTICS), "the failed fill's row offers it");
    let dir = tempfile::tempdir().unwrap();
    s.execute(SAVE_DIAGNOSTICS, json!({"folder": dir.path().to_string_lossy(), "fill": id})).unwrap();
}

#[test]
fn only_so_many_fills_are_kept_waiting() {
    let mut s = session(64, 64);
    install(&mut s, Fake { fail: Some(AiFailure::new(AiErrorKind::Network, "offline")), ..fake([0, 0, 0]) });
    s.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    for _ in 0..MAX_WAITING {
        assert!(s.execute(FILL, json!({"variations": 1})).unwrap_err().to_string().contains("offline"));
    }
    assert_eq!(s.generative_fills().len(), MAX_WAITING);
    assert!(!s.is_enabled(FILL));
    let e = s.execute(FILL, json!({"variations": 1})).unwrap_err().to_string();
    assert!(e.contains("discard one first"), "{e}");
    assert_eq!(s.generative_fills().len(), MAX_WAITING, "the refused one isn't kept");
    let id = s.generative_fills()[0].id;
    s.execute(DISCARD, json!({"fill": id})).unwrap();
    assert!(s.is_enabled(FILL));
}
