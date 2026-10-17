use std::sync::Arc;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s
}

fn xmp(s: &Session) -> Option<&str> {
    s.active().unwrap().doc.metadata.xmp.as_deref()
}

/// Put `packet` in the active document's XMP, as a file could have it.
fn set_xmp(s: &mut Session, packet: &str) {
    let packet = packet.to_string();
    s.edit("setup", |doc, _| {
        doc.metadata.xmp = Some(packet);
        Ok(())
    })
    .unwrap();
}

#[test]
fn user_data_sets_reads_lists_and_removes() {
    let mut s = session();
    assert_eq!(s.execute(USER_DATA, json!({})).unwrap(), json!({"data": {}}));
    assert_eq!(s.execute(USER_DATA, json!({"key": "pipeline"})).unwrap(), json!({"key": "pipeline", "value": null}));
    assert!(xmp(&s).is_none(), "reading never creates a packet");

    let tags = json!({"instances": [{"layer": 7, "product": "imageMain <&\"'> ünï 日本"}], "n": 1.5, "on": true});
    assert_eq!(s.execute(USER_DATA, json!({"key": "pipeline", "value": tags})).unwrap()["value"], tags);
    s.execute(USER_DATA, json!({"key": "other.tool-2", "value": "plain"})).unwrap();
    assert_eq!(s.execute(USER_DATA, json!({"key": "pipeline"})).unwrap()["value"], tags);
    assert_eq!(s.execute(USER_DATA, json!({})).unwrap()["data"], json!({"pipeline": tags, "other.tool-2": "plain"}));
    // One property, however often it is rewritten.
    for n in 0..5 {
        s.execute(USER_DATA, json!({"key": "pipeline", "value": {"n": n}})).unwrap();
    }
    let packet = xmp(&s).unwrap();
    assert_eq!(packet.matches("<photocraft:UserData>").count(), 1, "{packet}");
    assert_eq!(packet.matches("xmlns:photocraft=").count(), 1, "{packet}");
    assert!(packet.contains("xmlns:photocraft=\"https://github.com/storytold/photocraft/ns/1.0/\""), "the approved namespace: {packet}");

    assert_eq!(s.execute(USER_DATA, json!({"key": "pipeline", "remove": true})).unwrap(), json!({"key": "pipeline", "value": null, "removed": true}));
    assert_eq!(s.execute(USER_DATA, json!({"key": "pipeline", "remove": true})).unwrap()["removed"], false);
    assert_eq!(s.execute(USER_DATA, json!({})).unwrap()["data"], json!({"other.tool-2": "plain"}));
    // `remove` without a key clears everything.
    assert_eq!(s.execute(USER_DATA, json!({"remove": true})).unwrap(), json!({"data": {}, "removed": true}));
    assert!(!xmp(&s).unwrap().contains("UserData"), "the last key takes the property with it");
    assert_eq!(s.execute(USER_DATA, json!({"remove": true})).unwrap()["removed"], false);
}

#[test]
fn user_data_is_an_undo_step_and_unchanged_values_are_not() {
    let mut s = session();
    let st = s.active_mut().unwrap();
    st.saved_revision = st.revision;
    let steps = s.active().unwrap().history.entries().len();
    s.execute(USER_DATA, json!({"key": "a", "value": [1, 2]})).unwrap();
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
    assert!(s.active().unwrap().is_dirty(), "the document has something to save");
    // The same value again, a read and a missing key's removal change nothing.
    let revision = s.active().unwrap().revision;
    s.execute(USER_DATA, json!({"key": "a", "value": [1, 2]})).unwrap();
    s.execute(USER_DATA, json!({"key": "a"})).unwrap();
    s.execute(USER_DATA, json!({"key": "missing", "remove": true})).unwrap();
    assert_eq!((s.active().unwrap().history.entries().len(), s.active().unwrap().revision), (steps + 1, revision));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.execute(USER_DATA, json!({})).unwrap()["data"], json!({}));
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.execute(USER_DATA, json!({"key": "a"})).unwrap()["value"], json!([1, 2]));
}

