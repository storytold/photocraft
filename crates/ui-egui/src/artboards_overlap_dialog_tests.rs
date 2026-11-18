//! #13629: the Artboards to Files dialog has the radio pair Include Overlapping Areas (selected
//! by default) / Artboard Content Only right below the prefix, and OK passes the choice to
//! `file.export.artboardsToFiles` as `content`. Layer Comps to Files has no such choice.
use crate::PhotocraftApp;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

const ARTBOARDS: &str = "file.export.artboardsToFiles";
const OVERLAPPING: &str = "Include Overlapping Areas";
const CONTENT_ONLY: &str = "Artboard Content Only";

type H = Harness<'static, Box<PhotocraftApp>>;

fn harness() -> H {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.artboard", json!({"rect": [0, 0, 64, 64]})).unwrap();
    app.run("layerComp.new", json!({"name": "Comp"})).unwrap();
    app.sync_views();
    let h = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_max_steps(64)
        .with_context(app.ctx().clone())
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h
}

fn open(h: &mut H, command: &str) -> DialogId {
    let ctx = h.ctx.clone();
    let reply = crate::menus::invoke(h.state_mut(), &ctx, command, json!({})).unwrap();
    let want = reply["dialog"].as_str().unwrap().to_owned();
    let dialog = h.state().framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|d| d.value().to_string() == want).unwrap();
    h.run_steps(4);
    dialog
}

fn field(h: &H, dialog: DialogId, key: &str) -> Option<Value> {
    let form = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(dialog)?;
    serde_json::to_value(form.fields()).ok()?.get(key).cloned()
}

fn selected(h: &H, label: &str) -> bool {
    h.get_by_role_and_label(Role::RadioButton, label).accesskit_node().toggled() == Some(Toggled::True)
}

/// The top edge of the first node labelled `label`.
fn top(h: &H, label: &str) -> f32 {
    h.query_all_by_label(label).next().unwrap_or_else(|| panic!("{label} is shown")).rect().top()
}

#[test]
fn the_dialog_offers_the_two_radios_below_the_prefix_overlapping_first_and_selected() {
    let mut h = harness();
    let dialog = open(&mut h, ARTBOARDS);
    let radios: Vec<String> = h.query_all_by_role(Role::RadioButton).filter_map(|n| n.accesskit_node().label()).collect();
    assert_eq!(radios, [OVERLAPPING, CONTENT_ONLY]);
    assert!(selected(&h, OVERLAPPING) && !selected(&h, CONTENT_ONLY));
    let radio = top(&h, OVERLAPPING);
    assert!(top(&h, "File Name Prefix") < radio && radio < top(&h, "File Type"), "the radios sit between the prefix and the file type");
    assert_eq!(field(&h, dialog, "content"), Some(json!("overlapping")));
}

#[test]
fn clicking_artboard_content_only_passes_it_to_the_command() {
    let mut h = harness();
    let dialog = open(&mut h, ARTBOARDS);
    h.get_by_role_and_label(Role::RadioButton, CONTENT_ONLY).click();
    h.run_steps(4);
    assert!(selected(&h, CONTENT_ONLY) && !selected(&h, OVERLAPPING));
    assert_eq!(field(&h, dialog, "content"), Some(json!("artboard")));
    h.get_by_role_and_label(Role::RadioButton, OVERLAPPING).click();
    h.run_steps(4);
    assert_eq!(field(&h, dialog, "content"), Some(json!("overlapping")));
}

#[test]
fn layer_comps_to_files_has_no_overlap_choice() {
    let mut h = harness();
    let dialog = open(&mut h, "file.export.layerCompsToFiles");
    assert_eq!(h.query_all_by_role(Role::RadioButton).count(), 0);
    assert_eq!(field(&h, dialog, "content"), None);
}
