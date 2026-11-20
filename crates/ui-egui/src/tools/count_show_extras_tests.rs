//! #11770: adding a count marker while View > Show > Count is off switches it back on.

use egui::Modifiers;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;

fn app() -> Box<PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool = Tool::Count;
    app
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64, m: Modifiers) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x, y }, m);
}

fn points(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.measurement.count_groups.iter().map(|g| g.points.len()).sum()
}

fn past(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// A Count click turns View > Show > Count on only when it adds a marker; the group it adds to
/// keeps its own visibility, and an Alt-click that removes one leaves Show Count alone.
#[test]
fn a_count_click_turns_show_count_on_only_when_it_adds() {
    struct Row {
        /// A marker at (10, 10) exists first.
        seeded: bool,
        /// Its group is hidden.
        hidden: bool,
        show_before: bool,
        at: (f64, f64),
        mods: Modifiers,
        points_after: usize,
        show_after: bool,
        /// History steps the click adds, when the row pins them.
        steps: Option<usize>,
    }
    let rows = [
        Row { seeded: false, hidden: false, show_before: false, at: (40.0, 30.0), mods: Modifiers::NONE, points_after: 1, show_after: true, steps: Some(1) },
        Row { seeded: false, hidden: false, show_before: true, at: (40.0, 30.0), mods: Modifiers::NONE, points_after: 1, show_after: true, steps: None },
        Row { seeded: true, hidden: true, show_before: false, at: (90.0, 60.0), mods: Modifiers::NONE, points_after: 2, show_after: true, steps: None },
        Row { seeded: true, hidden: false, show_before: false, at: (10.0, 10.0), mods: Modifiers::ALT, points_after: 0, show_after: false, steps: None },
    ];
    for (n, row) in rows.iter().enumerate() {
        let mut app = app();
        if row.seeded {
            click(&mut app, 10.0, 10.0, Modifiers::NONE);
        }
        if row.hidden {
            app.run("count.setGroup", json!({"group": 0, "visible": false})).unwrap();
        }
        app.ui.view.show.count = row.show_before;
        let before = past(&app);
        click(&mut app, row.at.0, row.at.1, row.mods);
        assert_eq!(points(&app), row.points_after, "row {n}: markers");
        assert_eq!(app.ui.view.show.count, row.show_after, "row {n}: Show Count");
        if row.hidden {
            assert!(!app.session.active().unwrap().doc.measurement.count_groups[0].visible, "row {n}: the group stays hidden");
        }
        if let Some(steps) = row.steps {
            assert_eq!(past(&app), before + steps, "row {n}: history steps");
        }
    }
}
