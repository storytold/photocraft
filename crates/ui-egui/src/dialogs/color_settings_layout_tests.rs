//! #14227: Edit › Color Settings opens the reference application's dialog: the four groups
//! Working Spaces, Color Management Policies, Conversion Options and Advanced Controls with the
//! reference's control names in its order, the policy menus listing Off · Preserve Embedded Profiles ·
//! Convert to Working RGB/CMYK/Gray, the intent menu listing Perceptual · Saturation · Relative
//! Colorimetric · Absolute Colorimetric, and every working-space entry named by its profile
//! description instead of its internal id. The command's params keep taking the ids, and the settings
//! store reads back what the dialog chose.
//!
//! Before: one column of rows named from the parameter keys ("Working Rgb", "Ask On Mismatch",
//! "Spot Dot Gain") and menus listing "Srgb" and "Preserve · Convert · Off".

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{HostedParamDialog, ParamKind};
use egui::accesskit::{Role, Toggled};
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_engine::color_cmds::{ColorExt, ColorSettings, Policy};
use serde_json::{Value, json};

use super::*;
use crate::PhotocraftApp;

type H = Harness<'static, Box<PhotocraftApp>>;

/// The app storing `settings`, with the dialog open from the Edit menu and drawn.
fn opened(settings: ColorSettings) -> (H, DialogId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 24, "background": "white"})).unwrap();
    app.session.color_mut().settings = settings;
    let ctx = app.ctx().clone();
    let reply = crate::menus::invoke(&mut app, &ctx, COMMAND, json!({})).unwrap();
    let want = reply["dialog"].as_str().expect("the reply names a dialog").to_owned();
    let mut h = Harness::builder()
        .with_context(ctx)
        .with_size(vec2(1440.0, 900.0))
        .with_max_steps(64)
        .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    let id = h.state().framework.dialogs.open_dialogs().into_iter().map(|d| d.id).find(|d| d.value().to_string() == want).expect("the dialog is open");
    h.run_steps(4);
    (h, id)
}

/// The dialog open on PhotoCraft's own defaults.
fn default_dialog() -> (H, DialogId) {
    opened(ColorSettings::default())
}

/// Labels as the language the reference was measured in.
fn in_english<T>(run: impl FnOnce() -> T) -> T {
    crate::i18n::with_language(crate::i18n::Lang::EN, run)
}

/// The rects of every node named `label`, in reading order (top, then left).
fn rects(h: &H, role: Option<Role>, label: &str) -> Vec<egui::Rect> {
    let nodes: Vec<_> = match role {
        Some(role) => h.query_all_by_role_and_label(role, label).collect(),
        None => h.query_all_by_label(label).collect(),
    };
    assert!(!nodes.is_empty(), "nothing named {label:?} in the dialog");
    let mut list: Vec<egui::Rect> = nodes.iter().map(|n| n.rect()).collect();
    list.sort_by(|a, b| a.top().total_cmp(&b.top()).then(a.left().total_cmp(&b.left())));
    list
}

/// The top of the first node named `label` in reading order.
fn top(h: &H, label: &str) -> f32 {
    rects(h, None, label)[0].top()
}

/// The menu boxes named `label`, in the order the dialog builds them, with the entry each shows.
/// The working-space menus and the policy menus share the reference's short names, so the tests take
/// them by place: the working space's first, the policy's second.
fn menus(h: &H, label: &str) -> Vec<String> {
    h.query_all_by_role_and_label(Role::ComboBox, label).map(|n| n.value().unwrap_or_default().to_owned()).collect()
}

/// Opens the `at`:th menu named `label`.
fn open_menu(h: &mut H, label: &str, at: usize) {
    h.query_all_by_role_and_label(Role::ComboBox, label).nth(at).unwrap_or_else(|| panic!("no menu named {label:?}")).click();
    h.run_steps(3);
}

/// The entry `name`, picked from the menu that is open; a menu entry is the node built last.
fn pick(h: &mut H, name: &str) {
    h.query_all_by_label(name).last().unwrap_or_else(|| panic!("the open menu has no {name:?} entry")).click();
    h.run_steps(3);
}

