use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    s
}

#[test]
fn defaults_match_photoshop() {
    let p = Preferences::default();
    assert_eq!(p.performance.history_states, 50);
    assert_eq!(p.guides_grid_and_slices.subdivisions, 4);
    assert_eq!(p.guides_grid_and_slices.guide_color, "#4affff");
    assert_eq!(p.transparency_and_gamut.colors(), [[255, 255, 255], [204, 204, 204]]);
    assert_eq!(p.transparency_and_gamut.square(), Some(8.0));
    assert_eq!(p.cursors.painting, PaintingCursor::NormalTip);
    // Every dialog section is a key of the JSON form.
    let v = p.to_json();
    for (id, _) in SECTIONS {
        assert!(v.get(id).is_some_and(Value::is_object), "{id}");
    }
}

#[test]
fn get_set_reset_by_path() {
    let mut s = session();
    assert_eq!(s.execute("prefs.get", json!({"path": "performance.historyStates"})).unwrap(), json!(50));
    s.execute("prefs.set", json!({"path": "performance.historyStates", "value": 3})).unwrap();
    assert_eq!(s.prefs().performance.history_states, 3);
    // Applied to open documents: only three undo steps remain.
    for i in 0..6 {
        s.execute("layer.new.layer", json!({"name": format!("L{i}")})).unwrap();
    }
    let mut n = 0;
    while s.undo() {
        n += 1;
    }
    assert_eq!(n, 3);
    // New documents too.
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    assert_eq!(s.active().unwrap().history.max_states, 3);
    s.execute("prefs.reset", json!({"path": "performance"})).unwrap();
    assert_eq!(s.prefs().performance.history_states, 50);
    assert_eq!(s.active().unwrap().history.max_states, 50);
}

#[test]
fn reset_one_colour_setting() {
    let mut s = session();
    s.execute("prefs.set", json!({"values": {"colorSettings.bpc": false, "colorSettings.workingRgb": "display-p3"}})).unwrap();
    assert_eq!(s.execute("prefs.reset", json!({"path": "colorSettings.bpc"})).unwrap(), json!(true));
    assert!(s.color.settings.bpc);
    assert_eq!(s.color.settings.working_rgb, "display-p3", "other colour settings are untouched");
    s.execute("prefs.reset", json!({"path": "colorSettings.workingRgb"})).unwrap();
    assert_eq!(s.color.settings, crate::color_cmds::ColorSettings::default());
    assert!(s.execute("prefs.reset", json!({"path": "colorSettings.notAField"})).is_err());
}

#[test]
fn reset_one_keyed_override() {
    let mut s = session();
    s.execute(
        "prefs.set",
        json!({"values": {
            "shortcuts.edit.undo": "Ctrl+Alt+Z", "shortcuts.edit.redo": "Ctrl+Alt+Y",
            "menus.colors.edit.fill": "red", "menus.colors.filter.blur.gaussianBlur": "blue",
            "interface.theme": "studioLight"
        }}),
    )
    .unwrap();
    let undo = crate::commands::find("edit.undo").unwrap().shortcut;
    assert_eq!(s.prefs().shortcut("edit.undo", undo), Some("Ctrl+Alt+Z"));
    for path in ["shortcuts.edit.undo", "menus.colors.filter.blur.gaussianBlur"] {
        let revision = s.prefs.rev();
        assert_eq!(s.execute("prefs.reset", json!({"path": path})).unwrap(), Value::Null);
        assert!(s.prefs.rev() > revision);
        assert!(s.execute("prefs.get", json!({"path": path})).is_err(), "removed override: {path}");
        let mut restored = Session::new();
        restored.load_prefs_json(&s.prefs_to_json()).unwrap();
        assert_eq!(restored.prefs(), s.prefs(), "removal survives saving: {path}");
    }
    assert_eq!(s.execute("prefs.get", json!({"path": "shortcuts"})).unwrap(), json!({"edit.redo": "Ctrl+Alt+Y"}));
    assert_eq!(s.execute("prefs.get", json!({"path": "menus.colors"})).unwrap(), json!({"edit.fill": "red"}));
    assert_eq!(s.execute("prefs.get", json!({"path": "interface.theme"})).unwrap(), json!("studioLight"));
    assert_eq!(s.prefs().shortcut("edit.undo", undo), undo);
}

