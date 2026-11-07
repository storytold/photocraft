use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

/// Small synthetic documents cover every Properties routing branch without external assets.
fn fixture(kind: &str) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":160,"height":120,"background":"white"})).unwrap();
    match kind {
        "document" => {}
        "text" => {
            app.run("type.create", json!({"text":"Headline","size":24,"x":8,"y":40})).unwrap();
        }
        "shape" => {
            app.run("shape.create", json!({"kind":"rect","rect":[8,8,80,60],"fill":"#336699"})).unwrap();
        }
        "gradient" => {
            app.run("gradient.fill.create", json!({"from":[10,60],"to":[140,60]})).unwrap();
        }
        "artboard" => {
            app.run("layer.new.artboard", json!({"rect":[0,0,120,100]})).unwrap();
        }
        "group" => {
            app.run("layer.new.group", json!({})).unwrap();
        }
        "mask" | "adjustment" => {
            app.run("layer.newAdjustmentLayer.brightnessContrast", json!({})).unwrap();
            app.ui.mask_target = kind == "mask";
            app.ui.properties_layer = None;
        }
        _ => {
            app.run("layer.new.layer", json!({"name":"Paint"})).unwrap();
            app.run("select.rect", json!({"x":8,"y":8,"width":80,"height":60})).unwrap();
            app.run("edit.fill", json!({"contents":"color","color":"#336699"})).unwrap();
            app.run("select.deselect", json!({})).unwrap();
            if kind == "frame" {
                app.run("layer.new.frameFromLayers", json!({})).unwrap();
            }
            if kind == "vector-mask" {
                app.run("layer.vectorMask.revealAll", json!({})).unwrap();
                app.ui.vector_mask_target = true;
            }
            if kind == "smart" {
                app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
            }
        }
    }
    app
}

fn panel(kind: &str, width: f32, language: &str, theme: theme::ThemeKind) -> Harness<'static, PhotocraftApp> {
    let lang = crate::i18n::Lang::from_pref(language);
    let mut h = Harness::builder().with_size(vec2(width, 2200.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::i18n::with_language(lang, || properties_body(app, ui));
            }
        },
        fixture(kind),
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.run_steps(5);
    h
}

/// Controls fit narrow docks and share a bounded form on wide docks in translated layouts.
#[test]
fn properties_variants_keep_controls_inside_the_form() {
    let mut overflow = Vec::new();
    for kind in ["document", "pixel", "text", "mask", "shape", "gradient", "adjustment", "artboard", "group", "smart", "frame", "vector-mask"] {
        for language in ["en", "fr", "de"] {
            for width in [210.0, 290.0, 420.0, 1000.0] {
                let h = panel(kind, width, language, theme::ThemeKind::Pro);
                let right = (width - 12.0).min(12.0 + crate::props_layout::CONTROL_MAX_W);
                for role in [egui::Role::Button, egui::Role::SpinButton, egui::Role::ComboBox, egui::Role::TextInput, egui::Role::CheckBox] {
                    for node in h.query_all_by_role(role) {
                        let r = node.rect();
                        if !(r.left() >= 11.0 && r.right() <= right + 1.0) {
                            overflow.push(format!("{kind}/{language}/{width}: {role:?} {:?} {r:?} exceeds {right}", node.accesskit_node().label()));
                        }
                    }
                }
            }
        }
    }
    assert!(overflow.is_empty(), "{}", overflow.join("\n"));
}

/// Narrow text Properties retains every quick action, with one readable row per action.
#[test]
fn narrow_text_keeps_all_actions_and_formatting_commands() {
    let mut h = panel("text", 210.0, "fr", theme::ThemeKind::Pro);
    let actions = ["Convert to Shape", "Convert to Paragraph Text", "Create Warped Text", "Rasterize Type"];
    let mut bottom = 0.0;
    for action in actions {
        let r = h.get_by_role_and_label(egui::Role::Button, action).rect();
        assert!(r.top() > bottom);
        bottom = r.bottom();
    }
    let history = h.state().session.active().unwrap().history.entries().len();
    h.get_by_label("Faux Bold").click();
    h.run_steps(3);
    let st = h.state().session.active().unwrap();
    assert_eq!(st.history.entries().len(), history + 1);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(3);
    h.get_by_label("Convert to Paragraph Text").click();
    h.run_steps(3);
    assert!(h.query_by_label("Convert to Point Text").is_some());
}

/// Non-Pro Properties also uses the compact form for masks, adjustments and text.
#[test]
fn floating_theme_properties_controls_are_bounded() {
    for kind in ["pixel", "text", "mask", "shape", "adjustment", "artboard"] {
        let h = panel(kind, 1000.0, "fr", theme::ThemeKind::Studio);
        for role in [egui::Role::Button, egui::Role::SpinButton, egui::Role::ComboBox, egui::Role::TextInput] {
            for node in h.query_all_by_role(role) {
                assert!(node.rect().right() <= 373.0, "{kind}: {:?} {:?}", node.accesskit_node().label(), node.rect());
            }
        }
    }
}

/// An elided action remains named in full and exposes its translated caption on hover.
#[test]
fn narrow_action_exposes_its_full_caption() {
    let mut h = panel("text", 210.0, "fr", theme::ThemeKind::Pro);
    h.ctx.global_style_mut(|s| s.interaction.tooltip_delay = 0.0);
    h.get_by_label("Convert to Paragraph Text").hover();
    h.run_steps(4);
    let caption = crate::i18n::with_language(crate::i18n::Lang::from_pref("fr"), || tl!("Convert to Paragraph Text").to_owned());
    assert!(h.query_by_role_and_label(egui::Role::Label, &caption).is_some());
}
