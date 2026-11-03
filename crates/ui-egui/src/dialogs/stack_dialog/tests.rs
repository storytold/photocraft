use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;
use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};

/// `n` small PNGs named `names` in a fresh folder for `tag`: (folder, paths).
fn pngs(tag: &str, names: &[&str]) -> (String, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("photocraft-load-stack-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 8, "height": 6, "background": "#406080"})).unwrap();
    let paths = names
        .iter()
        .map(|n| {
            let p = dir.join(n).to_string_lossy().into_owned();
            s.execute("file.saveACopy", json!({"path": p})).unwrap();
            p
        })
        .collect();
    (dir.to_string_lossy().into_owned(), paths)
}

fn harness(answers: Vec<Option<FileDialogAnswer>>) -> (Harness<'static, PhotocraftApp>, std::rc::Rc<std::cell::RefCell<Vec<FileDialogRequest>>>) {
    let (show, asked) = crate::file_dialog::fake(answers);
    let h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_dialog: Some(show), ..Default::default() })
    });
    (h, asked)
}

fn dialog(h: &Harness<'_, PhotocraftApp>) -> Option<(u64, Map<String, Value>)> {
    h.state().ui.dialogs.iter().find(|d| owns(&d.fields)).map(|d| (d.id, d.fields.clone()))
}

fn listed(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    dialog(h).map(|(_, f)| files(&f).iter().map(|p| file_name(p).to_string()).collect()).unwrap_or_default()
}

#[test]
fn statistics_menu_opens_a_source_file_list_with_stack_mode() {
    let (mut h, _) = harness(vec![]);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.scripts.statistics", json!({})).unwrap();
    h.run_steps(4);
    for label in ["Choose Stack Mode:", "Source Files", "Browse…", "Remove", "Add Open Files", "Sort By Name", "Attempt to Automatically Align Source Images"]
    {
        assert!(h.query_by_label(label).is_some(), "Statistics is missing {label}");
    }
    assert!(h.get_by_label("OK").accesskit_node().is_disabled(), "choose source files before running statistics");
    assert!(h.query_by_label("Create Smart Object after Loading Layers").is_none(), "Statistics always creates its stack smart object");
    assert!(!dialog(&h).unwrap().1.contains_key("input"), "no generic Input field");
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(dialog(&h).is_none());
    assert!(h.state().session.documents().is_empty());
}

#[test]
fn statistics_browse_and_mode_selection_compute_the_chosen_statistic() {
    let (_, paths) = pngs("statistics-mean", &["first.png", "second.png"]);
    let mut source = photocraft_engine::Session::new();
    source.execute("file.new", json!({"width": 8, "height": 6, "background": "#80a0c0"})).unwrap();
    source.execute("file.saveACopy", json!({"path": paths[1]})).unwrap();
    let (mut h, _) = harness(vec![Some(FileDialogAnswer::Paths(paths))]);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, STATISTICS, json!({})).unwrap();
    h.run_steps(3);
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    assert_eq!(listed(&h), ["first.png", "second.png"]);
    assert!(h.get_by_label("first.png").rect().bottom() <= h.get_by_label("second.png").rect().top(), "source files must be vertical rows");
    h.get_by_value("Median").click();
    h.run_steps(2);
    for mode in photocraft_doc::StackMode::ALL {
        assert!(h.query_by_label(mode.label()).is_some(), "missing {}", mode.label());
    }
    h.get_by_label("Mean").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(4);
    assert!(dialog(&h).is_none());
    assert_eq!(h.state().session.documents().len(), 1);
    let doc = &h.state().session.active().unwrap().doc;
    assert_eq!(doc.layers.len(), 1);
    let photocraft_doc::LayerContent::Smart(smart) = &doc.layers[0].content else { panic!("expected a stack smart object") };
    assert_eq!(smart.stack_mode, Some(photocraft_doc::StackMode::Mean));
    let pixel = photocraft_compose::flatten(doc).px[0];
    for (actual, expected) in pixel.iter().zip([96.0, 128.0, 160.0, 255.0]) {
        assert!((actual * 255.0 - expected).abs() < 1.5, "{pixel:?}");
    }
}

