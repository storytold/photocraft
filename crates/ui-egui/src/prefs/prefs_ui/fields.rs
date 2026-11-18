//! The Preferences window's settings struct and rules: which engine preferences the window edits, and what each
//! row looks like beyond its type.

use std::collections::BTreeMap;

use craft::ui::patterns::preferences::SettingRule;
use photocraft_engine::prefs::{self, SECTIONS};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The engine's editable preference pages, by their serde names: the settings struct the window's document is
/// built over. The engine's other preference fields (shortcuts, layouts, remembered dialogs) are not Preferences
/// pages and stay out of the document.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct PrefPages {
    general: prefs::General,
    interface: prefs::Interface,
    workspace: prefs::Workspace,
    tools: prefs::Tools,
    history_log: prefs::HistoryLog,
    file_handling: prefs::FileHandling,
    export: prefs::Export,
    performance: prefs::Performance,
    scratch_disks: prefs::ScratchDisks,
    cursors: prefs::Cursors,
    transparency_and_gamut: prefs::TransparencyAndGamut,
    units_and_rulers: prefs::UnitsAndRulers,
    guides_grid_and_slices: prefs::GuidesGridAndSlices,
    plug_ins: prefs::PlugIns,
    #[serde(rename = "type")]
    type_: prefs::TypePrefs,
    enhanced_controls: prefs::EnhancedControls,
    raw_defaults: prefs::RawDefaults,
    integrations: prefs::Integrations,
}

impl PrefPages {
    /// The pages of `prefs`.
    pub(super) fn of(prefs: &prefs::Preferences) -> Self {
        PrefPages {
            general: prefs.general.clone(),
            interface: prefs.interface.clone(),
            workspace: prefs.workspace.clone(),
            tools: prefs.tools.clone(),
            history_log: prefs.history_log.clone(),
            file_handling: prefs.file_handling.clone(),
            export: prefs.export.clone(),
            performance: prefs.performance.clone(),
            scratch_disks: prefs.scratch_disks.clone(),
            cursors: prefs.cursors.clone(),
            transparency_and_gamut: prefs.transparency_and_gamut.clone(),
            units_and_rulers: prefs.units_and_rulers.clone(),
            guides_grid_and_slices: prefs.guides_grid_and_slices.clone(),
            plug_ins: prefs.plug_ins.clone(),
            type_: prefs.type_.clone(),
            enhanced_controls: prefs.enhanced_controls.clone(),
            raw_defaults: prefs.raw_defaults.clone(),
            integrations: prefs.integrations.clone(),
        }
    }

    /// The window's applied edits as engine calls: the `values` map of `prefs.set` (dotted key to value), and the
    /// paths of keys with no value, which `prefs.reset` takes one by one.
    pub(super) fn edits_to_values<V: Serialize>(edits: Vec<(String, Option<V>)>) -> (Value, Vec<String>) {
        let mut values = Map::new();
        let mut resets = Vec::new();
        for (key, value) in edits {
            match value {
                Some(value) => {
                    if let Ok(json) = serde_json::to_value(&value) {
                        values.insert(key, json);
                    }
                }
                None => resets.push(key),
            }
        }
        (Value::Object(values), resets)
    }
}

