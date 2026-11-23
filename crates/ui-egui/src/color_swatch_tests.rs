//! Colour swatches open PhotoCraft's Color Picker, not egui's compact popup (#2144: "use the same
//! color picker throughout the app"): filter dialog colours, Preferences colours, the Note and
//! Count tools' colours and an artboard's custom background. OK sets that colour (in sRGB, as
//! everywhere else); the foreground is untouched.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use craft::ui::dialog::{DialogId, OpenDialog};

use crate::color_picker_ui::title_for;

use crate::PhotocraftApp;
use crate::state::Tool;

const PICKED: &str = "#336699";
const PICKED_RGB: [f32; 3] = [0x33 as f32 / 255.0, 0x66 as f32 / 255.0, 0x99 as f32 / 255.0];

fn harness(setup: impl FnOnce(&mut PhotocraftApp) + 'static) -> Harness<'static, Box<PhotocraftApp>> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::AppTheme::pro());
        let mut app = PhotocraftApp::with_context(cc.egui_ctx.clone(), photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
        setup(&mut app);
        app
    });
    h.run_steps(6);
    h
}

/// Clicks the swatch called `label` (a colour button) and answers the Color Picker it opens with [`PICKED`]; the
/// picker shows `title`. Panics when no Color Picker opens.
fn pick(h: &mut Harness<'_, Box<PhotocraftApp>>, label: &str, title: &str) {
    // Panels scroll: bring the swatch into view first.
    h.get_by_role_and_label(Role::ColorWell, label).scroll_to_me();
    h.run_steps(2);
    h.get_by_role_and_label(Role::ColorWell, label).click();
    answer(h, label, Some(title));
}

/// The hosted dialog `id` as automation lists it: its key and its JSON fields.
fn open_dialog(app: &PhotocraftApp, id: DialogId) -> OpenDialog {
    app.framework.dialogs.open_dialogs().into_iter().find(|d| d.id == id).expect("the dialog is open")
}

/// Answers the Color Picker a swatch click just opened with [`PICKED`]; when `title` is given the picker shows it.
fn answer(h: &mut Harness<'_, Box<PhotocraftApp>>, label: &str, title: Option<&str>) {
    let fg = h.state().session.tools.foreground;
    h.run_steps(4);
    let id = crate::color_picker_ui::top(h.state()).unwrap_or_else(|| panic!("the {label} swatch opens the Color Picker"));
    assert!(!egui::Popup::is_any_open(&h.ctx), "{label}: no egui colour popup");
    assert!(open_dialog(h.state(), id).key.is_some(), "{label}: the picker is a keyed hosted dialog");
    if let Some(title) = title {
        assert!(h.query_all_by_label(title).next().is_some(), "{label}: the picker is titled {title}");
    }
    let ctx = h.ctx.clone();
    h.state_mut().framework.dialogs.set_json_field(&ctx, id, "color", json!(PICKED)).unwrap();
    crate::dialogs::testing::confirm(h.state_mut(), id).unwrap();
    h.run_steps(3);
    assert!(crate::color_picker_ui::top(h.state()).is_none(), "{label}: OK closes the picker");
    assert_eq!(h.state().session.tools.foreground, fg, "{label}: the foreground is untouched");
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1.0 / 255.0)
}

#[test]
fn filter_dialog_colours_open_the_color_picker() {
    for (cmd, key, label) in [("filter.other.colorToAlpha", "color", "Color"), ("filter.render.pictureFrame", "vineColor", "Vine Color")] {
        let mut h = harness(|_| {});
        let ctx = h.ctx.clone();
        crate::menus::invoke(h.state_mut(), &ctx, cmd, json!({})).unwrap();
        h.run_steps(4);
        let id = h.state().framework.dialogs.stack().top().expect("the filter dialog opens");
        pick(&mut h, label, &title_for(label));
        let d = open_dialog(h.state(), id);
        // A form reads a colour as its channels; `PICKED` is the hex the picker was given.
        let shown: Option<Vec<f32>> = d.fields.as_ref().and_then(|f| f.get(key)).and_then(|v| serde_json::from_value(v.clone()).ok());
        let want = PICKED_RGB;
        assert!(shown.is_some_and(|c| c.len() == 3 && close([c[0], c[1], c[2]], want)), "{cmd}: {key}");
    }
}

#[test]
fn preferences_colours_open_the_color_picker() {
    let mut h = harness(|_| {});
    crate::prefs_ui::open_preferences(h.state_mut(), "guidesGridAndSlices");
    h.run_steps(4);
    // The window's draft reaches the saved preferences on Apply: one colour key changes then.
    let values = |h: &Harness<'_, Box<PhotocraftApp>>| h.state().session.prefs().to_json()["guidesGridAndSlices"].clone();
    let before = values(&h);
    h.get_all_by_role(Role::ColorWell).next().expect("a colour swatch").click();
    answer(&mut h, "the first Preferences colour", None);
    h.get_by_label("Apply").click();
    h.run_steps(3);
    let after = values(&h);
    let changed: Vec<&str> = after.as_object().unwrap().iter().filter(|(k, v)| before.get(k.as_str()) != Some(v)).map(|(k, _)| k.as_str()).collect();
    assert_eq!(changed.len(), 1, "one colour changed: {changed:?}");
    assert_eq!(after[changed[0]], json!(PICKED));
}

#[test]
fn note_and_count_colours_open_the_color_picker() {
    let mut h = harness(|app| app.ui.tool = Tool::Note);
    pick(&mut h, "Color", &title_for("Color"));
    assert!(close(h.state().ui.analysis.note_color, PICKED_RGB), "note colour {:?}", h.state().ui.analysis.note_color);

    let mut h = harness(|app| {
        app.ui.tool = Tool::Count;
        app.run("count.newGroup", json!({})).unwrap();
    });
    // The Count swatch is "Count group color", its picker "Color Picker (Count Color)" (#11765).
    pick(&mut h, "Count group color", &title_for("Count Color"));
    let st = h.state().session.active().unwrap();
    let m = &st.doc.measurement;
    let g = &m.count_groups[m.active_count_group];
    assert!(close(g.color.to_rgb(), PICKED_RGB), "count group colour {:?}", g.color.to_rgb());
}
