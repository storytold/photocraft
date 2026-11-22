use super::*;

fn make_order_doc() -> (Session, [LayerId; 6], LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 500, "depth": 8, "background": "white"})).unwrap();
    let id_of = |v: Value| LayerId(v["layer"].as_u64().unwrap());

    let t1 = id_of(s.execute("type.create", json!({"text": "cat Cat CAT cats", "x": 10, "y": 40, "size": 12})).unwrap());
    let t2 = id_of(s.execute("type.create", json!({"text": "the cat sat", "x": 10, "y": 80, "size": 12})).unwrap());
    let t5 = id_of(s.execute("type.create", json!({"box": [10, 100, 200, 80], "text": "concatenate cat", "size": 12})).unwrap());
    let t6 = id_of(s.execute("type.create", json!({"text": "café cafe", "x": 10, "y": 200, "size": 12})).unwrap());
    let t7 = id_of(s.execute("type.create", json!({"text": "dog", "x": 10, "y": 240, "size": 12})).unwrap());

    // Group G1 holding T8, then an empty pixel layer on top.
    let t8 = id_of(s.execute("type.create", json!({"text": "grouped cat", "x": 10, "y": 280, "size": 12})).unwrap());
    s.execute("layer.new.groupFromLayers", json!({"name": "G1", "layers": [t8.0]})).unwrap();
    let top_pixel = id_of(s.execute("layer.new.layer", json!({"name": "empty"})).unwrap());

    // Verify bottom-to-top walk order of type layers is T1, T2, T5, T6, T7, T8.
    let walked_type_ids: Vec<LayerId> =
        s.active().unwrap().doc.walk().into_iter().filter_map(|(_, _, l)| matches!(l.content, LayerContent::Text(_)).then_some(l.id)).collect();
    assert_eq!(walked_type_ids, vec![t1, t2, t5, t6, t7, t8]);

    (s, [t1, t2, t5, t6, t7, t8], top_pixel)
}

fn find_next(s: &mut Session, forward: bool, all_layers: bool) -> Option<(LayerId, usize, usize, String)> {
    let r = s
        .execute(
            "edit.findAndReplaceText",
            json!({
                "find": "cat",
                "action": "find",
                "forward": forward,
                "allLayers": all_layers,
                "caseSensitive": false,
                "wholeWord": false,
            }),
        )
        .unwrap();
    let f = r.get("found")?;
    if f.is_null() {
        return None;
    }
    let layer = LayerId(f["layer"].as_u64().unwrap());
    let start = f["start"].as_u64().unwrap() as usize;
    let end = f["end"].as_u64().unwrap() as usize;
    let text = f["text"].as_str().unwrap().to_string();
    assert_eq!(s.active().unwrap().active_layer, Some(layer));
    Some((layer, start, end, text))
}

fn collect_round(s: &mut Session, forward: bool, all_layers: bool) -> Vec<(LayerId, usize, usize, String)> {
    let mut hits = Vec::new();
    while let Some(hit) = find_next(s, forward, all_layers) {
        assert!(hits.len() < 32, "search did not terminate: {hits:?}");
        hits.push(hit);
    }
    hits
}

#[test]
fn find_next_from_t1_forward_visits_upward_and_stops_at_end() {
    let (mut s, [t1, t2, t5, _t6, _t7, t8], _) = make_order_doc();
    s.execute("layer.select", json!({"layer": t1.0})).unwrap();
    let history_before = s.active().unwrap().history.past_len();

    let hits = collect_round(&mut s, true, true);

    assert_eq!(
        hits,
        vec![
            (t1, 0, 3, "cat".into()),
            (t1, 4, 7, "Cat".into()),
            (t1, 8, 11, "CAT".into()),
            (t1, 12, 15, "cat".into()),
            (t2, 4, 7, "cat".into()),
            (t5, 3, 6, "cat".into()),
            (t5, 12, 15, "cat".into()),
            (t8, 8, 11, "cat".into()),
        ]
    );
    assert_eq!(s.active().unwrap().history.past_len(), history_before, "Find Next adds no history state");

    // Next Find Next after the end starts a new round from the active layer (t8).
    assert_eq!(find_next(&mut s, true, true), Some((t8, 8, 11, "cat".into())));
}

