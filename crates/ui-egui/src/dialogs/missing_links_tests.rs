//! Cannot Locate Linked Assets (#14122): a `.pcraft` whose linked files were moved away opens with
//! the dialog; OK keeps it as read, Relink… relinks through Relink to File, automation opens list
//! the files in `warnings`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use craft::host::{PickKind, Picked, ScriptedFiles};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::{LayerContent, LayerId, SmartSource};
use photocraft_geom::Rect;
use serde_json::json;

use super::*;
use crate::{Services, control};

/// The file service, whose dialogs stay open until the test answers them.
type Open = Arc<ScriptedFiles>;

/// A folder holding `doc.pcraft`, a 40×40 document with three layers linked to `art.png` (a red
/// PNG placed linked, a duplicate sharing its contents, and a second placement of the same file);
/// `art.png` is gone by the time the test opens the document unless `keep` is set.
struct Saved {
    dir: tempfile::TempDir,
    doc: PathBuf,
    art: PathBuf,
    layers: [u64; 3],
}

/// The bytes of a solid `color` 10×10 PNG.
fn png(color: &str) -> Vec<u8> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 10, "height": 10, "background": color})).unwrap();
    photocraft_io::export(&s.active().unwrap().doc, "x.png", &Default::default()).unwrap().bytes
}

/// See [`Saved`]. `stored` is the path the links keep (`None`: `art.png`'s absolute path).
fn saved(keep: bool, stored: Option<&str>) -> Saved {
    let dir = tempfile::tempdir().unwrap();
    let (doc, art) = (dir.path().join("doc.pcraft"), dir.path().join("art.png"));
    std::fs::write(&art, png("#ff0000")).unwrap();
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 40, "height": 40})).unwrap();
    let place = |s: &mut photocraft_engine::Session| {
        s.execute("file.placeLinked", json!({"path": art.to_string_lossy()})).unwrap();
        s.active().unwrap().active_layer.unwrap().0
    };
    let a = place(&mut s);
    let b = s.execute("layer.duplicate", json!({"layer": a})).unwrap()["layer"].as_u64().unwrap();
    let c = place(&mut s);
    if let Some(stored) = stored {
        s.edit("relative", |d, _| {
            for id in [a, b, c] {
                if let LayerContent::Smart(sm) = &mut d.layer_mut(LayerId(id)).unwrap().content {
                    sm.source = SmartSource::Linked { path: stored.to_string() };
                }
            }
            Ok(())
        })
        .unwrap();
    }
    s.execute("file.saveACopy", json!({"path": doc.to_string_lossy()})).unwrap();
    if !keep {
        std::fs::remove_file(&art).unwrap();
    }
    Saved { dir, doc, art, layers: [a, b, c] }
}