#[test]
fn reset_keyed_overrides_is_idempotent_and_restores_disabled_shortcuts() {
    let mut s = session();
    s.execute("prefs.set", json!({"path": "shortcuts.edit.undo", "value": ""})).unwrap();
    let undo = crate::commands::find("edit.undo").unwrap().shortcut;
    assert_eq!(s.prefs().shortcut("edit.undo", undo), None);
    for path in ["shortcuts.edit.undo", "menus.colors.edit.fill"] {
        for _ in 0..2 {
            assert_eq!(s.execute("prefs.reset", json!({"section": path})).unwrap(), Value::Null);
        }
    }
    assert_eq!(s.prefs().shortcut("edit.undo", undo), undo);
    assert_eq!(s.prefs(), &Preferences::default());
}

#[test]
fn preferences_reset_removes_keyed_overrides() {
    let mut p = Preferences::default();
    for (path, value) in [("shortcuts.edit.undo", json!("Ctrl+Z")), ("menus.colors.edit.fill", json!("red"))] {
        p.set(path, value).unwrap();
        assert!(p.get(path).is_some());
        p.reset(Some(path)).unwrap();
        assert!(p.get(path).is_none());
        p.reset(Some(path)).unwrap();
    }
    assert_eq!(p, Preferences::default());
}

#[test]
fn keyed_reset_keeps_section_resets_and_unknown_path_errors() {
    let mut s = session();
    s.execute("prefs.set", json!({"values": {"shortcuts.edit.undo": "Ctrl+Z", "menus.colors.edit.fill": "red"}})).unwrap();
    let saved = s.prefs_value();
    let revision = s.prefs.rev();
    for path in ["shortcut.edit.undo", "menus.color.edit.fill", "menus.notAField", "general.notAField"] {
        assert!(matches!(s.execute("prefs.reset", json!({"path": path})), Err(EngineError::BadParams { .. })));
        assert_eq!(s.prefs_value(), saved, "rejected reset changes nothing: {path}");
        assert_eq!(s.prefs.rev(), revision);
    }
    assert_eq!(s.execute("prefs.reset", json!({"path": "shortcuts"})).unwrap(), json!({}));
    assert_eq!(s.execute("prefs.get", json!({"path": "menus.colors"})).unwrap(), json!({"edit.fill": "red"}));
    assert_eq!(s.execute("prefs.reset", json!({"path": "menus.colors"})).unwrap(), json!({}));
    s.execute("prefs.set", json!({"path": "shortcuts.edit.undo", "value": "Ctrl+Z"})).unwrap();
    let all = s.execute("prefs.reset", json!({})).unwrap();
    assert_eq!(all["shortcuts"], json!({}));
    assert_eq!(all["menus"]["colors"], json!({}));
    assert_eq!(s.prefs(), &Preferences::default());
}

#[test]
fn set_validates_and_is_all_or_nothing() {
    let mut s = session();
    for (path, value) in [
        ("cursors.painting", json!("huge")),
        ("performance.historyStates", json!(0)),
        ("performance.historyStates", json!("ten")),
        ("guidesGridAndSlices.guideColor", json!("cyan")),
        ("nope.nothing", json!(1)),
        ("general.notAField", json!(true)),
        ("shortcuts.edit.undo", json!("Cmd+Banana+Q")),
    ] {
        assert!(s.execute("prefs.set", json!({"path": path, "value": value})).is_err(), "{path}");
    }
    let r = s.execute("prefs.set", json!({"values": {"cursors.painting": "precise", "performance.historyStates": -4}}));
    assert!(r.is_err());
    assert_eq!(s.prefs().cursors.painting, PaintingCursor::NormalTip, "nothing applied");
    let r = s.execute("prefs.set", json!({"values": {"cursors.painting": "precise", "unitsAndRulers.rulers": "cm"}})).unwrap();
    assert_eq!(r["cursors.painting"], "precise");
    assert_eq!(s.prefs().units_and_rulers.rulers, Unit::Centimeters);
}

#[test]
fn whole_section_set_merges_keys() {
    let mut s = session();
    s.execute("prefs.set", json!({"path": "transparencyAndGamut", "value": {"gridSize": "large", "gridColors": "custom", "customDark": "#336699"}})).unwrap();
    let t = &s.prefs().transparency_and_gamut;
    assert_eq!(t.square(), Some(16.0));
    assert_eq!(t.colors()[1], [0x33, 0x66, 0x99]);
    assert_eq!(t.gamut_warning_color, "#808080", "untouched keys keep their values");
}

