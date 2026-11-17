use super::items::row;
use super::*;
use craft::menu::{THEME_COMMAND, ThemeChoice};

#[test]
fn live_lookup_finds_engine_and_shell_commands() {
    for command in photocraft_engine::commands::command_specs() {
        assert!(is_live(command.id), "{}", command.id);
    }
    for &(id, ..) in UI_COMMANDS {
        assert!(is_live(id), "{id}");
    }
    for id in ["", "missing.command", "💾"] {
        assert!(!is_live(id));
    }
}

/// The Appearance menu's current choice. `ThemeMenu` keeps it private; its `Debug` text names it.
fn current_choice(app: &PhotocraftApp) -> String {
    let text = format!("{:?}", crate::prefs_ui::theme_menu(app));
    text.split("current: ").nth(1).unwrap_or_default().to_string()
}

#[test]
fn sync_with_system_menu_keeps_saved_slots_and_its_checkmark() {
    use photocraft_engine::prefs::AppearanceMode;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = app.ctx().clone();
    app.run("prefs.set", json!({"values": {"interface.darkTheme": "studio", "interface.lightTheme": "classic"}})).unwrap();
    invoke(&mut app, &ctx, THEME_COMMAND, json!({"mode": "system"})).unwrap();
    assert_eq!(app.session.prefs().interface.appearance_mode, AppearanceMode::Auto);
    assert_eq!(app.session.prefs().interface.dark_theme, "craft.studio");
    assert_eq!(app.session.prefs().interface.light_theme, "craft.classic");
    assert!(current_choice(&app).starts_with("System"), "the system row is the checked radio");

    let studio = ThemeChoice::theme(craft::theme::ThemeId::parse("craft.studio").unwrap(), true).command();
    invoke(&mut app, &ctx, &studio.id, studio.params.clone()).unwrap();
    assert_eq!(app.session.prefs().interface.appearance_mode, AppearanceMode::Dark);
    assert!(current_choice(&app).starts_with("Theme"), "picking a theme leaves Auto: {}", current_choice(&app));
}

/// What a case expects of one menu row.
struct Want {
    id: &'static str,
    path: Option<&'static [&'static str]>,
    checked: Option<bool>,
    enabled: Option<bool>,
    /// The row sits directly after this one.
    after: Option<&'static str>,
}

impl Want {
    const NONE: Want = Want { id: "", path: None, checked: None, enabled: None, after: None };
}

fn new_doc(app: &mut PhotocraftApp) {
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
}

fn with_selection(app: &mut PhotocraftApp) {
    new_doc(app);
    app.run("select.rect", json!({"x": 2, "y": 2, "width": 8, "height": 8})).unwrap();
}

fn proofed(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
    app.sync_views();
    invoke(app, &ctx, "view.proofSetup.workingCmyk", Value::Null).unwrap();
    invoke(app, &ctx, "view.gamutWarning", json!({"on": true})).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    let lut = app.session.color.canvas_lut(&doc, 9).unwrap().unwrap();
    // Pure sRGB green is outside coated CMYK; mid grey is inside.
    let at = |r: usize, g: usize, b: usize| lut[(r + g * 9 + b * 81) * 4 + 3];
    assert_eq!((at(0, 8, 0), at(4, 4, 4)), (255, 0));
    // Custom… opens the generated Proof Setup dialog with a profile choice.
    assert!(invoke(app, &ctx, "view.proofSetup.custom", Value::Null).unwrap()["dialog"].is_string());
    use craft::ui::patterns::form_dialogs::{Param, ParamKind};
    let spec = photocraft_engine::commands::find("edit.convertToProfile").unwrap();
    let params = Param::parse_list(spec.params);
    assert!(matches!(&params[0].kind, ParamKind::Choice { options, .. } if options[0].0 == "srgb" && !options.iter().any(|(v, _)| v.contains('/'))));
}

fn painted_layer(app: &mut PhotocraftApp) {
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(8, 8, 32, 32), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
}

fn warp_box_open(app: &mut PhotocraftApp) {
    painted_layer(app);
    let ctx = app.ctx().clone();
    crate::transform_tool::begin_warp(app, &ctx).unwrap();
}

fn stored_warp(app: &mut PhotocraftApp) {
    painted_layer(app);
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("layer.smartObjects.warp", json!({"style": "cylinder", "values": [8, 32, 32, 8, 0.5, 0.5, 0.85], "rect": [8, 8, 32, 32]})).unwrap();
}

