use super::appearance::{AppearanceRows, appearance_rows, system_theme};
use super::dialogs::{mismatch_body, presets_body, presets_io_body, settings_height};
use super::graphics::{gpu_memory_lines, gpu_status_rows, memory_usage_row, read_at_launch, rendering_mode_row};
use super::shortcuts::shortcuts_body;
use super::{Color32, Map, PhotocraftApp, RichText, SECTIONS, Sense, Tokens, Value, json, prefs, vec2};

// ------------------------------------------------------------------ bodies

/// Render one of our dialogs' bodies.
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    if crate::text::direction::is_rtl() {
        // The preferences body mirrors as a whole: its blocks right-align and their nested rows
        // read from the right. The dialog host keeps the modal body in egui's default layout, so
        // the scope belongs here, where every preferences page is drawn.
        ui.with_layout(crate::text::direction::body_layout(), |ui| page(app, ui, f));
    } else {
        page(app, ui, f);
    }
}

/// The preferences page the dialog asks for.
fn page(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    match f.get("__prefsui").and_then(Value::as_str).unwrap_or("") {
        "prefs" => {
            let mut info = app.perf.gpu_info.lines();
            info.extend(gpu_memory_lines(app));
            f.insert("__gpuInfo".into(), json!(info));
            f.insert("__gpuAdapters".into(), json!(app.perf.gpu_info.adapters));
            let system = system_theme(app, ui.ctx());
            prefs_body(ui, f, system);
        }
        "shortcuts" => shortcuts_body(app, ui, f),
        "presets" => presets_body(app, ui, f),
        "presetsIO" => presets_io_body(ui, f),
        "mismatch" => mismatch_body(ui, f),
        _ => {}
    }
}