#[test]
fn statistics_explicit_parameters_bypass_the_dialog() {
    let (_, paths) = pngs("statistics-script", &["one.png", "two.png"]);
    let (mut h, _) = harness(vec![]);
    let ctx = h.ctx.clone();
    let result = crate::menus::invoke(h.state_mut(), &ctx, STATISTICS, json!({"input": paths, "mode": "maximum"})).unwrap();
    assert_eq!(result["images"], 2);
    assert_eq!(result["mode"], "maximum");
    assert!(dialog(&h).is_none());
}

#[test]
fn statistics_folder_and_alignment_reach_the_engine() {
    let (dir, paths) = pngs("statistics-folder", &["one.png", "two.png"]);
    // Auto-Align needs identifiable detail, not a uniform tiny colour swatch.
    let mut source = photocraft_engine::Session::new();
    source.execute("file.new", json!({"width": 96, "height": 72, "background": "#606060"})).unwrap();
    source
        .edit("texture", |doc, active| {
            let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            for k in 0..40 {
                let (x, y, v) = ((k * 37) % 88, (k * 23) % 66, (k % 7) as f32 / 6.0);
                surface.fill_rect(photocraft_geom::Rect::new(x, y, x + 3 + k % 5, y + 2 + k % 4), &[v, 1.0 - v, 0.5, 1.0]);
            }
            Ok(())
        })
        .unwrap();
    for path in paths {
        source.execute("file.saveACopy", json!({"path": path})).unwrap();
    }
    let (mut h, _) = harness(vec![Some(FileDialogAnswer::FolderTo(dir))]);
    let id = open_statistics(h.state_mut());
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("use".into(), json!("folder"));
    h.run_steps(3);
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    h.get_by_label("Attempt to Automatically Align Source Images").click();
    h.run_steps(2);
    assert_eq!(dialog(&h).unwrap().1["align"], true);
    let result = crate::dialogs::confirm(h.state_mut(), id).unwrap();
    assert_eq!(result["images"], 2);
    assert_eq!(result["mode"], "median");
    let doc = &h.state().session.active().unwrap().doc;
    let photocraft_doc::LayerContent::Smart(smart) = &doc.layers[0].content else { panic!("expected a stack smart object") };
    assert_eq!(smart.stack_mode, Some(photocraft_doc::StackMode::Median));
}

/// #4671 / #4696: the menu opens Photoshop's dialog, Browse… adds files (once each), Remove,
/// Sort By Name and Add Open Files edit the list, and OK loads it.
#[test]
fn source_files_list_is_built_with_browse_remove_sort_and_open_files() {
    let (_, paths) = pngs("files", &["b.png", "c.png", "a.png"]);
    let (mut h, asked) = harness(vec![
        Some(FileDialogAnswer::Paths(vec![paths[0].clone(), paths[1].clone()])),
        Some(FileDialogAnswer::Paths(vec![paths[2].clone(), paths[0].clone()])),
    ]);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    h.run_steps(4);
    assert!(dialog(&h).is_some(), "the menu opens the dialog");
    for label in [
        "Source Files",
        "Browse…",
        "Remove",
        "Add Open Files",
        "Sort By Name",
        "Attempt to Automatically Align Source Images",
        "Create Smart Object after Loading Layers",
    ] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    assert!(h.get_by_label("OK").accesskit_node().is_disabled(), "nothing to load yet");
    assert!(h.get_by_label("Remove").accesskit_node().is_disabled());

    h.get_by_label("Browse…").click();
    h.run_steps(4);
    assert!(matches!(asked.borrow()[0], FileDialogRequest::Open { multiple: true, .. }), "{:?}", asked.borrow());
    assert_eq!(listed(&h), ["b.png", "c.png"]);
    assert!(h.get_by_label("b.png").rect().bottom() <= h.get_by_label("c.png").rect().top(), "source files must be vertical rows");
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    assert_eq!(listed(&h), ["b.png", "c.png", "a.png"], "b.png only once");

    h.get_by_label("Sort By Name").click();
    h.run_steps(2);
    assert_eq!(listed(&h), ["a.png", "b.png", "c.png"]);
    h.get_by_label("b.png").click();
    h.run_steps(2);
    h.get_by_label("Remove").click();
    h.run_steps(2);
    assert_eq!(listed(&h), ["a.png", "c.png"]);

    // Add Open Files: the open documents that have a file (b.png, already saved; not the untitled one).
    h.state_mut().run("file.new", json!({"width": 4, "height": 4})).unwrap();
    h.state_mut().session.active_mut().unwrap().path = Some(paths[0].clone());
    h.state_mut().run("file.new", json!({"width": 4, "height": 4})).unwrap();
    h.run_steps(3);
    h.get_by_label("Add Open Files").click();
    h.run_steps(2);
    assert_eq!(listed(&h), ["a.png", "c.png", "b.png"]);

    let docs = h.state().session.documents().len();
    h.get_by_label("OK").click();
    h.run_steps(4);
    assert!(dialog(&h).is_none());
    assert_eq!(h.state().session.documents().len(), docs + 1, "a new document");
    let d = &h.state().session.active().unwrap().doc;
    let names: Vec<&str> = d.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["a.png", "c.png", "b.png"], "one layer per file, in the list's order");
}

