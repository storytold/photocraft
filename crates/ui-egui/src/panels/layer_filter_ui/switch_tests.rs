//! #10877: per-open-document filter ownership, switch and current-main reader integration.
use egui::vec2;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use serde_json::{Value, json};

use super::{Filter, apply, patched};
use crate::PhotocraftApp;

fn app() -> PhotocraftApp {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width":8,"height":8})).unwrap();
    for name in ["Red", "Blue"] {
        session.execute("layer.new.layer", json!({"name":name})).unwrap();
    }
    PhotocraftApp::new(session, crate::Services::default())
}

fn set(app: &mut PhotocraftApp, patch: Value) {
    let filter = patched(&Filter::of(app), &patch).unwrap();
    apply(app, filter);
}

fn listed(app: &PhotocraftApp) -> Vec<String> {
    let filter = Filter::of(app);
    let doc = &app.session.active().unwrap().doc;
    let context = super::FilterContext::of_app(app, &filter);
    crate::layer_tree_ui::display_rows(doc, filter.by != super::FilterBy::Artboard && filter.active_with(&context))
        .into_iter()
        .filter(|(_, l)| filter.shows_with(l, &context))
        .map(|(_, l)| l.name.clone())
        .collect()
}

fn call(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    let (request, _rx) = crate::control::ControlRequest::new(method, params);
    match crate::control::handle(app, &egui::Context::default(), &request) {
        crate::control::Outcome::Done(value) => value,
        _ => panic!("expected immediate {method} response"),
    }
}

#[test]
fn switching_off_keeps_criteria_and_lists_the_unfiltered_tree() {
    let mut app = app();
    let all = listed(&app);
    set(&mut app, json!({"by":"name","name":"Red"}));
    let before = Filter::of(&app);
    assert!(before.on);
    assert_eq!(listed(&app), ["Red"]);
    set(&mut app, json!({"on":false}));
    let mut off = before.clone();
    off.on = false;
    assert_eq!(Filter::of(&app), off);
    assert_eq!(listed(&app), all);
    set(&mut app, json!({"on":true}));
    assert_eq!(Filter::of(&app), before);
    assert_eq!(listed(&app), ["Red"]);
}

#[test]
fn changing_criteria_activates_or_deactivates_the_switch() {
    let mut f = Filter::default();
    assert!(!f.on);
    for (patch, on) in [
        (json!({"kinds":["pixel"]}), true),
        (json!({"kinds":[]}), false),
        (json!({"by":"name","name":"red"}), true),
        (json!({"name":""}), false),
        (json!({"by":"mode"}), true),
    ] {
        f = patched(&f, &patch).unwrap();
        assert_eq!(f.on, on, "{patch}");
    }
    f = patched(&f, &json!({"on":false})).unwrap();
    assert!(!f.on);
    assert!(!patched(&f, &json!({"mode":"Normal"})).unwrap().on, "unchanged criteria do not undo a manual off");
    assert!(patched(&f, &json!({"mode":"Multiply"})).unwrap().on);
    assert!(!patched(&f, &json!({"mode":"Multiply","on":false})).unwrap().on, "explicit switch wins in the same patch");
}

#[test]
fn documents_keep_whole_filters_when_indices_change_and_closed_state_is_dropped() {
    let mut app = app();
    let a = app.session.active().unwrap().doc.id;
    set(&mut app, json!({"by":"name","name":"Red","on":false}));
    let filter_a = Filter::of(&app);
    app.run("file.new", json!({"width":8,"height":8})).unwrap();
    let b = app.session.active().unwrap().doc.id;
    assert_eq!(Filter::of(&app), Filter::default());
    set(&mut app, json!({"by":"mode","mode":"Multiply"}));
    let filter_b = Filter::of(&app);
    app.run("document.activate", json!({"document":0})).unwrap();
    assert_eq!(Filter::of(&app), filter_a);
    app.run("document.activate", json!({"document":1})).unwrap();
    assert_eq!(Filter::of(&app), filter_b);
    app.session.close(0).unwrap();
    app.sync_views();
    assert!(!app.ui.layer_filters.contains_key(&a));
    assert_eq!(app.session.active().unwrap().doc.id, b);
    assert_eq!(Filter::of(&app), filter_b, "B is now index0 but keeps B's complete filter");
    app.run("file.new", json!({"width":8,"height":8})).unwrap();
    assert_eq!(Filter::of(&app), Filter::default());
}

