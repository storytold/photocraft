//! #12670: hover the actual shared Burn/Dodge controls, then use them with the pointer and keys.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::i18n::{Lang, tr};
use crate::state::Tool;

const CONTROLS: [(Role, &str, &str); 3] = [
    (Role::ComboBox, "Range:", "Painting Mode"),
    (Role::SpinButton, "Exposure:", "Set exposure for stroke"),
    (Role::CheckBox, "Protect Tones", "Minimizes clipping in shadows and highlights, and keeps colors from shifting hue"),
];

fn bar(tool: Tool, lang: Lang, width: f32, scale: f32, narrow: bool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    app.run("prefs.set", json!({"path": "workspace.enableNarrowOptionsBar", "value": narrow})).unwrap();
    app.ui.tool = tool;
    let mut h = Harness::builder()
        .with_size(vec2(width / scale, 300.0))
        .with_pixels_per_point(scale)
        .with_step_dt(1.0 / 60.0)
        .with_context_setup(|ctx| PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::ProMedium))
        .build_ui_state(
            move |ui, app| {
                let _language = crate::i18n::language_scope(lang);
                crate::panels::options_bar(app, ui);
            },
            app,
        );
    h.run_steps(4);
    h
}

fn control<'a>(h: &'a Harness<'static, PhotocraftApp>, role: Role, label: &str) -> egui_kittest::Node<'a> {
    h.query_all_by_role(role).find(|n| n.accesskit_node().label().as_deref() == Some(label)).unwrap_or_else(|| panic!("missing {role:?} {label:?}"))
}

fn hover(h: &mut Harness<'static, PhotocraftApp>, role: Role, label: &str, tip: &str) {
    control(h, role, label).scroll_to_me();
    h.run_steps(4);
    let rect = control(h, role, label).rect();
    assert!(h.ctx.content_rect().contains_rect(rect), "the control is reachable: {label} {rect:?}");
    h.hover_at(rect.center());
    h.run_steps(60);
    assert!(h.query_all_by_role(Role::Label).any(|n| n.accesskit_node().value().as_deref() == Some(tip)), "hovering {role:?} {label:?} must display {tip:?}");
}

#[test]
fn both_tools_show_exact_tooltips_on_the_controls_and_hide_them_on_leave() {
    for tool in [Tool::Burn, Tool::Dodge] {
        for (role, label, tip) in CONTROLS {
            let mut h = bar(tool, Lang::EN, 1600.0, 1.0, false);
            assert!(h.query_by_label(tip).is_none(), "no tooltip before hover");
            let before = h.state().ui.tool_options.clone();
            hover(&mut h, role, label, tip);
            assert_eq!(h.state().ui.tool_options.exposure, before.exposure);
            assert_eq!(h.state().ui.tool_options.tone_range, before.tone_range);
            assert_eq!(h.state().ui.tool_options.protect_tones, before.protect_tones);
            assert_eq!(h.ctx.memory(|m| m.focused()), None, "hover never takes keyboard focus");
            h.hover_at(egui::pos2(20.0, 200.0));
            h.run_steps(60);
            assert!(h.query_by_label(tip).is_none(), "tooltip goes away after leaving {label}");
        }
    }
}

#[test]
fn hover_uses_the_active_catalog_or_english_fallback() {
    for code in ["en", "de", "fr", "ja", "ar"] {
        let lang = Lang::from_code(code).unwrap();
        for tool in [Tool::Burn, Tool::Dodge] {
            for (role, label, tip) in CONTROLS {
                let mut h = bar(tool, lang, 1600.0, 1.0, false);
                // The existing scrub-caption link only names fields laid out left to right.
                // In RTL, select the sole numeric control rather than assuming that link exists.
                let name = if role == Role::SpinButton && lang.dir() == crate::i18n::Dir::Rtl {
                    h.get_by_role(role).accesskit_node().label().unwrap_or_default()
                } else {
                    tr(lang, label).to_owned()
                };
                hover(&mut h, role, &name, tr(lang, tip));
            }
        }
    }
}

