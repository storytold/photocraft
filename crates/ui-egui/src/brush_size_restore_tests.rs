use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

fn row(kind: theme::ThemeKind, size: f32, original: Option<f32>) -> Harness<'static, (f32, Option<f32>)> {
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(340.0, 160.0)).build_ui_state(
        move |ui, state: &mut (f32, Option<f32>)| {
            if !ready {
                crate::PhotocraftApp::setup_context(ui.ctx(), kind);
                ready = true;
                return;
            }
            size_row(ui, "Size", &mut state.0, 2500.0, state.1);
        },
        (size, original),
    );
    h.run_steps(3);
    h
}

/// #3825: the reset affordance occupies the same place for every tip and only enables an edit.
#[test]
fn size_restore_is_always_visible_and_disabled_without_a_change() {
    for kind in theme::ThemeKind::ALL {
        let mut h = row(kind, 37.25, None);
        assert_eq!(Tokens::get(&h.ctx), Tokens::for_kind(kind));
        let button = h.get_by_label("Restore original size");
        let rect = button.rect();
        assert!(button.accesskit_node().is_disabled(), "{kind:?}: no original bitmap size");
        let field = h.get_by_role(Role::SpinButton).rect();
        button.click();
        h.key_press(egui::Key::Space);
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        assert_eq!(h.state().0, 37.25, "disabled input must preserve fractional imported sizes");

        h.state_mut().1 = Some(37.25);
        h.run_steps(2);
        assert!(h.get_by_label("Restore original size").accesskit_node().is_disabled());
        assert_eq!(h.get_by_label("Restore original size").rect(), rect);
        assert_eq!(h.get_by_role(Role::SpinButton).rect(), field);

        h.state_mut().1 = Some(20.0);
        h.run_steps(2);
        assert!(!h.get_by_label("Restore original size").accesskit_node().is_disabled());
        assert_eq!(h.get_by_label("Restore original size").rect(), rect);
        h.get_by_label("Restore original size").click();
        h.run_steps(2);
        assert_eq!(h.state().0, 20.0);
        assert!(h.get_by_label("Restore original size").accesskit_node().is_disabled());
        assert_eq!(h.get_by_role(Role::SpinButton).rect(), field);
    }
}

/// A sampled tip may exceed the editable range; compare against the value that will be restored.
#[test]
fn size_restore_compares_the_clamped_target_without_rounding_idle_values() {
    for (original, expected) in [(0.0, 1.0), (8000.0, 2500.0), (20.0, 20.0)] {
        let mut h = row(theme::ThemeKind::ProMedium, 37.25, Some(original));
        h.get_by_label("Restore original size").click();
        h.run_steps(2);
        assert_eq!(h.state().0, expected);
        assert!(h.get_by_label("Restore original size").accesskit_node().is_disabled());
        h.run_steps(5);
        assert_eq!(h.state().0, expected);
    }
}
