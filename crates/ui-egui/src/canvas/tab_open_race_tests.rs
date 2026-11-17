//! #15318: opening several large JPGs and switching tabs while they land.
//!
//! Each file gets a tab while it decodes; when one finishes, its document takes its
//! place in the set and the tabs behind it shift. egui credits a release to the widget
//! carrying the interaction id that was pressed. When that id is the tab *position*, a
//! completion that shifts tabs mid-click moves the id onto another document, and the
//! release activates (or closes) it: select tab A, see image B. Keying the tab's id on
//! the document's stable `DocId` - as the engine already keys open completions - keeps
//! the click with the document it was pressed on, however the tabs move.

use egui::vec2;
use egui::{Event, Modifiers, PointerButton, pos2};
use egui_kittest::Harness;
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{DocId, Document, Size};
use photocraft_engine::jobs::JobId;
use serde_json::json;

use crate::PhotocraftApp;
use crate::jobs_ui::{self, OpenOrder, OpenTab};
use crate::theme::ThemeKind;

fn jpg_doc(name: &str) -> Document {
    Document::new(name, Size::new(24, 16), ColorMode::Rgb, SampleType::U8)
}

/// A real app window with background opens, one tab strip theme.
fn race_harness(theme: ThemeKind) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(8).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = true;
        app.run("prefs.set", json!({"values": {"interface.theme": theme.id()}})).unwrap();
        app
    });
    h.run_steps(4);
    h
}

/// The document tabs the strip drew, left to right.
fn tab_rects(h: &Harness<'_, PhotocraftApp>) -> Vec<(usize, egui::Rect)> {
    h.state().tab_strip.clone().expect("the strip is drawn").tabs
}

fn doc_names(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    h.state().session.documents().iter().map(|st| st.doc.name.clone()).collect()
}

/// A background open finishes: the engine has already added its document to the session
/// (what `poll_jobs` apply does); run the shell's completion exactly as a job event does.
fn complete_open(h: &mut Harness<'_, PhotocraftApp>, name: &str) -> DocId {
    let i = h.state_mut().jobs.opens.iter().position(|o| o.name == name).expect("an opening tab with this name");
    let tab = h.state_mut().jobs.opens.remove(i);
    let idx = h.state_mut().session.add_document(jpg_doc(name), None);
    let id = h.state_mut().session.documents()[idx].doc.id;
    jobs_ui::finish_open_tab(h.state_mut(), &tab, &json!({"document": idx, "docId": id.0})).unwrap();
    id
}