/// The rules of every setting from the engine's schema: choices, ranges, colours, hidden settings, the ones read
/// at launch, and the labels and groups the window shows.
pub(super) fn rules() -> Vec<SettingRule> {
    let defaults = prefs::Preferences::default().to_json();
    let mut rules: BTreeMap<String, SettingRule> = BTreeMap::new();
    for (section, _) in SECTIONS {
        let Some(table) = defaults.get(section).and_then(Value::as_object) else { continue };
        for (key, default) in table {
            let path = format!("{section}.{key}");
            if let Some(label) = label_override(key) {
                tweak(&mut rules, &path, |r| r.label(label));
            }
            if path == "interface.language" {
                let mut pairs: Vec<(String, String)> = vec![("auto".into(), "Auto".into())];
                pairs.extend(crate::i18n::Lang::for_picker().into_iter().map(|l| (l.code().to_string(), l.name().to_string())));
                let refs: Vec<(&str, &str)> = pairs.iter().map(|(id, label)| (id.as_str(), label.as_str())).collect();
                tweak(&mut rules, &path, |r| r.choices(&refs));
            } else if let Some(options) = prefs::choices(&path) {
                let labelled: Vec<(&str, String)> = options.iter().map(|id| (*id, choice_label(id))).collect();
                let refs: Vec<(&str, &str)> = labelled.iter().map(|(id, label)| (*id, label.as_str())).collect();
                tweak(&mut rules, &path, |r| r.choices(&refs));
            } else if let Some((lo, hi)) = prefs::range(&path) {
                if default.is_i64() || default.is_u64() {
                    tweak(&mut rules, &path, |r| r.int(lo as i64..=hi as i64));
                } else if default.is_f64() {
                    tweak(&mut rules, &path, |r| r.float(lo..=hi, 3));
                }
            } else if default.is_string() && prefs::is_color(&path) {
                tweak(&mut rules, &path, |r| r.color());
            }
            if prefs::HIDDEN_UNTIL_IMPLEMENTED.contains(&path.as_str()) {
                tweak(&mut rules, &path, |r| r.hidden());
            }
            if prefs::LINUX_ONLY.contains(&path.as_str()) {
                tweak(&mut rules, &path, |r| r.linux_only());
            }
            if read_at_launch(&path) {
                tweak(&mut rules, &path, |r| r.restart());
            }
        }
    }

    // Interface groups, in Photoshop's order.
    for key in ["canvasColor", "canvasCustomColor", "canvasBorder"] {
        tweak(&mut rules, &format!("interface.{key}"), |r| r.group("Appearance"));
    }
    for key in ["language", "uiFontSize", "uiScale"] {
        tweak(&mut rules, &format!("interface.{key}"), |r| r.group("Presentation"));
    }

    // Drawn elsewhere or not at all: the theme keys follow the Appearance row, View › Rulers sets showRulers, and the
    // GPU row draws the adapter, backend and GPU switch together.
    for path in [
        "interface.theme",
        "interface.lightTheme",
        "interface.darkTheme",
        "interface.favoriteThemes",
        "unitsAndRulers.showRulers",
        "performance.useGpu",
        "performance.gpuBackend",
    ] {
        tweak(&mut rules, path, |r| r.hidden());
    }
    // The web build has no Agents page (`custom_rows::AGENTS_PAGE`): it shows the settings PhotoCraft had before the
    // ArtCraft Launcher's switch.
    if cfg!(target_arch = "wasm32") {
        for key in ["connectAgents", "readRoot", "writeRoot", "whenAppClosed"] {
            tweak(&mut rules, &format!("integrations.{key}"), |r| r.hidden());
        }
    }
    // The browser presents frames itself: Low-Latency Drawing does nothing there.
    if cfg!(target_arch = "wasm32") {
        tweak(&mut rules, "performance.lowLatencyDrawing", |r| r.hidden());
    }
    // WinTab is a Windows-only opt-in, drawn with its note.
    if cfg!(target_os = "windows") {
        tweak(&mut rules, "tools.tabletInput", |r| r.custom());
    } else {
        tweak(&mut rules, "tools.tabletInput", |r| r.hidden());
    }

    // Rows the app draws itself (`PrefsCustom`).
    for path in [
        "performance.memoryUsagePercent",
        "performance.memoryUsageMb",
        "performance.renderingMode",
        "performance.gpuAdapter",
        "tools.pressureCurve",
        "scratchDisks.disks",
        "fileHandling.recentFiles",
        "type.favoriteFonts",
        "interface.appearanceMode",
    ] {
        tweak(&mut rules, path, |r| r.custom());
    }
    // The favourite fonts are the user's library, not a Type setting.
    tweak(&mut rules, "type.favoriteFonts", |r| r.keep_on_reset());

    // JPEG and WebP have independent settings; unrelated format controls stay out of view.
    tweak(&mut rules, "export.jpegQuality", |r| r.shown_when("export.quickExportFormat", "jpg"));
    tweak(&mut rules, "export.webpLossless", |r| r.shown_when("export.quickExportFormat", "webp"));
    tweak(&mut rules, "export.webpQuality", |r| r.shown_when("export.quickExportFormat", "webp").shown_when("export.webpLossless", "false"));
    tweak(&mut rules, "export.convertToSrgb", |r| r.shown_unless("export.quickExportFormat", "exr"));
    tweak(&mut rules, "export.metadata", |r| r.shown_unless("export.quickExportFormat", "exr"));
    // The pause that bundles arrow-key nudges means nothing with Bundle Arrow-Key Nudges off.
    tweak(&mut rules, "tools.nudgeBundlePauseMs", |r| r.shown_when("tools.bundleNudges", "true"));

    rules.into_values().collect()
}

