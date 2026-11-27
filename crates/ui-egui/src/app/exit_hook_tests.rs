use super::*;
use std::cell::Cell;
use std::rc::Rc;

/// The desktop app's exit hook runs in `on_exit`, the one exit path macOS has (its quit ends the
/// process before `run_native` returns), and only once.
#[test]
fn leaving_runs_the_exit_hook_once() {
    let runs = Rc::new(Cell::new(0));
    let counted = runs.clone();
    let services = Services { at_exit: Some(Box::new(move || counted.set(counted.get() + 1))), ..Default::default() };
    let mut app = PhotocraftApp::new(Session::new(), services);
    eframe::App::on_exit(&mut app);
    assert_eq!(runs.get(), 1);
    eframe::App::on_exit(&mut app);
    assert_eq!(runs.get(), 1, "a second exit must not run the hook again");
}
