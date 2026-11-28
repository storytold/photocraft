//! The two Show Tooltips settings reach the framework's component tooltips (#16833 review): either one off hides a
//! hovered control's tip, and with both on the tip shows as it did. The options bar of the Clone Stamp is the host:
//! its cells carry the reference application's tooltips.

use crate::PhotocraftApp;
use crate::app::framework::BuildApp as _;
use crate::state::Tool;
use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

/// The Aligned cell's tooltip on the Clone Stamp's bar.
const TIP: &str = "Keep the same source offset between strokes";

/// The Clone Stamp's options bar on an app whose Show Tooltips settings are `interface` and `tools`.
fn bar(interface: bool, tools: bool) -> Harness<'static, Box<PhotocraftApp>> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.run("prefs.set", json!({"path": "interface.showTooltips", "value": interface})).unwrap();
    app.run("prefs.set", json!({"path": "tools.showTooltips", "value": tools})).unwrap();
    assert_eq!(app.session.prefs().interface.show_tooltips, interface, "the Interface setting landed");
    assert_eq!(app.session.prefs().tools.show_tooltips, tools, "the Tools setting landed");
    app.ui.tool = Tool::CloneStamp;
    let mut h = Harness::builder().with_size(vec2(1800.0, 300.0)).with_step_dt(1.0 / 60.0).build_app(
        |ui, app| {
            crate::panels::options_bar(app, ui);
            crate::dialogs::run(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

/// Rest the pointer on the Aligned cell long enough for the framework's delay, and report which tips show.
fn hovered_tips(interface: bool, tools: bool) -> Vec<String> {
    let mut h = bar(interface, tools);
    let pos = h.get_by_label("Aligned").rect().center();
    h.hover_at(pos);
    h.run_steps(45);
    h.query_all_by_role(Role::Tooltip).map(|node| node.accesskit_node().label().unwrap_or_default().to_owned()).collect()
}

/// With both settings on - what the app starts with - the hovered cell shows its tooltip.
#[test]
fn both_settings_on_show_the_bars_tooltip_on_hover() {
    assert!(hovered_tips(true, true).iter().any(|tip| tip == TIP), "the hovered cell shows {TIP:?}");
}

/// The Interface page's Show Tooltips off hides it, like every other tooltip.
#[test]
fn the_interface_show_tooltips_setting_off_hides_the_bars_tooltip() {
    let tips = hovered_tips(false, true);
    assert!(!tips.iter().any(|tip| tip == TIP), "the Interface setting off hides {TIP:?}: {tips:?}");
}

/// The Tools page's Show Tooltips off hides it too; either setting off is enough.
#[test]
fn the_tools_show_tooltips_setting_off_hides_the_bars_tooltip() {
    let tips = hovered_tips(true, false);
    assert!(!tips.iter().any(|tip| tip == TIP), "the Tools setting off hides {TIP:?}: {tips:?}");
}
