use super::items::row;
use super::*;
use craft::keymap::CommandRef;
use craft::menu::{ItemKind, MenuRole, Node, THEME_COMMAND, ThemeChoice};
use photocraft_engine::color_cmds::ColorExt;

/// Inspect the real native tree without depending on a platform menu backend.
fn native_submenu<'a>(nodes: &'a [Node], title: &str) -> Option<&'a [Node]> {
    nodes.iter().find_map(|node| match node {
        Node::Submenu { label, children, .. } if label == title => Some(children.as_slice()),
        _ => None,
    })
}

/// Provider display text is data, including the label used to draw real separator rows.
#[test]
fn provider_resource_named_dashes_remains_a_command_in_window_native_and_search_menus() {
    let provider = crate::file_providers::Host::default();
    provider.set_session("menu-separator-fixture-0001".into()).unwrap();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_provider: Some(provider.clone()), ..Default::default() });
    let ctx = app.ctx().clone();
    let command = "file.openProviderRecent.dashes";
    assert!(row(model(&mut app, &ctx).rows(), command).is_none());
    provider.connect("menu-fixture".into(), "Separator fixtures".into(), "account".into()).unwrap();
    let connection = provider.snapshot()["connectionId"].as_str().unwrap().to_string();
    provider.set_resources(&connection, vec![crate::file_providers::Resource { id: "dashes".into(), name: "---".into(), updated_at: String::new() }]).unwrap();

    let items = menu_rows(&app);
    let resource = row(&items, command).unwrap();
    assert_eq!(resource.label, "---");
    assert!(resource.enabled && resource.kind == ItemKind::Plain && !resource.command.id.is_empty());
    assert!(items.iter().any(|row| row.command.id.is_empty() && row.label == "---"), "ordinary separator rows remain separators");
    let shared = model(&mut app, &ctx);
    assert_eq!(row(shared.rows(), command).unwrap().label, "---");
    let file = shared.bar.menu(MenuRole::File).unwrap();
    let recent = native_submenu(&file.children, "Open Recent").unwrap();
    let resources = native_submenu(recent, "Separator fixtures").unwrap();
    assert!(matches!(resources, [Node::Item(item)] if item.command.id == command && item.label == "---" && item.enabled));
    let native = native_menu::laid_out(&mut app, &ctx);
    assert!(native.bar.find(&CommandRef::new(command)).is_some_and(|item| item.label == "---" && item.enabled));
    let entries = help_entries(&mut app, &ctx);
    let listed: Vec<_> = entries.iter().filter(|entry| entry.command().id == command).collect();
    assert_eq!(listed.len(), 1, "search includes the resource once and excludes separators");
    assert_eq!(listed[0].label().as_str(), "---");
    provider.set_enabled(false).unwrap();
    assert!(row(model(&mut app, &ctx).rows(), command).is_none(), "idle host changes invalidate the shared menu cache");
}

#[test]
fn provider_resource_names_remain_literal_in_localized_menus_and_search() {
    if !crate::i18n::needs_catalogs() {
        return;
    }
    let french = crate::i18n::Lang::from_code("fr").unwrap();
    assert_eq!(crate::i18n::tr(french, "File"), "Fichier");
    let provider = crate::file_providers::Host::default();
    provider.set_session("localized-menu-fixture-0001".into()).unwrap();
    provider.connect("menu-fixture".into(), "File".into(), "account".into()).unwrap();
    let connection = provider.snapshot()["connectionId"].as_str().unwrap().to_string();
    let names = [("catalog", "File"), ("ellipsis", "File…")];
    let resources =
        names.into_iter().map(|(id, name)| crate::file_providers::Resource { id: id.into(), name: name.into(), updated_at: String::new() }).collect();
    provider.set_resources(&connection, resources).unwrap();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_provider: Some(provider), ..Default::default() });
    let ctx = app.ctx().clone();
    let items = menu_rows(&app);
    let _language = crate::i18n::language_scope(french);
    let layout = native_menu::laid_out(&mut app, &ctx);
    for (id, name) in names {
        let command = format!("file.openProviderRecent.{id}");
        let item = row(&items, &command).unwrap();
        assert_eq!(translated_menu_label(french, &item.command.id, &item.label), name);
        assert_eq!(layout.bar.find(&CommandRef::new(&command)).map(|item| item.label.as_str()), Some(name));
    }
    let entries = help_entries(&mut app, &ctx);
    for (id, name) in names {
        let command = CommandRef::new(format!("file.openProviderRecent.{id}"));
        let entry = entries.iter().find(|entry| entry.command() == &command).unwrap();
        assert_eq!(entry.label().as_str(), name);
        assert_eq!(entry.english().map_or(entry.label().as_str(), |english| english.as_str()), name);
    }
    let file = layout.bar.menu(MenuRole::File).unwrap();
    assert_eq!(file.title, "Fichier");
    let recent = native_submenu(&file.children, crate::i18n::tr(french, "Open Recent")).unwrap();
    assert!(native_submenu(recent, "File").is_some(), "the provider title is literal too");
    assert_eq!(translated_menu_label(french, "fixture.catalog-control", "File"), "Fichier");
}

