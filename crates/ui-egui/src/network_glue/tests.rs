use super::*;
use craft_print::wsd::adapter::{DeviceUrl, Request as WireRequest, Response, TransportKind, WireError};
use craft_print::wsd::testing::{Ending, FakeConfig as Config, FakePrinter, MemTransport};
use craft_ui::printing::network::Outcome;
use net::{JobView, Query};
use photocraft_print::network::{ErrorKind, Phase, RetrySafety};
use serde_json::json;
use std::time::Duration;

const ADDRESS: &str = "127.0.0.1:5357/printer";

fn tasks_with(transport: Transport) -> NetTasks {
    let mut tasks = NetTasks::default();
    tasks.set_transport(transport);
    tasks.state.draft = net::Draft::for_address(ADDRESS);
    tasks
}

fn app(config: Config) -> (NetTasks, Arc<FakePrinter>) {
    let printer = FakePrinter::new(config);
    (tasks_with(Arc::new(MemTransport(printer.clone()))), printer)
}

fn wait(app: &mut NetTasks, done: impl Fn(&NetTasks) -> bool) {
    let ctx = egui::Context::default();
    for _ in 0..400 {
        app.poll(&ctx);
        if done(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting");
}

fn state(app: &NetTasks) -> &net::State {
    &app.state
}

/// Ask the printer, as the Query button does, and wait for the answer.
fn query(app: &mut NetTasks) {
    let address_ok = validate(&app.state.draft.normalized_url()).is_ok();
    let worker = query_printer(app.transport(), None, Arc::clone(&app.stations));
    let started = app.state.start_query(&worker, address_ok);
    assert_eq!(started, address_ok);
    wait(app, |a| !state(a).draft.querying());
}

/// A real print of a small RGB document on A4, laid out by the engine as the dialog does.
fn print() -> NetworkPrint {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 60, "height": 40, "name": "photo"})).unwrap();
    let doc = session.active().unwrap().doc.clone();
    photocraft_engine::print_cmds::prepare_network_print(&doc, &json!({"paper": "a4", "scaleToFit": true})).unwrap()
}

fn send(app: &mut NetTasks) -> bool {
    app.send(print())
}

#[test]
fn the_policy_refuses_what_it_should_before_anything_is_sent() {
    for bad in ["printer.local", "http://8.8.8.8/", "https://10.0.0.1/", "http://user@10.0.0.1/", "http://0.0.0.0/"] {
        assert_eq!(validate(&net::normalize_url(bad)).map_err(|f| f.kind), Err(FailureKind::Policy), "{bad}");
    }
    assert!(validate(&net::normalize_url("192.168.1.50:5357/service")).is_ok());
    assert!(validate("http://[::1]:5357/x").is_ok());
}

#[test]
fn errors_map_onto_the_failures_the_dialog_words_and_keep_whether_the_outcome_is_unknown() {
    for (kind, want) in [
        (ErrorKind::InvalidRequest, FailureKind::Invalid),
        (ErrorKind::UnsupportedFormat, FailureKind::NoCommonFormat),
        (ErrorKind::Unsupported, FailureKind::Unsupported),
        (ErrorKind::Unavailable, FailureKind::Transport),
        (ErrorKind::Timeout, FailureKind::Transport),
        (ErrorKind::Failed, FailureKind::Protocol),
        (ErrorKind::Rejected, FailureKind::Fault),
        (ErrorKind::Cancelled, FailureKind::Stopped),
    ] {
        assert_eq!(failure(&PrintError::new(kind, "x"), false).kind, want, "{kind:?}");
    }
    assert_eq!(failure(&PrintError::new(ErrorKind::InvalidRequest, "x"), true).kind, FailureKind::Policy);
    // A document that could not be made into a format the printer takes is "couldn't be prepared".
    assert_eq!(failure(&PrintError::new(ErrorKind::Failed, "x").with_phase(Phase::Prepare), false).kind, FailureKind::Encode);
    assert!(failure(&PrintError::new(ErrorKind::Timeout, "x").with_safety(RetrySafety::OutcomeUnknown), false).outcome_unknown);
    assert!(!failure(&PrintError::new(ErrorKind::Busy, "x").with_safety(RetrySafety::Safe), false).outcome_unknown);
}