/// Use: Folder loads the images of the folder Browse… chose, listed in the dialog; with Create
/// Smart Object they go into one smart object.
#[test]
fn a_folder_loads_its_images() {
    let (dir, _) = pngs("folder", &["one.png", "two.png"]);
    let (mut h, asked) = harness(vec![Some(FileDialogAnswer::FolderTo(dir.clone()))]);
    let id = open(h.state_mut());
    h.run_steps(3);
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("use".into(), json!("folder"));
    h.run_steps(2);
    assert!(h.get_by_label("Remove").accesskit_node().is_disabled() && h.get_by_label("Add Open Files").accesskit_node().is_disabled());
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    assert!(matches!(asked.borrow()[0], FileDialogRequest::Folder { .. }));
    assert!(h.query_by_label("one.png").is_some() && h.query_by_label("two.png").is_some(), "the folder's images are listed");
    // Create Smart Object after Loading Layers: both images inside one smart object.
    h.get_by_label("Create Smart Object after Loading Layers").click();
    h.run_steps(2);
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let d = &h.state().session.active().unwrap().doc;
    assert_eq!(d.layers.len(), 1);
    assert!(matches!(d.layers[0].content, photocraft_doc::LayerContent::Smart(_)));
}

/// Two overlapping crops of one blocky random scene, saved as PNGs in a fresh folder for `tag`
/// (enough detail for Photomerge to register them): (folder, paths).
fn panorama_pngs(tag: &str) -> (String, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("photocraft-photomerge-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cell = |x: u64, y: u64, c: u64| (((x * 73_856_093) ^ (y * 19_349_663) ^ (c * 83_492_791)).wrapping_mul(2_654_435_761) >> 7) % 256;
    let paths = [("left.png", 0), ("right.png", 25)]
        .into_iter()
        .map(|(name, offset)| {
            let mut s = photocraft_engine::Session::new();
            s.execute("file.new", json!({"width": 160, "height": 120})).unwrap();
            s.edit("texture", |doc, active| {
                let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                for cy in 0..30 {
                    for cx in 0..40 {
                        let rgb = [0, 1, 2].map(|c| cell(cx + offset, cy, c) as f32 / 255.0);
                        let (x, y) = (cx as i32 * 4, cy as i32 * 4);
                        surface.fill_rect(photocraft_geom::Rect::new(x, y, x + 4, y + 4), &[rgb[0], rgb[1], rgb[2], 1.0]);
                    }
                }
                Ok(())
            })
            .unwrap();
            let p = dir.join(name).to_string_lossy().into_owned();
            s.execute("file.saveACopy", json!({"path": p})).unwrap();
            p
        })
        .collect();
    (dir.to_string_lossy().into_owned(), paths)
}

/// File › Automate › Photomerge… opens Photoshop's dialog: Layout, the Source Files list (Use:
/// Files picks single files with Browse…), the four options; OK merges the chosen files.
#[test]
fn photomerge_merges_the_files_chosen_with_browse() {
    let (_, paths) = panorama_pngs("files");
    let (mut h, asked) = harness(vec![Some(FileDialogAnswer::Paths(vec![paths[0].clone()])), Some(FileDialogAnswer::Paths(vec![paths[1].clone()]))]);
    h.state_mut().background_jobs = false;
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, PHOTOMERGE, json!({})).unwrap();
    h.run_steps(4);
    for label in [
        "Layout:",
        "Source Files",
        "Browse…",
        "Remove",
        "Add Open Files",
        "Blend Images Together",
        "Vignette Removal",
        "Geometric Distortion Correction",
        "Content Aware Fill Transparent Areas",
    ] {
        assert!(h.query_by_label(label).is_some(), "Photomerge is missing {label}");
    }
    assert!(h.query_by_label("Create Smart Object after Loading Layers").is_none());
    let (id, f) = dialog(&h).unwrap();
    assert!(!f.contains_key("paths"), "no generic Paths text field");

    h.get_by_label("Browse…").click();
    h.run_steps(4);
    assert!(matches!(asked.borrow()[0], FileDialogRequest::Open { multiple: true, .. }), "{:?}", asked.borrow());
    assert_eq!(listed(&h), ["left.png"]);
    assert!(h.get_by_label("OK").accesskit_node().is_disabled(), "Photomerge needs two images");
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    assert_eq!(listed(&h), ["left.png", "right.png"]);
    assert!(!h.get_by_label("OK").accesskit_node().is_disabled());

    h.get_by_label("Blend Images Together").click();
    h.run_steps(2);
    assert!(h.get_by_label("Content Aware Fill Transparent Areas").accesskit_node().is_disabled(), "fill needs the blended composite");
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("layout".into(), json!("reposition"));
    h.run_steps(2);
    let r = crate::dialogs::confirm(h.state_mut(), id).unwrap();
    assert_eq!(r["layout"], "reposition", "{r}");
    assert_eq!(r["placed"], json!(["left.png", "right.png"]), "{r}");
    assert!((r["width"].as_i64().unwrap() - 260).abs() <= 2, "{r}");
    let d = &h.state().session.active().unwrap().doc;
    assert_eq!(d.layers.len(), 2);
    assert!(d.layers.iter().all(|l| l.mask.is_none()), "Blend Images Together off: plain layers");
}

