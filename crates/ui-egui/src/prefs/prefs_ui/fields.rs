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
            if section == "interface" {
                tweak(&mut rules, &path, |r| r.group(interface_group(key)));
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
    // WinTab is a Windows-only opt-in, drawn with its note; the row labels itself.
    if cfg!(target_os = "windows") {
        tweak(&mut rules, "tools.tabletInput", |r| r.custom_rows());
    } else {
        tweak(&mut rules, "tools.tabletInput", |r| r.hidden());
    }

    // Rows the app draws itself (`PrefsCustom`).
    for path in [
        "performance.memoryUsagePercent",
        "performance.gpuAdapter",
        "tools.pressureCurve",
        "scratchDisks.disks",
        "fileHandling.recentFiles",
        "type.favoriteFonts",
    ] {
        tweak(&mut rules, path, |r| r.custom());
    }
    // Rows that draw their own label or heading, so the window adds none: the Appearance page (Appearance Mode, the
    // themes), Rendering Mode's select and the Memory Usage block.
    for path in ["interface.appearanceMode", "performance.renderingMode", "performance.memoryUsageMb"] {
        tweak(&mut rules, path, |r| r.custom_rows());
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

/// The Interface group of `key`: Appearance (the mode, its themes and the canvas), Presentation (language, font size,
/// scale) and Options for the rest, as in Photoshop. Groups show in the order of their first row, so
/// `prefs::Interface` lists its fields in this order.
fn interface_group(key: &str) -> &'static str {
    match key {
        "appearanceMode" | "darkTheme" | "lightTheme" | "favoriteThemes" | "canvasColor" | "canvasCustomColor" | "canvasBorder" => "Appearance",
        "language" | "uiFontSize" | "uiScale" => "Presentation",
        _ => "Options",
    }
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
