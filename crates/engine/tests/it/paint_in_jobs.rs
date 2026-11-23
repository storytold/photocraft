//! Tests that run paint commands, moved from the engine-core `jobs` tests: 2 test(s).

use std::time::{Duration, Instant};

use serde_json::json;

use photocraft_engine::jobs::*;

use photocraft_engine::Session;

/// A document `w`×`h` with a textured pixel layer (so blurs change every pixel).
fn session(w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("pattern", |doc, active| {
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let r = photocraft_geom::Rect::new(0, 0, w as i32, h as i32);
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let v = ((x * 7 + y * 13) % 31) as f32 / 30.0;
                px.extend_from_slice(&[v, 1.0 - v, ((x / 5 + y / 3) % 2) as f32, 1.0]);
            }
        }
        surf.write_region(r, &px);
        Ok(())
    })
    .unwrap();
    s
}

fn pixels(s: &Session) -> Vec<f32> {
    let d = s.active().unwrap();
    let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
    l.surface().unwrap().read_region(d.doc.bounds())
}

/// Poll until job `id` ends (or 60 s pass); returns its event.
fn wait_event(s: &mut Session, id: JobId) -> JobEvent {
    let t = Instant::now();
    loop {
        if let Some(e) = s.poll_jobs().into_iter().find(|e| e.id == id) {
            return e;
        }
        assert!(t.elapsed() < Duration::from_secs(60), "job {id:?} did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn job(r: Started) -> JobId {
    match r {
        Started::Job(id) => id,
        Started::Done(v) => panic!("expected a background job, ran inline: {v}"),
    }
}

#[test]
fn content_aware_move_runs_as_a_job() {
    let mut inline = session(160, 120);
    let mut s = session(160, 120);
    for t in [&mut inline, &mut s] {
        t.execute("select.rect", json!({"x": 20, "y": 40, "width": 30, "height": 30})).unwrap();
    }
    let p = json!({"offset": [90, 10], "structure": 3, "color": 4});
    inline.execute("paint.contentAwareMove", p.clone()).unwrap();
    let before = s.active().unwrap().doc.clone();
    let id = job(s.start("paint.contentAwareMove", p.clone()).unwrap());
    let e = wait_event(&mut s, id);
    let JobOutcome::Done(v) = &e.outcome else { panic!("{e:?}") };
    assert_eq!(v["offset"], json!([90, 10]));
    assert_eq!(pixels(&s), pixels(&inline), "same result as the synchronous command");
    let sel = |s: &Session| s.active().unwrap().doc.selection.as_ref().unwrap().read_region(s.active().unwrap().doc.bounds());
    assert_eq!(sel(&s), sel(&inline), "the selection moved the same way");
    // Cancelled at once: the document stays as it was.
    s.undo();
    let id = job(s.start("paint.contentAwareMove", p).unwrap());
    assert!(s.cancel_job(id));
    s.join_cancelled_jobs();
    assert_eq!(wait_event(&mut s, id).outcome, JobOutcome::Cancelled);
    assert_eq!(pixels(&s), before.layer(s.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().read_region(before.bounds()));
}

#[test]
fn remove_runs_as_a_job() {
    let mut inline = session(160, 120);
    let mut s = session(160, 120);
    let p = json!({"points": [[60, 50], [100, 70]], "size": 24});
    inline.execute("paint.remove", p.clone()).unwrap();
    let before = pixels(&s);
    let id = job(s.start("paint.remove", p.clone()).unwrap());
    let e = wait_event(&mut s, id);
    let JobOutcome::Done(v) = &e.outcome else { panic!("{e:?}") };
    assert!(v["damage"].is_array(), "{v}");
    assert_eq!(pixels(&s), pixels(&inline), "same result as the synchronous command");
    // Cancelled at once: the document stays as it was.
    s.undo();
    let id = job(s.start("paint.remove", p).unwrap());
    assert!(s.cancel_job(id));
    s.join_cancelled_jobs();
    assert_eq!(wait_event(&mut s, id).outcome, JobOutcome::Cancelled);
    assert_eq!(pixels(&s), before);
}