#[test]
fn reopening_psd_or_pcraft_does_not_restore_the_closed_filter() {
    for name in ["filter.pcraft", "filter.psd"] {
        let mut app = app();
        app.services.import = Some(Box::new(|name, bytes, options| {
            photocraft_io::import_with_options(name, bytes, options).map(|result| (result.document, result.warnings)).map_err(|error| error.to_string())
        }));
        let id = app.session.active().unwrap().doc.id;
        set(&mut app, json!({"by":"name","name":"Red"}));
        let bytes = photocraft_io::export(&app.session.active().unwrap().doc, name, &Default::default()).unwrap().bytes;
        app.session.close(0).unwrap();
        // The read guard also protects the interval before the next sync_views cleanup.
        app.open_bytes(name, &bytes).unwrap();
        if name.ends_with(".pcraft") {
            assert_eq!(app.session.active().unwrap().doc.id, id, "the native format preserves document identity");
        }
        assert_eq!(Filter::of(&app), Filter::default(), "{name} opens with Kind/no criterion/Off");
        app.sync_views();
        assert!(!app.ui.layer_filters.contains_key(&id), "{name} cannot retain the closed document's filter");
    }
}

#[test]
fn ordinary_edits_and_undo_keep_the_documents_filter() {
    let mut app = app();
    set(&mut app, json!({"by":"name","name":"Red"}));
    let filter = Filter::of(&app);
    app.run("layer.new.layer", json!({"name":"Green"})).unwrap();
    assert_eq!(Filter::of(&app), filter);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(Filter::of(&app), filter);
}

#[test]
fn ui_set_inspect_and_invalid_patch_keep_one_atomic_document_filter() {
    let mut app = app();
    let result = call(&mut app, "ui.set", json!({"layerFilter":{"by":"name","name":"Red","on":false}}));
    assert_eq!(result["ok"], true, "{result}");
    let inspected = call(&mut app, "ui.inspect", json!({}));
    assert_eq!(inspected["result"]["layerFilter"]["on"], false);
    assert_eq!(inspected["result"]["layerFilter"]["name"], "Red");
    let before = Filter::of(&app);
    for invalid in [json!({"on":"yes"}), json!({"on":null}), json!({"on":true,"mode":"bad"}), json!({"name":"Blue","extra":true})] {
        assert_eq!(call(&mut app, "ui.set", json!({"layerFilter":invalid}))["ok"], false);
        assert_eq!(Filter::of(&app), before);
    }
    assert_eq!(call(&mut app, "ui.set", json!({"layerFilter":{"on":true}}))["ok"], true);
    assert_eq!(listed(&app), ["Red"]);
}

#[test]
fn filter_patch_without_a_document_does_not_seed_future_documents() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert_eq!(call(&mut app, "ui.set", json!({"layerFilter":{"by":"mode"}}))["ok"], false);
    app.run("file.new", json!({"width":8,"height":8})).unwrap();
    assert_eq!(Filter::of(&app), Filter::default());
}

fn harness() -> Harness<'static, PhotocraftApp> {
    let app = app();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(8);
    h
}

fn drawn(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    let doc = &h.state().session.active().unwrap().doc;
    let mut rows = crate::layer_row_ui::recorded(&h.ctx);
    rows.sort_by(|a, b| a.row.top().total_cmp(&b.row.top()));
    rows.iter().filter_map(|r| doc.walk().into_iter().find(|(_, _, l)| l.id.0 == r.layer).map(|(_, _, l)| l.name.clone())).collect()
}

#[test]
fn switch_widget_disables_the_type_and_controls_but_preserves_the_criteria() {
    let mut h = harness();
    set(h.state_mut(), json!({"by":"name","name":"Red"}));
    h.run_steps(4);
    assert_eq!(drawn(&h), ["Red"]);
    h.get_by_label("Turn layer filtering on or off").click();
    h.run_steps(4);
    assert!(!Filter::of(h.state()).on);
    assert_eq!(Filter::of(h.state()).name, "Red");
    assert!(h.get_by_value("Name").accesskit_node().is_disabled());
    assert!(drawn(&h).iter().any(|name| name == "Blue"));
    h.get_by_label("Turn layer filtering on or off").click();
    h.run_steps(4);
    assert_eq!(drawn(&h), ["Red"]);
}