#[test]
fn provider_recents_stay_separate_from_local_files_and_refresh_without_input() {
    let provider = crate::file_providers::Host::default();
    provider.set_session("menu-recents-fixture-0001".into()).unwrap();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_provider: Some(provider.clone()), ..Default::default() });
    *app.ui.recent_files = vec!["/tmp/drawing.psd".into()];
    let ctx = app.ctx().clone();
    let command = CommandRef::new("file.openProviderRecent.drawing");
    let provider_path = ["File", "Open Recent", "File Provider"];
    let local_path = ["File", "Open Recent", "Local Recent Files"];
    let local_id = if cfg!(target_arch = "wasm32") { "file.localRecentHint" } else { "file.openRecent.0" };

    let initial = model(&mut app, &ctx);
    assert_eq!(row(initial.rows(), "file.openRecent.0").unwrap().path, ["File", "Open Recent"]);
    assert!(initial.bar.find(&command).is_none());

    let connection = provider.connect("menu-fixture".into(), "Local Recent Files".into(), "account".into()).unwrap();
    let connected = model(&mut app, &ctx);
    let empty = row(connected.rows(), "file.providerRecentEmpty").unwrap();
    assert_eq!(empty.path, provider_path);
    assert!(!empty.enabled);
    assert_eq!(row(connected.rows(), local_id).unwrap().path, local_path);

    let resources = |name: &str| vec![crate::file_providers::Resource { id: "drawing".into(), name: name.into(), updated_at: String::new() }];
    provider.set_resources(&connection, resources("drawing.psd")).unwrap();
    let listed = model(&mut app, &ctx);
    assert!(row(listed.rows(), "file.providerRecentEmpty").is_none());
    assert_eq!(row(listed.rows(), &command.id).unwrap().path, provider_path);
    assert_eq!(listed.bar.find(&command).unwrap().label, "drawing.psd");
    assert_eq!(row(listed.rows(), local_id).unwrap().path, local_path);
    assert_eq!(*app.ui.recent_files, vec!["/tmp/drawing.psd".to_string()]);

    let renamed = "full.name.with.ellipsis…";
    provider.set_resources(&connection, resources(renamed)).unwrap();
    assert_eq!(row(model(&mut app, &ctx).rows(), &command.id).unwrap().label, renamed);
    assert_eq!(native_menu::laid_out(&mut app, &ctx).bar.find(&command).unwrap().label, renamed);

    provider.set_enabled(false).unwrap();
    assert!(!is_enabled(&app, &command.id));
    let disabled = model(&mut app, &ctx);
    assert!(row(disabled.rows(), &command.id).is_none(), "idle provider changes invalidate the shared menu cache");
    assert_eq!(row(disabled.rows(), local_id).unwrap().path, local_path);
    assert!(native_menu::laid_out(&mut app, &ctx).bar.find(&command).is_none());
}

#[test]
fn provider_recent_commands_use_the_menu_gate_and_provider_dispatch() {
    let provider = crate::file_providers::Host::default();
    provider.set_session("menu-gate-fixture-0001".into()).unwrap();
    provider.connect("menu-fixture".into(), "Example Storage".into(), "account".into()).unwrap();
    let connection = provider.connection_id().unwrap();
    provider
        .set_resources(&connection, vec![crate::file_providers::Resource { id: "drawing".into(), name: "File…".into(), updated_at: String::new() }])
        .unwrap();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_provider: Some(provider.clone()), ..Default::default() });
    let ctx = app.ctx().clone();
    let command = CommandRef::new("file.openProviderRecent.drawing");
    assert!(is_live(&command.id) && is_enabled(&app, &command.id) && allows(&app, &command));
    invoke(&mut app, &ctx, &command.id, json!({})).unwrap();
    assert!(provider.take_events().iter().any(|event| event.metadata["type"] == "openRequest" && event.metadata["resourceId"] == "drawing"));
    assert!(app.ui.recent_files.is_empty());
    app.run("file.new", json!({"width": 2, "height": 2})).unwrap();
    app.sync_views();
    invoke(&mut app, &ctx, "image.imageSize", json!({})).unwrap();
    assert!(app.dialog_open());
    assert!(!allows(&app, &command), "dynamic provider commands obey the same dialog gate as catalogue commands");
}

#[test]
fn provider_recent_dispatch_correlates_the_exact_resource_id() {
    let provider = crate::file_providers::Host::default();
    provider.set_session("menu-prefix-fixture-0001".into()).unwrap();
    provider.connect("menu-fixture".into(), "Example Storage".into(), "account".into()).unwrap();
    let connection = provider.connection_id().unwrap();
    let resource = "drawing";
    provider
        .set_resources(&connection, vec![crate::file_providers::Resource { id: resource.into(), name: "Drawing.psd".into(), updated_at: String::new() }])
        .unwrap();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_provider: Some(provider.clone()), ..Default::default() });
    let ctx = app.ctx().clone();
    let command = format!("file.openProviderRecent.{resource}");
    assert_eq!(row(&menu_rows(&app), &command).unwrap().label, "Drawing.psd");
    invoke(&mut app, &ctx, &command, json!({})).unwrap();
    let events = provider.take_events();
    let opens: Vec<_> = events.iter().filter(|event| event.metadata["type"] == "openRequest").collect();
    assert_eq!(opens.len(), 1);
    assert_eq!(opens[0].metadata["resourceId"], resource);
    assert_eq!(opens[0].metadata["filename"], "Drawing.psd");
    assert_eq!(opens[0].metadata["connectionId"], connection);
    assert!(app.ui.recent_files.is_empty());
}