/// Use: Folder still merges every image of the chosen folder.
#[test]
fn photomerge_merges_a_folder() {
    let (dir, _) = panorama_pngs("folder");
    let (mut h, _) = harness(vec![Some(FileDialogAnswer::FolderTo(dir))]);
    h.state_mut().background_jobs = false;
    let id = open_photomerge(h.state_mut());
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("use".into(), json!("folder"));
    h.run_steps(3);
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    assert!(h.query_by_label("left.png").is_some() && h.query_by_label("right.png").is_some(), "the folder's images are listed");
    let r = crate::dialogs::confirm(h.state_mut(), id).unwrap();
    assert_eq!(r["placed"].as_array().unwrap().len(), 2, "{r}");
    let d = &h.state().session.active().unwrap().doc;
    assert!(d.layers.iter().all(|l| l.mask.is_some()), "blended by default: one masked layer per image");
}

/// The params of the last Photomerge the session ran.
fn photomerge_params(h: &Harness<'_, PhotocraftApp>) -> Value {
    h.state().session.journal.iter().rev().find(|(id, _)| id == PHOTOMERGE).map(|(_, p)| p.clone()).unwrap()
}

/// Use: Folder enables OK only for a folder with two images or more, as the engine needs; the
/// folder is listed once, not every frame.
#[test]
fn photomerge_folder_needs_two_images() {
    let (one, _) = pngs("photomerge-one", &["only.png"]);
    let (two, _) = panorama_pngs("folder-count");
    let (mut h, _) = harness(vec![Some(FileDialogAnswer::FolderTo(two.clone()))]);
    let id = open_photomerge(h.state_mut());
    for (dir, ok) in [("", false), (one.as_str(), false), (two.as_str(), true)] {
        let d = h.state_mut().ui.dialog_mut(id).unwrap();
        d.fields.insert("use".into(), json!("folder"));
        d.fields.insert("folder".into(), json!(dir));
        h.run_steps(2);
        assert_eq!(h.get_by_label("OK").accesskit_node().is_disabled(), !ok, "{dir:?}");
    }
    // A third image added while the dialog is open: the listing just made is kept (no listing
    // every frame), then it shows once the listing is RELIST_SECS old, or at once with Browse….
    let listed_now = |h: &mut Harness<'_, PhotocraftApp>| {
        let now = h.ctx.input(|i| i.time);
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("__listedAt".into(), json!(now));
    };
    let extra = std::path::Path::new(&two).join("extra.png");
    std::fs::copy(std::path::Path::new(&two).join("left.png"), &extra).unwrap();
    listed_now(&mut h);
    h.run_steps(1);
    assert!(h.query_by_label("extra.png").is_none(), "the listing is kept between frames");
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("__listedAt".into(), json!(-RELIST_SECS));
    h.run_steps(1);
    assert!(h.query_by_label("extra.png").is_some(), "listed again after RELIST_SECS");
    std::fs::remove_file(&extra).unwrap();
    listed_now(&mut h);
    h.get_by_label("Browse…").click();
    h.run_steps(3); // the answer lands a frame later; still under RELIST_SECS (0.25 s a step)
    assert!(h.query_by_label("extra.png").is_none(), "Browse… lists the folder again at once");
}