#[test]
fn json_round_trip_tolerates_unknown_and_missing_keys() {
    let mut s = session();
    s.execute("prefs.set", json!({"values": {"interface.theme": "studioLight", "fileHandling.autosaveMinutes": 5, "shortcuts.edit.fill": "Cmd+Shift+F"}}))
        .unwrap();
    s.execute("edit.colorSettings", json!({"workingRgb": "display-p3", "policyRgb": "convert"})).unwrap();
    let text = s.prefs_to_json();
    let mut t = Session::new();
    t.load_prefs_json(&text).unwrap();
    assert_eq!(t.prefs(), s.prefs());
    assert_eq!(t.color.settings.working_rgb, "display-p3");
    assert_eq!(t.color.settings.policy_rgb, crate::color_cmds::Policy::Convert);
    // Older/newer files: unknown keys ignored, missing ones default.
    let mut u = Session::new();
    u.load_prefs_json(r#"{"general": {"beepWhenDone": true, "futureThing": 1}, "someNewSection": {}}"#).unwrap();
    assert!(u.prefs().general.beep_when_done);
    assert_eq!(u.prefs().performance.history_states, 50);
    assert!(u.load_prefs_json("not json").is_err());
}

#[test]
fn low_resolution_previews_default_on_and_switch_off() {
    // A preferences file from before the setting keeps the fast previews.
    let mut s = Session::new();
    s.load_prefs_json(r#"{"performance": {"historyStates": 20}}"#).unwrap();
    assert!(s.prefs().performance.low_resolution_previews);
    s.execute("prefs.set", json!({"path": "performance.lowResolutionPreviews", "value": false})).unwrap();
    assert!(!s.prefs().performance.low_resolution_previews);
    let mut t = Session::new();
    t.load_prefs_json(&s.prefs_to_json()).unwrap();
    assert!(!t.prefs().performance.low_resolution_previews);
}

#[test]
fn units_convert_both_ways() {
    for u in [Unit::Pixels, Unit::Inches, Unit::Centimeters, Unit::Millimeters, Unit::Points, Unit::Picas, Unit::Percent] {
        let v = u.from_px(450.0, 300.0, 900.0, 72.0);
        assert!((u.to_px(v, 300.0, 900.0, 72.0) - 450.0).abs() < 1e-9, "{u:?}");
    }
    assert_eq!(Unit::Inches.from_px(600.0, 300.0, 0.0, 72.0), 2.0);
    assert_eq!(Unit::Points.from_px(300.0, 300.0, 0.0, 72.0), 72.0);
    assert_eq!(Unit::Percent.from_px(450.0, 300.0, 900.0, 72.0), 50.0);
    let r = UnitsAndRulers { rulers: Unit::Centimeters, ..Default::default() };
    assert_eq!(r.format(300.0, 300.0, 0.0), "2.54");
}

#[test]
fn shortcuts_normalise_and_detect_conflicts() {
    assert_eq!(normalize_shortcut("shift+cmd+n").as_deref(), Some("Cmd+Shift+N"));
    assert_eq!(normalize_shortcut("Alt+Cmd+f7").as_deref(), Some("Cmd+Alt+F7"));
    assert_eq!(normalize_shortcut("Cmd++").as_deref(), Some("Cmd++"));
    assert_eq!(normalize_shortcut("Cmd+Shift"), None);
    assert_eq!(normalize_shortcut("Cmd+A+B"), None);
    let c = conflicts([("a", "Cmd+J"), ("b", "cmd+j"), ("c", "Cmd+K")]);
    assert_eq!(c, vec![("Cmd+J".to_string(), vec!["a".to_string(), "b".to_string()])]);
}

#[test]
fn keyboard_shortcuts_reassign_and_reset() {
    let mut s = session();
    let undo = crate::commands::find("edit.undo").unwrap().shortcut;
    assert_eq!(s.prefs().shortcut("edit.undo", undo), Some("Cmd+Z"));
    // Give ⌘Z to Fill: Undo loses it (Photoshop moves a shortcut rather than duplicating it).
    let r = s.execute("edit.keyboardShortcuts", json!({"set": {"edit.fill": "Cmd+Z"}})).unwrap();
    assert_eq!(s.prefs().shortcut("edit.fill", None), Some("Cmd+Z"));
    assert_eq!(s.prefs().shortcut("edit.undo", undo), None);
    assert!(r["conflicts"].as_array().unwrap().is_empty());
    // Keeping the duplicate reports a conflict.
    let r = s.execute("edit.keyboardShortcuts", json!({"set": {"edit.undo": "Cmd+Z"}, "removeConflicts": false})).unwrap();
    assert_eq!(r["conflicts"][0]["shortcut"], "Cmd+Z");
    assert!(s.execute("edit.keyboardShortcuts", json!({"set": {"no.such.command": "F9"}})).is_err());
    s.execute("edit.keyboardShortcuts", json!({"reset": true})).unwrap();
    assert!(s.prefs().shortcuts.is_empty());
    assert_eq!(s.prefs().shortcut("edit.undo", undo), Some("Cmd+Z"));
    // Listing with a filter.
    let r = s.execute("edit.keyboardShortcuts", json!({"filter": "gaussian"})).unwrap();
    assert!(r["commands"].as_array().unwrap().iter().any(|c| c["id"] == "filter.blur.gaussianBlur"));
}

/// #249: temporary tools (held keys) and the fill / colour keys are bindable like commands.
#[test]
fn temporary_tools_and_fill_keys_are_rebindable() {
    let mut s = session();
    let r = s.execute("edit.keyboardShortcuts", json!({"filter": "tools.temporary"})).unwrap();
    let ids: Vec<&str> = r["commands"].as_array().unwrap().iter().filter_map(|c| c["id"].as_str()).collect();
    assert!(ids.contains(&"tools.temporary.hand") && ids.contains(&"tools.temporary.zoomIn") && ids.contains(&"tools.temporary.zoomOut"), "{ids:?}");
    assert!(r["commands"].as_array().unwrap().iter().all(|c| c["hold"] == true));
    // Rebind the temporary zoom; an unknown id still fails.
    s.execute("edit.keyboardShortcuts", json!({"set": {"tools.temporary.zoomIn": "Ctrl+Space"}})).unwrap();
    assert_eq!(s.prefs().shortcut("tools.temporary.zoomIn", Some("Cmd+Space")), Some("Ctrl+Space"));
    assert!(s.execute("edit.keyboardShortcuts", json!({"set": {"tools.temporary.unknown": "Space"}})).is_err());
    // Taking a temporary tool's key for a command removes it from the temporary tool.
    s.execute("edit.keyboardShortcuts", json!({"set": {"edit.fillForeground": "Space"}})).unwrap();
    assert_eq!(s.prefs().shortcut("tools.temporary.hand", Some("Space")), None);
    assert_eq!(s.prefs().shortcut("edit.fillForeground", Some("Alt+Backspace")), Some("Space"));
    // D / X and the fill keys are listed with their Photoshop defaults.
    let r = s.execute("edit.keyboardShortcuts", json!({"list": true})).unwrap();
    let def = |id: &str| r["commands"].as_array().unwrap().iter().find(|c| c["id"] == id).map(|c| c["default"].clone());
    assert_eq!(def("tools.swapColors"), Some(json!("X")));
    assert_eq!(def("tools.defaultColors"), Some(json!("D")));
    assert_eq!(def("edit.fillBackground"), Some(json!("Cmd+Backspace")));
    // Bad values fail.
    assert!(s.execute("edit.keyboardShortcuts", json!({"set": {"tools.temporary.hand": 7}})).is_err());
}

#[test]
fn menus_and_toolbar_customisation_persist() {
    let mut s = session();
    s.execute("edit.menus", json!({"hide": ["edit.fade"], "color": {"edit.fill": "red"}})).unwrap();
    assert_eq!(s.prefs().menus.hidden, vec!["edit.fade".to_string()]);
    assert_eq!(s.prefs().menus.colors["edit.fill"], "red");
    assert!(s.execute("edit.menus", json!({"color": {"edit.fill": "plaid"}})).is_err());
    s.execute("edit.menus", json!({"show": ["edit.fade"], "color": {"edit.fill": "none"}})).unwrap();
    assert!(s.prefs().menus.hidden.is_empty() && s.prefs().menus.colors.is_empty());
    s.execute("edit.toolbar", json!({"hidden": ["Sponge"]})).unwrap();
    let back: Preferences = serde_json::from_str(&s.prefs_to_json()).unwrap();
    assert_eq!(back.toolbar.hidden, vec!["Sponge".to_string()]);
}

#[test]
fn preference_sections_are_commands() {
    let mut s = Session::new();
    for (id, title) in SECTIONS {
        let r = s.execute(&format!("edit.preferences.{id}"), json!({})).unwrap();
        assert_eq!(r["section"], id);
        assert_eq!(r["title"], title);
        assert!(r["values"].is_object(), "{id}");
    }
}

#[test]
fn choice_and_range_tables_cover_enum_fields() {
    let p = Preferences::default().to_json();
    for (id, _) in SECTIONS {
        for (k, v) in p[id].as_object().unwrap() {
            let path = format!("{id}.{k}");
            if let Some(c) = choices(&path) {
                if path == "performance.renderingMode" && v.is_null() {
                    assert_eq!(p["performance"]["renderingMode"], json!(null), "legacy mode is inferred until explicitly selected");
                } else {
                    assert!(c.contains(&v.as_str().unwrap()), "{path}");
                }
            }
            if let Some((lo, hi)) = range(&path) {
                let x = v.as_f64().unwrap();
                assert!(x >= lo && x <= hi, "{path} default {x} outside {lo}..{hi}");
            }
            if is_color(&path) {
                assert!(parse_hex(v.as_str().unwrap()).is_some(), "{path}");
            }
        }
    }
}

#[test]
fn right_click_with_painting_tools_pref() {
    let mut s = session();
    assert_eq!(s.prefs().tools.right_click_with_painting_tools, RightClickPaint::BrushPicker, "Photoshop: the Brush Preset picker");
    assert_eq!(s.execute("prefs.get", json!({"path": "tools.rightClickWithPaintingTools"})).unwrap(), json!("brushPicker"));
    assert_eq!(choices("tools.rightClickWithPaintingTools"), Some(RightClickPaint::NAMES));
    s.execute("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": "erase"})).unwrap();
    assert_eq!(s.prefs().tools.right_click_with_painting_tools, RightClickPaint::Erase);
    for bad in [json!("smudge"), json!(1), json!(null), json!(["erase"])] {
        assert!(s.execute("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": bad})).is_err());
    }
    assert_eq!(s.prefs().tools.right_click_with_painting_tools, RightClickPaint::Erase, "rejected values change nothing");
    // Preferences saved before the option existed load with the default.
    let old: Tools = serde_json::from_value(json!({"showTooltips": false})).unwrap();
    assert_eq!(old.right_click_with_painting_tools, RightClickPaint::BrushPicker);
}