#[test]
fn provider_recent_rejects_prefix_shaped_resource_ids_without_requests() {
    let provider = crate::file_providers::Host::default();
    provider.set_session("menu-invalid-prefix-fixture-0001".into()).unwrap();
    let connection = provider.connect("menu-fixture".into(), "Example Storage".into(), "account".into()).unwrap();
    let valid = crate::file_providers::Resource { id: "drawing".into(), name: "Drawing.psd".into(), updated_at: String::new() };
    provider.set_resources(&connection, vec![valid.clone()]).unwrap();
    let invalid = "file.openProviderRecent.drawing";
    assert!(provider.set_resources(&connection, vec![crate::file_providers::Resource { id: invalid.into(), ..valid.clone() }]).is_err());
    assert_eq!(provider.resources(), vec![valid], "the rejected dotted ID cannot replace valid Recents");
    assert!(provider.take_events().is_empty());
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_provider: Some(provider.clone()), ..Default::default() });
    let ctx = app.ctx().clone();
    let command = format!("file.openProviderRecent.{invalid}");
    assert!(row(&menu_rows(&app), &command).is_none());
    assert!(invoke(&mut app, &ctx, &command, json!({})).is_err());
    assert!(provider.take_events().is_empty(), "a rejected resource ID cannot create an openRequest");
}

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

/// Every shared menu choice has its own English alias, including choices that share a command id.
#[test]
fn shared_menu_choices_keep_exact_english_aliases_and_titles() {
    use craft::i18n::Words;
    use craft::menu::{LANGUAGE_COMMAND, LanguageLabels, Target, ThemeLabels};

    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = app.ctx().clone();
    let az = crate::i18n::Lang::from_code("az").unwrap();
    let _language = crate::i18n::language_scope(az);
    let i18n = crate::i18n::framework_i18n();
    let words = Words::from_i18n(i18n.clone(), i18n.from_code("az").unwrap());
    let studio = craft::theme::ThemeId::parse("craft.studio").unwrap();
    let theme_choices = [
        (ThemeChoice::System.command(), "Sync with System".to_string()),
        (ThemeChoice::NextMode.command(), "Next Mode".to_string()),
        (ThemeChoice::theme(studio.clone(), false).command(), "Studio (Light)".to_string()),
        (ThemeChoice::theme(studio, true).command(), "Studio (Dark)".to_string()),
    ];
    let language_choices = [
        (CommandRef::with_params(LANGUAGE_COMMAND, json!({"lang": "auto"})), "Automatic".to_string()),
        (CommandRef::with_params(LANGUAGE_COMMAND, json!({"lang": "en"})), i18n.name(i18n.from_code("en").unwrap()).to_string()),
        (CommandRef::with_params(LANGUAGE_COMMAND, json!({"lang": "az"})), i18n.name(i18n.from_code("az").unwrap()).to_string()),
    ];
    let mac = Target::current() == Target::Mac;
    let visible_choices = theme_choices.iter().chain(language_choices.iter().filter(|_| mac));
    let shared = model(&mut app, &ctx);
    for (command, english) in visible_choices.clone() {
        let item = shared.bar.find(command).expect("the shared command is visible on this platform");
        assert_eq!(item.english.as_deref().unwrap_or(&item.label), english.as_str(), "{command:?}");
    }
    assert!(shared.bar.find(&CommandRef::with_params(THEME_COMMAND, json!({"mode": "unknown"}))).is_none());
    assert!(shared.bar.find(&CommandRef::with_params(LANGUAGE_COMMAND, json!({"lang": "unknown"}))).is_none());
    let theme_labels = words.set::<ThemeLabels>();
    let language_labels = words.set::<LanguageLabels>();
    let titles = [(theme_labels.theme.as_str(), "Appearance"), (language_labels.language.as_str(), "Language")];
    for (shown, expected) in titles.into_iter().take(if mac { 2 } else { 1 }) {
        let english = shared.bar.menus.iter().flat_map(|menu| &menu.children).find_map(|node| match node {
            Node::Submenu { label, english, .. } if label == shown => Some(english.as_deref().unwrap_or(label)),
            _ => None,
        });
        assert_eq!(english, Some(expected));
    }
    let listed = help_entries(&mut app, &ctx);
    for (command, english) in visible_choices {
        let entry = listed.iter().find(|entry| entry.command() == command).expect("the visible shared menu choice is searchable");
        assert_eq!(entry.english().map_or(entry.label().as_str(), |label| label.as_str()), english.as_str(), "{command:?}");
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
    let lut = app.session.color().canvas_lut(&doc, 9).unwrap().unwrap();
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