#[test]
fn user_data_keeps_the_rest_of_the_packet_and_file_info() {
    let mut s = session();
    s.execute("file.fileInfo", json!({"title": "Shot 010", "keywords": ["a", "b"]})).unwrap();
    s.execute(USER_DATA, json!({"key": "k", "value": {"v": 1}})).unwrap();
    s.execute("file.fileInfo", json!({"description": "after"})).unwrap();
    let info = s.execute("file.fileInfo", json!({})).unwrap();
    assert_eq!((info["title"].as_str(), info["description"].as_str()), (Some("Shot 010"), Some("after")));
    assert_eq!(info["keywords"], json!(["a", "b"]));
    assert_eq!(s.execute(USER_DATA, json!({"key": "k"})).unwrap()["value"], json!({"v": 1}));
}

/// Data that is there but can't be read is an error for reads and changes alike, so a set never
/// silently replaces it; `remove` without a key is the way to clear it.
#[test]
fn unreadable_user_data_is_never_overwritten() {
    let mut s = session();
    let damaged = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
        <rdf:Description rdf:about=\"\" xmlns:photocraft=\"https://github.com/storytold/photocraft/ns/1.0/\">\
        <photocraft:UserData>{\"other tool\": unquoted}</photocraft:UserData></rdf:Description></rdf:RDF></x:xmpmeta>";
    set_xmp(&mut s, damaged);
    for p in [json!({}), json!({"key": "k"}), json!({"key": "k", "value": 1}), json!({"key": "k", "remove": true})] {
        let e = s.execute(USER_DATA, p.clone()).unwrap_err().to_string();
        assert!(e.contains("can't be read") && e.contains("remove"), "{p}: {e}");
    }
    assert_eq!(xmp(&s), Some(damaged), "untouched");
    assert_eq!(s.execute(USER_DATA, json!({"remove": true})).unwrap()["removed"], true);
    s.execute(USER_DATA, json!({"key": "k", "value": 1})).unwrap();
    assert_eq!(s.execute(USER_DATA, json!({})).unwrap()["data"], json!({"k": 1}));
    // A packet too damaged to hold the property is refused, not replaced.
    set_xmp(&mut s, "<x:xmpmeta>garbage");
    let e = s.execute(USER_DATA, json!({"key": "k", "value": 2})).unwrap_err().to_string();
    assert!(e.contains("damaged"), "{e}");
    assert_eq!(xmp(&s), Some("<x:xmpmeta>garbage"));
}

/// Characters XML can't carry are refused: one would make strict readers drop the whole packet.
#[test]
fn user_data_refuses_characters_xml_cant_carry() {
    let mut s = session();
    for bad in ["\u{FFFE}", "\u{FFFF}", "\u{FDD0}", "\u{10FFFF}"] {
        let e = s.execute(USER_DATA, json!({"key": "k", "value": {"text": format!("a{bad}")}})).unwrap_err();
        assert!(matches!(&e, EngineError::BadParams { .. }) && e.to_string().contains("XML can't carry"), "{bad:?}: {e}");
    }
    assert!(xmp(&s).is_none(), "nothing was stored");
}

/// The limits count the data as stored, escaped: `<` takes four bytes in the packet.
#[test]
fn user_data_limits_count_the_escaped_size() {
    let mut s = session();
    let angles = "<".repeat(MAX_VALUE / 4 + 16);
    assert!(json!(angles).to_string().len() < MAX_VALUE, "fits unescaped");
    let e = s.execute(USER_DATA, json!({"key": "k", "value": angles})).unwrap_err().to_string();
    assert!(e.contains("as stored"), "{e}");
    // The total too: values that each fit can't grow the packet past the limit.
    let chunk = "&".repeat(MAX_VALUE / 5 - 16);
    let mut stored = 0;
    for n in 0..8 {
        match s.execute(USER_DATA, json!({"key": format!("k{n}"), "value": chunk})) {
            Ok(_) => stored += 1,
            Err(e) => {
                assert!(e.to_string().contains("would be larger"), "{e}");
                break;
            }
        }
    }
    assert!((1..8).contains(&stored), "stored {stored}");
    assert!(photocraft_io::user_data::stored_len(&s.execute(USER_DATA, json!({})).unwrap()["data"]) <= MAX_TOTAL);
}

