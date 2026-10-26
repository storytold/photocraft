use egui::{Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;
use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};

/// PNGs named `names`, `w`×`h` each, in a fresh folder for `tag`: (folder, paths).
fn pngs(tag: &str, names: &[(&str, u32, u32)]) -> (String, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("photocraft-pdf-presentation-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let paths = names
        .iter()
        .map(|(n, w, h)| {
            let mut s = photocraft_engine::Session::new();
            s.execute("file.new", json!({"width": w, "height": h, "background": "#406080"})).unwrap();
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

fn open_dialog(h: &mut Harness<'static, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    h.run_steps(4);
}

fn dialog(h: &Harness<'_, PhotocraftApp>) -> Option<(u64, Map<String, Value>)> {
    h.state().ui.dialogs.iter().find(|d| owns(&d.fields)).map(|d| (d.id, d.fields.clone()))
}

fn listed(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    dialog(h).map(|(_, f)| files(&f).iter().map(|p| file_name(p).to_string()).collect()).unwrap_or_default()
}

fn select(h: &mut Harness<'static, PhotocraftApp>, name: &str) {
    h.get_by_label(name).click();
    h.run_steps(2);
}

/// Each page's MediaBox `[0 0 w h]`, in page order.
fn media_boxes(pdf: &[u8]) -> Vec<(f64, f64)> {
    let text = String::from_utf8_lossy(pdf).into_owned();
    text.match_indices("/MediaBox [0 0 ")
        .map(|(i, m)| {
            let rest = &text[i + m.len()..];
            let v: Vec<f64> = rest[..rest.find(']').unwrap()].split_whitespace().map(|n| n.parse().unwrap()).collect();
            (v[0], v[1])
        })
        .collect()
}

#[test]
fn menu_opens_source_files_and_output_options() {
    let (mut h, _) = harness(vec![]);
    open_dialog(&mut h);
    for label in [
        "Source Files",
        "Add Open Files",
        "Browse…",
        "Duplicate",
        "Remove",
        "Sort By Name",
        "Output Options",
        "Multi-Page Document",
        "Presentation",
        "Presentation Options",
        "Advance Every",
        "Seconds",
        "Loop after Last Page",
    ] {
        assert!(h.query_by_label(label).is_some(), "PDF Presentation is missing {label}");
    }
    // A label and its dropdown, which is named after it.
    for label in ["Background", "Transition"] {
        assert!(h.query_all_by_label(label).count() >= 2, "PDF Presentation is missing {label}");
    }
    assert!(h.get_by_label("Save…").accesskit_node().is_disabled(), "add files before saving");
    for label in ["Advance Every", "Loop after Last Page"] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled(), "{label} waits for Save As Presentation");
    }
    h.get_by_label("Presentation").click();
    h.run_steps(2);
    for label in ["Advance Every", "Loop after Last Page"] {
        assert!(!h.get_by_label(label).accesskit_node().is_disabled(), "{label} applies to a presentation");
    }
    assert!(h.get_by_label("Add Open Files").accesskit_node().is_disabled(), "no open files to add");
    assert!(h.get_by_label("Duplicate").accesskit_node().is_disabled(), "nothing selected");
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(dialog(&h).is_none());
}

#[test]
fn browse_duplicate_remove_sort_then_save_writes_the_pages_in_list_order() {
    let (dir, paths) = pngs("save", &[("b-wide.png", 30, 20), ("a-tall.png", 10, 40), ("c-square.png", 16, 16)]);
    let out = format!("{dir}/deck.pdf");
    let (mut h, asked) = harness(vec![Some(FileDialogAnswer::Paths(paths)), Some(FileDialogAnswer::SaveTo(out.clone()))]);
    open_dialog(&mut h);
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    assert_eq!(listed(&h), ["b-wide.png", "a-tall.png", "c-square.png"]);
    let (b, a) = (h.get_by_label("b-wide.png").rect(), h.get_by_label("a-tall.png").rect());
    assert!(b.bottom() <= a.top(), "rows are stacked: {b:?} {a:?}");
    select(&mut h, "a-tall.png");
    h.get_by_label("Duplicate").click();
    h.run_steps(2);
    assert_eq!(listed(&h), ["b-wide.png", "a-tall.png", "a-tall.png", "c-square.png"], "the copy goes right after the original");
    select(&mut h, "c-square.png");
    h.get_by_label("Remove").click();
    h.run_steps(2);
    h.get_by_label("Sort By Name").click();
    h.run_steps(2);
    assert_eq!(listed(&h), ["a-tall.png", "a-tall.png", "b-wide.png"]);
    h.get_by_value("White").click();
    h.run_steps(2);
    h.get_by_label("Black").click();
    h.run_steps(2);
    assert_eq!(dialog(&h).unwrap().1["background"], "black");
    h.get_by_label("Save…").click();
    h.run_steps(4);
    assert!(dialog(&h).is_none());
    assert!(matches!(asked.borrow().last(), Some(FileDialogRequest::Save { suggested }) if suggested.ends_with(".pdf")), "{:?}", asked.borrow());
    let pdf = std::fs::read(&out).unwrap();
    let shape: Vec<bool> = media_boxes(&pdf).iter().map(|(w, h)| h > w).collect();
    assert_eq!(shape, [true, true, false], "tall, tall, wide: the list's order");
    assert!(h.state().session.documents().is_empty(), "nothing is left open");
}

#[test]
fn dragging_a_row_reorders_the_pages() {
    let (_, paths) = pngs("drag", &[("one.png", 8, 8), ("two.png", 8, 8), ("three.png", 8, 8)]);
    let (mut h, _) = harness(vec![Some(FileDialogAnswer::Paths(paths))]);
    open_dialog(&mut h);
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    let from = h.get_by_label("one.png").rect().center();
    let below = h.get_by_label("three.png").rect();
    let to = Pos2::new(below.center().x, below.bottom() - 2.0);
    h.hover_at(from);
    h.run_steps(1);
    h.drag_at(from);
    h.run_steps(1);
    for i in 1..=6 {
        h.hover_at(from + (to - from) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(3);
    assert_eq!(listed(&h), ["two.png", "three.png", "one.png"], "dropped below the last row");
    assert_eq!(dialog(&h).unwrap().1["selected"], 2, "the moved row stays selected");
}

#[test]
fn add_open_files_adds_and_removes_the_open_documents() {
    let (_, paths) = pngs("open", &[("open.png", 8, 8), ("disk.png", 8, 8)]);
    let (mut h, _) = harness(vec![Some(FileDialogAnswer::Paths(vec![paths[1].clone()]))]);
    // An open document with a file (open.png), and an untitled one Add Open Files leaves out.
    h.state_mut().run("file.new", json!({"width": 4, "height": 4})).unwrap();
    h.state_mut().session.active_mut().unwrap().path = Some(paths[0].clone());
    h.state_mut().run("file.new", json!({"width": 4, "height": 4})).unwrap();
    h.run_steps(2);
    open_dialog(&mut h);
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    let warning = |h: &Harness<'_, PhotocraftApp>| h.query_by_label_contains("Uses the saved file").is_some();
    assert!(!warning(&h), "disk.png isn't open");
    h.get_by_label("Add Open Files").click();
    h.run_steps(2);
    assert_eq!(listed(&h), ["disk.png", "open.png"]);
    let dirty = h.state().session.documents()[0].is_dirty();
    assert_eq!(warning(&h), dirty, "warned only while open.png has unsaved changes");
    // An edit to open.png that isn't saved: the PDF would show the file as last saved.
    h.state_mut().session.set_active(0);
    h.state_mut().run("image.adjustments.invert", json!({})).unwrap();
    h.run_steps(2);
    assert!(h.query_by_label_contains("save open.png first").is_some(), "unsaved changes are pointed out, by file");
    assert!(h.query_by_label_contains("disk.png first").is_none());
    h.get_by_label("Add Open Files").click();
    h.run_steps(2);
    assert_eq!(listed(&h), ["disk.png"], "unchecking takes the open files back out");
    assert!(!warning(&h));
}

#[test]
fn drop_index_accounts_for_the_row_leaving_its_place() {
    assert_eq!(drop_index(0, 2, true), 2);
    assert_eq!(drop_index(0, 2, false), 1);
    assert_eq!(drop_index(2, 0, false), 0);
    assert_eq!(drop_index(2, 0, true), 1);
}

#[test]
fn save_as_presentation_passes_its_options_to_the_command() {
    let (dir, paths) = pngs("show", &[("one.png", 8, 8), ("two.png", 8, 8)]);
    let out = format!("{dir}/show.pdf");
    let (mut h, _) = harness(vec![Some(FileDialogAnswer::Paths(paths)), Some(FileDialogAnswer::SaveTo(out.clone()))]);
    open_dialog(&mut h);
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    h.get_by_label("Presentation").click();
    h.run_steps(2);
    h.get_by_label("Loop after Last Page").click();
    h.run_steps(2);
    h.get_by_value("None").click();
    h.run_steps(2);
    for t in photocraft_io::pdf::Transition::ALL {
        assert!(h.query_by_label(t.label()).is_some(), "Transition list is missing {}", t.label());
    }
    h.get_by_label("Box Out").click();
    h.run_steps(2);
    h.get_by_label("Save…").click();
    h.run_steps(4);
    let text = String::from_utf8_lossy(&std::fs::read(&out).unwrap()).into_owned();
    assert!(text.contains("/PageMode /FullScreen"));
    assert_eq!(text.matches("/Dur 5").count(), 2, "Advance Every 5 Seconds is on by default");
    assert_eq!(text.matches("/S /Box /M /O").count(), 2);
    assert!(text.contains("app.fs.loop = true;"));
}

#[test]
fn turning_advance_off_leaves_pages_until_the_viewer_moves_on() {
    let (dir, paths) = pngs("manual", &[("one.png", 8, 8)]);
    let out = format!("{dir}/manual.pdf");
    let (mut h, _) = harness(vec![Some(FileDialogAnswer::Paths(paths)), Some(FileDialogAnswer::SaveTo(out.clone()))]);
    open_dialog(&mut h);
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    h.get_by_label("Presentation").click();
    h.run_steps(2);
    h.get_by_label("Advance Every").click();
    h.run_steps(2);
    h.get_by_label("Save…").click();
    h.run_steps(4);
    let text = String::from_utf8_lossy(&std::fs::read(&out).unwrap()).into_owned();
    assert!(text.contains("/PageMode /FullScreen") && !text.contains("/Dur") && !text.contains("JavaScript"));
}