/// The label a preference key shows in the dialog: a few keys have their own wording, the rest
/// split camelCase into words (`convertToSrgb` → "Convert to srgb").
pub(super) fn humanize(key: &str) -> String {
    match key {
        "appearanceMode" => return "Appearance Mode".into(),
        "exrAcesCg" => return "Open color-less EXR as ACEScg".into(),
        "darkTheme" => return "Dark Theme".into(),
        "lightTheme" => return "Light Theme".into(),
        "automaticallyCheckUpdates" => return "Automatically Check for Updates".into(),
        _ => {}
    }
    // These controls appear only for WebP, so reuse the existing translated labels.
    if key == "webpLossless" {
        return "Lossless".into();
    }
    if key == "webpQuality" {
        return "Quality".into();
    }
    // The auto-hide notice settings read best with their own labels (Interface › Notification, #2022).
    if key == "notificationAutoHide" {
        return "Auto Hide Notifications".into();
    }
    if key == "notificationDurationSeconds" {
        return "Notification Duration (seconds)".into();
    }
    // Tools: arrow-key nudges bundle into one History state (#2259).
    if key == "bundleNudges" {
        return "Bundle Arrow-Key Nudges".into();
    }
    if key == "nudgeBundlePauseMs" {
        return "Nudge Bundle Pause (ms)".into();
    }
    // Performance: presentation without vsync (#1343); like the other graphics settings, it
    // takes effect when the window next opens.
    // File Handling: Photoshop's wording (#8080).
    if key == "doNotAppendCopy" {
        return "Do not append \"copy\" to filename when saving a copy".into();
    }
    // Plug-ins: Photoshop's wording (#9392).
    if key == "generatorEnabled" {
        return "Enable Generator".into();
    }
    if key == "lowLatencyDrawing" {
        return "Low-Latency Drawing (vsync off, applies at next launch)".into();
    }
    // Performance: the canvas compositor's GPU memory, 0 for the automatic budget (L3).
    if key == "gpuMemoryMb" {
        return "GPU Memory (MB, 0 = Automatic)".into();
    }
    let mut s = String::new();
    for (i, ch) in key.chars().enumerate() {
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

pub(super) fn choice_label(v: &str) -> String {
    match v {
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
    format!("{section}.{key}").to_lowercase().contains(query) || label.to_lowercase().contains(query) || tl!(&label).to_lowercase().contains(query)
}

pub(super) fn preference_sections(values: &Value, search: &str) -> Vec<(&'static str, &'static str)> {
    let query = search.trim().to_lowercase();
    SECTIONS
        .iter()
        .copied()
        .filter(|(id, title)| {
            query.is_empty()
                || title.to_lowercase().contains(&query)
                || tl!(title).to_lowercase().contains(&query)
                || values
                    .get(*id)
                    .and_then(Value::as_object)
                    .is_some_and(|fields| fields.keys().any(|key| !prefs::is_hidden(&format!("{id}.{key}")) && preference_matches(id, key, &query)))
        })
        .collect()
}

/// Width of the section list column.
const SIDEBAR_W: f32 = 170.0;

/// Preferences: section list on the left, the section's settings on the right.
fn prefs_body(ui: &mut egui::Ui, f: &mut Map<String, Value>, system: Option<egui::Theme>) {
    let t = Tokens::get(ui.ctx());
    let mut section = f.get("section").and_then(Value::as_str).unwrap_or("general").to_string();
    let mut values = f.get("values").cloned().unwrap_or(Value::Null);
    // The dialog follows the language being edited, so a change shows before OK.
    let lang = crate::i18n::Lang::from_pref(values.pointer("/interface/language").and_then(Value::as_str).unwrap_or("auto"));
    let mut search = f.get("__search").and_then(Value::as_str).unwrap_or("").to_string();
    crate::widgets::text_field(ui, ui.scope_id().with("prefs-search"), &mut search, tl!("Search settings…"), crate::text::direction::row_width(ui), false);
    let found = preference_sections(&values, &search);
    if !found.iter().any(|(id, _)| *id == section)
        && let Some((first, _)) = found.first()
    {
        section = (*first).to_string();
    }
    ui.add_space(6.0);
    ui.horizontal_top(|ui| {
        // Section list. Its rect is pinned to the row's reading-start edge: in a mirrored row a
        // child ui inherits the row's whole available rect and lands on top of the settings.
        let row = ui.available_rect_before_wrap();
        let sidebar = if crate::text::direction::is_rtl() {
            egui::Rect::from_min_max(egui::pos2(row.right() - SIDEBAR_W, row.top()), row.right_bottom())
        } else {
            egui::Rect::from_min_max(row.left_top(), egui::pos2(row.left() + SIDEBAR_W, row.bottom()))
        };
        ui.scope_builder(egui::UiBuilder::new().max_rect(sidebar).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
            prefs_sidebar(ui, t, &found, &mut section, &values)
        });
        // The settings fill the dialog's resizable area down to the button row (#7420). Sizing them
        // from the window instead left a resized dialog with empty space and the buttons mid-way.
        // The area grows to fit its content, so this must never ask for more than it was given.
        let content_height = settings_height(ui.available_height(), ui.spacing().item_spacing.y, crate::widgets::button_height(ui.ctx()));
        crate::widgets::vline(ui, content_height);
        // The page's content column follows the body's direction: in RTL its blocks — the title
        // included — align at the reading start and the scroll area's content inherits the layout.
        // LTR is egui's default `top_down(Align::Min)`, unchanged.
        ui.with_layout(crate::text::direction::body_layout(), |ui| {
            ui.set_width((crate::text::direction::row_width(ui) - 12.0).max(340.0));
            let title = SECTIONS.iter().find(|(id, _)| *id == section).map_or("General", |(_, t)| *t);
            ui.label(RichText::new(tl!(&title)).font(crate::theme::semibold(14.0)).color(t.text));
            let show_all = search.trim().is_empty()
                || title.to_lowercase().contains(&search.trim().to_lowercase())
                || tl!(&title).to_lowercase().contains(&search.trim().to_lowercase());
            let filter = if show_all { "" } else { search.trim() };
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(content_height - 30.0).id_salt("prefs-scroll").show(ui, |ui| {
                let order: Vec<String> =
                    f.get("__order").and_then(|o| o.get(&section)).and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
                if !has_visible_fields(&values, &section) {
                    ui.add_space(4.0);
                    ui.label(RichText::new(tl!("These settings aren't available in PhotoCraft yet.")).color(t.text_faint));
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

/// The section list column of the Preferences dialog.
fn prefs_sidebar(ui: &mut egui::Ui, t: Tokens, found: &[(&'static str, &'static str)], section: &mut String, values: &Value) {
    ui.set_width(SIDEBAR_W);
    if found.is_empty() {
        ui.label(RichText::new(tl!("No matching settings")).color(t.text_dim));
    }
    for &(id, title) in found {
        let sel = *section == id;
        let empty = !has_visible_fields(values, id);
        let (rect, resp) = ui.allocate_exact_size(vec2(SIDEBAR_W, 22.0), Sense::click());
        // The row is a named button, so agents and screen readers can find a section.
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!(title).to_string()));
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
        // The galley is painted by hand, so its anchor mirrors explicitly; the row order
        // comes from the dialog body's mirrored layout.
        let x = if crate::text::direction::is_rtl() { rect.right() - 8.0 - galley.size().x } else { rect.left() + 8.0 };
        ui.painter().galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, color);
        let resp = if elided { resp.on_hover_text(tl!(title)) } else { resp };
        if resp.clicked() {
            *section = id.to_string();
        }
    }
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
pub(super) fn tools_field_visible(obj: &Map<String, Value>, key: &str) -> bool {
    key != "nudgeBundlePauseMs" || obj.get("bundleNudges").and_then(Value::as_bool) != Some(false)
}

/// A titled, outlined group of settings (Photoshop's Interface page has Appearance,
/// Presentation and Options).
fn pref_group(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    let t = Tokens::get(ui.ctx());
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.card_border)).corner_radius(t.radius).inner_margin(10.0).show(ui, |ui| {
        ui.set_width(crate::text::direction::row_width(ui));
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
            grid_scope(ui, |ui| {
                grid("interface-appearance").show(ui, |ui| {
                    appearance_rows(ui, obj, system, rows);
                    if !appearance.is_empty() {
                        // The theme rows above are owned by appearance.rs; the canvas rows go
                        // through the same live grid via a row adapter, unchanged in both directions.
                        let ctx = ui.ctx().clone();
                        let mut row = |caption: egui::WidgetText, editor: &mut dyn FnMut(&mut egui::Ui, egui::Id)| {
                            let cap = std::cell::Cell::new(egui::Id::NULL);
                            crate::text::direction::grid_cells(ui, |ui| cap.set(ui.label(caption).id), |ui| editor(ui, cap.get()));
                            ui.end_row();
                        };
                        field_rows(&ctx, "interface", obj, appearance, lang, &mut row);
                    }
                });
            });
        });
    }
    if !presentation.is_empty() {
        pref_group(ui, "Presentation", |ui| {
            let ctx = ui.ctx().clone();
            let label_w = max_label_width(ui, obj, &presentation);
            crate::text::direction::settings_grid(ui, ("prefs-grid", "interface-presentation"), [14.0, 7.0], label_w, Some(150.0), |row| {
                field_rows(&ctx, "interface", obj, presentation, lang, row)
            });
        });
    }
    if !checks.is_empty() || !other.is_empty() {
        pref_group(ui, "Options", |ui| {
            if !checks.is_empty() {
                let ctx = ui.ctx().clone();
                crate::text::direction::settings_grid(ui, "prefs-grid-interface-checks", [0.0, 7.0], 0.0, None, |row| {
                    field_rows(&ctx, "interface", obj, checks, lang, row)
                });
            }
            if !other.is_empty() {
                let ctx = ui.ctx().clone();
                let label_w = max_label_width(ui, obj, &other);
                crate::text::direction::settings_grid(ui, ("prefs-grid", "interface-options"), [14.0, 7.0], label_w, Some(150.0), |row| {
                    field_rows(&ctx, "interface", obj, other, lang, row)
                });
            }
        });
    }
}

/// Where a clicked colour swatch leaves its preference path and label for the dialog body, which
/// holds the fields the Color Picker request goes in.
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
        || (section == "export" && !export_field_visible(obj, k))
        || (section == "tools" && !tools_field_visible(obj, k));
    (query.is_empty() || preference_matches(section, k, query))
        && !drawn_elsewhere
        && !prefs::is_hidden(&path)
        && obj.get(k).is_some_and(|v| !matches!(v, Value::Null | Value::Object(_)))
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
    let query = search.to_lowercase();
    let mut keys: Vec<String> = order.iter().filter(|k| obj.contains_key(*k)).cloned().collect();
    keys.extend(obj.keys().filter(|k| !order.contains(k)).cloned());
    if section == "interface" {
        interface_groups(ui, obj, &keys, lang, system, &query);
        return;
    }
    if section == "performance" && (query.is_empty() || preference_matches(section, "renderingMode", &query)) {
        rendering_mode_row(ui, obj);
    }
    if section == "performance" && (query.is_empty() || preference_matches(section, "memoryUsageMb", &query)) {
        memory_usage_row(ui, obj);
    }
    keys.retain(|k| field_visible(section, obj, k, &query));
    let ctx = ui.ctx().clone();
    let label_w = max_label_width(ui, obj, &keys);
    crate::text::direction::settings_grid(ui, ("prefs-grid", section), [14.0, 7.0], label_w, None, |row| field_rows(&ctx, section, obj, keys, lang, row));
}

/// The width of the widest row label, as the two-column grid's label column sizes to it in LTR
/// (never below the grid's minimum cell width); [`crate::text::direction::settings_grid`] takes
/// it as the RTL label column so both directions share one column geometry. Checkbox rows draw
/// an empty label, so bool keys are excluded — they would inflate the RTL column past the LTR
/// grid's width.
fn max_label_width(ui: &egui::Ui, obj: &Map<String, Value>, keys: &[String]) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    keys.iter()
        .filter(|k| !matches!(obj.get(*k), Some(Value::Bool(_))))
        .map(|k| ui.painter().layout_no_wrap(tl!(&humanize(k)).to_string(), font.clone(), egui::Color32::PLACEHOLDER).size().x)
        .fold(ui.spacing().interact_size.x, f32::max)
}

/// A `Grid` cell whose content hugs the cell's reading-start edge: the fork's `Grid` aligns
/// every cell left, which leaves the right-to-left reading start of a label column ragged.
fn reading_start_cell<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.with_layout(crate::text::direction::row(), add).inner
}

/// The fork's `Grid` cannot place its cells under a right-to-left cursor, so a grid inside the
/// mirrored content column runs in an explicit left-to-right scope; `grid_cells` emits the
/// cells in reading order. A no-op in LTR, where that is the inherited layout already.
pub(super) fn grid_scope<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.with_layout(egui::Layout::top_down(egui::Align::Min), add).inner
}

/// One row per setting in `keys` (already filtered by `field_visible`), emitted through `row`
/// as `row(label, editor)` in reading order; the grid helper places the cells, so the label
/// column keeps its fixed width at the reading start in both directions and the
/// label<->editor gap matches LTR. Checkbox rows pass an empty label so the bare editor hugs
/// the reading start. The editor closure receives the label's Id for accessibility labeling.
fn field_rows(
    ctx: &egui::Context,
    section: &str,
    obj: &mut Map<String, Value>,
    keys: Vec<String>,
    lang: crate::i18n::Lang,
    row: &mut dyn FnMut(egui::WidgetText, &mut dyn FnMut(&mut egui::Ui, egui::Id)),
) {
    let t = Tokens::get(ctx);
    for k in keys {
        let path = format!("{section}.{k}");
        let v = obj.get(&k).cloned().unwrap_or(Value::Null);
        let human = humanize(&k);
        let label = tl!(&human).to_string();
        let caption: egui::WidgetText = RichText::new(tl!(&label)).color(t.text_dim).into();
        match &v {
            Value::Bool(b) => {
                let mut b = *b;
                // The checkbox hugs the row's reading-start edge, as in Photoshop; the empty
                // label takes the other column.
                row(egui::WidgetText::from(""), &mut |ui: &mut egui::Ui, _cap| {
                    if read_at_launch(&path) {
                        reading_start_cell(ui, |ui| {
                            let align = if crate::text::direction::is_rtl() { egui::Align::Max } else { egui::Align::Min };
                            ui.with_layout(egui::Layout::top_down(align), |ui| {
                                crate::widgets::checkbox(ui, &mut b, &label);
                                ui.label(RichText::new(tl!("Applies at next launch.")).color(t.text_dim));
                            });
                        })
                    } else {
                        reading_start_cell(ui, |ui| {
                            crate::widgets::checkbox(ui, &mut b, &label);
                        })
                    }
                });
                obj.insert(k, json!(b));
            }
            Value::String(s) if path == "interface.language" => {
                row(caption, &mut |ui, _cap| {
                    let mut pairs: Vec<(String, &str)> = vec![("auto".into(), tl!("Auto"))];
                    pairs.extend(crate::i18n::Lang::for_picker().into_iter().map(|l| (l.code().to_string(), l.name())));
                    let mut cur = s.clone();
                    crate::widgets::dropdown(ui, &format!("pref-{path}"), tl!(&label), &mut cur, &pairs, 220.0);
                    obj.insert(k.clone(), json!(cur));
                });
            }
            Value::String(s) if prefs::choices(&path).is_some() => {
                let opts = prefs::choices(&path).unwrap_or(&[]);
                let labels: Vec<String> = opts.iter().map(|o| choice_label(o)).collect();
                let pairs: Vec<(String, &str)> = opts.iter().map(|o| o.to_string()).zip(labels.iter().map(String::as_str)).collect();
                let mut cur = s.clone();
                row(caption, &mut |ui, _cap| {
                    crate::widgets::dropdown(ui, &format!("pref-{path}"), tl!(&label), &mut cur, &pairs, 220.0);
                    obj.insert(k.clone(), json!(cur));
                });
            }
            Value::String(s) if prefs::is_color(&path) => {
                let c = prefs::parse_hex(s).unwrap_or([128, 128, 128]);
                let mut rgb = c;
                row(caption, &mut |ui, _cap| {
                    ui.horizontal(|ui| {
                        // PhotoCraft's Color Picker, as everywhere else (#2144).
                        if crate::widgets::color_swatch_button(ui, egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]), &label).clicked() {
                            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique(PICK_ID), (path.clone(), label.clone())));
                        }
                        hex_field(ui, &path, &mut rgb);
                    });
                });
                obj.insert(k, json!(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])));
                let _ = color_of(s);
            }
            Value::String(s) => {
                let mut s = s.clone();
                row(caption, &mut |ui, _cap| {
                    crate::widgets::rtl_singleline_edit(ui, &mut s, 260.0, "").accessible_name(tl!(&label));
                });
                obj.insert(k, json!(s));
            }
            Value::Number(n) => {
                let (lo, hi) = prefs::range(&path).unwrap_or((-1e9, 1e9));
                let mut int_value = None;
                let mut float_value = None;
                row(caption, &mut |ui, cap| {
                    let resp = if n.is_u64() || n.is_i64() {
                        let mut x = n.as_i64().unwrap_or(0);
                        let resp = ui.add(egui::DragValue::new(&mut x).range(lo as i64..=hi as i64).custom_parser(crate::widgets::parse_num));
                        int_value = Some(x);
                        resp
                    } else {
                        let mut x = n.as_f64().unwrap_or(0.0);
                        let resp = ui.add(egui::DragValue::new(&mut x).range(lo..=hi).speed(0.1).max_decimals(3).custom_parser(crate::widgets::parse_num));
                        float_value = Some(x);
                        resp
                    };
                    if cap != egui::Id::NULL {
                        resp.labelled_by(cap);
                    }
                });
                if let Some(x) = int_value {
                    obj.insert(k, json!(x));
                } else if let Some(x) = float_value {
                    obj.insert(k, json!(x));
                }
                // A row of its own, so the label stays level with its field.
                if read_at_launch(&path) {
                    row(egui::WidgetText::from(""), &mut |ui, _cap| {
                        ui.label(RichText::new(tl!("Applies at next launch.")).color(t.text_dim));
                    });
                }
            }
            Value::Array(_) if path == "tools.pressureCurve" => {
                row(caption, &mut |ui, _cap| {
                    if let Some(next) = crate::pressure_curve_ui::editor(ui, &v) {
                        obj.insert(k.clone(), next);
                    }
                });
            }
            Value::Array(items) if k == "disks" => {
                let mut items = items.clone();
                let caption: egui::WidgetText = RichText::new(tl!("Scratch disks")).color(t.text_dim).into();
                row(caption, &mut |ui, _cap| {
                    ui.vertical(|ui| {
                        for d in &mut items {
                            ui.horizontal(|ui| {
                                let mut on = d.get("enabled").and_then(Value::as_bool).unwrap_or(false);
                                let mut path = d.get("path").and_then(Value::as_str).unwrap_or("").to_string();
                                crate::widgets::checkbox(ui, &mut on, tl!("Enabled"));
                                crate::widgets::rtl_singleline_edit(ui, &mut path, 240.0, "").accessible_name(tl!("Path"));
                                *d = json!({"enabled": on, "path": path});
                            });
                        }
                        if ui.small_button(tl!("Add disk")).clicked() {
                            items.push(json!({"enabled": true, "path": ""}));
                        }
                    });
                });
                obj.insert(k, Value::Array(items));
            }
            Value::Array(items) => {
                let count = crate::i18n::trn(lang, items.len() as u64, "{n} item", "{n} items");
                let clear = !items.is_empty();
                row(caption, &mut |ui, _cap| {
                    ui.label(RichText::new(count.clone()).color(t.text_faint));
                    if clear && ui.small_button(tl!("Clear")).clicked() {
                        obj.insert(k.clone(), json!([]));
                    }
                });
            }
            _ => continue,
        }
    }
}