#[test]
fn gpu_backend_round_trips_and_validates() {
    let mut s = session();
    assert_eq!(s.prefs().performance.gpu_backend, GpuBackend::Auto);
    assert_eq!(s.execute("prefs.get", json!({"path": "performance.gpuBackend"})).unwrap(), json!("auto"));
    s.execute("prefs.set", json!({"path": "performance.gpuBackend", "value": "dx12"})).unwrap();
    assert_eq!(s.prefs().performance.gpu_backend, GpuBackend::Dx12);
    // Unknown names and wrong types are errors, and leave the value alone.
    assert!(s.execute("prefs.set", json!({"path": "performance.gpuBackend", "value": "directx"})).is_err());
    assert!(s.execute("prefs.set", json!({"path": "performance.gpuBackend", "value": 3})).is_err());
    assert_eq!(s.prefs().performance.gpu_backend, GpuBackend::Dx12);
    // Persisted and restored with the rest of the preferences.
    let text = s.prefs_to_json();
    let mut t = Session::new();
    t.load_prefs_json(&text).unwrap();
    assert_eq!(t.prefs().performance.gpu_backend, GpuBackend::Dx12);
    // Files from before the setting load as `auto`.
    let mut u = Session::new();
    u.load_prefs_json(r#"{"performance": {"useGpu": true}}"#).unwrap();
    assert_eq!(u.prefs().performance.gpu_backend, GpuBackend::Auto);
    assert_eq!(choices("performance.gpuBackend"), Some(GpuBackend::NAMES));
    for n in GpuBackend::NAMES {
        assert_eq!(GpuBackend::parse(n).map(GpuBackend::name), Some(*n));
    }
}