/// An app that imports with the real decoders, opens inline (no worker) and whose file dialogs
/// stay open until the test answers them.
fn app() -> (Box<PhotocraftApp>, Open) {
    let open: Open = Arc::new(ScriptedFiles::held());
    let services = Services {
        import: Some(Box::new(|name, bytes, opts| {
            photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
        })),
        files: Some(open.clone()),
        automation_read: Some(Box::new(|path: &str| {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            let name = Path::new(path).file_name().unwrap().to_string_lossy().into_owned();
            Ok(crate::AutomationReadResult { name, bytes, save_path: Some(path.to_string()) })
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.background_jobs = false;
    (app, open)
}

/// Opens `path` as File › Open does, then lets its look at the linked files answer, as the
/// passes after the open do.
fn open_file(app: &mut PhotocraftApp, path: &Path) {
    let p = path.to_string_lossy().into_owned();
    app.open_file(&p, &std::fs::read(path).unwrap()).unwrap();
    settle_looks(app);
}

/// Shows the dialogs of the looks still running, waiting (at most 10 s) until each has answered.
fn settle_looks(app: &mut PhotocraftApp) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !app.link_looks.is_empty() {
        assert!(std::time::Instant::now() < deadline, "the look at the linked files answers");
        std::thread::sleep(std::time::Duration::from_millis(1));
        take_late_looks(app);
    }
}

/// The open Cannot Locate Linked Assets dialogs.
fn dialogs(app: &PhotocraftApp) -> Vec<&MissingLinksDialog> {
    open_ids(app).into_iter().filter_map(|id| app.framework.dialogs.get_by_id::<MissingLinksDialog>(id)).collect()
}

/// The id of the one open Cannot Locate Linked Assets dialog.
fn dialog_id_of(app: &PhotocraftApp) -> DialogId {
    let ids = open_ids(app);
    assert_eq!(ids.len(), 1, "one dialog is open");
    ids[0]
}

/// The files `dialog` lists, as `ui.inspect` reports them.
fn listed(dialog: &MissingLinksDialog) -> serde_json::Value {
    dialog.json_fields().unwrap()["files"].clone()
}

/// Whether no dialog is open on the app's dialog host.
fn no_dialogs(app: &PhotocraftApp) -> bool {
    app.framework.dialogs.open_dialogs().is_empty()
}

/// Each linked layer's stored path, in `layers` order.
fn links(app: &PhotocraftApp, layers: [u64; 3]) -> Vec<String> {
    let doc = &app.session.active().unwrap().doc;
    layers
        .iter()
        .map(|id| match &doc.layer(LayerId(*id)).unwrap().content {
            LayerContent::Smart(sm) => match &sm.source {
                SmartSource::Linked { path } => path.clone(),
                other => panic!("not linked: {other:?}"),
            },
            other => panic!("not a smart object: {}", other.kind_name()),
        })
        .collect()
}

/// The composite's colour at (20, 20), where every placement lies.
fn centre(app: &PhotocraftApp) -> [u8; 3] {
    let p = photocraft_compose::render(&app.session.active().unwrap().doc, Rect::new(20, 20, 21, 21)).px[0];
    [p[0], p[1], p[2]].map(|c| (c * 255.0).round() as u8)
}

/// Answers the file dialog the user is in.
fn answer(open: &Open, picked: Picked) {
    open.answer(picked).expect("a file dialog is open");
}

/// Sends the control request `method` and returns its immediate reply.
fn call(app: &mut PhotocraftApp, method: &str, params: serde_json::Value) -> serde_json::Value {
    let (req, _rx) = control::ControlRequest::new(method, params);
    let ctx = app.ctx().clone();
    let control::Reply::Done(r) = control::handle(app, &ctx, &req) else { panic!("{method} answers at once") };
    r
}

fn abs(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn an_open_with_a_missing_link_lists_the_file_once_by_its_absolute_path() {
    let s = saved(false, None);
    let (mut app, _) = app();
    open_file(&mut app, &s.doc);
    let shown = dialogs(&app);
    assert_eq!(shown.len(), 1, "{:?}", app.framework.dialogs.open_dialogs());
    assert_eq!(shown[0].document(), app.session.active().unwrap().doc.id);
    let f = shown[0].json_fields().unwrap();
    assert_eq!(f["document"], json!(app.session.active().unwrap().doc.id.0));
    // Three layers, two smart objects, one file: one row.
    assert_eq!(f["files"], json!([{"file": abs(&s.art), "paths": [abs(&s.art)]}]));
}

#[test]
fn a_relative_link_is_listed_by_the_file_it_names_beside_the_document() {
    let s = saved(false, Some("art.png"));
    let (mut app, _) = app();
    open_file(&mut app, &s.doc);
    assert_eq!(listed(dialogs(&app)[0]), json!([{"file": abs(&s.art), "paths": ["art.png"]}]));
}

#[test]
fn every_missing_file_gets_a_row_in_path_order() {
    let s = saved(false, None);
    let other = s.dir.path().join("a-first.png");
    std::fs::write(&other, png("#00ff00")).unwrap();
    let mut session = photocraft_engine::Session::new();
    let bytes = std::fs::read(&s.doc).unwrap();
    photocraft_engine::file_cmds::open_bytes_as(&mut session, "doc.pcraft", &bytes, None, Some(abs(&s.doc))).unwrap();
    session.execute("file.placeLinked", json!({"path": abs(&other)})).unwrap();
    session.execute("file.saveACopy", json!({"path": abs(&s.doc), "overwrite": true})).unwrap();
    std::fs::remove_file(&other).unwrap();
    let (mut app, _) = app();
    open_file(&mut app, &s.doc);
    let rows = listed(dialogs(&app)[0]);
    assert_eq!(rows, json!([{"file": abs(&other), "paths": [abs(&other)]}, {"file": abs(&s.art), "paths": [abs(&s.art)]}]));
}

#[test]
fn links_that_are_there_or_no_links_open_without_the_dialog() {
    let s = saved(true, None);
    let (mut app, _) = app();
    open_file(&mut app, &s.doc);
    assert!(no_dialogs(&app), "every linked file is there");
    let plain = s.dir.path().join("plain.png");
    std::fs::write(&plain, png("#ffffff")).unwrap();
    open_file(&mut app, &plain);
    assert!(no_dialogs(&app), "no links");
}

#[test]
fn ok_and_cancel_keep_the_document_as_it_was_read() {
    let s = saved(false, None);
    for ok in [true, false] {
        let (mut app, _) = app();
        open_file(&mut app, &s.doc);
        let history = app.session.active().unwrap().history.past_len();
        let id = dialog_id_of(&app);
        if ok {
            crate::dialogs::testing::confirm(&mut app, id).unwrap();
        } else {
            crate::dialogs::testing::cancel(&mut app, id).unwrap();
        }
        assert!(no_dialogs(&app));
        assert_eq!(app.session.documents().len(), 1, "the document stays open");
        assert_eq!(links(&app, s.layers), [abs(&s.art), abs(&s.art), abs(&s.art)], "the links are kept");
        assert_eq!(centre(&app), [255, 0, 0], "the last pixels show");
        assert_eq!(app.session.active().unwrap().history.past_len(), history, "no History state");
    }
}

#[test]
fn relink_links_every_layer_of_the_file_to_the_one_picked_and_the_row_leaves() {
    let s = saved(false, None);
    let found = s.dir.path().join("found.png");
    std::fs::write(&found, png("#0000ff")).unwrap();
    let (mut app, open) = app();
    open_file(&mut app, &s.doc);
    let id = dialog_id_of(&app);
    // Cancelling the file dialog changes nothing.
    relink(&mut app, id, &abs(&s.art)).unwrap();
    assert_eq!(open.open(), 1);
    assert_eq!(open.requests().last().unwrap().kind, PickKind::Open, "one file is asked for");
    answer(&open, Picked::Cancelled);
    app.poll_file_dialog();
    assert_eq!(links(&app, s.layers), [abs(&s.art), abs(&s.art), abs(&s.art)]);
    assert_eq!(dialogs(&app)[0].files().len(), 1);
    // A picked file relinks all three layers through Relink to File: one state per smart object.
    let history = app.session.active().unwrap().history.past_len();
    relink(&mut app, id, &abs(&s.art)).unwrap();
    answer(&open, Picked::Paths(vec![found.clone()]));
    app.poll_file_dialog();
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(links(&app, s.layers), [abs(&found), abs(&found), abs(&found)]);
    assert_eq!(centre(&app), [0, 0, 255]);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), history + 2);
    assert_eq!(st.history.entries()[history + 1..], ["Relink to File", "Relink to File"].map(String::from)[..], "{:?}", st.history.entries());
    assert_eq!(listed(dialogs(&app)[0]), json!([]), "the row left the list");
}

#[test]
fn relink_refuses_a_file_without_a_path_and_a_closed_dialog() {
    let s = saved(false, None);
    let (mut app, _) = app();
    open_file(&mut app, &s.doc);
    let id = dialog_id_of(&app);
    let err = relink_to(&mut app, id, &abs(&s.art), "art.png").unwrap_err();
    assert!(err.contains("needs a file on disk"), "{err}");
    assert!(relink_to(&mut app, id, "/elsewhere/other.png", &abs(&s.doc)).is_err(), "not listed");
    assert_eq!(dialogs(&app)[0].files().len(), 1);
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    assert!(relink_to(&mut app, id, &abs(&s.art), &abs(&s.doc)).is_err(), "the dialog was closed");
    assert_eq!(links(&app, s.layers), [abs(&s.art), abs(&s.art), abs(&s.art)]);
}

#[test]
fn automation_opens_list_the_files_in_warnings_without_the_dialog() {
    let s = saved(false, None);
    let warning = format!("Cannot locate linked asset: {}", abs(&s.art));
    // The control channel's app.open.
    let (mut app, _) = app();
    let r = call(&mut app, "app.open", json!({"path": abs(&s.doc)}));
    assert_eq!(r["result"]["warnings"], json!([warning]), "{r}");
    assert!(no_dialogs(&app));
    // The engine's open (CLI, agents, Batch).
    let mut session = photocraft_engine::Session::new();
    let bytes = std::fs::read(&s.doc).unwrap();
    let r = photocraft_engine::file_cmds::open_bytes_as(&mut session, "doc.pcraft", &bytes, None, Some(abs(&s.doc))).unwrap();
    assert_eq!(r["warnings"], json!([warning]));
    // With the file back, nothing is reported.
    std::fs::write(&s.art, png("#ff0000")).unwrap();
    let r = photocraft_engine::file_cmds::open_bytes_as(&mut session, "doc.pcraft", &bytes, None, Some(abs(&s.doc))).unwrap();
    assert_eq!(r["warnings"], json!([]));
}

/// The dialog as drawn: the heading, a row of Relink…, chain and path, the footer, OK and Cancel
/// on the right; Relink… asks for a file and OK closes it.
#[test]
fn the_dialog_draws_the_rows_and_relinks_from_its_button() {
    let s = saved(false, None);
    let found = s.dir.path().join("found.png");
    std::fs::write(&found, png("#0000ff")).unwrap();
    let (mut app, open) = app();
    open_file(&mut app, &s.doc);
    let ctx = app.ctx().clone();
    let mut h = Harness::<'static, Box<PhotocraftApp>>::builder()
        .with_context(ctx)
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui_state(|ui: &mut egui::Ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    h.run_steps(3);
    let heading = h.get_by_label("Cannot locate linked assets:").rect();
    let button = h.get_by_label("Relink…").rect();
    let path = h.get_by_label(&abs(&s.art)).rect();
    let footer = h.get_by_label_contains("Relink to Library Graphic").rect();
    let (ok, cancel) = (h.get_by_label("OK").rect(), h.get_by_label("Cancel").rect());
    assert!(heading.bottom() <= button.top() && button.bottom() <= footer.top() && footer.bottom() <= ok.top(), "top to bottom");
    assert!((button.center().y - path.center().y).abs() < 4.0, "Relink… and the path on one row");
    assert!(path.left() - button.right() >= 16.0, "Relink… left of the path, room for the chain between");
    assert!(ok.left() > button.right() && cancel.left() > button.right(), "OK and Cancel on the right");
    h.get_by_label("Relink…").click();
    h.run_steps(2);
    assert_eq!(open.open(), 1, "Relink… asks for a file");
    answer(&open, Picked::Paths(vec![found.clone()]));
    h.state_mut().poll_file_dialog();
    h.run_steps(2);
    assert_eq!(links(h.state(), s.layers), [abs(&found), abs(&found), abs(&found)]);
    assert!(h.query_by_label("Relink…").is_none(), "the row left");
    h.get_by_label("OK").click();
    h.run_steps(2);
    assert!(no_dialogs(h.state()));
}

/// A document that keeps no path (a directory `.pcraft`, a template, a read-only source) still has
/// its relative links taken from the file it was opened from.
#[test]
fn automation_warnings_take_relative_links_from_the_opened_file() {
    let s = saved(false, Some("art.png"));
    let (mut app, _) = app();
    let r = call(&mut app, "app.open", json!({"path": abs(&s.doc)}));
    let warning = format!("Cannot locate linked asset: {}", abs(&s.art));
    assert_eq!(r["result"]["warnings"], json!([warning]), "{r}");
    app.session.active_mut().unwrap().path = None;
    assert_eq!(app.missing_link_warnings(&abs(&s.doc)), [warning]);
    let doc = &app.session.active().unwrap().doc;
    assert!(photocraft_engine::link_watch::missing_file_warnings(doc, None, None).is_empty(), "nothing to take a relative link from");
    // The interactive look answers the same files.
    let looked = photocraft_engine::link_watch::missing_files_within(doc, Some(&s.doc), std::time::Duration::from_secs(10));
    assert_eq!(looked, Some(vec![(s.art.clone(), vec!["art.png".to_string()])]));
}

/// A file dated before 1970 has no watcher stamp, but it is there: not listed.
#[test]
fn a_file_dated_before_1970_is_not_missing() {
    let s = saved(true, None);
    let old = std::time::UNIX_EPOCH - std::time::Duration::from_secs(10 * 365 * 24 * 3600);
    std::fs::File::options().write(true).open(&s.art).unwrap().set_modified(old).unwrap();
    let (mut app, _) = app();
    open_file(&mut app, &s.doc);
    assert!(no_dialogs(&app), "{:?}", app.framework.dialogs.open_dialogs());
}

/// Refuses every path, as an automation policy refuses one outside its granted root.
fn refuse_paths(_: &str, p: &serde_json::Value) -> photocraft_engine::Result<()> {
    match p.get("path") {
        Some(_) => Err(photocraft_engine::EngineError::Other("outside the granted root".into())),
        None => Ok(()),
    }
}

/// An automation open looks only at files its policy lets it read: a refused link is not reported,
/// so the warnings can't tell whether a file outside the granted root exists.
#[test]
fn automation_warnings_skip_files_the_policy_refuses() {
    let s = saved(false, None);
    let (mut app, _) = app();
    app.services.automation_authorize = Some(refuse_paths);
    let r = call(&mut app, "app.open", json!({"path": abs(&s.doc)}));
    assert_eq!(r["result"]["warnings"], json!([]), "{r}");
    // The engine's open under the same gate.
    let mut session = photocraft_engine::Session::new();
    session.authorize = Some(refuse_paths);
    let bytes = std::fs::read(&s.doc).unwrap();
    let r = photocraft_engine::file_cmds::open_bytes_as(&mut session, "doc.pcraft", &bytes, None, Some(abs(&s.doc))).unwrap();
    assert_eq!(r["warnings"], json!([]));
}

/// Raw Defaults › Resolution goes by the import's own notes, not by a missing link's path that
/// happens to contain the developed-raw note.
#[test]
fn a_missing_link_named_like_the_raw_note_keeps_the_resolution() {
    let s = saved(true, None);
    let named = s.dir.path().join(format!("{}.png", photocraft_io::raw::DEVELOPED_NOTE));
    let mut session = photocraft_engine::Session::new();
    let bytes = std::fs::read(&s.doc).unwrap();
    photocraft_engine::file_cmds::open_bytes_as(&mut session, "doc.pcraft", &bytes, None, Some(abs(&s.doc))).unwrap();
    std::fs::write(&named, png("#00ff00")).unwrap();
    session.execute("file.placeLinked", json!({"path": abs(&named)})).unwrap();
    session.execute("file.saveACopy", json!({"path": abs(&s.doc), "overwrite": true})).unwrap();
    std::fs::remove_file(&named).unwrap();
    let dpi = session.active().unwrap().doc.resolution_dpi;
    session.edit_prefs(|p| p.raw_defaults.resolution = 300.0);
    let bytes = std::fs::read(&s.doc).unwrap();
    let r = photocraft_engine::file_cmds::open_bytes_as(&mut session, "doc.pcraft", &bytes, None, Some(abs(&s.doc))).unwrap();
    assert_eq!(r["warnings"], json!([format!("Cannot locate linked asset: {}", abs(&named))]));
    assert_eq!(session.active().unwrap().doc.resolution_dpi, dpi);
}

/// The desktop agent policy as far as linked files go: every smart-object command that reads a
/// file is refused, whatever its path (`photocraft_automation::workspace::authorize_engine_command`).
fn refuse_smart_object_files(id: &str, _: &serde_json::Value) -> photocraft_engine::Result<()> {
    if id.starts_with("layer.smartObjects.") { Err(photocraft_engine::EngineError::Other(format!("{id} is disabled"))) } else { Ok(()) }
}

/// Stores `stored` as the links of the three layers of `s`, one each, in its saved document.
fn store_links(s: &Saved, stored: [&str; 3]) {
    let mut session = photocraft_engine::Session::new();
    let bytes = std::fs::read(&s.doc).unwrap();
    photocraft_engine::file_cmds::open_bytes_as(&mut session, "doc.pcraft", &bytes, None, Some(abs(&s.doc))).unwrap();
    session
        .edit("links", |d, _| {
            for (id, path) in s.layers.iter().zip(stored) {
                if let LayerContent::Smart(sm) = &mut d.layer_mut(LayerId(*id)).unwrap().content {
                    sm.source = SmartSource::Linked { path: path.to_string() };
                }
            }
            Ok(())
        })
        .unwrap();
    session.execute("file.saveACopy", json!({"path": abs(&s.doc), "overwrite": true})).unwrap();
}

/// An agent's `app.open` names its file below a read folder and runs under a policy that refuses
/// every smart-object file command. Its missing linked files are looked for through the read
/// folder: the ones below it are reported, relative links taken from where the folder holds the
/// document, and a link that leaves the folder is not looked at, there or not.
#[test]
fn automation_opens_look_for_linked_files_through_the_read_folder() {
    let s = saved(false, None);
    let root = s.dir.path().to_path_buf();
    let elsewhere = tempfile::tempdir().unwrap();
    let (inside, outside) = (root.join("in.png"), elsewhere.path().join("out.png"));
    store_links(&s, ["art.png", &abs(&inside), &abs(&outside)]);
    // The directory form of the same document.
    let doc = photocraft_format::load_from_bytes(&std::fs::read(&s.doc).unwrap()).unwrap();
    photocraft_format::PcraftWriter::new().save_dir(&doc, &root.join("folder.pcraft"), &photocraft_format::SaveOptions::default()).unwrap();
    let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::<PathBuf>::new()));
    let services = || {
        let (read, dir, locate, look, asked) = (root.clone(), root.clone(), root.clone(), root.clone(), asked.clone());
        Services {
            import: Some(Box::new(|name, bytes, opts| {
                photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            automation_read: Some(Box::new(move |path: &str| {
                let bytes = std::fs::read(read.join(path)).map_err(|e| e.to_string())?;
                Ok(crate::AutomationReadResult { name: path.to_string(), bytes, save_path: None })
            })),
            automation_open_pcraft: Some(Box::new(move |path: &str| {
                let at = dir.join(path);
                if !at.is_dir() {
                    return Ok(None);
                }
                photocraft_format::load_path(&at).map(|doc| Some((path.to_string(), doc))).map_err(|e| e.to_string())
            })),
            automation_locate: Some(Box::new(move |path: &str| Some(abs(&locate.join(path))))),
            automation_link_missing: Some(Box::new(move |file: &Path| {
                asked.borrow_mut().push(file.to_path_buf());
                file.strip_prefix(&look).ok().map(|_| !file.exists())
            })),
            automation_authorize: Some(refuse_smart_object_files),
            ..Default::default()
        }
    };
    let warnings = |files: &[&Path]| json!(files.iter().map(|f| format!("Cannot locate linked asset: {}", abs(f))).collect::<Vec<_>>());
    for (request, there) in [("doc.pcraft", false), ("folder.pcraft", false), ("doc.pcraft", true)] {
        // What lies outside the read folder changes nothing, there or not.
        if there {
            std::fs::write(&outside, png("#00ff00")).unwrap();
        }
        asked.borrow_mut().clear();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services());
        app.background_jobs = false;
        let r = call(&mut app, "app.open", json!({"path": request}));
        assert_eq!(r["result"]["warnings"], warnings(&[&s.art, &inside]), "{request}: {r}");
        assert!(no_dialogs(&app));
        let mut looked = asked.borrow().clone();
        looked.sort();
        let mut links = vec![s.art.clone(), inside.clone(), outside.clone()];
        links.sort();
        assert_eq!(looked, links, "{request}: every link is looked for through the read folder only");
    }
    // A file that comes back is no longer reported.
    std::fs::write(&inside, png("#0000ff")).unwrap();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services());
    app.background_jobs = false;
    let r = call(&mut app, "app.open", json!({"path": "doc.pcraft"}));
    assert_eq!(r["result"]["warnings"], warnings(&[&s.art]), "{r}");
}

/// A look that has not answered when the open stops waiting (a share that is slow to answer) is
/// kept: the document opens without the dialog, and the dialog shows once the look answers.
#[test]
fn a_look_that_answers_late_shows_the_dialog_when_it_arrives() {
    let s = saved(true, None);
    let (mut app, _) = app();
    open_file(&mut app, &s.doc);
    assert!(no_dialogs(&app), "every linked file is there");
    let document = app.session.active().unwrap().doc.id;
    let ctx = app.ctx().clone();
    let (answer, look) = std::sync::mpsc::channel();
    let mut looked_from = None;
    ask_with(&mut app, document, Some(&abs(&s.doc)), |_, opened| {
        looked_from = opened.map(Path::to_path_buf);
        look
    });
    assert_eq!(looked_from.as_deref(), Some(s.doc.as_path()), "relative links are taken from the opened file");
    assert!(no_dialogs(&app), "the open does not wait for the look");
    crate::dialogs::take_results(&mut app, &ctx);
    assert!(no_dialogs(&app), "nothing to show before the look answers");
    answer.send(vec![(s.art.clone(), vec![abs(&s.art)])]).unwrap();
    crate::dialogs::take_results(&mut app, &ctx);
    let shown = dialogs(&app);
    assert_eq!(shown.len(), 1, "{:?}", app.framework.dialogs.open_dialogs());
    assert_eq!(shown[0].document(), document);
    assert_eq!(listed(shown[0]), json!([{"file": abs(&s.art), "paths": [abs(&s.art)]}]));
    // The answer is shown once.
    let id = dialog_id_of(&app);
    crate::dialogs::testing::confirm(&mut app, id).unwrap();
    crate::dialogs::take_results(&mut app, &ctx);
    assert!(no_dialogs(&app));
    assert!(app.link_looks.is_empty());
}

/// A late answer lists only files the document still links to, and shows nothing for a document
/// that was closed meanwhile or when the look ends without an answer.
#[test]
fn a_late_answer_follows_what_happened_to_the_document_meanwhile() {
    let s = saved(true, None);
    let other = s.dir.path().join("other.png");
    std::fs::write(&other, png("#00ff00")).unwrap();
    let late = |app: &mut PhotocraftApp| {
        let document = app.session.active().unwrap().doc.id;
        let (answer, look) = std::sync::mpsc::channel();
        ask_with(app, document, Some(&abs(&s.doc)), |_, _| look);
        assert_eq!(app.link_looks.len(), 1);
        answer
    };
    let both = vec![(s.art.clone(), vec![abs(&s.art)]), (other.clone(), vec![abs(&other)])];
    // One of the two files the look names is linked: only that one is listed.
    let (mut app, _) = self::app();
    open_file(&mut app, &s.doc);
    let ctx = app.ctx().clone();
    let answer = late(&mut app);
    answer.send(both.clone()).unwrap();
    crate::dialogs::take_results(&mut app, &ctx);
    assert_eq!(listed(dialogs(&app)[0]), json!([{"file": abs(&s.art), "paths": [abs(&s.art)]}]));
    // Every layer was relinked while the look ran: nothing is left to list.
    let (mut app, _) = self::app();
    open_file(&mut app, &s.doc);
    let ctx = app.ctx().clone();
    let answer = late(&mut app);
    for layer in [s.layers[0], s.layers[2]] {
        app.run("layer.smartObjects.relinkToFile", json!({"layer": layer, "path": abs(&other)})).unwrap();
    }
    assert_eq!(links(&app, s.layers), [abs(&other), abs(&other), abs(&other)]);
    answer.send(vec![(s.art.clone(), vec![abs(&s.art)])]).unwrap();
    crate::dialogs::take_results(&mut app, &ctx);
    assert!(no_dialogs(&app), "{:?}", app.framework.dialogs.open_dialogs());
    assert!(app.link_looks.is_empty());
    // The document was closed while the look ran.
    let (mut app, _) = self::app();
    open_file(&mut app, &s.doc);
    let ctx = app.ctx().clone();
    let answer = late(&mut app);
    let index = app.session.active_index().unwrap();
    app.session.close(index);
    crate::dialogs::take_results(&mut app, &ctx);
    assert!(app.link_looks.is_empty(), "the look of a closed document is not waited for");
    assert!(answer.send(both).is_err(), "nothing listens for its answer");
    assert!(no_dialogs(&app));
    // The look ended without an answer.
    let (mut app, _) = self::app();
    open_file(&mut app, &s.doc);
    let ctx = app.ctx().clone();
    drop(late(&mut app));
    crate::dialogs::take_results(&mut app, &ctx);
    assert!(no_dialogs(&app));
    assert!(app.link_looks.is_empty());
}

/// The open never waits for its look at the linked files, not even for one that answers soon (a
/// share that is slow to answer): the document opens at once, and the dialog shows on a later pass.
#[test]
fn an_open_does_not_wait_for_a_look_that_answers_soon() {
    let s = saved(true, None);
    let (mut app, _) = app();
    open_file(&mut app, &s.doc);
    let document = app.session.active().unwrap().doc.id;
    let ctx = app.ctx().clone();
    let (answer, look) = std::sync::mpsc::channel();
    let art = s.art.clone();
    let worker = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        answer.send(vec![(art.clone(), vec![abs(&art)])]).unwrap();
    });
    let started = std::time::Instant::now();
    ask_with(&mut app, document, Some(&abs(&s.doc)), |_, _| look);
    assert!(started.elapsed() < std::time::Duration::from_millis(200), "the open went on without waiting: {:?}", started.elapsed());
    assert!(no_dialogs(&app), "nothing is shown before the look answers");
    assert_eq!(app.link_looks.len(), 1, "the look is kept");
    worker.join().unwrap();
    crate::dialogs::take_results(&mut app, &ctx);
    let shown = dialogs(&app);
    assert_eq!(shown.len(), 1, "{:?}", app.framework.dialogs.open_dialogs());
    assert_eq!(listed(shown[0]), json!([{"file": abs(&s.art), "paths": [abs(&s.art)]}]));
    assert!(app.link_looks.is_empty());
}