/// Changes the rule of `key`, starting one when it has none.
fn tweak(rules: &mut BTreeMap<String, SettingRule>, key: &str, change: impl FnOnce(SettingRule) -> SettingRule) {
    let rule = rules.remove(key).unwrap_or_else(|| SettingRule::new(key));
    rules.insert(key.to_string(), change(rule));
}

/// The English label of the keys that have their own wording; the rest the window spells out from the key.
fn label_override(key: &str) -> Option<&'static str> {
    Some(match key {
        "appearanceMode" => "Appearance Mode",
        "exrAcesCg" => "Open color-less EXR as ACEScg",
        "darkTheme" => "Dark Theme",
        "lightTheme" => "Light Theme",
        "automaticallyCheckUpdates" => "Automatically Check for Updates",
        // Units & Rulers › Column Size (in points), as translated for New Guide Layout.
        "columnWidth" => "Column width",
        // These controls appear only for WebP, so they reuse the translated labels.
        "webpLossless" => "Lossless",
        "webpQuality" => "Quality",
        "notificationAutoHide" => "Auto Hide Notifications",
        "notificationDurationSeconds" => "Notification Duration (seconds)",
        "bundleNudges" => "Bundle Arrow-Key Nudges",
        "nudgeBundlePauseMs" => "Nudge Bundle Pause (ms)",
        "doNotAppendCopy" => "Do not append \"copy\" to filename when saving a copy",
        "pcraftFullComposite" => "Save Full-Size Composite in .pcraft Files",
        "generatorEnabled" => "Enable Generator",
        "lowLatencyDrawing" => "Low-Latency Drawing (vsync off, applies at next launch)",
        "sizeLimitMb" => "Scratch Size Limit (MB, 0 = No Limit)",
        "gpuMemoryMb" => "GPU Memory (MB, 0 = Automatic)",
        _ => return None,
    })
}

/// The English label of a stored choice value (`windowsInk` -> "Windows Ink (default)").
pub(super) fn choice_label(v: &str) -> String {
    match v {
        "windowsInk" => "Windows Ink (default)".into(),
        "winTab" => "WinTab".into(),
        "exr" => "EXR".into(),
        "cm" => "Centimeters".into(),
        "mm" => "Millimeters".into(),
        "75" | "80" | "85" | "90" | "95" | "100" | "105" | "110" | "115" | "120" | "125" | "150" | "175" | "200" | "250" | "300" => format!("{v}%"),
        "8" => "8 Bits/Channel".into(),
        "16" => "16 Bits/Channel".into(),
        "postScript" => "PostScript (72 points/inch)".into(),
        "traditional" => "Traditional (72.27 points/inch)".into(),
        "vulkan" => "Vulkan".into(),
        "dx12" => "DirectX 12".into(),
        "metal" => "Metal".into(),
        "gl" => "OpenGL".into(),
        "ahd" => "AHD (adaptive homogeneity)".into(),
        "mhc" => "Malvar–He–Cutler".into(),
        "cpu" => "CPU (no GPU acceleration)".into(),
        // MCP › When PhotoCraft is closed.
        "launch" => "Open PhotoCraft".into(),
        "headless" => "Work without a window".into(),
        "error" => "Ask to open PhotoCraft".into(),
        v => humanize(v),
    }
}

/// An editable `#rrggbb` field beside a colour swatch (#668): typing a valid value sets the
/// colour; while it's being typed a partial value is kept, and it shows the colour otherwise.
pub(super) fn hex_field(ui: &mut egui::Ui, key: &str, rgb: &mut [u8; 3]) {
    let id = egui::Id::unique(("pref-hex", key));
    let shown = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
    let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| shown.clone());
    let r = crate::widgets::text_field_ex(ui, id, &mut text, "", 72.0, true, Some(7)).accessible_name(tl!("Color"));
    if r.changed()
        && let Some(c) = prefs::parse_hex(&text)
    {
        *rgb = c;
    }
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
}