#[test]
fn hovering_does_not_block_exposure_editing_range_selection_or_protect_tones() {
    for tool in [Tool::Burn, Tool::Dodge] {
        let mut h = bar(tool, Lang::EN, 1600.0, 1.0, false);
        let history = h.state().session.active().unwrap().history.past_len();
        let (role, label, tip) = CONTROLS[1];
        hover(&mut h, role, label, tip);
        control(&h, role, label).click();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text("37".into()));
        h.run_steps(2);
        let focused = h.ctx.memory(|m| m.focused());
        assert!(focused.is_some(), "Exposure receives keyboard focus");
        h.run_steps(60);
        assert_eq!(h.ctx.memory(|m| m.focused()), focused, "a tooltip cannot steal focus");
        h.key_press(egui::Key::Enter);
        h.run_steps(4);
        assert_eq!(h.state().ui.tool_options.exposure, 37.0);

        let (role, label, tip) = CONTROLS[0];
        hover(&mut h, role, label, tip);
        control(&h, role, label).click();
        h.run_steps(4);
        assert!(egui::Popup::is_any_open(&h.ctx), "Range opens after hover");
        h.get_by_label("Highlights").click();
        h.run_steps(4);
        assert_eq!(h.state().ui.tool_options.tone_range, "highlights");
        assert!(!egui::Popup::is_any_open(&h.ctx));
        control(&h, role, label).click();
        h.run_steps(4);
        h.key_press(egui::Key::ArrowUp);
        h.run_steps(2);
        h.key_press(egui::Key::Escape);
        h.run_steps(4);
        assert_eq!(h.state().ui.tool_options.tone_range, "midtones", "Range still accepts keyboard navigation");

        let (role, label, tip) = CONTROLS[2];
        hover(&mut h, role, label, tip);
        control(&h, role, label).click();
        h.run_steps(4);
        assert!(!h.state().ui.tool_options.protect_tones, "Protect Tones toggles after hover");
        control(&h, role, label).click();
        h.run_steps(4);
        assert!(h.state().ui.tool_options.protect_tones);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history, "option edits do not paint or add history");
    }
}

#[test]
fn tool_switching_clears_the_old_tooltip_and_leaves_other_percentage_fields_unchanged() {
    for tool in [Tool::Blur, Tool::Sharpen, Tool::Smudge] {
        let mut h = bar(Tool::Burn, Lang::EN, 1600.0, 1.0, false);
        hover(&mut h, CONTROLS[1].0, CONTROLS[1].1, CONTROLS[1].2);
        h.state_mut().ui.tool = tool;
        h.run_steps(60);
        assert!(h.query_by_label(CONTROLS[1].2).is_none());
        control(&h, Role::SpinButton, "Strength:").hover();
        h.run_steps(60);
        assert!(h.query_by_label(CONTROLS[1].2).is_none(), "{tool:?} must not inherit the Exposure tooltip");
        control(&h, Role::SpinButton, "Strength:").click();
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text("42".into()));
        h.run_steps(2);
        h.key_press(egui::Key::Tab);
        h.run_steps(4);
        assert_eq!(h.state().ui.tool_options.strength, 42.0);
        h.state_mut().ui.tool = Tool::Dodge;
        h.run_steps(4);
        hover(&mut h, CONTROLS[1].0, CONTROLS[1].1, CONTROLS[1].2);
        assert_eq!(h.state().ui.tool_options.exposure, 50.0);
    }
}

/// Run explicitly to inspect the rendered tooltips; no GPU is required by the normal regressions.
#[test]
#[ignore = "offscreen visual inspection; writes PNGs to PHOTOCRAFT_TOOLTIP_IMAGES or the temp directory"]
fn render_tooltips_for_visual_inspection() {
    let out = std::env::var_os("PHOTOCRAFT_TOOLTIP_IMAGES").map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("photocraft-12670"));
    let lang = std::env::var("PHOTOCRAFT_TOOLTIP_LANGUAGE").ok().and_then(|code| Lang::from_code(&code)).unwrap_or(Lang::EN);
    std::fs::create_dir_all(&out).unwrap();
    for theme in crate::theme::ThemeKind::ALL {
        for tool in [Tool::Burn, Tool::Dodge] {
            for (i, (role, label, tip)) in CONTROLS.into_iter().enumerate() {
                let mut h = bar(tool, lang, 1000.0, 1.0, false);
                PhotocraftApp::setup_context(&h.ctx, theme);
                h.run_steps(4);
                h.render().unwrap().save(out.join(format!("{theme:?}-{tool:?}-{i}-before.png"))).unwrap();
                hover(&mut h, role, tr(lang, label), tr(lang, tip));
                h.render().unwrap().save(out.join(format!("{theme:?}-{tool:?}-{i}-hover.png"))).unwrap();
            }
        }
    }
    for tool in [Tool::Burn, Tool::Dodge] {
        for (i, (role, label, tip)) in CONTROLS.into_iter().enumerate() {
            let mut h = bar(tool, lang, 480.0, 3.0, true);
            hover(&mut h, role, tr(lang, label), tr(lang, tip));
            h.render().unwrap().save(out.join(format!("narrow-3x-{tool:?}-{i}.png"))).unwrap();
        }
    }
}

#[test]
fn tooltips_remain_reachable_in_narrow_bars_at_supported_scales() {
    for scale in [0.75, 0.8, 0.85, 0.9, 0.95, 1.0, 1.05, 1.1, 1.15, 1.2, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
        for tool in [Tool::Burn, Tool::Dodge] {
            for (role, label, tip) in CONTROLS {
                let mut h = bar(tool, Lang::EN, 480.0, scale, true);
                hover(&mut h, role, label, tip);
                let tooltip = h.get_by_label(tip).rect();
                assert!(h.ctx.content_rect().contains_rect(tooltip), "tooltip fits at {scale}x: {tooltip:?}");
            }
        }
    }
}
