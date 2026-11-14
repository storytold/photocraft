//! Direct Session consumers must respect the same shell-owned preview lease as the UI.
use photocraft_engine::{
    Session,
    actions_cmds::{Action, playback},
    jobs::Started,
};
use serde_json::{Value, json};

fn session() -> Session {
    let mut session = Session::new();
    session.execute("file.new", json!({"width":40,"height":30,"background":"#804020"})).unwrap();
    session
}

fn draft(session: &mut Session) -> u64 {
    let id = session.execute("actions.editor.open", json!({"name":"Unpublished"})).unwrap()["draft"].as_u64().unwrap();
    session.execute("actions.editor.step", json!({"draft":id,"op":"insert","step":0,"id":"layer.renameLayer","params":{"name":"Preview"}})).unwrap();
    session.execute("actions.editor.update", json!({"draft":id,"description":"Retain me","icon":"FutureIcon"})).unwrap();
    id
}

fn get(session: &mut Session, id: u64) -> Value {
    session.execute("actions.editor.get", json!({"draft":id})).unwrap()
}

#[test]
fn direct_commands_cannot_invalidate_preview_and_finish_restores_only_capture() {
    let mut session = session();
    let id = draft(&mut session);
    session.execute("actions.editor.record", json!({"draft":id})).unwrap();
    session.execute("select.all", json!({})).unwrap();
    let visible = get(&mut session, id)["action"].clone();
    let take = session.actions.take;
    let (snapshot, owner) = playback::preview(&mut session, id, 0).unwrap();
    assert_eq!(serde_json::to_value(snapshot.as_ref()).unwrap(), visible);
    assert_eq!(snapshot.description, "Retain me");
    assert_eq!(snapshot.icon, "FutureIcon");
    let before = session.actions.clone();
    for (command, params) in [
        ("actions.editor.save", json!({"draft":id})),
        ("actions.editor.discard", json!({"draft":id})),
        ("actions.editor.update", json!({"draft":id,"name":"Other"})),
        ("actions.record", json!({"name":"Intruder"})),
    ] {
        assert!(session.execute(command, params).is_err());
        assert_eq!(session.actions, before);
    }
    owner.validate(&session).unwrap();
    session.execute("layer.renameLayer", json!({"name":"Document work"})).unwrap();
    owner.finish(&mut session);
    assert_eq!(session.actions.take, take);
    assert_eq!(get(&mut session, id)["action"], visible);
    assert_eq!(get(&mut session, id)["recording"], true);
    assert!(session.actions.list.is_empty());
    assert!(session.actions.held_playback.is_none());
    assert_eq!(session.active().unwrap().doc.layer(session.active().unwrap().active_layer.unwrap()).unwrap().name, "Document work");
}

#[test]
fn stale_take_never_restores_over_a_later_classic_recording() {
    let mut session = session();
    let id = draft(&mut session);
    session.execute("actions.editor.record", json!({"draft":id})).unwrap();
    let (_, owner) = playback::preview(&mut session, id, 0).unwrap();
    session.actions.take += 1;
    session.actions.list.push(Action::new("Later", Vec::new()));
    let later = Some((session.journal.len(), 0));
    session.actions.recording = later;
    assert!(owner.validate(&session).is_err());
    owner.finish(&mut session);
    assert_eq!(session.actions.recording, later);
    assert!(session.actions.held_playback.is_none());
    assert_eq!(get(&mut session, id)["recording"], false);
}

#[test]
fn a_refused_definition_edit_cannot_drop_the_floating_selection_first() {
    let mut session = session();
    let id = draft(&mut session);
    let (_, owner) = playback::preview(&mut session, id, 0).unwrap();
    session.execute("select.rect", json!({"x":1,"y":1,"width":5,"height":4})).unwrap();
    session.execute("select.float", json!({"dx":3,"dy":0,"copy":true})).unwrap();
    let document = session.active().unwrap().doc.clone();
    let history = session.active().unwrap().history.entries();
    let journal = session.journal.clone();
    assert!(photocraft_engine::commands::float_cmds::floating(session.active().unwrap()).is_some());
    assert!(session.execute("actions.editor.update", json!({"draft":id,"name":"Rejected"})).is_err());
    assert!(std::sync::Arc::ptr_eq(&document, &session.active().unwrap().doc));
    assert_eq!(session.active().unwrap().history.entries(), history);
    assert_eq!(session.journal, journal);
    assert!(photocraft_engine::commands::float_cmds::floating(session.active().unwrap()).is_some());
    owner.finish(&mut session);
    assert!(photocraft_engine::commands::float_cmds::floating(session.active().unwrap()).is_some());
}

#[test]
fn preview_waits_for_prior_jobs_and_later_unowned_jobs_stay_out_of_capture() {
    let mut session = session();
    session
        .edit("pattern", |doc, active| {
            let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            let mut pixels = Vec::new();
            for y in 0..30 {
                for x in 0..40 {
                    let v = ((x * 7 + y * 13) % 31) as f32 / 30.0;
                    pixels.extend_from_slice(&[v, 1.0 - v, 0.5, 1.0]);
                }
            }
            surface.write_region(photocraft_geom::Rect::new(0, 0, 40, 30), &pixels);
            Ok(())
        })
        .unwrap();
    let id = draft(&mut session);
    session.execute("actions.editor.record", json!({"draft":id})).unwrap();
    let Started::Job(first) = session.start("filter.blur.gaussianBlur", json!({"radius":1})).unwrap() else { panic!("real job required") };
    let before = session.actions.clone();
    assert!(playback::preview(&mut session, id, 0).is_err());
    assert_eq!(session.actions, before);
    session.wait_job(first).unwrap();
    let visible = get(&mut session, id)["action"].clone();
    let (_, owner) = playback::preview(&mut session, id, 0).unwrap();
    let Started::Job(later) = session.start("filter.blur.gaussianBlur", json!({"radius":2})).unwrap() else { panic!("real job required") };
    owner.finish(&mut session);
    session.wait_job(later).unwrap();
    assert_eq!(get(&mut session, id)["action"], visible, "the unowned late job does not enter the restored capture");
    assert_eq!(get(&mut session, id)["recording"], true);
}
