use photocraft_engine::jobs::JobCtx;

#[test]
fn negative_zero_progress_does_not_pin_fraction() {
    let c = JobCtx::new();
    c.progress(-0.0, "");
    c.progress(0.5, "");
    assert_eq!(c.fraction(), 0.5);
}