#[test]
fn a_query_fills_in_the_printer_and_what_it_lists() {
    let (mut app, printer) = app(Config::default());
    query(&mut app);
    let d = &state(&app).draft;
    assert!(matches!(d.query, Query::Done(_)), "{:?}", d.query);
    assert_eq!(d.offered_sides().len(), 2);
    assert_eq!(d.max_copies(), 10);
    // Reading is all that happened: no job exists.
    assert_eq!(printer.jobs(), 0);
}

#[test]
fn a_refused_address_is_never_contacted_and_a_failed_query_is_shown() {
    let (mut app, printer) = app(Config { http_error: Some(503), ..Default::default() });
    app.state.draft.url = "8.8.8.8".into();
    query(&mut app);
    assert!(!state(&app).draft.querying() && printer.actions().is_empty());
    app.state.draft.url = "127.0.0.1:1/x".into();
    query(&mut app);
    assert!(matches!(&state(&app).draft.query, Query::Failed(f) if f.kind == FailureKind::Transport), "{:?}", state(&app).draft.query);
}

#[test]
fn nothing_is_sent_without_the_confirmation_and_a_query() {
    let (mut app, printer) = app(Config::default());
    assert!(!send(&mut app), "not queried");
    query(&mut app);
    assert!(!send(&mut app), "not confirmed");
    assert_eq!(printer.jobs(), 0);
}

#[test]
fn a_confirmed_job_runs_to_the_end_and_spends_the_confirmation() {
    let (mut app, printer) = app(Config::default());
    query(&mut app);
    app.state.draft.confirmed = true;
    app.state.draft.copies = 2;
    assert!(send(&mut app));
    assert!(!state(&app).draft.confirmed && state(&app).draft.running());
    wait(&mut app, |a| !state(a).draft.running());
    match &state(&app).draft.job {
        JobView::Finished(r) => assert_eq!(r.outcome, Outcome::Completed),
        other => panic!("{other:?}"),
    }
    assert_eq!(printer.jobs(), 1);
    // The next print needs a new yes.
    assert!(!send(&mut app));
    assert_eq!(printer.jobs(), 1);
}

#[test]
fn the_job_is_accepted_first_and_only_then_followed_to_its_end() {
    let (mut app, printer) = app(Config { ending: Ending::Hangs, ..Default::default() });
    query(&mut app);
    app.state.draft.confirmed = true;
    assert!(send(&mut app));
    // The printer has the document and the job is still going: accepted, not finished.
    wait(&mut app, |a| matches!(state(a).draft.job, JobView::Accepted { .. }));
    assert_eq!(printer.jobs(), 1);
    assert!(state(&app).draft.running());
    // It carries on at the printer; the dialog can cancel it, once.
    assert!(app.state.cancel_job());
    wait(&mut app, |a| !state(a).draft.running());
    assert!(matches!(&state(&app).draft.job, JobView::Finished(r) if r.outcome == Outcome::Canceled), "{:?}", state(&app).draft.job);
    assert_eq!(printer.actions().iter().filter(|a| *a == "SendDocument").count(), 1);
    assert_eq!(printer.actions().iter().filter(|a| *a == "CancelJob").count(), 1);
}

#[test]
fn the_documents_page_count_gives_the_progress_a_total_the_printer_does_not_report() {
    let (mut app, _printer) = app(Config { ending: Ending::Hangs, ..Default::default() });
    query(&mut app);
    let draft = &mut app.state.draft;
    draft.confirmed = true;
    draft.copies = 2;
    assert!(send(&mut app));
    // One sheet, two copies: the app supplied the page count, craft-ui the estimate; WS-Print reports no total.
    assert_eq!(state(&app).draft.expected_pages, Some(1));
    wait(&mut app, |a| matches!(&state(a).draft.job, JobView::Accepted { progress, .. } if progress.sheets_total.is_some()));
    match &state(&app).draft.job {
        JobView::Accepted { progress, .. } => assert_eq!(progress.sheets_total, Some(2), "{progress:?}"),
        other => panic!("{other:?}"),
    }
    assert!(app.state.cancel_job());
    wait(&mut app, |a| !state(a).draft.running());
}

#[test]
fn a_job_the_printer_aborts_is_reported_as_such() {
    let (mut app, _printer) = app(Config { ending: Ending::Aborts("DocumentFormatError"), ..Default::default() });
    query(&mut app);
    app.state.draft.confirmed = true;
    assert!(send(&mut app));
    wait(&mut app, |a| !state(a).draft.running());
    assert!(matches!(&state(&app).draft.job, JobView::Finished(r) if r.outcome == Outcome::Aborted), "{:?}", state(&app).draft.job);
}

