use super::view::{ChannelTarget, ChannelView};
use super::{Session, json};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 20, "depth": 8})).unwrap();
    s
}

fn view(s: &Session) -> ChannelView {
    s.active().unwrap().channel_view.clone()
}

fn calc(s: &mut Session, result: &str) {
    s.execute("image.calculations", json!({"source1": {"channel": "red"}, "source2": {"channel": "green"}, "blending": "normal", "result": result})).unwrap();
}

fn color_hidden(s: &Session) -> Vec<bool> {
    let d = s.active().unwrap();
    (0..3).map(|c| d.channel_view.color_hidden.get(c).copied().unwrap_or(false)).collect()
}

fn alpha_visible(s: &Session) -> Vec<bool> {
    let d = s.active().unwrap();
    (0..d.doc.channels.len()).map(|c| d.channel_view.alpha_visible.get(c).copied().unwrap_or(false)).collect()
}

#[test]
fn new_channel_result_is_targeted_and_the_only_visible_channel() {
    let mut s = session();
    calc(&mut s, "newChannel");
    assert_eq!(s.active().unwrap().doc.channels.len(), 1);
    assert_eq!(view(&s).target, ChannelTarget::Alpha(0));
    assert_eq!(color_hidden(&s), vec![true, true, true]);
    assert_eq!(alpha_visible(&s), vec![true]);
}

#[test]
fn undo_from_the_composite_matches_new_channel() {
    let mut s = session();
    let before = view(&s);
    calc(&mut s, "newChannel");
    assert!(s.undo());
    let after_calc = view(&s);
    assert_eq!(after_calc.target, ChannelTarget::Composite);
    assert_eq!(color_hidden(&s), vec![false, false, false]);
    assert_eq!(alpha_visible(&s), Vec::<bool>::new());

    let mut n = session();
    n.execute("channel.new", json!({})).unwrap();
    assert!(n.undo());
    assert_eq!(after_calc, view(&n), "same as New Channel");
    assert_eq!(after_calc.target, before.target);
}

#[test]
fn undo_from_a_colour_target_matches_new_channel() {
    let mut s = session();
    s.execute("channel.target", json!({"channel": "red"})).unwrap();
    calc(&mut s, "newChannel");
    assert!(s.undo());
    let after_calc = view(&s);

    let mut n = session();
    n.execute("channel.target", json!({"channel": "red"})).unwrap();
    n.execute("channel.new", json!({})).unwrap();
    assert!(n.undo());
    assert_eq!(after_calc, view(&n), "same as New Channel, whatever that restores");
}

#[test]
fn selection_and_new_document_results_leave_the_channel_view_alone() {
    for result in ["selection", "newDocument"] {
        let mut s = session();
        s.execute("channel.target", json!({"channel": "red"})).unwrap();
        let before = view(&s);
        let active = s.active_index().unwrap();
        calc(&mut s, result);
        s.set_active(active);
        assert_eq!(view(&s), before, "{result}");
    }
}
