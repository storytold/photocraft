//! #13629: the Artboards to Files dialog has the radio pair Include Overlapping Areas (selected
//! by default) / Artboard Content Only right below the prefix, and OK passes the choice to
//! `file.export.artboardsToFiles` as `content`. Layer Comps to Files has no such choice.
use crate::PhotocraftApp;
use egui::accesskit::{Role, Toggled};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

const ARTBOARDS: &str = "file.export.artboardsToFiles";
const OVERLAPPING: &str = "Include Overlapping Areas";
const CONTENT_ONLY: &str = "Artboard Content Only";

fn harness() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    });
    let app = h.state_mut();
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [0, 0, 64, 64]})).unwrap();
    app.run("layerComp.new", json!({"name": "Comp"})).unwrap();
    app.sync_views();
    h
}

fn open(h: &mut Harness<'_, PhotocraftApp>, id: &str) -> u64 {
    let ctx = h.ctx.clone();
    let dialog = crate::menus::invoke(h.state_mut(), &ctx, id, json!({})).unwrap()["dialog"].as_u64().unwrap();
    h.run_steps(4);
    dialog
}

fn selected(h: &Harness<'_, PhotocraftApp>, label: &str) -> bool {
    h.get_by_role_and_label(Role::RadioButton, label).accesskit_node().toggled() == Some(Toggled::True)
}

#[test]
fn the_dialog_offers_the_two_radios_below_the_prefix_overlapping_first_and_selected() {
    let mut h = harness();
    let dialog = open(&mut h, ARTBOARDS);
    let radios: Vec<String> = h.query_all_by_role(Role::RadioButton).filter_map(|n| n.accesskit_node().label()).collect();
    assert_eq!(radios, [OVERLAPPING, CONTENT_ONLY]);
    assert!(selected(&h, OVERLAPPING) && !selected(&h, CONTENT_ONLY));
    let fields = &h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
    let keys = crate::view_cmds::form_keys(fields);
    assert_eq!(keys.iter().position(|k| k == "content"), keys.iter().position(|k| k == "prefix").map(|i| i + 1), "{keys:?}");
    assert_eq!(crate::filter_dialog::params_of(fields)["content"], "overlapping");
}

#[test]
fn clicking_artboard_content_only_passes_it_to_the_command() {
    let mut h = harness();
    let dialog = open(&mut h, ARTBOARDS);
    h.get_by_role_and_label(Role::RadioButton, CONTENT_ONLY).click();
    h.run_steps(4);
    assert!(selected(&h, CONTENT_ONLY) && !selected(&h, OVERLAPPING));
    let fields = &h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
    assert_eq!(crate::filter_dialog::params_of(fields)["content"], "artboard");
    h.get_by_role_and_label(Role::RadioButton, OVERLAPPING).click();
    h.run_steps(4);
    let fields = &h.state_mut().ui.dialog_mut(dialog).unwrap().fields;
    assert_eq!(crate::filter_dialog::params_of(fields)["content"], "overlapping");
}

#[test]
fn layer_comps_to_files_has_no_overlap_choice() {
    let mut h = harness();
    let dialog = open(&mut h, "file.export.layerCompsToFiles");
    assert_eq!(h.query_all_by_role(Role::RadioButton).count(), 0);
    assert!(h.state_mut().ui.dialog_mut(dialog).unwrap().fields.get("content").is_none());
}