/// The tops of the entries `entries`, as the open menu lists them.
fn entry_tops(h: &H, entries: &[&str]) -> Vec<f32> {
    entries.iter().map(|e| h.query_all_by_label(e).last().unwrap_or_else(|| panic!("the open menu has no {e:?} entry")).rect().top()).collect()
}

/// The open dialog's values as the command would take them.
fn sent(h: &H, id: DialogId) -> Value {
    let fields = h.state().framework.dialogs.get_by_id::<HostedParamDialog>(id).unwrap().fields().clone();
    crate::filter_dialog::command_params(COMMAND, &fields)
}

/// The option ids and labels of the row `key`, as [`rows`] builds it.
fn options(key: &str) -> Vec<(String, String)> {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let rows = rows(&app);
    let row = rows.iter().find(|p| p.key == key).unwrap_or_else(|| panic!("no {key} row"));
    let ParamKind::Choice { options, .. } = &row.kind else { panic!("{key} is no menu") };
    options.iter().map(|(id, label)| (id.clone(), label.as_str().to_owned())).collect()
}

#[test]
fn the_four_groups_stand_in_the_reference_order() {
    in_english(|| {
        let (h, _) = default_dialog();
        let groups = ["Working Spaces", "Color Management Policies", "Conversion Options", "Advanced Controls"];
        let mut placed: Vec<(f32, f32, &str)> = Vec::new();
        for group in groups {
            let rect = rects(&h, None, group)[0];
            placed.push((rect.top(), rect.left(), group));
        }
        placed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let order: Vec<&str> = placed.iter().map(|(_, _, group)| *group).collect();
        assert_eq!(order, groups, "the groups read in the reference's order");
        for generated in ["Working Rgb", "Policy Rgb", "Ask On Mismatch", "Ask On Missing", "Spot Dot Gain"] {
            assert!(h.query_all_by_label(generated).next().is_none(), "the generated caption {generated:?} is gone");
        }
    });
}

#[test]
fn each_group_holds_the_reference_controls_in_its_order() {
    in_english(|| {
        let (h, _) = default_dialog();
        // Working Spaces: RGB, CMYK, Gray, Spot. Each of the three short names is used twice, here and
        // in Color Management Policies; the working space's menu stands first.
        for name in ["RGB", "CMYK", "Gray"] {
            let list = rects(&h, Some(Role::ComboBox), name);
            assert_eq!(list.len(), 2, "{name}: the working space's menu and the policy's");
            assert!(list[0].top() < list[1].top() || list[0].right() <= list[1].left(), "{name}: the working space's menu comes first");
        }
        let working = [top(&h, "RGB"), top(&h, "CMYK"), top(&h, "Gray"), top(&h, "Spot")];
        for pair in working.windows(2) {
            assert!(pair[0] < pair[1], "Working Spaces order: {pair:?}");
        }
        // Color Management Policies: the three menus, then the reference's two labelled groups of boxes.
        let policies = [
            rects(&h, Some(Role::ComboBox), "RGB")[1].top(),
            rects(&h, Some(Role::ComboBox), "CMYK")[1].top(),
            rects(&h, Some(Role::ComboBox), "Gray")[1].top(),
        ];
        for pair in policies.windows(2) {
            assert!(pair[0] < pair[1], "Color Management Policies order: {pair:?}");
        }
        let mismatches = top(&h, "Profile Mismatches:");
        let missing = top(&h, "Missing Profiles:");
        let ask = rects(&h, Some(Role::CheckBox), "Ask When Opening");
        assert_eq!(ask.len(), 2, "one Ask When Opening under each heading");
        let paste = top(&h, "Ask When Pasting");
        assert!(mismatches < ask[0].top() && ask[0].top() < paste && paste < missing && missing < ask[1].top(), "the boxes stand under their own headings");
        // Conversion Options: Intent, then the two check boxes with the reference's names.
        let conversion = [top(&h, "Intent"), top(&h, "Use Black Point Compensation"), top(&h, "Use Dither (8-bit/channel images)")];
        for pair in conversion.windows(2) {
            assert!(pair[0] < pair[1], "Conversion Options order: {pair:?}");
        }
        // Advanced Controls: the gamma row, then what is PhotoCraft's own, below it.
        assert!(top(&h, "Blend Text Colors Using Gamma") < top(&h, "Reset"), "the extra controls stand under the reference ones");
        assert!(top(&h, "Reset") < top(&h, "Monitor Profile"), "and the monitor row below those");
    });
}