#[test]
fn empty_result_is_named_and_hide_keeps_the_same_filter_applied() {
    let mut h = harness();
    set(h.state_mut(), json!({"by":"name","name":"no such layer"}));
    h.run_steps(4);
    assert!(drawn(&h).is_empty());
    assert!(h.query_by_label("No layers match the filter.").is_some());
    let before = Filter::of(h.state());
    h.state_mut().ui.layers_panel_options.show_filters = false;
    h.run_steps(4);
    assert!(h.query_by_label("Turn layer filtering on or off").is_none());
    assert!(h.query_by_label("No layers match the filter.").is_some());
    assert_eq!(Filter::of(h.state()), before);
    set(h.state_mut(), json!({"name":"Red"}));
    h.run_steps(4);
    assert_eq!(drawn(&h), ["Red"]);
    h.state_mut().ui.layers_panel_options.show_filters = true;
    h.run_steps(4);
    assert_eq!(drawn(&h), ["Red"]);
    assert!(h.query_by_label("Turn layer filtering on or off").is_some());
}

#[test]
fn color_and_smart_keep_each_documents_whole_filter_when_off() {
    let mut app = app();
    set(&mut app, json!({"by":"color","color":"red","on":false}));
    let color = Filter::of(&app);
    assert_eq!(color.color, photocraft_doc::LabelColor::Red);
    assert!(!color.on);
    let all = listed(&app);
    app.run("file.new", json!({"width":8,"height":8})).unwrap();
    assert_eq!(Filter::of(&app), Filter::default());
    set(&mut app, json!({"by":"smartObject","smart":["missing","embedded","missing"]}));
    let enabled = Filter::of(&app);
    assert!(enabled.on);
    assert_eq!(enabled.smart, ["missing", "embedded"]);
    set(&mut app, json!({"on":false}));
    let smart = Filter::of(&app);
    app.session.set_active(0);
    assert_eq!(Filter::of(&app), color);
    assert_eq!(listed(&app), all, "Off preserves the unfiltered rows");
    app.session.set_active(1);
    assert_eq!(Filter::of(&app), smart);
    app.run("layer.new.layer", json!({"name":"Ordinary edit"})).unwrap();
    assert_eq!(Filter::of(&app), smart, "document/context changes do not undo manual Off");
    set(&mut app, json!({"on":true}));
    assert_eq!(Filter::of(&app), enabled);
    set(&mut app, json!({"smart":[]}));
    assert!(!Filter::of(&app).on, "empty Smart Object toggles deactivate criteria");
}

#[test]
fn current_main_control_criteria_validate_atomically_and_inspect_one_document() {
    let mut app = app();
    let ok = call(&mut app, "ui.set", json!({"layerFilter":{"by":"color","color":"blue","effect":"satin","smart":["linked"],"on":false}}));
    assert_eq!(ok["ok"], true, "{ok}");
    let before = Filter::of(&app);
    for patch in [
        json!({"effect":"colorOverlay","on":true}),
        json!({"effect":null}),
        json!({"color":"crimson","on":true}),
        json!({"smart":["library"]}),
        json!({"smart":"linked"}),
        json!({"color":"red","on":"yes"}),
    ] {
        let bad = call(&mut app, "ui.set", json!({"layerFilter":patch}));
        assert_eq!(bad["ok"], false, "{bad}");
        assert_eq!(Filter::of(&app), before);
    }
    let inspected = call(&mut app, "ui.inspect", json!({}));
    let f = &inspected["result"]["layerFilter"];
    assert_eq!(f["color"], "blue");
    assert_eq!(f["effect"], "satin");
    assert_eq!(f["smart"], json!(["linked"]));
    assert_eq!(f["on"], false);
    set(&mut app, json!({"by":"smartObject","smart":["embedded"],"on":false}));
    assert!(!Filter::of(&app).on, "explicit on wins over new criterion activation");
}