/// Use: Open Files merges every open document, unsaved ones included.
#[test]
fn photomerge_merges_the_open_documents() {
    let (_, paths) = panorama_pngs("open");
    let (mut h, _) = harness(vec![]);
    h.state_mut().background_jobs = false;
    // left.png open from its file; right.png as an unsaved document whose name has a slash.
    for (p, saved) in paths.iter().zip([true, false]) {
        let mut doc = photocraft_io::import(file_name(p), &std::fs::read(p).unwrap()).unwrap().document;
        if !saved {
            doc.name = "shot 1/2".into();
        }
        h.state_mut().session.add_document(doc, saved.then(|| p.clone()));
    }
    let id = open_photomerge(h.state_mut());
    h.run_steps(3);
    assert_eq!(open_files(h.state()).len(), 1, "Add Open Files skips the unsaved document");
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("use".into(), json!("open"));
    h.run_steps(3);
    assert!(h.query_by_label("left.png").is_some() && h.query_by_label("shot 1/2").is_some(), "the open documents are listed by name");
    assert!(h.get_by_label("Browse…").accesskit_node().is_disabled() && h.get_by_label("Add Open Files").accesskit_node().is_disabled());
    assert!(!h.get_by_label("OK").accesskit_node().is_disabled());
    let r = crate::dialogs::confirm(h.state_mut(), id).unwrap();
    assert_eq!(r["placed"].as_array().unwrap().len(), 2, "{r}");
    let p = photomerge_params(&h);
    assert_eq!(p["useOpenDocuments"], true, "{p}");
    assert!(p.get("paths").is_none(), "{p}");
}

/// Vignette Removal, Geometric Distortion Correction and Content Aware Fill Transparent Areas
/// reach the engine when checked; the fill adds its layer.
#[test]
fn photomerge_options_reach_the_engine() {
    let (_, paths) = panorama_pngs("options");
    let (mut h, _) = harness(vec![]);
    h.state_mut().background_jobs = false;
    let id = open_photomerge(h.state_mut());
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("files".into(), json!(paths));
    h.run_steps(3);
    for label in ["Vignette Removal", "Geometric Distortion Correction", "Content Aware Fill Transparent Areas"] {
        h.get_by_label(label).click();
        h.run_steps(2);
    }
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let p = photomerge_params(&h);
    for key in ["blend", "vignetteRemoval", "geometricCorrection", "contentAwareFill"] {
        assert_eq!(p[key], true, "{key}: {p}");
    }
    let d = &h.state().session.active().unwrap().doc;
    assert!(d.layers.iter().any(|l| l.name == "Content-Aware Fill"), "the fill layer");
}

