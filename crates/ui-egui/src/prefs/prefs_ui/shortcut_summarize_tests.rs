use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};

type Requests = Rc<RefCell<Vec<FileDialogRequest>>>;
type Paths = Rc<RefCell<Vec<String>>>;
type Opened = Arc<Mutex<Vec<String>>>;

fn click(h: &mut Harness<'_, PhotocraftApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(2);
}

fn harness(answers: Vec<&str>, open_result: Option<Result<(), &'static str>>) -> (Harness<'static, PhotocraftApp>, Requests, Paths, Opened) {
    let written: Paths = Rc::default();
    let written_log = written.clone();
    let opened: Opened = Arc::default();
    let opened_log = opened.clone();
    let (picker, requests) = crate::file_dialog::fake(answers.into_iter().map(|a| Some(FileDialogAnswer::SaveTo(a.into()))).collect());

    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 760.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let services = crate::Services {
            file_dialog: Some(picker),
            write: Some(Box::new(move |path: &str, _| {
                written_log.borrow_mut().push(path.to_string());
                Ok(())
            })),
            open_url: open_result.map(|res| {
                let open_fn: crate::services::OpenUrlFn = Box::new(move |url: &str| {
                    opened_log.lock().unwrap().push(url.to_string());
                    res.map_err(str::to_owned)
                });
                open_fn
            }),
            ..Default::default()
        };
        PhotocraftApp::new(photocraft_engine::Session::new(), services)
    });
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.keyboardShortcuts", json!({})).unwrap();
    h.run_steps(4);
    (h, requests, written, opened)
}

#[test]
fn summarize_suggests_htm_and_opens_saved_file() {
    let (mut h, requests, written, opened) = harness(vec!["/tmp/Defaults.htm", "/tmp/Defaults.kys"], Some(Ok(())));
    let ctx = h.ctx.clone();
    click(&mut h, "Summarize…");
    h.state_mut().poll_file_dialog(&ctx, None);
    click(&mut h, "Export…");
    h.state_mut().poll_file_dialog(&ctx, None);

    let reqs = requests.borrow();
    assert!(matches!(&reqs[0], FileDialogRequest::Save { suggested } if suggested.ends_with(".htm")), "{reqs:?}");
    assert_eq!(written.borrow().as_slice(), ["/tmp/Defaults.htm", "/tmp/Defaults.kys"]);
    assert_eq!(opened.lock().unwrap().as_slice(), ["/tmp/Defaults.htm"]);
}

#[test]
fn summarize_open_failure_sets_status_and_keeps_saved_file() {
    let (mut h, _, written, opened) = harness(vec!["/tmp/Defaults.htm"], Some(Err("browser unavailable")));
    let ctx = h.ctx.clone();
    click(&mut h, "Summarize…");
    h.state_mut().poll_file_dialog(&ctx, None);

    assert_eq!(written.borrow().as_slice(), ["/tmp/Defaults.htm"]);
    assert_eq!(opened.lock().unwrap().as_slice(), ["/tmp/Defaults.htm"]);
    assert_eq!(h.state().ui.status, "browser unavailable");
}

#[test]
fn summarize_without_open_url_service_succeeds() {
    let (mut h, _, written, opened) = harness(vec!["/tmp/Defaults.htm"], None);
    let ctx = h.ctx.clone();
    click(&mut h, "Summarize…");
    h.state_mut().poll_file_dialog(&ctx, None);

    assert_eq!(written.borrow().as_slice(), ["/tmp/Defaults.htm"]);
    assert!(opened.lock().unwrap().is_empty());
}