pub(super) fn color_of(s: &str) -> Color32 {
    prefs::parse_hex(s).map_or(Color32::GRAY, |c| Color32::from_rgb(c[0], c[1], c[2]))
}

/// Search the same visible, editable preference labels used in the dialog.
pub(super) fn preference_matches(section: &str, key: &str, query: &str) -> bool {
    let label = humanize(key);
    let en = crate::i18n::Lang::EN;
    en.contains_ignore_case(&format!("{section}.{key}"), query)
        || en.contains_ignore_case(&label, query)
        || crate::i18n::current().contains_ignore_case(tl!(&label), query)
        // The Tablet input row and its Pen Tablet note are the visible terms, not just the key.
        || (cfg!(target_os = "windows") && section == "tools" && key == "tabletInput" && en.contains_ignore_case("windows ink pen tablet wintab tablet input", query))
}

pub(super) fn preference_sections(values: &Value, search: &str) -> Vec<(&'static str, &'static str)> {
    let query = search.trim();
    SECTIONS
        .iter()
        .copied()
        .filter(|(id, title)| {
            query.is_empty()
                || crate::i18n::Lang::EN.contains_ignore_case(title, query)
                || crate::i18n::current().contains_ignore_case(tl!(title), query)
                || values
                    .get(*id)
                    .and_then(Value::as_object)
                    .is_some_and(|fields| fields.keys().any(|key| !prefs::is_hidden(&format!("{id}.{key}")) && preference_matches(id, key, query)))
        })
        .collect()
}