fn press(h: &mut Harness<'_, PhotocraftApp>, at: egui::Pos2) {
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

fn release(h: &mut Harness<'_, PhotocraftApp>, at: egui::Pos2) {
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

/// #15318: three JPGs picked together in File > Open; the big first file is still decoding
/// when the two behind it finish. The user presses on tab "page_02.jpg", the slow file lands
/// in front (taking its place in the set, which shifts every tab), and the release finds a
/// different document under the pointer. egui credits the release to the widget carrying the
/// id that was pressed: with an id on the tab position, the click activates the shifted
/// document (select tab A, see image B). With the id on the document, the click lands on the
/// tab the user pressed even as it slid right.
#[test]
fn a_tab_click_racing_a_completion_activates_the_just_opened_not_a_shifted_tab() {
    for theme in [ThemeKind::Pro, ThemeKind::Studio] {
        let mut h = race_harness(theme);
        let batch = jobs_ui::begin_batch(h.state_mut());
        let tab = |seq: usize| OpenTab {
            job: JobId(900 + seq as u64),
            name: format!("page_0{seq}.jpg"),
            path: None,
            slot: None,
            order: Some(OpenOrder { batch, seq }),
        };
        h.state_mut().jobs.opens.extend((0..3).map(tab));
        // page_01 and page_02 decode first; page_00 (the big one) is still opening.
        for seq in [1usize, 2] {
            let i = h.state_mut().jobs.opens.iter().position(|o| o.order.is_some_and(|x| x.seq == seq)).unwrap();
            let t = h.state_mut().jobs.opens.remove(i);
            let idx = h.state_mut().session.add_document(jpg_doc(&t.name), None);
            let id = h.state_mut().session.documents()[idx].doc.id;
            jobs_ui::finish_open_tab(h.state_mut(), &t, &json!({"document": idx, "docId": id.0})).unwrap();
        }
        jobs_ui::end_batch(h.state_mut());
        assert_eq!(doc_names(&h), ["page_01.jpg", "page_02.jpg"], "page_00.jpg is still opening");
        h.state_mut().session.set_active(0);
        h.run_steps(1);
        // Press on the second tab, "page_02.jpg", as the big file's decode finishes.
        let pressed = h.state().session.documents()[1].doc.id;
        let at = tab_rects(&h)[1].1.center();
        press(&mut h, at);
        // page_00.jpg lands while the button is down: it takes its slot at the front of the
        // set and the opened tabs shift right, one tab width under the still-held pointer.
        let _just_opened = complete_open(&mut h, "page_00.jpg");
        assert_eq!(doc_names(&h), ["page_00.jpg", "page_01.jpg", "page_02.jpg"]);
        // The pointer never moved: the release lands where "page_02.jpg" was and "page_01.jpg"
        // sits there now. Crediting the old position activates the wrong image (#15318); the
        // click must belong to the document that was pressed.
        release(&mut h, at);
        assert_eq!(
            h.state().session.active().map(|st| st.doc.id),
            Some(pressed),
            "{theme:?}: the shift must not activate the document that slid under the pointer"
        );
    }
}

/// #15318, same race on the tab's x: the close must hit the document whose x was pressed,
/// never one that slid under the pointer while the button was down.
#[test]
fn a_tab_close_racing_a_completion_closes_no_wrong_document() {
    for theme in [ThemeKind::Pro, ThemeKind::Studio] {
        let mut h = race_harness(theme);
        let batch = jobs_ui::begin_batch(h.state_mut());
        let tab = |seq: usize| OpenTab {
            job: JobId(900 + seq as u64),
            name: format!("page_0{seq}.jpg"),
            path: None,
            slot: None,
            order: Some(OpenOrder { batch, seq }),
        };
        h.state_mut().jobs.opens.extend((0..4).map(tab));
        for seq in [1usize, 2, 3] {
            let i = h.state_mut().jobs.opens.iter().position(|o| o.order.is_some_and(|x| x.seq == seq)).unwrap();
            let t = h.state_mut().jobs.opens.remove(i);
            let idx = h.state_mut().session.add_document(jpg_doc(&t.name), None);
            let id = h.state_mut().session.documents()[idx].doc.id;
            jobs_ui::finish_open_tab(h.state_mut(), &t, &json!({"document": idx, "docId": id.0})).unwrap();
        }
        jobs_ui::end_batch(h.state_mut());
        h.run_steps(1);
        assert_eq!(doc_names(&h), ["page_01.jpg", "page_02.jpg", "page_03.jpg"]);
        let pro = crate::theme::Tokens::get(&h.ctx).pro;
        let r = tab_rects(&h)[2].1;
        let at = pos2(r.right() - if pro { 13.0 } else { 12.0 }, r.center().y);
        press(&mut h, at);
        let _slow = complete_open(&mut h, "page_00.jpg");
        release(&mut h, at);
        // Under the index-keyed id the release lands on page_02.jpg's x and closes it; with the
        // id on the document the close belongs to page_03.jpg, whose x was pressed (#15318).
        assert_eq!(
            doc_names(&h),
            ["page_00.jpg", "page_01.jpg", "page_02.jpg"],
            "{theme:?}: the x must close the document it was pressed on, not one shifted under the pointer"
        );
    }
}

/// Neighbor case, guard (passes before and after): a completion that only appends behind the
/// pressed tab shifts nothing, and the click must land on its document as usual.
#[test]
fn a_tab_click_still_lands_when_a_completion_appends_behind_it() {
    let mut h = race_harness(ThemeKind::Studio);
    h.state_mut().session.add_document(jpg_doc("page_01.jpg"), None);
    h.state_mut().session.add_document(jpg_doc("page_02.jpg"), None);
    h.state_mut().session.set_active(0);
    h.state_mut().jobs.opens.push(OpenTab { job: JobId(900), name: "page_00.jpg".into(), path: None, slot: None, order: None });
    h.run_steps(1);
    let opened = h.state().session.documents().iter().find(|st| st.doc.name == "page_02.jpg").unwrap().doc.id;
    let at = tab_rects(&h)[1].1.center();
    press(&mut h, at);
    let _appended = complete_open(&mut h, "page_00.jpg");
    release(&mut h, at);
    assert_eq!(h.state().session.active().map(|st| st.doc.id), Some(opened), "the click lands: the pointer never crossed a shift");
}

/// Neighbor case: a tab close and a completion both land mid-click; when the same document
/// ends up under the pointer, the click still activates the document it was pressed on.
#[test]
fn a_tab_click_racing_a_close_and_a_completion_lands_on_its_document() {
    let mut h = race_harness(ThemeKind::Studio);
    let batch = jobs_ui::begin_batch(h.state_mut());
    let tab =
        |seq: usize| OpenTab { job: JobId(900 + seq as u64), name: format!("page_0{seq}.jpg"), path: None, slot: None, order: Some(OpenOrder { batch, seq }) };
    h.state_mut().jobs.opens.extend((0..3).map(tab));
    for seq in [1usize, 2] {
        let i = h.state_mut().jobs.opens.iter().position(|o| o.order.is_some_and(|x| x.seq == seq)).unwrap();
        let t = h.state_mut().jobs.opens.remove(i);
        let idx = h.state_mut().session.add_document(jpg_doc(&t.name), None);
        let id = h.state_mut().session.documents()[idx].doc.id;
        jobs_ui::finish_open_tab(h.state_mut(), &t, &json!({"document": idx, "docId": id.0})).unwrap();
    }
    jobs_ui::end_batch(h.state_mut());
    h.state_mut().session.set_active(0);
    h.run_steps(1);
    let pressed = h.state().session.documents()[1].doc.id;
    let at = tab_rects(&h)[1].1.center();
    press(&mut h, at);
    // The first tab closes (its x, same frame of the race), and page_00.jpg lands in front:
    // page_02.jpg stays under the held pointer, at its own tab position.
    let closed = h.state().session.documents()[0].doc.id;
    let ctx = h.ctx.clone();
    let _ = crate::menus::invoke(h.state_mut(), &ctx, "file.close", json!({"document": 0}));
    let _slow = complete_open(&mut h, "page_00.jpg");
    release(&mut h, at);
    assert_eq!(h.state().session.active().map(|st| st.doc.id), Some(pressed), "the click still lands on the document it was pressed on");
    assert!(!h.state().session.documents().iter().any(|st| st.doc.id == closed), "the closed tab stayed closed");
}