#[test]
fn effect_criteria_and_manual_off_belong_to_each_document() {
    let mut app = app();
    set(&mut app, json!({"by":"effect","effect":"satin"}));
    assert!(Filter::of(&app).on && Filter::of(&app).active());
    set(&mut app, json!({"on":false}));
    let a = Filter::of(&app);
    assert_eq!(a.effect, super::effect::EffectKind::Satin);
    app.run("file.new", json!({"width":8,"height":8})).unwrap();
    assert_eq!(Filter::of(&app), Filter::default());
    set(&mut app, json!({"by":"effect","effect":"dropShadow"}));
    let b = Filter::of(&app);
    app.session.set_active(0);
    assert_eq!(Filter::of(&app), a);
    assert_eq!(listed(&app).len(), 3, "Off lists both ordinary layers and Background");
    set(&mut app, json!({"effect":"satin"}));
    assert!(!Filter::of(&app).on, "unchanged criterion preserves a manual Off");
    set(&mut app, json!({"effect":"dropShadow"}));
    assert!(Filter::of(&app).on, "changing the active criterion arms filtering");
    app.session.set_active(1);
    assert_eq!(Filter::of(&app), b);
}

#[test]
fn artboard_context_follows_selection_without_changing_the_switch() {
    let mut app = app();
    app.run("layer.new.layer", json!({"name":"Loose"})).unwrap();
    let id = |app: &PhotocraftApp, name: &str| app.session.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.name == name).unwrap().2.id;
    let (red, blue, loose) = (id(&app, "Red"), id(&app, "Blue"), id(&app, "Loose"));
    app.run("layer.select", json!({"layer":red.0})).unwrap();
    app.run("layer.groupLayers", json!({"name":"Inner"})).unwrap();
    let inner = id(&app, "Inner");
    app.run("layer.new.artboardFromLayers", json!({"name":"Red board"})).unwrap();
    app.run("layer.select", json!({"layer":blue.0})).unwrap();
    app.run("layer.new.artboardFromLayers", json!({"name":"Blue board"})).unwrap();
    app.run("layer.setExpanded", json!({"all":true,"expanded":true})).unwrap();
    app.run("layer.select", json!({"layer":loose.0})).unwrap();
    let all = listed(&app);
    set(&mut app, json!({"by":"artboard"}));
    let armed = Filter::of(&app);
    assert!(armed.on, "choosing Artboard arms the switch even with no eligible selection");
    assert_eq!(listed(&app), all);
    app.run("select.deselectLayers", json!({})).unwrap();
    assert_eq!(listed(&app), all);
    assert_eq!(Filter::of(&app), armed);
    app.run("layer.select", json!({"layer":red.0})).unwrap();
    assert_eq!(listed(&app), ["Red board", "Inner", "Red"]);
    assert_eq!(Filter::of(&app), armed, "derived rows are not stored user criteria");
    app.run("layer.setExpanded", json!({"layer":inner.0,"expanded":false})).unwrap();
    assert_eq!(listed(&app), ["Red board", "Inner"], "Artboard keeps nested groups closed");
    app.run("layer.select", json!({"layer":blue.0})).unwrap();
    assert_eq!(listed(&app), ["Blue board", "Blue"]);
    set(&mut app, json!({"on":false}));
    let off = Filter::of(&app);
    let full_closed_tree = listed(&app);
    app.run("layer.select", json!({"layer":red.0})).unwrap();
    assert_eq!(listed(&app), full_closed_tree);
    assert_eq!(Filter::of(&app), off, "a manual Off survives selection changes");
    set(&mut app, json!({"on":true}));
    assert_eq!(listed(&app), ["Red board", "Inner"]);
    assert_eq!(Filter::of(&app), armed);
}