/// Content Aware Fill is greyed out without Blend Images Together, and is not sent then.
#[test]
fn photomerge_does_not_fill_without_blend() {
    let (_, paths) = panorama_pngs("no-blend-fill");
    let (mut h, _) = harness(vec![]);
    h.state_mut().background_jobs = false;
    let id = open_photomerge(h.state_mut());
    let d = h.state_mut().ui.dialog_mut(id).unwrap();
    d.fields.insert("files".into(), json!(paths));
    d.fields.insert("contentAwareFill".into(), json!(true));
    d.fields.insert("blend".into(), json!(false));
    h.run_steps(2);
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let p = photomerge_params(&h);
    assert_eq!((&p["blend"], &p["contentAwareFill"]), (&json!(false), &json!(false)), "{p}");
}

/// Alt+Cancel → Reset puts Layout and the options back to their defaults and keeps the files.
#[test]
fn photomerge_alt_reset_restores_the_options() {
    let (mut h, _) = harness(vec![]);
    let id = open_photomerge(h.state_mut());
    let mut f = h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
    let fresh = f.clone();
    f.insert("files".into(), json!(["a.png", "b.png"]));
    f.insert("layout".into(), json!("cylindrical"));
    for (key, _, on) in PHOTOMERGE_OPTIONS {
        f.insert(key.into(), json!(!on));
    }
    assert!(crate::dialog_alt_reset::supported(&f));
    let ctx = h.ctx.clone();
    crate::dialog_alt_reset::apply(h.state_mut(), &ctx, &mut f);
    for key in ["layout", "blend", "vignetteRemoval", "geometricCorrection", "contentAwareFill"] {
        assert_eq!(f[key], fresh[key], "{key}");
    }
    assert_eq!(f["files"], json!(["a.png", "b.png"]), "the source files stay");
}

/// A listing slower than RELIST_SECS waits RELIST_SECS from when it finished, not from the frame
/// it started in, so a slow folder isn't listed again on every frame.
#[test]
fn a_slow_folder_is_not_listed_every_frame() {
    let mut f = Map::new();
    f.insert("folder".into(), json!("/slow/share"));
    let listings = std::cell::Cell::new(0);
    let slow = |_: &str| {
        listings.set(listings.get() + 1);
        (Ok(vec!["a.png".to_string(), "b.png".to_string()]), 1.5)
    };
    assert_eq!(relist(&mut f, 0.0, slow).len(), 2);
    // The next frame starts once the listing is done, 1.5 s later.
    relist(&mut f, 1.6, slow);
    relist(&mut f, 2.4, slow);
    assert_eq!(listings.get(), 1, "within RELIST_SECS of the listing's end");
    relist(&mut f, 2.6, slow);
    assert_eq!(listings.get(), 2, "RELIST_SECS after the listing's end");
}

/// A folder that can't be listed shows why under it; the error goes once it lists again.
#[test]
fn a_folder_that_cannot_be_read_says_why() {
    let missing = std::env::temp_dir().join(format!("photocraft-photomerge-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&missing);
    let (mut h, _) = harness(vec![]);
    let id = open_photomerge(h.state_mut());
    let d = h.state_mut().ui.dialog_mut(id).unwrap();
    d.fields.insert("use".into(), json!("folder"));
    d.fields.insert("folder".into(), json!(missing.to_string_lossy()));
    h.run_steps(2);
    let error = dialog(&h).unwrap().1.get("__listError").and_then(Value::as_str).map(str::to_string).expect("the listing error");
    assert!(h.query_by_label(&error).is_some(), "{error} is shown");
    assert!(h.get_by_label("OK").accesskit_node().is_disabled());

    let (dir, _) = panorama_pngs("readable-again");
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("folder".into(), json!(dir));
    h.run_steps(2);
    assert!(!dialog(&h).unwrap().1.contains_key("__listError"));
    assert!(h.query_by_label(&error).is_none());
}