/// Preferences: section list on the left, the section's settings on the right.
fn prefs_body(ui: &mut egui::Ui, f: &mut Map<String, Value>, system: Option<egui::Theme>) {
    let t = Tokens::get(ui.ctx());
    let mut section = f.get("section").and_then(Value::as_str).unwrap_or("general").to_string();
    let mut values = f.get("values").cloned().unwrap_or(Value::Null);
    // The dialog follows the language being edited, so a change shows before OK.
    let lang = crate::i18n::Lang::from_pref(values.pointer("/interface/language").and_then(Value::as_str).unwrap_or("auto"));
    let mut search = f.get("__search").and_then(Value::as_str).unwrap_or("").to_string();
    crate::widgets::text_field(ui, ui.scope_id().with("prefs-search"), &mut search, tl!("Search settings…"), ui.available_width(), false);
    let found = preference_sections(&values, &search);
    if !found.iter().any(|(id, _)| *id == section)
        && let Some((first, _)) = found.first()
    {
        section = (*first).to_string();
    }
    ui.add_space(6.0);
    ui.horizontal_top(|ui| {
        // Section list.
        ui.vertical(|ui| {
            ui.set_width(170.0);
            if found.is_empty() {
                ui.label(RichText::new(tl!("No matching settings")).color(t.text_dim));
            }
            for &(id, title) in &found {
                let sel = section == id;
                let empty = !has_visible_fields(&values, id);
                let (rect, resp) = ui.allocate_exact_size(vec2(170.0, 22.0), Sense::click());
                if sel {
                    ui.painter().rect_filled(rect, t.radius_sm, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, t.radius_sm, t.hover);
                }
                let color = if sel {
                    t.text
                } else if empty {
                    t.text_faint
                } else {
                    t.text_dim
                };
                // Translated section names can be longer than the column: elide them (the full
                // name is the tooltip) instead of drawing over the settings.
                let mut job = egui::text::LayoutJob::simple_singleline(tl!(title).to_string(), crate::theme::medium(12.5), color);
                job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 12.0);
                let galley = ui.painter().layout_job(job);
                let elided = galley.elided;
                ui.painter().galley(rect.left_center() + vec2(8.0, -galley.size().y / 2.0), galley, color);
                let resp = if elided { resp.on_hover_text(tl!(title)) } else { resp };
                if resp.clicked() {
                    section = id.to_string();
                }
            }
        });
        // The settings fill the dialog's resizable area down to the button row (#7420). Sizing them
        // from the window instead left a resized dialog with empty space and the buttons mid-way.
        // The area grows to fit its content, so this must never ask for more than it was given.
        let content_height = settings_height(ui.available_height(), ui.spacing().item_spacing.y, crate::widgets::button_height(ui.ctx()));
        crate::widgets::vline(ui, content_height);
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - 12.0).max(340.0));
            let title = SECTIONS.iter().find(|(id, _)| *id == section).map_or("General", |(_, t)| *t);
            ui.label(RichText::new(tl!(&title)).font(crate::theme::semibold(14.0)).color(t.text));
            let show_all = search.trim().is_empty()
                || crate::i18n::Lang::EN.contains_ignore_case(title, search.trim())
                || lang.contains_ignore_case(tl!(&title), search.trim());
            let filter = if show_all { "" } else { search.trim() };
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(content_height - 30.0).id_salt("prefs-scroll").show(ui, |ui| {
                let order: Vec<String> =
                    f.get("__order").and_then(|o| o.get(&section)).and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
                if !has_visible_fields(&values, &section) {
                    ui.add_space(4.0);
                    ui.label(RichText::new(tl!("These settings aren't available in PhotoCraft yet.")).color(t.text_faint));
                } else if section == "integrations"
                    && f.get("__agentLink").and_then(|s| s.get("mcpFlag")).and_then(Value::as_bool) == Some(true)
                    && let Some(obj) = values.get_mut(&section).and_then(Value::as_object_mut)
                {
                    super::agents::integration_rows(ui, obj, f);
                    ui.add_space(8.0);
                } else if section == "integrations"
                    && let Some(obj) = values.get_mut(&section).and_then(Value::as_object_mut)
                {
                    // Without the AI-MCP feature flag: the settings PhotoCraft had before.
                    let mut shown: Map<String, Value> =
                        obj.iter().filter(|(k, _)| LEGACY_INTEGRATIONS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
                    section_fields(ui, &section, &mut shown, &order, lang, system, filter);
                    obj.extend(shown);
                } else if let Some(obj) = values.get_mut(&section).and_then(Value::as_object_mut) {
                    section_fields(ui, &section, obj, &order, lang, system, filter);
                    if section == "fileHandling" {
                        ui.label(
                            RichText::new(tl!("0 turns this threshold off; SVG groups nested deeper than 100 levels are always rasterized."))
                                .color(t.text_faint),
                        );
                    }
                    if section == "performance" {
                        gpu_status_rows(ui, f.get("__gpuInfo"), f.get("__gpuAdapters"), obj);
                    }
                    ui.add_space(8.0);
                }
                if search.trim().is_empty()
                    && has_visible_fields(&values, &section)
                    && crate::widgets::secondary_button(ui, tl!("Reset Section"), 110.0).clicked()
                    && let Some(mut def) = prefs::Preferences::default().get(&section)
                {
                    // The favourite fonts are the user's library, not a Type setting: kept, as
                    // `prefs.reset {"path": "type"}` keeps them.
                    if section == "type"
                        && let Some(favorites) = values.get(&section).and_then(|v| v.get("favoriteFonts")).cloned()
                        && let Some(def) = def.as_object_mut()
                    {
                        def.insert("favoriteFonts".into(), favorites);
                    }
                    values[&section] = def;
                }
            });
        });
    });
    if let Some(index) = ui.ctx().data_mut(|data| data.remove_temp::<usize>(egui::Id::unique("prefs-scratch-browse"))) {
        f.insert("__scratchBrowse".into(), json!(index));
    }
    f.insert("section".into(), json!(section));
    f.insert("__search".into(), json!(search));
    f.insert("values".into(), values);
    // A colour swatch was clicked: open the Color Picker on that value (#2144).
    if let Some((path, name)) = ctx_take_pick(ui) {
        crate::color_picker_ui::request_field(f, &format!("/values/{}", path.replace('.', "/")), &crate::color_picker_ui::title_for(&name));
    }
}

/// Does `section` have any setting the dialog shows (see [`prefs::HIDDEN_UNTIL_IMPLEMENTED`])?
pub(super) fn has_visible_fields(values: &Value, section: &str) -> bool {
    values.get(section).and_then(Value::as_object).is_some_and(|o| o.keys().any(|k| !prefs::is_hidden(&format!("{section}.{k}"))))
}