/// Delivers every request to the fake and loses the reply to `SendDocument`: the printer did the
/// work and the client saw the connection close with no answer.
struct LoseSendReply(Arc<FakePrinter>);

impl craft_print::wsd::adapter::Transport for LoseSendReply {
    fn post(&self, _: &DeviceUrl, request: WireRequest) -> Result<Response, WireError> {
        let (status, content_type, body) = self.0.handle(&request.content_type, &request.body);
        if request.soap_action.ends_with("SendDocument") {
            return Err(WireError::transport(TransportKind::ConnectionClosed, "closed without a reply"));
        }
        Ok(Response::new(status, Some(content_type), body))
    }
}

#[test]
fn a_send_whose_outcome_is_unknown_asks_to_check_the_printer_and_is_never_repeated() {
    let printer = FakePrinter::new(Config::default());
    let mut app = tasks_with(Arc::new(LoseSendReply(printer.clone())));
    query(&mut app);
    app.state.draft.confirmed = true;
    assert!(send(&mut app));
    wait(&mut app, |a| !state(a).draft.running());
    match &state(&app).draft.job {
        JobView::Failed(f) => assert!(f.outcome_unknown, "{f:?}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(printer.actions().iter().filter(|a| *a == "SendDocument").count(), 1, "{:?}", printer.actions());
    assert!(!printer.actions().iter().any(|a| a == "CancelJob"), "a job that may be printing is left alone");
    // The confirmation was spent: sending again needs a new yes.
    assert!(!send(&mut app));
}

#[test]
fn options_carry_only_what_was_requested() {
    let r = Request {
        copies: Some(2),
        sides: Some(net::Sides::LongEdge),
        orientation: Some(net::Orientation::Landscape),
        quality: Some(net::Quality::High),
        color: Some(net::ColorMode::Monochrome),
        resolution: Some(net::Resolution::new(600, 300)),
        media_size: Some("iso_a4_210x297mm".into()),
        expected_pages: Some(5),
    };
    let (job, o) = options(&r, "a\u{7}b.pdf");
    assert_eq!((job.copies, job.title.as_str(), job.duplex), (2, "ab.pdf", Some(Duplex::LongEdge)));
    assert_eq!(o.orientation, Some(Orientation::Landscape));
    assert_eq!(o.quality, Some(Quality::High));
    assert_eq!((o.colour, o.resolution), (craft_print::ColourMode::Mono, Some(craft_print::Resolution::new(600, 300))));
    assert_eq!((o.media.as_deref(), o.expected_pages), (Some("iso_a4_210x297mm"), Some(5)));
    let (job, o) = options(&Request::default(), "");
    assert_eq!((job.title, job.copies, job.duplex), (Job::default().title, 1, None));
    assert_eq!(o, JobOptions::new());
    // Photo is craft-print's own quality now: it goes to the registry like the others.
    let (_, o) = options(&Request { quality: Some(net::Quality::Photo), ..Request::default() }, "x");
    assert_eq!(o.quality, Some(Quality::Photo));
}

fn reads(printer: &FakePrinter) -> usize {
    printer.actions().iter().filter(|a| *a == "GetPrinterElements").count()
}

#[test]
fn photo_quality_chosen_in_the_dialog_reaches_the_ticket() {
    let (mut app, printer) = app(Config { qualities: vec!["Normal", "Photo"], ..Default::default() });
    query(&mut app);
    assert!(state(&app).draft.offered_qualities().contains(&net::Quality::Photo), "the printer lists it, so the dialog offers it");
    app.state.draft.confirmed = true;
    app.state.draft.quality = Some(net::Quality::Photo);
    assert!(send(&mut app));
    wait(&mut app, |a| !state(a).draft.running());
    assert!(matches!(&state(&app).draft.job, JobView::Finished(r) if r.outcome == Outcome::Completed), "{:?}", state(&app).draft.job);
    let ticket = printer.job(1).unwrap().ticket;
    assert!(ticket.contains("Photo"), "Photo missing from {ticket}");
}

#[test]
fn a_printer_without_pdf_gets_the_pages_as_raster_and_is_read_twice_for_the_tracked_job() {
    let (mut app, printer) = app(Config { formats: vec!["image/pwg-raster"], resolutions: vec![(150, 150), (600, 600)], ..Default::default() });
    query(&mut app);
    assert_eq!(reads(&printer), 1, "the query");
    app.state.draft.confirmed = true;
    assert!(send(&mut app));
    wait(&mut app, |a| !state(a).draft.running());
    assert!(matches!(&state(&app).draft.job, JobView::Finished(r) if r.outcome == Outcome::Completed), "{:?}", state(&app).draft.job);
    let sent = printer.job(1).unwrap().document.unwrap();
    assert_eq!(&sent[..4], b"RaS2");
    // The listed resolution nearest 300 dpi (150), in the raster header's HWResolution.
    assert_eq!(u32::from_be_bytes(sent[4 + 276..4 + 280].try_into().unwrap()), 150);
    // The tracked `PrintJob` reads the printer's details itself (the registry's choice of format), then
    // the backend reads its capabilities to plan the ticket: two reads during the job after the query's.
    assert_eq!(reads(&printer), 3, "{:?}", printer.actions());
}

#[test]
fn a_printer_that_lists_pdf_gets_the_print_ready_pdf_unchanged() {
    let (mut app, printer) = app(Config::default());
    query(&mut app);
    app.state.draft.confirmed = true;
    assert!(send(&mut app));
    wait(&mut app, |a| !state(a).draft.running());
    assert_eq!(printer.job(1).unwrap().document.as_deref(), Some(&print().pdf[..]));
    assert_eq!(reads(&printer), 3, "the query, the registry's and the backend's own: {:?}", printer.actions());
}

#[test]
fn the_notice_for_a_print_that_ended_names_the_printer_and_no_address() {
    let labels = labels();
    let finished = |outcome| JobView::Finished(net::Report { outcome, printer: "http://127.0.0.1:5357/printer".to_owned(), sheets: 1, notes: Vec::new() });
    let aborted = notice_text(&labels, &finished(Outcome::Aborted), "Office Laser").unwrap();
    assert_eq!(aborted, format!("Office Laser: {}", labels.outcome_aborted));
    let unnamed = notice_text(&labels, &finished(Outcome::Aborted), "").unwrap();
    assert_eq!(unnamed, labels.outcome_aborted);
    // A report that carries an address instead of a name never shows it.
    let sent = notice_text(&labels, &finished(Outcome::Completed), "").unwrap();
    assert!(!sent.contains("127.0.0.1") && !sent.contains("http"), "{sent}");
    let failure = net::Failure::new(net::FailureKind::Transport, "no answer from http://127.0.0.1:5357/printer").with_outcome_unknown(true);
    let failed = notice_text(&labels, &JobView::Failed(failure), "Office Laser").unwrap();
    assert!(failed.starts_with("Office Laser: "), "{failed}");
    assert!(!failed.contains("127.0.0.1") && !failed.contains("http"), "{failed}");
    assert_eq!(notice_text(&labels, &JobView::Idle, "Office Laser"), None);
}

#[test]
fn the_raster_a_printer_without_pdf_gets_is_the_page_at_the_resolution_chosen() {
    let (mut app, printer) = app(Config { formats: vec!["image/pwg-raster"], resolutions: vec![(150, 150)], ..Default::default() });
    query(&mut app);
    app.state.draft.confirmed = true;
    assert!(send(&mut app));
    wait(&mut app, |a| !state(a).draft.running());
    let sent = printer.job(1).unwrap().document.unwrap();
    // A4 at 150 dpi is 1241 x 1754 pixels (the sheet, rounded up); the PWG header holds Width and Height.
    let word = |at: usize| u32::from_be_bytes(sent[4 + at..4 + at + 4].try_into().unwrap());
    assert_eq!((word(372), word(376)), (1241, 1754));
}

#[test]
fn a_printer_that_lists_neither_pdf_nor_raster_is_refused_before_a_job_exists() {
    let (mut app, printer) = app(Config { formats: vec!["application/postscript"], ..Default::default() });
    query(&mut app);
    app.state.draft.confirmed = true;
    // The dialog itself says what is missing: no format in common.
    let block = app.state.draft.block(true, true, FORMATS);
    assert!(block.is_some(), "{block:?}");
    assert!(!send(&mut app) || printer.jobs() == 0);
    wait(&mut app, |a| !state(a).draft.running());
    assert_eq!(printer.jobs(), 0);
}

/// The whole path through the Print dialog, drawn offscreen: pick the network destination, query the in-process
/// printer, tick the confirmation, press Print, and see the page arrive and the status line name the printer.
#[test]
fn the_print_dialog_sends_the_page_to_a_network_printer() {
    use crate::{PhotocraftApp, file_ui};
    let printer = FakePrinter::new(Config::default());
    let transport: Transport = Arc::new(MemTransport(printer.clone()));
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"name": "photo", "width": 60, "height": 40})).unwrap();
        app.set_network_transport(transport.clone());
        app
    });
    harness.run_steps(4);
    // An in-process printer list: the system's spooler is never asked.
    harness.state_mut().session.printer_source =
        Some(Arc::new(|| vec![craft_print::Printer { name: "Front Desk".into(), default: true, two_sided: None, description: String::new() }]));
    let id = file_ui::open_print(harness.state_mut());
    harness.state_mut().ui.dialog_mut(id).unwrap().fields.remove("__printers");
    {
        let app = harness.state_mut();
        app.net_print.selected = true;
        app.net_print.state.draft = net::Draft::for_address(ADDRESS);
    }
    harness.run_steps(3);
    let blocked =
        |h: &egui_kittest::Harness<'_, PhotocraftApp>| h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["__netBlocked"] == json!(true);
    assert!(blocked(&harness), "nothing queried and nothing confirmed yet");
    assert!(!file_ui::can_confirm(&harness.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields));
    {
        let app = harness.state_mut();
        let worker = query_printer(app.net_print.transport(), None, Arc::clone(&app.net_print.stations));
        assert!(app.net_print.state.start_query(&worker, true));
    }
    for _ in 0..200 {
        harness.run_steps(1);
        if !harness.state().net_print.state.draft.querying() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.state_mut().net_print.state.draft.confirmed = true;
    harness.run_steps(3);
    assert!(!blocked(&harness), "queried and confirmed");
    assert_eq!(printer.jobs(), 0, "a query and a ticked box send nothing");
    crate::dialogs::confirm(harness.state_mut(), id).unwrap();
    for _ in 0..400 {
        harness.run_steps(1);
        if !harness.state().net_print.state.draft.running() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    harness.run_steps(2);
    assert_eq!(printer.jobs(), 1);
    let sent = printer.job(1).unwrap().document.unwrap();
    assert!(sent.starts_with(b"%PDF-"), "a printer that lists PDF gets the print-ready PDF");
    assert!(harness.state().ui.status.contains("Sent to") || harness.state().ui.status.contains("Printed"), "{}", harness.state().ui.status);
}

/// A print that ends with the dialog closed is told on the status line; one the printer stopped, or one
/// that failed, is told as an error, and one that printed is not.
#[test]
fn a_print_that_ended_badly_is_an_error_on_the_status_line() {
    use crate::PhotocraftApp;
    let ended = |transport: Transport| {
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.set_network_transport(transport.clone());
            app
        });
        harness.run_steps(2);
        {
            let app = harness.state_mut();
            app.net_print.state.draft = net::Draft::for_address(ADDRESS);
            query(&mut app.net_print);
            app.net_print.state.draft.confirmed = true;
            // The opposite of what the end should leave, so the line is seen to be set.
            app.ui.status_error = true;
            assert!(send(&mut app.net_print));
        }
        for _ in 0..400 {
            harness.run_steps(1);
            if !harness.state().net_print.state.draft.running() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let app = harness.state();
        assert!(!app.net_print.state.draft.running(), "the print ended");
        (app.ui.status.clone(), app.ui.status_error)
    };
    let aborts = FakePrinter::new(Config { ending: Ending::Aborts("DocumentFormatError"), ..Default::default() });
    let (status, error) = ended(Arc::new(MemTransport(aborts)));
    assert!(status.contains(labels().outcome_aborted) && error, "{status} (error: {error})");
    let lost = FakePrinter::new(Config::default());
    let (status, error) = ended(Arc::new(LoseSendReply(lost)));
    assert!(error, "a failed send is an error: {status}");
    let (status, error) = ended(Arc::new(MemTransport(FakePrinter::new(Config::default()))));
    assert!(!error, "a print that printed is no error: {status}");
}
