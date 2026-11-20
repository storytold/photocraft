//! Files opened together whose embedded profile doesn't match the working space (File › Open with
//! several selected, Ask When Opening on). Checked in Photoshop 27.11: one Embedded Profile
//! Mismatch prompt at a time, naming its file, before that file opens; OK opens it and asks about
//! the next; Cancel opens neither it nor the files still to come.

use crate::{PhotocraftApp, Services};
use craft::color_ui::ui::ProfileMismatchDialog;
use craft::ui::dialog::DialogId;
use egui::vec2;
use egui_kittest::Harness;
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use photocraft_engine::Session;
use photocraft_engine::color_cmds::ColorExt;
use serde_json::json;

/// What the prompt's `action` field calls each choice.
const USE_EMBEDDED: &str = "useEmbedded";
const CONVERT: &str = "convertToWorking";
const DISCARD: &str = "discard";

/// A PNG tagged with Adobe RGB, which doesn't match the sRGB working space.
fn adobe_png() -> Vec<u8> {
    let mut d = Document::with_background("t", Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::rgba(0.2, 0.6, 0.9, 1.0));
    d.icc_profile = Some(craft::cms::Builtin::from_id("adobe-rgb-compat").unwrap().profile().to_bytes());
    photocraft_io::export(&d, "t.png", &Default::default()).unwrap().bytes
}

