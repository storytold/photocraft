//! #7366: a background Remove on a Red-only target keeps G/B/alpha intact when a Type click
//! selects another layer while the job is pending.

use photocraft_doc::LayerId;
use photocraft_engine::Session;
use photocraft_engine::jobs::Started;
use photocraft_ui_egui::{PhotocraftApp, Services, control};

const REMOVE: &str = r#"{"points":[[12,10],[16,12]],"size":6}"#;

type Harness = egui_kittest::Harness<'static, Box<PhotocraftApp>>;

fn run(s: &mut Session, id: &str, p: &str) {
    s.execute(id, p.parse().unwrap()).unwrap();
}

/// Context for assertion messages: who is active, whether a text edit is open, and whether the job is done.
fn state_line(app: &PhotocraftApp, job: photocraft_engine::jobs::JobId) -> String {
    format!(
        "active layer {:?}, text-edit session open {}, job finished {}",
        app.session.active().and_then(|d| d.active_layer),
        app.ui.text_edit.is_some(),
        app.session.job(job).is_none()
    )
}

// Snapshot only existing public read-only inspection fields, before injection and after each
// existing pass. No extra frame or wait is introduced: preserve the pending-job regression.
fn delivery_snapshot(h: &Harness, job: photocraft_engine::jobs::JobId, text: LayerId) -> serde_json::Value {
    let state = control::inspect(h.state(), &h.ctx);
    let text_bounds = h.state().session.active().and_then(|d| d.doc.layer(text)).and_then(|layer| {
        let photocraft_doc::LayerContent::Text(t) = &layer.content else { return None };
        let r = t.cache.as_ref()?.content_bounds();
        Some([r.x0, r.y0, r.x1, r.y1])
    });
    serde_json::json!({
        "state": state_line(h.state(), job),
        "textCacheBounds": text_bounds,
        "views": state.get("views"),
        "rects": state.get("rects"),
        "jobs": state.get("jobs"),
        "textEdit": state.get("textEdit"),
        "tool": state.get("tool"),
        "frame": state.get("frame"),
    })
}

fn pixels(s: &Session, doc: usize, id: LayerId) -> Vec<f32> {
    let d = &s.documents()[doc].doc;
    d.layer(id).unwrap().surface().unwrap().read_region(d.bounds())
}

#[derive(Clone, Copy)]
enum Input {
    None,
    ControlPointer,
}

fn probe(input: Input) {
    let mut a = Session::new();
    run(&mut a, "file.new", r#"{"name":"A","width":32,"height":24,"background":"transparent"}"#);
    run(&mut a, "type.create", r#"{"text":"X","x":2,"y":18,"size":14}"#);
    let text = a.active().unwrap().active_layer.unwrap();
    run(&mut a, "layer.new.layer", r#"{"name":"Target"}"#);
    let target = a.active().unwrap().active_layer.unwrap();
    a.edit("fixture", |d, l| {
        let q = d.layer_mut(l.unwrap()).unwrap().surface_mut().unwrap();
        for y in 0..24 {
            for x in 0..32 {
                q.write_pixel(x, y, &[((x * 7 + y * 13) % 31) as f32 / 30., (x % 5) as f32 / 4., (y % 7) as f32 / 6., 1.]);
            }
        }
        Ok(())
    })
    .unwrap();
    run(&mut a, "file.new", r##"{"name":"B","width":32,"height":24,"background":"#6a219c"}"##);
    a.set_active(0);
    run(&mut a, "channel.target", r#"{"channel":"red"}"#);
    let b = a.documents()[1].doc.clone();
    let text_before = a.documents()[0].doc.layer(text).unwrap().clone();

    // The inline Red-only Remove is the oracle.
    let mut inline = Session::new();
    inline.add_document(a.documents()[0].doc.as_ref().clone(), None);
    inline.select_layer(target).unwrap();
    run(&mut inline, "channel.target", r#"{"channel":"red"}"#);
    run(&mut inline, "paint.remove", REMOVE);
    let expected = pixels(&inline, 0, target);

    // The app runs in a harness so the canvas takes the control channel's pointer input on the
    // frames after the call.
    let mut h = Harness::builder().with_size(egui::vec2(1100.0, 720.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        // The app shares the harness's context, which is where the control channel's pointer input arrives.
        PhotocraftApp::with_context(cc.egui_ctx.clone(), a, Services::default())
    });
    h.run_steps(4);

    let mut delivery_trace = Vec::new();
    let job = match h.state_mut().session.start("paint.remove", REMOVE.parse().unwrap()).unwrap() {
        Started::Job(id) => id,
        v => panic!("must start a background job: {v:?}"),
    };
    match input {
        Input::None => {}
        Input::ControlPointer => {
            delivery_trace.push(delivery_snapshot(&h, job, text));
            let ctx = h.ctx.clone();
            let (req, _rx) = control::ControlRequest::new("ui.pointer", r#"{"tool":"type","events":[{"kind":"down","x":4,"y":14}]}"#.parse().unwrap());
            match control::handle(h.state_mut(), &ctx, &req) {
                control::Reply::Done(reply) => assert_eq!(reply["ok"], true, "the pointer request is accepted: {reply}"),
                _ => panic!("ui.pointer must reply immediately"),
            }
            // The control channel queues input for the canvas; wait for delivery rather than
            // assuming two passes suffice on every host.
            let host = craft::ui::runtime::windows::WindowHost::of(&ctx);
            for _ in 0..8 {
                if host.injected_idle() {
                    break;
                }
                h.run_steps(1);
                delivery_trace.push(delivery_snapshot(&h, job, text));
            }
            let state = format!("{}; delivery trace {delivery_trace:?}", state_line(h.state(), job));
            assert!(host.injected_idle(), "the canvas consumed the queued pointer input; {state}");
            // `injected_idle` is also true when the step was withdrawn (focus change before its
            // pass) instead of delivered, so delivery is asserted separately.
            assert!(!host.take_injected_withdrawn(), "the injected press was delivered, not withdrawn; {state}");
        }
    }
    let selected = if matches!(input, Input::None) { target } else { text };
    assert_eq!(
        h.state().session.active().unwrap().active_layer,
        Some(selected),
        "the input selects the text layer; {}; delivery trace {delivery_trace:?}",
        state_line(h.state(), job)
    );

    let a = h.state_mut();
    a.session.set_active(1);
    a.session.wait_job(job).unwrap();
    assert_eq!(a.session.documents()[1].doc, b, "B is unchanged");
    let st = &a.session.documents()[0];
    assert_eq!(st.doc.layer(text).unwrap(), &text_before, "the text layer is unchanged");
    assert_eq!(st.active_layer, Some(selected), "the user's selection survives the job");
    let actual = pixels(&a.session, 0, target);
    let wrong = expected.as_chunks::<4>().0.iter().zip(actual.as_chunks::<4>().0).filter(|(e, r)| e[1..] != r[1..]).count();
    assert_eq!(wrong, 0, "pixels with G/B/alpha changed outside the Red target");
    assert_eq!(actual, expected);
}

#[test]
fn no_input_keeps_red_restriction() {
    probe(Input::None);
}

#[test]
fn control_pointer_click_keeps_red_restriction() {
    probe(Input::ControlPointer);
}
