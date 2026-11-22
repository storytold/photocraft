//! #15977: Find Sun's dialog as a keyboard or screen-reader user meets it, its Copy buttons, and
//! its reruns of `image.analysis.findSun` when an option changes.
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use craft::ui::dialog::StackDialog as _;

use super::FindSunDialog;
use crate::PhotocraftApp;

type H = Harness<'static, Box<PhotocraftApp>>;

/// A 32-bit 2:1 sky with a small bright sun at azimuth 90°, elevation 45°, and the dialog open.
fn harness() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 128, "height": 64, "depth": 32})).unwrap();
    app.session
        .edit("sky", |doc, active| {
            let b = doc.bounds();
            let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            let fmt = surf.format();
            let mut data = Vec::new();
            for y in b.y0..b.y1 {
                for x in b.x0..b.x1 {
                    let sun = (x - 96).pow(2) + (y - 16).pow(2) <= 1;
                    data.extend(photocraft_raster::from_rgba(&fmt, if sun { [500.0, 480.0, 450.0, 1.0] } else { [0.3, 0.45, 0.7, 1.0] }));
                }
            }
            surf.write_region(b, &data);
            Ok(())
        })
        .unwrap();
    app.sync_views();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "image.analysis.findSun", json!({})).unwrap();
    h.run_steps(4);
    h
}

fn result(h: &H) -> Value {
    h.state().framework.dialogs.get::<FindSunDialog>().unwrap().result.clone().unwrap()
}

#[test]
fn the_choices_and_copy_buttons_are_named_and_copy_rotation_copies_the_light_rotation() {
    let mut h = harness();
    for name in ["Copy direction", "Copy rotation", "Copy colour", "Copy intensity"] {
        assert_eq!(h.query_all_by_role_and_label(Role::Button, name).count(), 1, "one {name} button");
    }
    // Each choice is a combo box named by the label shown beside it.
    for name in ["Axes", "Image centre faces"] {
        let roles: Vec<Role> = h.query_all_by_label(name).map(|n| n.accesskit_node().role()).collect();
        assert!(roles.contains(&Role::ComboBox), "{name}: {roles:?}");
    }
    h.get_by_role_and_label(Role::Button, "Copy rotation").click();
    let mut copied = Vec::new();
    for _ in 0..4 {
        h.step();
        copied.extend(h.output().platform_output.commands.iter().filter_map(|c| if let egui::OutputCommand::CopyText(t) = c { Some(t.clone()) } else { None }));
    }
    assert_eq!(copied.len(), 1, "{copied:?}");
    assert!(copied[0].starts_with("Quaternion(") && copied[0].ends_with(')'), "{copied:?}");
}

#[test]
fn changing_which_way_the_image_centre_faces_measures_again() {
    let mut h = harness();
    let yaw = |r: &Value| r["lightRotation"]["euler"][1].as_f64().unwrap();
    let before = yaw(&result(&h));
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.get_mut::<FindSunDialog>().unwrap().set_json_field(&ctx, "facing", json!(0)).unwrap();
    h.run_steps(4);
    let after = yaw(&result(&h));
    // +X to +Z turns the scene's sun, and the light, a quarter turn.
    let turned = (before - after).rem_euclid(360.0);
    assert!((turned - 90.0).abs() < 2.0, "before {before} after {after}");
}

#[test]
fn an_error_keeps_find_automatically_and_a_sun_put_by_hand_stays_with_its_document() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    let set_at = |h: &mut H, at: Value| {
        h.state_mut().framework.dialogs.get_mut::<FindSunDialog>().unwrap().set_json_field(&ctx, "at", at).unwrap();
        h.run_steps(4);
    };
    // Off the image: the command refuses, and the way back stays in the dialog.
    set_at(&mut h, json!([500, 10]));
    assert!(h.state().framework.dialogs.get::<FindSunDialog>().unwrap().result.is_err());
    h.get_by_role_and_label(Role::Button, "Find Automatically").click();
    h.run_steps(4);
    let dialog = h.state().framework.dialogs.get::<FindSunDialog>().unwrap();
    assert!(dialog.options.at.is_none() && dialog.result.is_ok());

    // A sun put on this image isn't carried to the next one.
    set_at(&mut h, json!([10, 10]));
    assert_eq!(result(&h)["picked"], json!(true));
    h.state_mut().run("file.new", json!({"width": 128, "height": 64, "depth": 32})).unwrap();
    h.run_steps(4);
    let dialog = h.state().framework.dialogs.get::<FindSunDialog>().unwrap();
    assert!(dialog.options.at.is_none(), "{:?}", dialog.options.at);
    assert_eq!(result(&h)["picked"], json!(false));

    // A press on a newly active document before the dialog has measured it keeps its sun.
    h.state_mut().run("file.new", json!({"width": 128, "height": 64, "depth": 32})).unwrap();
    assert!(super::place(h.state_mut(), [20.0, 20.0]));
    h.run_steps(4);
    assert_eq!(h.state().framework.dialogs.get::<FindSunDialog>().unwrap().options.at, Some([20.0, 20.0]));
    assert_eq!(result(&h)["picked"], json!(true));
}
