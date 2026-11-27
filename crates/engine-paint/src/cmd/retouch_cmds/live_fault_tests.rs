use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;

use super::{DabFault, DabNeighborhood, LiveDab, parse_brush};

#[test]
fn a_fault_in_the_first_points_stops_the_live_stroke_at_once() {
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 600, "height": 90, "depth": 8, "mode": "rgb"})).unwrap();
    let p = json!({"points": [[10, 40], [590, 40]], "size": 40, "hardness": 100});
    let (stroke, id) = parse_brush(&s, &p, "paint.burn").unwrap();
    let dabs = Arc::new(AtomicUsize::new(0));
    let counted = dabs.clone();
    let result = LiveDab::with_neighborhood(&s, "paint.burn", &p, stroke, id, DabNeighborhood::Halo(0), move |_| {
        let fault = DabFault::default();
        let set = fault.clone();
        let effect: super::DabEffect = Box::new(move |_, _| {
            counted.fetch_add(1, Ordering::Relaxed);
            set.set(photocraft_engine_core::EngineError::Other("out of memory for the test".into()));
        });
        (effect, fault)
    });
    assert!(result.is_err_and(|e| e.to_string().contains("out of memory for the test")));
    assert_eq!(dabs.load(Ordering::Relaxed), 1, "no dab ran after the first one failed");
}