fn harness(background: bool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::with_context(
            cc.egui_ctx.clone(),
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
    h.state_mut().run("edit.colorSettings", serde_json::json!({"askOnMismatch": true, "policyRgb": "preserve"})).unwrap();
    h
}

/// Open a.png, b.png and c.png together and let every read finish.
fn open_three(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    let png = adobe_png();
    for name in ["a.png", "b.png", "c.png"] {
        h.state_mut().open_file(&format!("/pics/{name}"), &png).unwrap();
    }
    settle(h);
}

/// Run frames until no open is still reading.
fn settle(h: &mut Harness<'static, Box<PhotocraftApp>>) {
    for _ in 0..200 {
        h.run_steps(2);
        if h.state().jobs.opens.is_empty() && h.state().session.jobs().is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.run_steps(6);
}

fn docs(h: &Harness<'static, Box<PhotocraftApp>>) -> Vec<String> {
    h.state().session.documents().iter().map(|d| d.doc.name.clone()).collect()
}

/// The one prompt up: its host id.
fn prompt(h: &Harness<'static, Box<PhotocraftApp>>) -> DialogId {
    let host = &h.state().framework.dialogs;
    assert!(host.is_open::<ProfileMismatchDialog>(), "a prompt is up");
    let open = host.open_dialogs();
    assert_eq!(open.len(), 1, "one prompt at a time");
    open[0].id
}

/// Whether no dialog is open on the host.
fn no_prompt(h: &Harness<'static, Box<PhotocraftApp>>) -> bool {
    h.state().framework.dialogs.is_empty()
}

/// Sets the prompt's `action` and, when `remember`, ticks "Remember this choice"; then presses OK.
fn answer_with(h: &mut Harness<'static, Box<PhotocraftApp>>, action: &str, remember: bool) {
    let (ctx, id) = (h.ctx.clone(), prompt(h));
    let host = &mut h.state_mut().framework.dialogs;
    host.set_json_field(&ctx, id, "action", json!(action)).unwrap();
    if remember {
        host.set_json_field(&ctx, id, "remember", json!(true)).unwrap();
    }
    crate::dialogs::confirm(h.state_mut(), &ctx, id).expect("OK answers the prompt");
    settle(h);
}

fn answer(h: &mut Harness<'static, Box<PhotocraftApp>>, action: &str) {
    answer_with(h, action, false);
}

fn profile(h: &Harness<'static, Box<PhotocraftApp>>, name: &str) -> Option<String> {
    let st = h.state().session.documents().iter().find(|d| d.doc.name == name).unwrap();
    st.doc.icc_profile.as_ref().map(|b| photocraft_engine::color_cmds::profile_from_bytes(b).unwrap().description.clone())
}

/// The file the prompt on screen asks about: the front of the held files.
fn asked(h: &Harness<'static, Box<PhotocraftApp>>) -> &'static str {
    prompt(h);
    let name = &h.state().held_opens.front().expect("a file is held").name;
    ["a.png", "b.png", "c.png", "d.png"].into_iter().find(|n| n == name).unwrap_or_else(|| panic!("an unexpected file is held: {name}"))
}

/// Files read on the spot ask in the order they were given. Read in the background, they ask in
/// the order their reads finish, which is the same order unless a later read overtakes an earlier
/// one: the tests hold either way.
fn in_order(background: bool, order: &[&str]) {
    if !background {
        assert_eq!(order, &["a.png", "b.png", "c.png"][..order.len()]);
    }
}

#[test]
fn files_opened_together_ask_one_at_a_time_before_each_opens() {
    for background in [false, true] {
        let mut h = harness(background);
        open_three(&mut h);
        let mut answered: Vec<&str> = Vec::new();
        for _ in 0..3 {
            assert_eq!(docs(&h), answered, "background {background}: a file opens only after its prompt");
            let name = asked(&h);
            assert!(!answered.contains(&name), "background {background}: {name} asked twice");
            answer(&mut h, USE_EMBEDDED);
            answered.push(name);
        }
        in_order(background, &answered);
        assert!(no_prompt(&h));
        assert_eq!(docs(&h), answered, "background {background}: each opened when it was answered");
        assert_eq!(h.state().session.active().map(|d| d.doc.name.as_str()), answered.last().copied());
    }
}

/// #4559 with a held file: type in a font that isn't installed is reported with the import
/// warnings while the file still waits for its profile prompt, and shown once when it opens.
#[test]
fn a_held_file_still_reports_its_missing_fonts() {
    const MISSING: &str = "NoSuchFamily3471";
    let warning = format!("font '{MISSING}' is not installed; text may render using a fallback face");
    let mut s = Session::new();
    s.execute("file.new", serde_json::json!({"width": 40, "height": 20})).unwrap();
    s.execute("type.create", serde_json::json!({"text": "Hi", "x": 2, "y": 12, "size": 10, "font": MISSING})).unwrap();
    let mut doc = (*s.active().unwrap().doc).clone();
    doc.icc_profile = Some(craft::cms::Builtin::from_id("adobe-rgb-compat").unwrap().profile().to_bytes());
    let services =
        Services { import: Some(Box::new(move |_: &str, _: &[u8], _: &photocraft_io::ImportOptions| Ok((doc.clone(), Vec::new())))), ..Default::default() };
    let mut app = PhotocraftApp::new(Session::new(), services);
    app.run("edit.colorSettings", serde_json::json!({"askOnMismatch": true, "policyRgb": "preserve"})).unwrap();
    assert_eq!(app.open_bytes("poster.psd", b"x").unwrap(), vec![warning.clone()]);
    assert!(app.session.documents().is_empty() && app.ui.notices.is_empty(), "held: nothing shown yet");
    crate::open_prompt::answer(&mut app, Some("preserve")).unwrap();
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.ui.notices.len(), 1);
    assert_eq!(app.ui.notices[0].lines, vec![warning]);
}

#[test]
fn each_answer_applies_to_its_own_file() {
    for background in [false, true] {
        let mut h = harness(background);
        open_three(&mut h);
        // Whichever file a prompt names gets that file's own answer.
        let mut answered: Vec<&str> = Vec::new();
        for _ in 0..3 {
            let name = asked(&h);
            answer(
                &mut h,
                match name {
                    "a.png" => CONVERT,
                    "b.png" => DISCARD,
                    _ => USE_EMBEDDED,
                },
            );
            answered.push(name);
        }
        in_order(background, &answered);
        assert_eq!(docs(&h), answered);
        assert_eq!(profile(&h, "a.png").as_deref(), Some("sRGB IEC61966-2.1"), "background {background}: a converted");
        assert_eq!(profile(&h, "b.png"), None, "background {background}: b discarded");
        assert!(profile(&h, "c.png").unwrap().starts_with("Adobe RGB"), "background {background}: c kept");
    }
}

#[test]
fn cancel_opens_neither_the_file_nor_the_ones_still_to_come() {
    for background in [false, true] {
        let mut h = harness(background);
        open_three(&mut h);
        let first = asked(&h);
        in_order(background, &[first]);
        answer(&mut h, USE_EMBEDDED);
        let (ctx, id) = (h.ctx.clone(), prompt(&h));
        assert!(h.state_mut().framework.dialogs.cancel_by_id(&ctx, id), "Cancel closes the second prompt");
        settle(&mut h);
        assert!(no_prompt(&h), "background {background}: no prompt left");
        assert_eq!(docs(&h), [first], "background {background}");
        // A later open asks again.
        h.state_mut().open_file("/pics/d.png", &adobe_png()).unwrap();
        settle(&mut h);
        assert_eq!(asked(&h), "d.png");
    }
}

/// "Remember this choice": one answer opens the files still waiting with it, saves it as the
/// Color Settings policy and stops asking, so the next mismatched file opens without a prompt.
#[test]
fn remember_answers_the_waiting_files_and_stops_asking() {
    for background in [false, true] {
        let mut h = harness(background);
        open_three(&mut h);
        answer_with(&mut h, CONVERT, true);
        assert!(no_prompt(&h), "background {background}: no prompt is left");
        let mut opened = docs(&h);
        opened.sort();
        assert_eq!(opened, ["a.png", "b.png", "c.png"], "background {background}");
        for name in ["a.png", "b.png", "c.png"] {
            assert_eq!(profile(&h, name).as_deref(), Some("sRGB IEC61966-2.1"), "background {background}: {name} converted");
        }
        let settings = &h.state().session.color().settings;
        assert_eq!(settings.policy(photocraft_doc::ColorMode::Rgb), photocraft_engine::color_cmds::Policy::Convert);
        assert!(!settings.ask_on_mismatch, "background {background}");
        h.state_mut().open_file("/pics/d.png", &adobe_png()).unwrap();
        settle(&mut h);
        assert!(no_prompt(&h), "background {background}: the next file doesn't ask");
        assert_eq!(profile(&h, "d.png").as_deref(), Some("sRGB IEC61966-2.1"), "background {background}");
    }
}

/// Without Remember, OK answers only the file asked about.
#[test]
fn without_remember_the_next_file_still_asks() {
    let mut h = harness(false);
    open_three(&mut h);
    answer(&mut h, CONVERT);
    assert!(h.state().framework.dialogs.is_open::<ProfileMismatchDialog>(), "the next file asks");
    assert!(h.state().session.color().settings.ask_on_mismatch);
}