/// Rows sit where Photoshop puts them and show their state after the setup: path, checkmark, enablement, order.
#[test]
fn menu_rows_sit_where_photoshop_puts_them_and_show_their_state() {
    type Setup = fn(&mut PhotocraftApp);
    let cases: &[(&str, Setup, &[Want])] = &[
        (
            "select",
            with_selection,
            &[
                Want { id: "select.all", path: Some(&["Select"]), ..Want::NONE },
                Want { id: "select.colorRange", path: Some(&["Select"]), ..Want::NONE },
                // Feather stays in Select › Modify, as in the reference menus.
                Want { id: "select.modify.feather", path: Some(&["Select", "Modify"]), ..Want::NONE },
            ],
        ),
        (
            "Color to Alpha (#1576) joins Filter › Other right after Offset",
            new_doc,
            &[
                Want { id: "filter.other.offset", path: Some(&["Filter", "Other"]), ..Want::NONE },
                Want {
                    id: "filter.other.colorToAlpha",
                    path: Some(&["Filter", "Other"]),
                    enabled: Some(true),
                    after: Some("filter.other.offset"),
                    ..Want::NONE
                },
            ],
        ),
        (
            "proof setup",
            proofed,
            &[Want { id: "view.proofColors", checked: Some(true), ..Want::NONE }, Want { id: "view.gamutWarning", checked: Some(true), ..Want::NONE }],
        ),
        ("Warp on a plain layer", painted_layer, &[Want { id: "edit.transform.warp", checked: Some(false), ..Want::NONE }]),
        ("Warp with its box open (#9798)", warp_box_open, &[Want { id: "edit.transform.warp", checked: Some(true), ..Want::NONE }]),
        ("Warp stored on a smart object (#9798)", stored_warp, &[Want { id: "edit.transform.warp", checked: Some(true), ..Want::NONE }]),
    ];
    for (name, setup, wants) in cases {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        setup(&mut app);
        let rows = menu_rows(&app);
        for want in *wants {
            let found = row(&rows, want.id).unwrap_or_else(|| panic!("{name}: {} missing", want.id));
            if let Some(path) = want.path {
                assert_eq!(found.path, path, "{name}: {}", want.id);
            }
            if let Some(checked) = want.checked {
                assert_eq!(found.kind.checked(), Some(checked), "{name}: {}", want.id);
            }
            if let Some(enabled) = want.enabled {
                assert_eq!(found.enabled, enabled, "{name}: {}", want.id);
            }
            if let Some(after) = want.after {
                let at = |id: &str| rows.iter().position(|r| r.command.id == id).unwrap();
                assert_eq!(at(want.id), at(after) + 1, "{name}: {} follows {after}", want.id);
            }
        }
    }

    // The context menus' Select actions all live under Select; Make Work Path is a Paths panel action.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    with_selection(&mut app);
    let rows = menu_rows(&app);
    let menus = crate::canvas_tool_menu::SELECTION_MENU.iter().chain(crate::canvas_tool_menu::NO_SELECTION_MENU).flatten();
    for &(label, id) in menus.filter(|(_, id)| id.starts_with("select.") && *id != "select.toWorkPath") {
        assert!(rows.iter().any(|r| r.command.id == id && r.path.first().is_some_and(|p| p == "Select")), "Select menu missing {label} ({id})");
    }
}

#[test]
fn window_panel_and_workspace_ids_drive_the_shell() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    // Essentials keeps History as an icon: its Window id opens the icon's flyout, which checks its row.
    assert!(!crate::panel_docking::is_checked(&app, "history"));
    invoke(&mut app, &ctx, "window.panel.history", Value::Null).unwrap();
    assert!(crate::panel_docking::is_checked(&app, "history"));

    invoke(&mut app, &ctx, "window.workspace.painting", Value::Null).unwrap();
    assert_eq!(app.workspaces.set.current, "painting");
    assert!(!crate::panel_docking::is_shown(&app, "properties"));
    let rows = menu_rows(&app);
    assert_eq!(row(&rows, "window.workspace.painting").and_then(|r| r.kind.checked()), Some(true));

    invoke(&mut app, &ctx, "window.workspace.essentials", Value::Null).unwrap();
    assert!(crate::panel_docking::is_shown(&app, "properties"));

    // Live in the menu, and no duplicate "Layers" entry.
    let rows = menu_rows(&app);
    assert!(row(&rows, "window.panel.layers").is_some_and(|r| r.enabled));
    assert_eq!(rows.iter().filter(|r| r.path == ["Window"] && r.label == "Layers").count(), 1);
    assert!(row(&rows, "edit.stroke").is_some());
}

/// Layer › Copy CSS puts the layer's CSS on the clipboard, as Photoshop's does (#9372).
#[test]
fn copy_css_puts_the_layers_css_on_the_clipboard() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("layer.new.layer", json!({"name": "Box"})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(100, 80, 300, 220), &[0.2, 0.4, 0.8, 1.0]);
            Ok(())
        })
        .unwrap();
    app.sync_views();
    let rows = menu_rows(&app);
    assert!(row(&rows, "layer.copyCSS").is_some_and(|r| r.path == ["Layer"]), "Layer › Copy CSS");
    let ctx = app.ctx().clone();
    let mut result = None;
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        result = Some(invoke(&mut app, ui.ctx(), "layer.copyCSS", json!({})));
    });
    out.textures_delta.clear();
    let css = result.unwrap().unwrap()["css"].as_str().unwrap().to_string();
    assert!(css.starts_with(".Box {\n  background-image: url(\"Box.png\");\n"), "{css}");
    assert_eq!(out.platform_output.commands, [egui::OutputCommand::CopyText(css)]);
}