#[test]
fn find_next_from_t5_forward_wraps_and_stops_before_t5() {
    let (mut s, [t1, t2, t5, _t6, _t7, t8], _) = make_order_doc();
    s.execute("layer.select", json!({"layer": t5.0})).unwrap();

    let hits = collect_round(&mut s, true, true);

    assert_eq!(
        hits,
        vec![
            (t5, 3, 6, "cat".into()),
            (t5, 12, 15, "cat".into()),
            (t8, 8, 11, "cat".into()),
            (t1, 0, 3, "cat".into()),
            (t1, 4, 7, "Cat".into()),
            (t1, 8, 11, "CAT".into()),
            (t1, 12, 15, "cat".into()),
            (t2, 4, 7, "cat".into()),
        ]
    );
}

#[test]
fn find_next_from_t5_backward_reverses_only_in_layer_matches() {
    let (mut s, [t1, t2, t5, _t6, _t7, t8], _) = make_order_doc();
    s.execute("layer.select", json!({"layer": t5.0})).unwrap();

    let hits = collect_round(&mut s, false, true);

    assert_eq!(
        hits,
        vec![
            (t5, 12, 15, "cat".into()),
            (t5, 3, 6, "cat".into()),
            (t8, 8, 11, "cat".into()),
            (t1, 12, 15, "cat".into()),
            (t1, 8, 11, "CAT".into()),
            (t1, 4, 7, "Cat".into()),
            (t1, 0, 3, "cat".into()),
            (t2, 4, 7, "cat".into()),
        ]
    );
}

#[test]
fn find_next_from_t7_backward_goes_upward_to_t8_first() {
    let (mut s, [_t1, _t2, _t5, _t6, t7, t8], _) = make_order_doc();
    s.execute("layer.select", json!({"layer": t7.0})).unwrap();

    assert_eq!(find_next(&mut s, false, true), Some((t8, 8, 11, "cat".into())));
}

#[test]
fn find_next_single_layer_and_change_all_preserved() {
    let (mut s, [_t1, _t2, t5, _t6, _t7, _t8], _) = make_order_doc();
    s.execute("layer.select", json!({"layer": t5.0})).unwrap();

    let fwd = collect_round(&mut s, true, false);
    assert_eq!(fwd, vec![(t5, 3, 6, "cat".into()), (t5, 12, 15, "cat".into())]);

    let bwd = collect_round(&mut s, false, false);
    assert_eq!(bwd, vec![(t5, 12, 15, "cat".into()), (t5, 3, 6, "cat".into())]);

    let r = s.execute("edit.findAndReplaceText", json!({"find": "cat", "replace": "fox", "action": "changeAll"})).unwrap();
    assert_eq!((r["count"].as_u64(), r["layers"].as_u64()), (Some(8), Some(4)));
}

#[test]
fn changed_query_starts_a_new_round_and_reaches_the_old_origin() {
    let (mut s, [t1, t2, ..], _) = make_order_doc();
    s.execute("layer.select", json!({"layer": t1.0})).unwrap();
    // The round starts at T1: four hits there, then the match in T2.
    for _ in 0..4 {
        assert_eq!(find_next(&mut s, true, true).map(|h| h.0), Some(t1));
    }
    assert_eq!(find_next(&mut s, true, true), Some((t2, 4, 7, "cat".into())));

    // A query that matches only in the old origin (T1) must be reachable from the cursor in T2.
    let r = s.execute("edit.findAndReplaceText", json!({"find": "CAT", "action": "find", "forward": true, "allLayers": true, "caseSensitive": true})).unwrap();
    assert_eq!((r["found"]["layer"].as_u64(), r["found"]["text"].as_str()), (Some(t1.0), Some("CAT")));
}

#[test]
fn find_next_from_top_pixel_layer_wraps_to_bottom_type_layer() {
    let (mut s, [t1, ..], top_pixel) = make_order_doc();
    s.execute("layer.select", json!({"layer": top_pixel.0})).unwrap();

    assert_eq!(find_next(&mut s, true, true), Some((t1, 0, 3, "cat".into())));
}
