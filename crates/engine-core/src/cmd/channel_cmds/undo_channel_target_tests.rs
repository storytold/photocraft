//! Undo of a step that targeted a new alpha channel brings back the channel view from before it:
//! the targeted colour channel and the eyes (#14683).

use super::*;
use crate::channel_cmds::ChannelView;

fn session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 40, "height": 20})).unwrap();
    s
}

fn view(s: &Session) -> ChannelView {
    s.active().unwrap().channel_view.clone()
}

/// Red targeted (shown alone), then `command`, then undo: Red is the target again and the only
/// colour channel shown, as before the command.
fn undo_returns_to_red(command: &str, params: Value) {
    let mut s = session();
    s.execute("channel.target", json!({"channel": "red"})).unwrap();
    let before = view(&s);
    assert_eq!(before.target, ChannelTarget::Color(0));
    s.execute(command, params).unwrap();
    assert!(s.undo());
    assert!(doc(&s).channels.is_empty());
    let after = view(&s);
    assert_eq!((after.target, &after.color_hidden), (before.target, &before.color_hidden), "{command}");
    assert_eq!(channels_json(s.active().unwrap()), channels_json_of(before, &s), "{command}");
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}

/// The Channels panel as `view` would show the current document.
fn channels_json_of(view: ChannelView, s: &Session) -> Value {
    let mut st = s.active().unwrap().clone();
    st.channel_view = view;
    channels_json(&st)
}

#[test]
fn undo_of_new_channel_targets_the_colour_channel_again() {
    undo_returns_to_red("channel.new", json!({}));
}

#[test]
fn undo_of_calculations_new_channel_targets_the_colour_channel_again() {
    undo_returns_to_red("image.calculations", json!({"result": "newChannel"}));
}

/// From the composite, undo returns to the composite with every colour channel shown, as today.
#[test]
fn undo_from_the_composite_keeps_working() {
    for command in ["channel.new", "image.calculations"] {
        let mut s = session();
        let before = view(&s);
        s.execute(command, json!({})).unwrap();
        assert!(s.undo());
        let after = view(&s);
        assert_eq!(after.target, ChannelTarget::Composite, "{command}");
        assert_eq!(channels_json(s.active().unwrap()), channels_json_of(before, &s), "{command}");
    }
}

/// Redo brings the new channel back, and undo after it returns to Red again.
#[test]
fn undo_after_redo_returns_to_red_again() {
    let mut s = session();
    s.execute("channel.target", json!({"channel": "red"})).unwrap();
    s.execute("channel.new", json!({})).unwrap();
    assert!(s.undo());
    assert!(s.redo());
    assert_eq!(doc(&s).channels.len(), 1);
    assert!(s.undo());
    assert_eq!(view(&s).target, ChannelTarget::Color(0));
}

/// Other commands' undo leaves the channel view as it is: a selection made with Red targeted,
/// then Green targeted, undone, keeps Green.
#[test]
fn other_steps_do_not_restore_the_view() {
    let mut s = session();
    s.execute("channel.target", json!({"channel": "red"})).unwrap();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap();
    s.execute("channel.target", json!({"channel": "green"})).unwrap();
    assert!(s.undo());
    assert_eq!(view(&s).target, ChannelTarget::Color(1));
}

/// New Channel from Red, undone, then a selection made from the same state with Green targeted:
/// undo of the selection lands on the state New Channel started from but undoes the selection, so
/// Green stays targeted.
#[test]
fn undo_of_a_later_step_from_the_same_state_keeps_the_target() {
    let mut s = session();
    s.execute("channel.target", json!({"channel": "red"})).unwrap();
    s.execute("channel.new", json!({})).unwrap();
    assert!(s.undo());
    s.execute("channel.target", json!({"channel": "green"})).unwrap();
    s.execute("select.rect", json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap();
    assert!(s.undo());
    assert_eq!(view(&s).target, ChannelTarget::Color(1));
}

/// The remembered view stays with the New Channel step: every undo of it, after any number of
/// redos, returns to Red.
#[test]
fn every_undo_of_new_channel_returns_to_red() {
    let mut s = session();
    s.execute("channel.target", json!({"channel": "red"})).unwrap();
    s.execute("channel.new", json!({})).unwrap();
    for round in 0..3 {
        assert!(s.undo());
        assert_eq!(view(&s).target, ChannelTarget::Color(0), "undo {round}");
        s.execute("channel.target", json!({"channel": "blue"})).unwrap();
        assert!(s.redo());
        assert_eq!(doc(&s).channels.len(), 1);
    }
}