#[test]
fn user_data_bad_params_fail_gracefully() {
    let mut s = session();
    let long_key = "k".repeat(MAX_KEY + 1);
    let huge = "x".repeat(MAX_VALUE);
    for p in [
        json!({"key": ""}),
        json!({"key": long_key}),
        json!({"key": "has space"}),
        json!({"key": "sl/ash"}),
        json!({"key": "é"}),
        json!({"key": 7}),
        json!({"key": ["a"]}),
        json!({"value": 1}),
        json!({"remove": "yes"}),
        json!({"key": "k", "remove": "yes"}),
        json!({"key": "k", "value": 1, "remove": true}),
        json!({"key": "k", "value": huge}),
    ] {
        assert!(s.execute(USER_DATA, p.clone()).is_err(), "{}", &p.to_string()[..p.to_string().len().min(80)]);
    }
    assert!(xmp(&s).is_none(), "nothing was stored");
    assert!(Session::new().execute(USER_DATA, json!({})).is_err(), "no document: disabled");
}

/// While a background job runs on the document, reads answer and changes wait: a change made
/// then would be lost when the job applies.
#[test]
fn user_data_changes_are_refused_while_a_job_runs_on_the_document() {
    let mut s = session();
    s.execute(USER_DATA, json!({"key": "k", "value": 1})).unwrap();
    let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let open = gate.clone();
    let started = s
        .start_job(
            "test.gated",
            json!({}),
            "Gated Job",
            true,
            move |ctx| {
                while !open.load(std::sync::atomic::Ordering::Relaxed) {
                    ctx.check()?;
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Ok(())
            },
            |s, ()| {
                s.edit("Gated Job", |_, _| Ok(()))?;
                Ok(json!({}))
            },
        )
        .unwrap();
    let crate::jobs::Started::Job(job) = started else { panic!("expected a background job") };
    for params in [json!({"key": "k", "value": 2}), json!({"key": "k", "remove": true}), json!({"remove": true})] {
        let e = s.execute(USER_DATA, params.clone()).unwrap_err();
        assert!(matches!(&e, EngineError::Disabled(_, why) if why.contains("Gated Job")), "{params}: {e}");
    }
    assert_eq!(s.execute(USER_DATA, json!({"key": "k"})).unwrap()["value"], 1);
    let layer = s.active().unwrap().active_layer.unwrap().0;
    assert!(s.execute(PERSISTENT_ID, json!({"layer": layer})).is_ok(), "a read");
    gate.store(true, std::sync::atomic::Ordering::Relaxed);
    s.wait_job(job).unwrap();
    s.execute(USER_DATA, json!({"key": "k", "value": 2})).unwrap();
}

// ---- persistent layer ids ----

/// A new layer (which becomes the active one), as its session id.
fn new_layer(s: &mut Session, name: &str) -> u64 {
    s.execute("layer.new.layer", json!({"name": name})).unwrap();
    s.active().unwrap().active_layer.unwrap().0
}

/// A layer's persistent id.
fn pid(s: &Session, id: u64) -> u64 {
    u64::from(s.active().unwrap().doc.layer(LayerId(id)).unwrap().psd_id.unwrap())
}

/// Every layer's persistent id, top-down, as `document.inspect` reports it.
fn inspected(s: &mut Session) -> Vec<(String, Value)> {
    fn walk(layers: &Value, out: &mut Vec<(String, Value)>) {
        for l in layers.as_array().into_iter().flatten() {
            out.push((l["name"].as_str().unwrap_or_default().to_string(), l["persistentId"].clone()));
            walk(&l["children"], out);
        }
    }
    let mut out = Vec::new();
    walk(&s.execute("document.inspect", json!({})).unwrap()["layers"], &mut out);
    out
}

fn png_bytes(w: u32, h: u32) -> Vec<u8> {
    let mut d = photocraft_doc::Document::new("p", photocraft_doc::Size::new(w, h), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut px = photocraft_raster::Surface::new(d.pixel_format());
    px.fill_rect(photocraft_geom::Rect::new(0, 0, w as i32, h as i32), &[1.0, 0.0, 0.0, 1.0]);
    d.layers.push(photocraft_doc::Layer::new("p", photocraft_doc::LayerContent::Raster(px)));
    photocraft_io::export(&d, "png", &Default::default()).unwrap().bytes
}

/// Every layer gets its id when it is created, whatever creates it, and `document.inspect` always
/// reports it. The command only reads it: no history step, a clean document stays clean.
#[test]
fn every_layer_gets_an_id_when_it_is_created() {
    let mut s = session();
    new_layer(&mut s, "new");
    s.execute("layer.duplicate", json!({})).unwrap();
    s.execute("layer.groupLayers", json!({})).unwrap();
    crate::file_cmds::place_bytes(&mut s, "placed.png", png_bytes(4, 4), None, &json!({})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    let bg = s.active().unwrap().doc.layers[0].id;
    s.select_layer(bg).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    s.execute("edit.paste", json!({})).unwrap();
    let all = inspected(&mut s);
    assert!(all.len() >= 6, "{all:?}");
    let ids: Vec<u64> = all.iter().map(|(name, id)| id.as_u64().unwrap_or_else(|| panic!("{name} has no id: {all:?}"))).collect();
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "unique: {all:?}");

    let st = s.active_mut().unwrap();
    st.saved_revision = st.revision;
    let (steps, revision) = (s.active().unwrap().history.entries().len(), s.active().unwrap().revision);
    let active = s.active().unwrap().active_layer.unwrap().0;
    let r = s.execute(PERSISTENT_ID, json!({})).unwrap();
    assert_eq!(r, json!({"layer": active, "persistentId": pid(&s, active)}), "the active layer by default");
    assert_eq!((s.active().unwrap().history.entries().len(), s.active().unwrap().revision), (steps, revision), "no step");
    assert!(!s.active().unwrap().is_dirty());

    // An opened file's layers without ids get them too.
    let opened = photocraft_io::import("p.png", &png_bytes(4, 4)).unwrap().document;
    assert!(opened.layers.iter().all(|l| l.psd_id.is_none()));
    s.add_document(opened, None);
    assert!(inspected(&mut s).iter().all(|(_, id)| id.as_u64().is_some()));
}

/// The review's case: new layers a, b and c; undo. Ids stay with their layers and none is given
/// out twice, through undo, redo and a new layer after an undo.
#[test]
fn ids_survive_undo_and_redo_and_are_never_reused() {
    let mut s = session();
    let a = new_layer(&mut s, "a");
    let b = new_layer(&mut s, "b");
    let c = new_layer(&mut s, "c");
    let (ia, ib, ic) = (pid(&s, a), pid(&s, b), pid(&s, c));
    assert!(ia != ib && ib != ic && ia != ic);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!((pid(&s, a), pid(&s, b)), (ia, ib), "undo keeps the others' ids");
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(pid(&s, c), ic, "redo brings c back with its id");
    s.execute("edit.undo", json!({})).unwrap();
    let d = new_layer(&mut s, "d");
    assert!(![ia, ib, ic].contains(&pid(&s, d)), "the undone layer's id is not given to the next one");
    // A duplicate gets its own id; the original keeps its.
    s.execute("layer.duplicate", json!({"layer": a})).unwrap();
    let copy = s.active().unwrap().active_layer.unwrap().0;
    assert_ne!(copy, a);
    assert!(![ia, ib, ic, pid(&s, d)].contains(&pid(&s, copy)));
    assert_eq!(pid(&s, a), ia);
    // Editing a layer in place keeps its id: its properties, or rasterizing it.
    s.execute("layer.setProps", json!({"layer": a, "name": "renamed", "opacity": 0.5})).unwrap();
    assert_eq!(pid(&s, a), ia);
    let text = s.execute("type.create", json!({"x": 2, "y": 12, "text": "Hi", "size": 10, "color": "#000000"})).unwrap()["layer"].as_u64().unwrap();
    let it = pid(&s, text);
    s.execute("layer.rasterize.type", json!({"layer": text})).unwrap();
    assert_eq!(pid(&s, text), it, "a rasterized layer keeps its id");
    assert_eq!(pid(&s, b), ib);
}

/// A snapshot brings layers back with their ids; Revert brings back the file's.
#[test]
fn snapshots_and_revert_keep_the_ids() {
    let mut s = session();
    let a = new_layer(&mut s, "a");
    let ia = pid(&s, a);
    s.execute("history.newSnapshot", json!({})).unwrap();
    s.execute("layer.delete", json!({"layer": a})).unwrap();
    let b = new_layer(&mut s, "b");
    s.execute("history.selectSnapshot", json!({"snapshot": 1})).unwrap();
    assert_eq!(pid(&s, a), ia, "the snapshot's layer keeps its id");
    let after = new_layer(&mut s, "after");
    assert_ne!(pid(&s, after), ia);
    assert!(s.active().unwrap().doc.layer(LayerId(b)).is_none());

    let dir = std::env::temp_dir().join(format!("pc-pipeline-revert-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("work.psd").to_string_lossy().into_owned();
    // Save As: the copy becomes the document's file.
    s.execute("file.saveACopy", json!({"path": path})).unwrap();
    s.active_mut().unwrap().saved_to(path.clone());
    let saved = inspected(&mut s);
    s.execute("layer.delete", json!({"layer": a})).unwrap();
    let extra = new_layer(&mut s, "extra");
    let extra_id = pid(&s, extra);
    s.execute("file.revert", json!({})).unwrap();
    assert_eq!(inspected(&mut s), saved, "the file's own ids, not new ones");
    let next = new_layer(&mut s, "next");
    assert!(pid(&s, next) > extra_id, "ids given before the revert are not given again");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An opened file keeps its ids; two layers sharing one (a damaged file) end up unique.
#[test]
fn an_opened_file_keeps_its_ids_and_duplicates_are_renumbered() {
    let mut d = photocraft_doc::Document::new("f", photocraft_doc::Size::new(8, 8), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    for (name, id) in [("x", 30), ("y", 30), ("z", 12)] {
        let mut l = photocraft_doc::Layer::new(name, photocraft_doc::LayerContent::Raster(photocraft_raster::Surface::new(d.pixel_format())));
        l.psd_id = Some(id);
        d.layers.push(l);
    }
    let mut s = Session::new();
    s.add_document(d, None);
    let ids: Vec<(String, Value)> = inspected(&mut s);
    assert_eq!(ids, [("z", json!(12)), ("y", json!(31)), ("x", json!(30))].map(|(n, i)| (n.to_string(), i)), "top-down");
    let fresh = new_layer(&mut s, "fresh");
    assert_eq!(pid(&s, fresh), 32, "after the file's highest id");
}

/// What it is for: the records and the ids are still there after a save and reopen, in every
/// format that keeps the layers, at every depth.
#[test]
fn records_survive_psd_psb_tiff_and_pcraft_round_trips() {
    let formats = [
        ("round.psd", photocraft_io::ExportOptions::default()),
        ("round.psb", photocraft_io::ExportOptions { force_psb: true, ..Default::default() }),
        ("round.tif", photocraft_io::ExportOptions { tiff_layers: true, ..Default::default() }),
        ("round.pcraft", photocraft_io::ExportOptions::default()),
    ];
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16, "depth": depth})).unwrap();
        new_layer(&mut s, "plain");
        let tagged = new_layer(&mut s, "tagged");
        let id = pid(&s, tagged);
        let tags = json!({"instances": [{"layer": id, "product": "imageHero <&> ü"}]});
        s.execute(USER_DATA, json!({"key": "pipeline", "value": tags})).unwrap();
        let before = inspected(&mut s);
        let doc = s.active().unwrap().doc.clone();
        for (name, opts) in &formats {
            let bytes = photocraft_io::export(&doc, name, opts).unwrap().bytes;
            let back = photocraft_io::import(name, &bytes).unwrap().document;
            let mut reopened = Session::new();
            reopened.add_document(back, None);
            assert_eq!(reopened.execute(USER_DATA, json!({"key": "pipeline"})).unwrap()["value"], tags, "{name} at {depth} bits");
            assert_eq!(inspected(&mut reopened), before, "{name} at {depth} bits: the same ids");
            let fresh = new_layer(&mut reopened, "fresh");
            assert!(pid(&reopened, fresh) > id, "{name}: a new layer never takes a saved id");
        }
    }
}

#[test]
fn persistent_id_bad_params_fail_gracefully() {
    let mut s = session();
    for p in [json!({"layer": 999_999}), json!({"layer": "x"}), json!({"layer": -1}), json!({"layer": 1.5}), json!({"layer": [1]})] {
        assert!(s.execute(PERSISTENT_ID, p.clone()).is_err(), "{p}");
    }
    assert!(Session::new().execute(PERSISTENT_ID, json!({})).is_err(), "no document: disabled");
}