/// Compile with the ordinary UI tests; execute only under a separate renderer allocation.
#[test]
#[ignore = "manual offscreen Image Layers filter capture; needs an authorized wgpu adapter"]
fn layer_filter_switch_visual_fixture() {
    assert_eq!(std::env::var("PHOTOCRAFT_LAYER_FILTER_CAPTURE").as_deref(), Ok("1"), "renderer allocation required");
    let output = std::path::PathBuf::from(std::env::var_os("PHOTOCRAFT_LAYER_FILTER_CAPTURE_DIR").expect("explicit capture directory"));
    let names = ["01-kind-off.png", "02-name-red.png", "03-name-off.png", "04-no-match.png", "05-hidden-filter.png"];
    for name in names {
        assert!(!output.join(name).exists(), "preserve existing capture {name}");
    }
    std::fs::create_dir_all(&output).expect("create capture directory");
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_pixels_per_point(1.0).with_max_steps(64).wgpu().build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = app();
        app.session.edit_prefs(|p| p.interface.language = "en".into());
        app.ui.theme = crate::theme::ThemeKind::ProMedium;
        let rs = cc.wgpu_render_state.as_ref().expect("offscreen renderer");
        eprintln!("layer-filter capture adapter: {:?}", rs.adapter.get_info());
        app.set_wgpu(rs.clone());
        app
    });
    h.run_steps(8);
    for (name, patch, hidden) in [
        (names[0], json!({"by":"kind","kinds":[],"on":false}), false),
        (names[1], json!({"by":"name","name":"Red","on":true}), false),
        (names[2], json!({"on":false}), false),
        (names[3], json!({"name":"no such layer","on":true}), false),
        (names[4], json!({"name":"Red","on":true}), true),
    ] {
        let result = call(h.state_mut(), "ui.set", json!({"layerFilter":patch}));
        assert_eq!(result["ok"], true, "{result}");
        h.state_mut().ui.layers_panel_options.show_filters = !hidden;
        h.run_steps(8);
        let inspected = call(h.state_mut(), "ui.inspect", json!({}));
        eprintln!("capture {name}: filter={} rows={:?}", inspected["result"]["layerFilter"], drawn(&h));
        h.render().expect("render layer filter state").save(output.join(name)).expect("save layer filter capture");
    }
}

/// Returning to a tab must not reapply its unchanged predicate to a later selection.
#[test]
fn selection_application_belongs_to_the_open_document() {
    let mut app = app();
    let blue = app.session.active().unwrap().doc.walk().into_iter().find(|(_, _, layer)| layer.name == "Blue").unwrap().2.id;
    set(&mut app, json!({"by": "name", "name": "Red"}));
    super::selection::follow(&mut app);
    assert!(app.session.active().unwrap().selected_layers().is_empty());
    app.run("layer.select", json!({"layer": blue.0})).unwrap();
    super::selection::follow(&mut app);
    assert_eq!(app.session.active().unwrap().selected_layers(), [blue], "later selection is allowed while hidden");
    app.run("file.new", json!({"width":8,"height":8})).unwrap();
    set(&mut app, json!({"by": "name", "name": "other"}));
    super::selection::follow(&mut app);
    app.run("document.activate", json!({"document":0})).unwrap();
    super::selection::follow(&mut app);
    assert_eq!(app.session.active().unwrap().selected_layers(), [blue], "tab activation is not a criterion edit");
    set(&mut app, json!({"by": "name", "name": "Red"}));
    super::selection::follow(&mut app);
    assert_eq!(app.session.active().unwrap().selected_layers(), [blue], "an identical patch is not a new application");
    set(&mut app, json!({"name": "Blue"}));
    super::selection::follow(&mut app);
    set(&mut app, json!({"name": "Red"}));
    super::selection::follow(&mut app);
    assert!(app.session.active().unwrap().selected_layers().is_empty(), "a real criterion edit still prunes");
}

/// Attribute is owned with the rest of each document's filter, while its pixel cache stays derived.
#[test]
fn attribute_criteria_are_per_document_and_read_back_atomically() {
    let mut app = app();
    set(&mut app, json!({"by":"attribute", "attribute":"notVisible"}));
    let before = Filter::of(&app);
    assert!(before.on);
    let inspected = call(&mut app, "ui.inspect", json!({}));
    assert_eq!(inspected["result"]["layerFilter"]["attribute"], "notVisible");
    assert_eq!(call(&mut app, "ui.set", json!({"layerFilter":{"on":false,"attribute":"bad"}}))["ok"], false);
    assert_eq!(Filter::of(&app), before);
    app.run("file.new", json!({"width":8,"height":8})).unwrap();
    assert_eq!(Filter::of(&app), Filter::default());
    app.run("document.activate", json!({"document":0})).unwrap();
    assert_eq!(Filter::of(&app), before);
    set(&mut app, json!({"on":false}));
    assert_eq!(Filter::of(&app).attribute, before.attribute);
    assert_eq!(listed(&app).len(), 3, "Off keeps the Attribute criterion and lists the full tree");
}