#[test]
fn the_policy_menus_list_the_reference_entries_in_their_order() {
    in_english(|| {
        for (key, space) in [("policyRgb", "RGB"), ("policyCmyk", "CMYK"), ("policyGray", "Gray")] {
            let built = options(key);
            let labels: Vec<String> = built.iter().map(|(_, label)| label.clone()).collect();
            assert_eq!(labels, vec!["Off".to_owned(), "Preserve Embedded Profiles".to_owned(), format!("Convert to Working {space}")], "{key}");
            assert_eq!(built.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["off", "preserve", "convert"], "{key} keeps the command's ids");
        }
        let (mut h, _) = default_dialog();
        assert_eq!(menus(&h, "RGB").len(), 2, "the working space's menu and the policy's");
        assert_eq!(menus(&h, "RGB")[1], "Preserve Embedded Profiles", "the stored policy shows as the reference names it");
        open_menu(&mut h, "RGB", 1);
        let tops = entry_tops(&h, &["Off", "Preserve Embedded Profiles", "Convert to Working RGB"]);
        assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "Off · Preserve Embedded Profiles · Convert to Working RGB: {tops:?}");
    });
}

#[test]
fn the_intent_menu_lists_the_reference_entries_in_their_order() {
    in_english(|| {
        let options = options("intent");
        assert_eq!(
            options.iter().map(|(_, label)| label.as_str()).collect::<Vec<_>>(),
            ["Perceptual", "Saturation", "Relative Colorimetric", "Absolute Colorimetric"]
        );
        assert_eq!(
            options.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            ["perceptual", "saturation", "relative", "absolute"],
            "the ids stay the command's"
        );
        let (mut h, _) = default_dialog();
        assert_eq!(menus(&h, "Intent")[0], "Relative Colorimetric", "the stored intent shows as the reference names it");
        open_menu(&mut h, "Intent", 0);
        let tops = entry_tops(&h, &["Perceptual", "Saturation", "Relative Colorimetric", "Absolute Colorimetric"]);
        assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "the reference's order: {tops:?}");
    });
}

#[test]
fn the_working_space_menus_name_their_profiles_by_description() {
    in_english(|| {
        let (h, id) = default_dialog();
        // The stored settings show as profile descriptions, never as the ids the command takes.
        assert_eq!(menus(&h, "RGB")[0], "sRGB IEC61966-2.1");
        assert_eq!(menus(&h, "Gray")[0], "Dot Gain 20%-compatible (gamma 1.732, PhotoCraft)");
        for key in ["workingRgb", "workingCmyk", "workingGray"] {
            for (id, label) in options(key) {
                assert_ne!(label, id, "{key}: {id} is not shown as its id");
            }
        }
        // And OK still takes the id.
        assert_eq!(sent(&h, id)["workingRgb"], "srgb");
    });
}

#[test]
fn the_ask_boxes_carry_the_reference_names_and_the_stored_values() {
    in_english(|| {
        let stored = ColorSettings { ask_on_missing: true, ..ColorSettings::default() };
        let (h, _) = opened(stored);
        let ask = h.query_all_by_role_and_label(Role::CheckBox, "Ask When Opening").map(|n| n.accesskit_node().toggled()).collect::<Vec<_>>();
        assert_eq!(ask, vec![Some(Toggled::True), Some(Toggled::True)], "one box under each heading, both stored as asking");
        let paste = h.get_by_role_and_label(Role::CheckBox, "Ask When Pasting").accesskit_node();
        assert_eq!(paste.toggled(), Some(Toggled::True), "askOnPaste");
        for generated in ["Ask On Mismatch", "Ask On Paste", "Ask On Missing"] {
            assert!(h.query_all_by_label(generated).next().is_none(), "{generated} is gone");
        }
    });
}