#[cfg(test)]
mod rtl_direction_tests {
    use super::*;
    use crate::text::direction::Dir;
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable};

    /// The dialog's working fields for `section`, as `open_preferences` builds them.
    fn prefs_fields(section: &str) -> Map<String, Value> {
        let all = prefs::Preferences::default().to_json();
        let mut values = Map::new();
        for (id, _) in SECTIONS {
            if let Some(v) = all.get(id) {
                values.insert(id.to_string(), v.clone());
            }
        }
        let mut f = Map::new();
        f.insert("section".into(), json!(section));
        f.insert("values".into(), Value::Object(values));
        f
    }

    /// Render `prefs_body` for `section` under `dir`; return the search field's rect and the
    /// accessible label `needle`'s rect.
    fn probe(section: &str, dir: Dir, needle: &str) -> (egui::Rect, egui::Rect) {
        let _scope = crate::text::direction::direction_scope(dir);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .with_context_setup(|ctx| crate::PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::Studio))
            .build_ui_state(
                |ui, f: &mut Map<String, Value>| {
                    // The dialog host draws every page inside the body layout (fields::body).
                    ui.with_layout(crate::text::direction::body_layout(), |ui| super::prefs_body(ui, f, None));
                },
                prefs_fields(section),
            );
        h.run_steps(4);
        let search = h.query_all_by_role(Role::TextInput).map(|n| n.rect()).min_by(|a, b| a.top().total_cmp(&b.top())).expect("the search field");
        let label = h.query_all_by_role_and_label(Role::Label, needle).next().expect(needle).rect();
        (search, label)
    }

    /// RTL: the active page's title hugs the content column's reading-start (right) edge.
    #[test]
    fn rtl_prefs_page_title_sits_at_the_content_reading_start() {
        let (search, title) = probe("general", Dir::Rtl, "General");
        assert!(
            title.right() > search.center().x && title.right() < search.right() - 150.0,
            "RTL: the title hugs the content column's right edge: {title:?} in {search:?}"
        );
    }

    /// LTR twin: the title keeps the content column's left edge (after the section list).
    #[test]
    fn ltr_prefs_page_title_keeps_the_content_left_edge() {
        let (search, title) = probe("general", Dir::Ltr, "General");
        assert!(
            title.left() > search.left() + 150.0 && title.left() < search.center().x,
            "LTR: the title stays at the content column's left: {title:?} in {search:?}"
        );
    }

    /// RTL: a page's settings rows start at the content column's reading-start (right) edge.
    #[test]
    fn rtl_prefs_field_labels_sit_at_the_reading_start() {
        let (search, label) = probe("general", Dir::Rtl, "Image interpolation");
        assert!(
            label.right() > search.center().x && label.right() < search.right() - 150.0,
            "RTL: the settings label hugs the content column's right edge: {label:?} in {search:?}"
        );
    }

    /// LTR twin: the same rows keep the content column's left edge.
    #[test]
    fn ltr_prefs_field_labels_keep_the_left_edge() {
        let (search, label) = probe("general", Dir::Ltr, "Image interpolation");
        assert!(
            label.left() > search.left() + 150.0 && label.left() < search.center().x,
            "LTR: the settings label stays at the content column's left: {label:?} in {search:?}"
        );
    }

    /// Render `prefs_body` for `section` under `dir` with the real direction flag; return the
    /// label<->editor gap of the field whose label is `needle` (its editor is the combobox
    /// carrying the same accessible name).
    fn probe_gap(section: &str, dir: Dir, needle: &str) -> f32 {
        let _scope = crate::text::direction::direction_scope(dir);
        let rtl = dir == Dir::Rtl;
        let mut h = Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .with_context_setup(move |ctx| {
                crate::PhotocraftApp::setup_context(ctx, crate::theme::ThemeKind::Studio);
                ctx.all_styles_mut(|s| s.right_to_left = rtl);
            })
            .build_ui_state(
                |ui, f: &mut Map<String, Value>| {
                    ui.with_layout(crate::text::direction::body_layout(), |ui| super::prefs_body(ui, f, None));
                },
                prefs_fields(section),
            );
        h.run_steps(6);
        let label = h.query_all_by_role_and_label(Role::Label, needle).next().expect(needle).rect();
        let editor = h.query_all_by_role_and_label(Role::ComboBox, needle).next().expect(needle).rect();
        if dir == Dir::Rtl { label.left() - editor.right() } else { editor.left() - label.right() }
    }

    /// The per-row label<->editor gap of a settings row equals the LTR value in RTL too: the
    /// direction-aware grid fixes the label column at the LTR width at the reading start, so
    /// the gap is `label_w - label_width + 14` in both directions instead of the leftover row
    /// width the fork's `Grid` gave the last column under RTL.
    #[test]
    fn prefs_label_editor_gap_matches_between_ltr_and_rtl() {
        let ltr = probe_gap("general", Dir::Ltr, "Image interpolation");
        let rtl = probe_gap("general", Dir::Rtl, "Image interpolation");
        assert!((ltr - rtl).abs() < 1.5, "gap differs: LTR {ltr} vs RTL {rtl}");
        assert!(ltr < 100.0 && rtl < 100.0, "the gap stays the grid's 14 px + label slack: LTR {ltr}, RTL {rtl}");
    }
}
