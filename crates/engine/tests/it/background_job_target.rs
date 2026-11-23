//! #7366: a background job applies to the layer and channels it started on, whatever the user
//! selects while it runs. A job's result is applied only by `poll_jobs` or `wait_job` on the
//! session's thread, so a selection made after `start` always comes first.

use photocraft_doc::LayerId;
use photocraft_engine::Session;
use photocraft_engine::jobs::Started;

const REMOVE: &str = r#"{"points":[[12,10],[16,12]],"size":6}"#;

fn run(s: &mut Session, id: &str, p: &str) {
    s.execute(id, p.parse().unwrap()).unwrap();
}

fn pixels(s: &Session, doc: usize, id: LayerId) -> Vec<f32> {
    let d = &s.documents()[doc].doc;
    d.layer(id).unwrap().surface().unwrap().read_region(d.bounds())
}

/// Starts a Red-only Remove on the target layer, optionally selects the text layer (what a Type
/// click does) while it is pending, then waits for it.
fn probe(select_text_while_pending: bool) {
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

    let job = match a.start("paint.remove", REMOVE.parse().unwrap()).unwrap() {
        Started::Job(id) => id,
        v => panic!("must start a background job: {v:?}"),
    };
    let selected = if select_text_while_pending { text } else { target };
    if select_text_while_pending {
        a.select_layer(text).unwrap();
    }
    a.set_active(1);
    a.wait_job(job).unwrap();
    assert_eq!(a.documents()[1].doc, b, "B is unchanged");
    let st = &a.documents()[0];
    assert_eq!(st.doc.layer(text).unwrap(), &text_before, "the text layer is unchanged");
    assert_eq!(st.active_layer, Some(selected), "the user's selection survives the job");
    let actual = pixels(&a, 0, target);
    let wrong = expected.as_chunks::<4>().0.iter().zip(actual.as_chunks::<4>().0).filter(|(e, r)| e[1..] != r[1..]).count();
    assert_eq!(wrong, 0, "pixels with G/B/alpha changed outside the Red target");
    assert_eq!(actual, expected);
}

#[test]
fn a_background_job_applies_to_the_layer_and_channels_it_started_on() {
    probe(false);
    probe(true);
}