/// JPEG and WebP have independent settings; unrelated format controls stay out of view.
pub(super) fn export_field_visible(obj: &Map<String, Value>, key: &str) -> bool {
    let format = obj.get("quickExportFormat").and_then(Value::as_str).unwrap_or("png");
    match key {
        "convertToSrgb" | "metadata" => format != "exr",
        "jpegQuality" => format == "jpg",
        "webpLossless" => format == "webp",
        "webpQuality" => format == "webp" && obj.get("webpLossless").and_then(Value::as_bool) != Some(true),
        _ => true,
    }
}

/// The pause that bundles arrow-key nudges means nothing with Bundle Arrow-Key Nudges off (#2259).
/// WinTab is a Windows-only opt-in.
pub(super) fn tools_field_visible(obj: &Map<String, Value>, key: &str) -> bool {
    match key {
        "tabletInput" => cfg!(target_os = "windows"),
        "nudgeBundlePauseMs" => obj.get("bundleNudges").and_then(Value::as_bool) != Some(false),
        _ => true,
    }
}

/// A titled, outlined group of settings (Photoshop's Interface page has Appearance,
/// Presentation and Options).
fn pref_group(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    let t = Tokens::get(ui.ctx());
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.card_border)).corner_radius(t.radius).inner_margin(10.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(tl!(title)).strong().color(t.text));
        ui.add_space(4.0);
        add(ui);
    });
    ui.add_space(8.0);
}

/// Preferences › Interface laid out like Photoshop's: the theme and canvas settings, then UI
/// language, font size and scaling, then everything else. Compact enough that the language
/// shows without scrolling in a small window (#2532). A search (`query`, lowercased) hides the
/// rows that don't match and skips a group left empty.
fn interface_groups(ui: &mut egui::Ui, obj: &mut Map<String, Value>, keys: &[String], lang: crate::i18n::Lang, system: Option<egui::Theme>, query: &str) {
    const APPEARANCE: [&str; 3] = ["canvasColor", "canvasCustomColor", "canvasBorder"];
    const PRESENTATION: [&str; 3] = ["language", "uiFontSize", "uiScale"];
    let shown = |k: &str| keys.iter().any(|key| key == k) && field_visible("interface", obj, k, query);
    // In Photoshop's order, not the stored one.
    let pick = |names: &[&str]| names.iter().filter(|n| shown(n)).map(|n| n.to_string()).collect::<Vec<_>>();
    let appearance = pick(&APPEARANCE);
    let presentation = pick(&PRESENTATION);
    let rest = keys.iter().filter(|k| !APPEARANCE.contains(&k.as_str()) && !PRESENTATION.contains(&k.as_str()) && shown(k));
    // Checkboxes line up at the left edge, as in Photoshop; the other options keep a label column.
    let (checks, other): (Vec<String>, Vec<String>) = rest.cloned().partition(|k| obj.get(k).is_some_and(Value::is_boolean));
    let rows = AppearanceRows::matching(query);
    let grid = |id: &str| egui::Grid::new(("prefs-grid", id.to_string())).num_columns(2).spacing([14.0, 7.0]).min_col_width(150.0);
    if rows.any() || !appearance.is_empty() {
        pref_group(ui, "Appearance", |ui| {
            grid("interface-appearance").show(ui, |ui| {
                appearance_rows(ui, obj, system, rows);
                field_rows(ui, "interface", obj, appearance, lang);
            });
        });
    }
    if !presentation.is_empty() {
        pref_group(ui, "Presentation", |ui| {
            grid("interface-presentation").show(ui, |ui| field_rows(ui, "interface", obj, presentation, lang));
        });
    }
    if !checks.is_empty() || !other.is_empty() {
        pref_group(ui, "Options", |ui| {
            egui::Grid::new("prefs-grid-interface-checks").num_columns(2).spacing([0.0, 7.0]).show(ui, |ui| field_rows(ui, "interface", obj, checks, lang));
            grid("interface-options").show(ui, |ui| field_rows(ui, "interface", obj, other, lang));
        });
    }
}

/// Where a clicked colour swatch leaves its preference path and label for the dialog body, which
/// holds the fields the Color Picker request goes in.
/// Preferences › AI Integrations without the AI-MCP feature flag: the settings PhotoCraft had
/// before the agent connection page.
const LEGACY_INTEGRATIONS: [&str; 2] = ["allowAgentControl", "controlPort"];