#[test]
fn a_policy_chosen_in_the_menu_stores_that_policy() {
    in_english(|| {
        let (mut h, id) = default_dialog();
        open_menu(&mut h, "RGB", 1);
        pick(&mut h, "Convert to Working RGB");
        assert_eq!(menus(&h, "RGB")[1], "Convert to Working RGB", "the menu shows the pick");
        assert_eq!(sent(&h, id)["policyRgb"], "convert", "and the command takes its id");
        crate::dialogs::testing::confirm(h.state_mut(), id).unwrap();
        h.run_steps(3);
        let stored = h.state().session.color().settings.clone();
        assert_eq!(stored.policy_rgb, Policy::Convert, "the policy is stored");
        assert_eq!((stored.policy_cmyk, stored.policy_gray), (Policy::Preserve, Policy::Preserve), "the others stay");
        assert_eq!(stored.working_rgb, "srgb", "and the working space was not touched");
    });
}

#[test]
fn an_intent_chosen_in_the_menu_stores_that_intent() {
    in_english(|| {
        let (mut h, id) = default_dialog();
        open_menu(&mut h, "Intent", 0);
        pick(&mut h, "Absolute Colorimetric");
        assert_eq!(sent(&h, id)["intent"], "absolute");
        crate::dialogs::testing::confirm(h.state_mut(), id).unwrap();
        h.run_steps(3);
        assert_eq!(h.state().session.color().settings.intent, "absolute");
    });
}

#[test]
fn cancel_leaves_the_stored_settings_alone() {
    in_english(|| {
        let (mut h, id) = default_dialog();
        h.get_by_role_and_label(Role::CheckBox, "Use Dither (8-bit/channel images)").click();
        h.run_steps(3);
        assert_eq!(sent(&h, id)["dither"], json!(false), "the dialog shows the change");
        crate::dialogs::testing::cancel(h.state_mut(), id).unwrap();
        h.run_steps(3);
        let stored = h.state().session.color().settings.clone();
        assert!(stored.dither, "Cancel writes nothing");
        assert_eq!(stored.policy_rgb, Policy::Preserve);
    });
}

#[test]
fn the_monitor_profile_and_its_line_stay_below_the_reference_controls() {
    in_english(|| {
        let (h, id) = default_dialog();
        assert_eq!(menus(&h, "Monitor Profile").len(), 1, "the Monitor Profile menu stays");
        assert_eq!(menus(&h, "Monitor Profile")[0], "Auto", "and keeps its own entry for the main display");
        assert!(top(&h, "Monitor Profile") > top(&h, "Blend Text Colors Using Gamma"), "below the reference controls");
        let notes = h.query_all_by_label_contains("Monitor profile in use").count();
        assert_eq!(notes, 1, "the line naming the profile in use stays");
        assert_eq!(sent(&h, id)["monitorProfile"], "auto", "and the command still takes its id");
    });
}

/// The generic parse still names the rows of the other command dialogs; only Color Settings builds
/// its own.
#[test]
fn another_schema_dialog_keeps_its_generated_rows() {
    in_english(|| {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 24, "background": "white"})).unwrap();
        crate::filter_dialog::open(&mut app, "filter.blur.gaussianBlur").unwrap();
        let mut h = Harness::builder()
            .with_context(app.ctx().clone())
            .with_size(vec2(1440.0, 900.0))
            .with_max_steps(64)
            .build_ui_state(|ui, app: &mut Box<PhotocraftApp>| crate::dialogs::run(app, ui), app);
        PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.run_steps(4);
        assert!(h.query_all_by_label("Radius").next().is_some(), "Gaussian Blur keeps the caption the params doc gives it");
        let rows = h.query_all_by_role_and_label(Role::Slider, "Radius").count() + h.query_all_by_role_and_label(Role::SpinButton, "Radius").count();
        assert!(rows > 0, "and the range row it built for `radius`");
    });
}
