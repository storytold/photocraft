//! #11764: the Count tool's options-bar readout: "N" for one group (or none), "V (C)" for two or
//! more, V the visible groups' markers and C the current group's (0 while it is hidden).

use serde_json::{Value, json};

use super::count_readout;

fn session(groups: &[usize]) -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 100, "height": 100})).unwrap();
    for (g, n) in groups.iter().enumerate() {
        s.execute("count.newGroup", json!({"name": format!("G{g}")})).unwrap();
        for i in 0..*n {
            s.execute("count.add", json!({"x": 10 + 5 * i, "y": 10 + 10 * g})).unwrap();
        }
    }
    s
}

/// The readout by groups' markers, the `count.setGroup` edits applied in order and a stale current
/// index: "N" for one group or none (a hidden single group still counts), "V (C)" for two or more.
#[test]
fn count_readout_counts_visible_and_current() {
    struct Row {
        groups: &'static [usize],
        edits: Vec<Value>,
        stale_current: Option<usize>,
        want: &'static str,
    }
    let row = |groups, edits, stale_current, want| Row { groups, edits, stale_current, want };
    let rows = [
        row(&[], vec![], None, "0"),
        row(&[3], vec![], None, "3"),
        row(&[3], vec![json!({"visible": false})], None, "3"),
        row(&[3, 2], vec![], None, "5 (2)"),
        row(&[3, 2], vec![json!({"visible": false})], None, "3 (0)"),
        row(&[3, 2], vec![json!({"group": 0, "visible": false})], None, "2 (2)"),
        row(&[3, 2], vec![json!({"group": 0, "visible": false}), json!({"group": 0, "visible": true, "active": true})], None, "5 (3)"),
        row(&[3, 2, 4], vec![], None, "9 (4)"),
        row(&[0, 0], vec![], None, "0 (0)"),
        row(&[3, 2], vec![], Some(9), "5 (2)"),
    ];
    for (n, r) in rows.iter().enumerate() {
        let mut s = session(r.groups);
        for edit in &r.edits {
            s.execute("count.setGroup", edit.clone()).unwrap();
        }
        let mut m = s.active().unwrap().doc.measurement.clone();
        if let Some(current) = r.stale_current {
            m.active_count_group = current;
        }
        assert_eq!(count_readout(&m), r.want, "row {n}: groups {:?}, edits {:?}, stale {:?}", r.groups, r.edits, r.stale_current);
    }
}