const PICK_ID: &str = "prefs-pick-color";

/// The colour swatch clicked this frame, if any: (preference path, label).
fn ctx_take_pick(ui: &egui::Ui) -> Option<(String, String)> {
    ui.ctx().data_mut(|d| d.remove_temp::<(String, String)>(egui::Id::unique(PICK_ID)))
}

/// Whether `section.key` gets a generic row for this search (`query`, lowercased; empty matches
/// everything). Settings nothing reads yet stay out of the dialog (issue #204), as do those drawn
/// by their own widgets; their stored values pass through untouched.
pub(super) fn field_visible(section: &str, obj: &Map<String, Value>, k: &str, query: &str) -> bool {
    let path = format!("{section}.{k}");
    let drawn_elsewhere = (section == "interface" && matches!(k, "theme" | "appearanceMode" | "darkTheme" | "lightTheme" | "favoriteThemes"))
        // View › Rulers (⌘R) sets it, as in Photoshop.
        || path == "unitsAndRulers.showRulers"
        || (section == "performance" && matches!(k, "useGpu" | "gpuBackend" | "gpuAdapter" | "renderingMode" | "memoryUsageMb"))
        // The browser presents frames itself: Low-Latency Drawing does nothing there.
        || (cfg!(target_arch = "wasm32") && path == "performance.lowLatencyDrawing")
        // The browser can't start another program.
        || (cfg!(target_arch = "wasm32") && path == "rawDefaults.externalEditor")
        || (section == "export" && !export_field_visible(obj, k))
        || (section == "tools" && (!tools_field_visible(obj, k) || k == "tabletInput"));
    (query.is_empty() || preference_matches(section, k, query))
        && !drawn_elsewhere
        && !prefs::is_hidden(&path)
        && obj.get(k).is_some_and(|v| (path == "performance.memoryUsagePercent" && v.is_null()) || !matches!(v, Value::Null | Value::Object(_)))
}

/// Generic editor for a section's fields: checkboxes, dropdowns for choices, colour swatches,
/// number fields with the preference's range, text fields.
pub(super) fn section_fields(
    ui: &mut egui::Ui,
    section: &str,
    obj: &mut Map<String, Value>,
    order: &[String],
    lang: crate::i18n::Lang,
    system: Option<egui::Theme>,
    search: &str,
) {
    let query = search.trim();
    let mut keys: Vec<String> = order.iter().filter(|k| obj.contains_key(*k)).cloned().collect();
    keys.extend(obj.keys().filter(|k| !order.contains(k)).cloned());
    if section == "interface" {
        interface_groups(ui, obj, &keys, lang, system, query);
        return;
    }
    if section == "performance" && (query.is_empty() || preference_matches(section, "renderingMode", query)) {
        rendering_mode_row(ui, obj);
    }
    if section == "performance" && (query.is_empty() || preference_matches(section, "memoryUsageMb", query)) {
        memory_usage_row(ui, obj);
        v => spelled(v),
    }
}

/// A camelCase id as words (`convertToSrgb` -> "Convert to srgb"), with the acronyms capitalised.
fn spelled(id: &str) -> String {
    let mut s = String::new();
    for (i, ch) in id.chars().enumerate() {
        if i == 0 {
            s.extend(ch.to_uppercase());
        } else if ch.is_uppercase() {
            s.push(' ');
            s.extend(ch.to_lowercase());
        } else {
            s.push(ch);
        }
    }
    s.replace("Psd", "PSD")
        .replace("Gpu", "GPU")
        .replace("Ui ", "UI ")
        .replace("Mb", "(MB)")
        .replace("Exif", "EXIF")
        .replace("Hud", "HUD")
        .replace("Lut ", "LUT ")
}

/// Settings read only when the app starts, which say "Applies at next launch." under their row.
pub(super) fn read_at_launch(path: &str) -> bool {
    match path {
        // Only Windows and Linux draw their own title bar.
        "interface.systemTitleBar" => cfg!(all(not(target_arch = "wasm32"), not(target_os = "macos"))),
        // The WinTab context is opened once, when the window starts.
        "tools.tabletInput" => cfg!(target_os = "windows"),
        _ => false,
    }
}
