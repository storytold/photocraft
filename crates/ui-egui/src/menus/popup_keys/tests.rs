//! ↑ / ↓ stop at the ends of an egui popup menu (#15227) and Home / End go to them (#15219).

use egui::Key;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

/// The label of the widget that has the keyboard focus, if any.
pub(crate) fn focused<S>(h: &Harness<'_, S>) -> Option<String> {
    h.query_by(|n| n.is_focused()).and_then(|n| n.accesskit_node().label())
}

/// Press `key` and let egui settle: its menu keys move the focus a pass later.
pub(crate) fn press<S>(h: &mut Harness<'_, S>, key: Key) {
    h.key_press(key);
    h.run_steps(4);
}

/// A button whose right-click menu has Alpha, Bravo, a greyed Charlie, a More submenu (One, Two,
/// Three) and Echo, with the menu keys run ahead of the frame as `shortcuts::handle` runs them.
/// The menu is open, with nothing in it focused yet.
fn open_menu() -> Harness<'static> {
    let mut h = Harness::builder().with_size(egui::vec2(480.0, 360.0)).build_ui(|ui| {
        super::handle(ui.ctx());
        let _ = ui.button("Target").context_menu(|ui| {
            for (label, enabled) in [("Alpha", true), ("Bravo", true), ("Charlie", false)] {
                ui.add_enabled(enabled, egui::Button::new(label));
            }
            ui.menu_button("More", |ui| {
                for label in ["One", "Two", "Three"] {
                    let _ = ui.button(label);
                }
            });
            let _ = ui.button("Echo");
        });
    });
    h.run_steps(2);
    h.get_by_label("Target").click_secondary();
    h.run_steps(4);
    assert!(h.query_by_label("Echo").is_some(), "the menu opened");
    assert_eq!(focused(&h), None);
    h
}

/// Press each key in turn and return where the focus is after each.
fn walk(h: &mut Harness<'static>, keys: &[Key]) -> Vec<Option<String>> {
    keys.iter()
        .map(|&k| {
            press(h, k);
            focused(h)
        })
        .collect()
}

fn names(labels: &[&str]) -> Vec<Option<String>> {
    labels.iter().map(|l| Some((*l).to_owned())).collect()
}

#[test]
fn up_and_down_stop_at_the_first_and_last_entries() {
    use Key::{ArrowDown as Down, ArrowUp as Up};
    let mut h = open_menu();
    // ↑ on the first entry stays there (it went to Echo); the greyed Charlie is skipped; ↓ on the
    // last entry stays there (it went back to Alpha).
    assert_eq!(walk(&mut h, &[Down, Up, Up, Down, Down, Down, Down, Down]), names(&["Alpha", "Alpha", "Alpha", "Bravo", "More", "Echo", "Echo", "Echo"]));
    assert_eq!(walk(&mut h, &[Up, Up, Up, Up, Up]), names(&["More", "Bravo", "Alpha", "Alpha", "Alpha"]));
    // In a submenu too.
    assert_eq!(walk(&mut h, &[Down, Down, Key::ArrowRight, Up, Down, Down, Down]), names(&["Bravo", "More", "One", "One", "Two", "Three", "Three"]));
    // ↑ in a menu opened with the pointer, nothing focused yet, still starts from the last entry.
    let mut h = open_menu();
    assert_eq!(walk(&mut h, &[Up, Up, Down]), names(&["Echo", "More", "Echo"]));
}

#[test]
fn home_and_end_go_to_the_first_and_last_entries() {
    use Key::{ArrowRight as Right, ArrowUp as Up, End, Home};
    let mut h = open_menu();
    // Also before any entry is focused.
    assert_eq!(walk(&mut h, &[End, Home, End, Up]), names(&["Echo", "Alpha", "Echo", "More"]));
    // ↑ onto More opened its submenu; Home closes it, as an arrow moving off More does.
    assert!(h.query_by_label("One").is_some(), "the submenu opened");
    assert_eq!(walk(&mut h, &[Home]), names(&["Alpha"]));
    assert!(h.query_by_label("One").is_none(), "the submenu closed");
    // In the submenu they stay in it.
    assert_eq!(walk(&mut h, &[End, Up, Right, End, Home]), names(&["Echo", "More", "One", "Three", "One"]));
}
