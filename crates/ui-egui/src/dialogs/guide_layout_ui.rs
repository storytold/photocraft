//! Photoshop-style guide layout dialog. Its preview is a pure guide overlay, never an edit.

use egui::{Color32, Stroke};
use photocraft_doc::{Document, Guides};
use serde_json::{Map, Value, json};

use crate::{PhotocraftApp, theme::Tokens, widgets};

const COMMAND: &str = "view.newGuideLayout";
const MARK: &str = "__guideLayout";

/// A `"#rrggbb"` colour as drawn.
fn rgb(hex: &str) -> Option<Color32> {
    photocraft_engine::prefs::parse_hex(hex).map(|[r, g, b]| Color32::from_rgb(r, g, b))
}

/// The Color list: New Guide's colours (`GUIDE_COLORS`) by their `"#rrggbb"`, then `current` as
/// Custom when it's none of them.
fn color_options(current: &str) -> Vec<(String, &'static str)> {
    let mut colors: Vec<(String, &'static str)> = photocraft_engine::edit_cmds::GUIDE_COLORS
        .iter()
        .map(|(name, [r, g, b])| (format!("#{r:02x}{g:02x}{b:02x}"), crate::new_guide_ui::color_label(name)))
        .collect();
    if !colors.iter().any(|(v, _)| v == current) {
        colors.push((current.to_string(), "Custom"));
    }
    colors
}

pub fn owns(f: &Map<String, Value>) -> bool {
    f.contains_key(MARK)
}

/// Alt+Cancel (Reset): factory New Guide Layout values, not the last-used remembered layout.
pub fn reset(app: &PhotocraftApp, f: &mut Map<String, Value>) {
    if !owns(f) {
        return;
    }
    let Value::Object(defaults) = crate::view_cmds::default_guide_layout() else { return };
    // Drop layout params and interactive toggles; keep dialog chrome (`__command`, …).
    f.retain(|k, _| {
        k.starts_with("__")
            && !matches!(
                k.as_str(),
                "__preset" | "__columnsEnabled" | "__rowsEnabled" | "__marginEnabled" | "__previewGuides" | "__previewParams" | "__previewSource"
            )
    });
    f.extend(defaults);
    f.insert("color".into(), json!(app.session.prefs().guides_grid_and_slices.guide_color));
    f.insert("target".into(), json!("document"));
    f.insert("__preset".into(), json!("custom"));
    f.insert("width".into(), Value::Null);
    f.insert("height".into(), Value::Null);
    let margin = f.get("margin").cloned().unwrap_or(json!(0));
    if !margin.is_array() {
        f.insert("margin".into(), json!([margin.clone(), margin.clone(), margin.clone(), margin]));
    }
    f.insert("__preview".into(), json!(true));
}

fn on(f: &Map<String, Value>, key: &str, fallback: bool) -> bool {
    f.get(key).and_then(Value::as_bool).unwrap_or(fallback)
}
fn positive(f: &Map<String, Value>, key: &str) -> bool {
    f.get(key).and_then(Value::as_f64).is_some_and(|n| n > 0.0)
}

pub fn open(app: &mut PhotocraftApp) -> u64 {
    let mut f = app.ui.view.guide_layout.as_object().cloned().unwrap_or_default();
    f.insert(MARK.into(), json!(true));
    f.insert("__command".into(), json!(COMMAND));
    f.insert("__label".into(), json!("New Guide Layout…"));
    f.insert("__document".into(), app.session.active().map(|s| json!(s.doc.id)).unwrap_or(Value::Null));
    f.insert("__preview".into(), json!(true));
    f.insert("color".into(), json!(app.session.prefs().guides_grid_and_slices.guide_color));
    f.entry("target").or_insert_with(|| json!("document"));
    if f.get("target").and_then(Value::as_str) == Some("selectedArtboards") && photocraft_engine::guide_layout::selected_artboards(&app.session).is_empty() {
        f.insert("target".into(), json!("document"));
    }
    f.entry("__preset").or_insert_with(|| json!("custom"));
    f.entry("width").or_insert(Value::Null);
    f.entry("height").or_insert(Value::Null);
    let margin = f.get("margin").cloned().unwrap_or(json!(0));
    if !margin.is_array() {
        f.insert("margin".into(), json!([margin.clone(), margin.clone(), margin.clone(), margin]));
    }
    for key in ["__previewGuides", "__previewParams", "__previewSource"] {
        f.remove(key);
    }
    app.ui.open_dialog(crate::state::DialogKind::Command, f)
}

/// Preserve legacy command parameter names for automation and remembered defaults.
pub fn params(f: &Map<String, Value>) -> Value {
    let mut p: Map<String, Value> = ["columns", "rows", "width", "height", "gutter", "rowGutter", "margin", "centerColumns", "clearExisting", "target"]
        .into_iter()
        .filter_map(|k| f.get(k).cloned().map(|v| (k.to_string(), v)))
        .collect();
    for (toggle, key) in [("__columnsEnabled", "columns"), ("__rowsEnabled", "rows")] {
        if !on(f, toggle, positive(f, key)) {
            p.insert(key.into(), json!(0));
        }
    }
    if !on(f, "__marginEnabled", true) {
        p.insert("margin".into(), json!(0));
    }
    for key in ["width", "height"] {
        if let Some(Value::String(s)) = p.get(key) {
            let value = if s.trim().is_empty() {
                Value::Null
            } else {
                widgets::parse_num(s).and_then(serde_json::Number::from_f64).map(Value::Number).unwrap_or_else(|| json!(s))
            };
            p.insert(key.into(), value);
        }
    }
    // A disabled axis must not be rejected for a stale width or gap typed before disabling it.
    for (key, size, gap) in [("columns", "width", "gutter"), ("rows", "height", "rowGutter")] {
        if p.get(key).and_then(Value::as_u64) == Some(0) {
            p.insert(size.into(), Value::Null);
            p.insert(gap.into(), json!(0));
        }
    }
    Value::Object(p)
}

fn source(app: &PhotocraftApp) -> Value {
    app.session.active().map(|st| json!([st.doc.id, st.revision, st.active_layer, st.selected_layers])).unwrap_or(Value::Null)
}

fn calculate(app: &PhotocraftApp, f: &Map<String, Value>) -> Result<Guides, String> {
    let st = app.session.active().ok_or("no document")?;
    if f.get("__document") != Some(&json!(st.doc.id)) {
        return Err("the guide layout belongs to another document".into());
    }
    let params = params(f);
    // Painting an unchanged dialog must not repeat the duplicate scans every frame.
    if f.get("__previewParams") == Some(&params)
        && f.get("__previewSource") == Some(&source(app))
        && let Some(guides) = f.get("__previewGuides").and_then(|v| serde_json::from_value(v.clone()).ok())
    {
        return Ok(guides);
    }
    photocraft_engine::guide_layout::calculate(&app.session, &params).map(|l| l.guides).map_err(|e| e.to_string())
}

pub fn validate(app: &PhotocraftApp, f: &Map<String, Value>) -> Result<(), String> {
    let color = f.get("color").and_then(Value::as_str).unwrap_or("#4affff");
    rgb(color).ok_or("invalid guide color")?;
    calculate(app, f).map(|_| ())
}

pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    validate(app, f)?;
    let color = f.get("color").and_then(Value::as_str).unwrap_or("#4affff");
    // Colour is the existing global guide display preference, not document data.
    let color = rgb(color).ok_or("invalid guide color")?;
    let hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    let result = app.run(COMMAND, params(f))?;
    // The preview is visible even when guides were hidden. Keep a committed layout visible too.
    app.ui.view.extras = true;
    app.ui.view.show.canvas_guides = true;
    app.ui.extras.guides = true;
    app.session.prefs.edit(|prefs| prefs.guides_grid_and_slices.guide_color = hex);
    let mut remembered = f.clone();
    for key in [MARK, "__command", "__label", "__document", "__previewGuides", "__previewParams", "__previewSource"] {
        remembered.remove(key);
    }
    app.ui.view.guide_layout = Value::Object(remembered);
    Ok(result)
}

/// Full replacement guide set for the dialog's own document. Cancel/removal drops it immediately.
pub fn preview(app: &PhotocraftApp, doc: &Document) -> Option<(Guides, Color32)> {
    let dialog = app.ui.dialogs.iter().rev().find(|d| owns(&d.fields))?;
    let f = &dialog.fields;
    if !on(f, "__preview", true) || f.get("__document") != Some(&json!(doc.id)) {
        return None;
    }
    let guides = calculate(app, f).ok().or_else(|| f.get("__previewGuides").and_then(|v| serde_json::from_value(v.clone()).ok()))?;
    let color = f.get("color").and_then(Value::as_str).and_then(rgb).unwrap_or(Color32::from_rgb(74, 255, 255));
    Some((guides, color))
}

fn check(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, label: &str, fallback: bool) -> bool {
    let mut value = on(f, key, fallback);
    // In a right-to-left UI the fork's available-width read inside `widgets::checkbox` balloons,
    // which would stretch this row (and the column around it) to the full body width; bound the
    // row to its natural width there.
    let resp = if crate::direction::is_rtl() {
        let w = label_width(ui, &[tl!(label)]) + 20.0 + ui.spacing().item_spacing.x;
        ui.allocate_ui_with_layout(egui::vec2(w, ui.spacing().interact_size.y), egui::Layout::top_down(egui::Align::Min), |ui| {
            widgets::checkbox(ui, &mut value, label)
        })
        .inner
    } else {
        widgets::checkbox(ui, &mut value, label)
    };
    if resp.changed() {
        f.insert(key.into(), json!(value));
    }
    value
}

fn number(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, name: &str, count: bool) {
    let mut value = f.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    if widgets::value_field(ui, &mut value, (if count { 1.0 } else { 0.0 })..=if count { 1000.0 } else { 300_000.0 }, if count { "" } else { "px" }, 68.0)
        .accessible_name(tl!(name))
        .changed()
        // A disabled field still clamps its value into range (a zero count to 1); writing that
        // back would switch the axis on through its `positive` fallback (#5625).
        && ui.is_enabled()
    {
        f.insert(key.into(), if count { json!(value.round() as u64) } else { json!(value) });
    }
}

fn optional(ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, label: egui::Id) {
    let mut value = match f.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(v) if v.is_number() => v.to_string(),
        _ => String::new(),
    };
    let t = Tokens::get(ui.ctx());
    let response = ui
        .add(egui::TextEdit::singleline(&mut value).id_salt(key).desired_width(60.0).font(crate::theme::mono(12.0)).background_color(t.field))
        .labelled_by(label);
    if ui.is_enabled() {
        crate::field_tab::register(ui.ctx(), response.id);
    }
    if response.gained_focus() {
        crate::field_tab::select_all(ui.ctx(), response.id, &value);
    }
    if response.changed() {
        f.insert(key.into(), json!(value));
    }
    response.on_hover_text(tl!("Leave empty for automatic size"));
}

fn group<R>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    egui::Frame::new().stroke(Stroke::new(1.0, t.field_border)).inner_margin(8).show(ui, |ui| column(ui, content)).inner
}

/// A column whose rows follow the UI direction: in a right-to-left UI its blocks sit at the
/// reading start and the rows inside it mirror. `Align::Min` in both directions: left-to-right
/// `body_layout` is already `Min`, and in a right-to-left UI a `Max` cross align would seed this
/// scope's min_rect at its full max_rect width, ballooning the column past its content.
fn column<R>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    // In a right-to-left UI the rows right-align (`Max`): a mirrored row hugs the column's
    // reading-start edge the way a left-to-right row hugs the left. Every right-to-left column
    // is width-capped by its caller, so the `Max` seed cannot balloon past that cap.
    let cross = if crate::direction::is_rtl() { egui::Align::Max } else { egui::Align::Min };
    ui.scope_builder(egui::UiBuilder::new().layout(egui::Layout::top_down(cross)), content).inner
}

/// The width of the widest of `labels`, so the editors of a settings column line up.
fn label_width(ui: &egui::Ui, labels: &[&str]) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    labels.iter().map(|l| ui.painter().layout_no_wrap((*l).to_owned(), font.clone(), Color32::WHITE).size().x).fold(0.0, f32::max)
}

/// The width an axis group box needs: its widest row (a settings line) plus the frame margin.
fn group_w(ui: &egui::Ui, rows: bool) -> f32 {
    let size_label = if rows { tl!("Height") } else { tl!("Width") };
    let label_w = label_width(ui, &[tl!("Number"), size_label, tl!("Spacing")]);
    let row_w = label_w + ui.spacing().item_spacing.x + 68.0;
    let title = if rows { tl!("Rows") } else { tl!("Columns") };
    row_w.max(label_width(ui, &[title]) + 24.0) + 16.0
}

/// The natural width of the fields column: the widest row `main_fields` draws. The
/// right-to-left body row needs it to keep its left-to-right width, so the action rail
/// stays on its mirror axis.
fn fields_w(ui: &egui::Ui) -> f32 {
    let sp = ui.spacing().item_spacing.x;
    let header = label_width(ui, &[tl!("Preset:"), tl!("Target:")]) + sp + 222.0;
    let color_row = label_width(ui, &[tl!("Color:")]) + sp + 174.0 + sp + 24.0;
    let color = 16.0 + label_width(ui, &[tl!("Color")]).max(color_row);
    let axes = group_w(ui, false) + 4.0 + group_w(ui, true);
    let labels = [tl!("Top:"), tl!("Left:"), tl!("Bottom:"), tl!("Right:")];
    let refs: Vec<&str> = labels.iter().map(|s| &**s).collect();
    let pair_w = label_width(ui, &refs).max(64.0);
    let margins = 16.0 + (4.0 * pair_w + 3.0 * sp).max(label_width(ui, &[tl!("Margin")]) + 26.0);
    let checks = label_width(ui, &[tl!("Center Columns"), tl!("Clear Existing Guides")]) + 26.0;
    header.max(color).max(axes).max(margins).max(checks)
}

fn axis(ui: &mut egui::Ui, f: &mut Map<String, Value>, rows: bool) {
    let (toggle, count, size, gap, title, size_label) = if rows {
        ("__rowsEnabled", "rows", "height", "rowGutter", tl!("Rows"), tl!("Height"))
    } else {
        ("__columnsEnabled", "columns", "width", "gutter", tl!("Columns"), tl!("Width"))
    };
    group(ui, |ui| {
        let enabled = check(ui, f, toggle, title, positive(f, count));
        if enabled && f.get(count).and_then(Value::as_f64) == Some(0.0) {
            f.insert(count.into(), json!(1));
        }
        ui.add_space(6.0);
        ui.add_enabled_ui(enabled, |ui| {
            // A two-column caption/editor form: the fork's `Grid` cannot run under a mirrored
            // body, so `form_grid` hand-draws the rows in reading order in a right-to-left UI.
            let label_w = label_width(ui, &[tl!("Number"), size_label, tl!("Spacing")]);
            crate::direction::form_grid(ui, count, [8.0, 6.0], label_w, |row| {
                row(tl!("Number"), &mut |ui, _| number(ui, f, count, "Number", true));
                row(size_label, &mut |ui, label| optional(ui, f, size, label));
                row(tl!("Spacing"), &mut |ui, _| number(ui, f, gap, "Spacing", false));
            });
        });
    });
}

/// The preset dropdown's options.
fn presets() -> Vec<(String, &'static str)> {
    vec![("custom".into(), "Custom"), ("five".into(), "5 Columns, 3 Rows"), ("eight".into(), "8 Columns"), ("twelve".into(), "12 Columns")]
}

/// A named preset's starting values; a later typed edit makes the preset custom again.
fn apply_preset(f: &mut Map<String, Value>, preset: &str) {
    let (cols, rows, gap) = match preset {
        "five" => (5, 3, 0),
        "eight" => (8, 0, 20),
        "twelve" => (12, 0, 24),
        _ => return,
    };
    for (k, v) in [
        ("columns", json!(cols)),
        ("rows", json!(rows)),
        ("gutter", json!(gap)),
        ("rowGutter", json!(0)),
        ("width", Value::Null),
        ("height", Value::Null),
        ("margin", json!([0, 0, 0, 0])),
        ("__columnsEnabled", json!(true)),
        ("__rowsEnabled", json!(rows > 0)),
        ("__marginEnabled", json!(true)),
        ("centerColumns", json!(false)),
    ] {
        f.insert(k.into(), v);
    }
}

fn main_fields(app: &PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let before: Map<String, Value> = f.iter().filter(|(key, _)| !key.starts_with("__preview")).map(|(k, v)| (k.clone(), v.clone())).collect();
    // A two-column caption/editor form: the fork's `Grid` cannot run under a mirrored body, so
    // `form_grid` hand-draws the rows in reading order in a right-to-left UI.
    let label_w = label_width(ui, &[tl!("Preset:"), tl!("Target:")]);
    crate::direction::form_grid(ui, "guide-header", [8.0, 6.0], label_w, |row| {
        let mut preset = f.get("__preset").and_then(Value::as_str).unwrap_or("custom").to_string();
        row(tl!("Preset:"), &mut |ui, _| {
            if widgets::dropdown(ui, "guide-preset", tl!("Preset:"), &mut preset, &presets(), 222.0) {
                f.insert("__preset".into(), json!(preset));
                apply_preset(f, &preset);
            }
        });
        let mut target = f.get("target").and_then(Value::as_str).unwrap_or("document").to_string();
        let mut options = vec![("document".to_string(), "Document")];
        if !photocraft_engine::guide_layout::selected_artboards(&app.session).is_empty() {
            options.push(("selectedArtboards".into(), "Selected Artboards"));
        }
        row(tl!("Target:"), &mut |ui, _| {
            if widgets::dropdown(ui, "guide-target", tl!("Target"), &mut target, &options, 222.0) {
                f.insert("target".into(), json!(target));
            }
        });
    });
    ui.add_space(10.0);
    group(ui, |ui| {
        ui.label(tl!("Color"));
        let color_row = |ui: &mut egui::Ui| {
            ui.label(tl!("Color:"));
            let mut color = f.get("color").and_then(Value::as_str).unwrap_or("#4affff").to_string();
            let colors = color_options(&color);
            if widgets::dropdown(ui, "guide-color", tl!("Color"), &mut color, &colors, 174.0) {
                f.insert("color".into(), json!(color));
            }
            let shown = rgb(&color).unwrap_or(Tokens::get(ui.ctx()).accent);
            if widgets::color_swatch_button(ui, shown, tl!("Guide color")).clicked() {
                crate::color_picker_ui::request(f, tl!("Guide color"));
            }
        };
        if crate::direction::is_rtl() {
            // A fixed width, mirrored: a plain `horizontal` allocates the fork's ballooned
            // available width in a right-to-left scope and would widen the whole dialog.
            let w = label_width(ui, &[tl!("Color:")]) + ui.spacing().item_spacing.x + 174.0 + ui.spacing().item_spacing.x + 24.0;
            ui.allocate_ui_with_layout(egui::vec2(w, ui.spacing().interact_size.y), egui::Layout::right_to_left(egui::Align::Min), color_row);
        } else {
            ui.horizontal(color_row);
        }
    });
    ui.add_space(10.0);
    if crate::direction::is_rtl() {
        // A mirrored row scope capped to its natural width, anchored at the column's
        // reading-start edge: an uncapped scope balloons its min_rect to the full column
        // width in this fork, which would push the action rail off its mirror axis. Each
        // group box gets the width it needs, so the pair matches the left-to-right row.
        let w = group_w(ui, false) + 4.0 + group_w(ui, true);
        let right = ui.max_rect().right();
        let top = ui.next_widget_position().y;
        let row_rect = egui::Rect::from_min_size(egui::pos2(right - w, top), egui::vec2(w, ui.available_height()));
        ui.scope_builder(egui::UiBuilder::new().max_rect(row_rect).layout(egui::Layout::right_to_left(egui::Align::Min)), |ui| {
            let w = group_w(ui, false);
            ui.allocate_ui_with_layout(egui::vec2(w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| axis(ui, f, false));
            ui.add_space(4.0);
            let w = group_w(ui, true);
            ui.allocate_ui_with_layout(egui::vec2(w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| axis(ui, f, true));
        });
    } else {
        ui.horizontal_top(|ui| {
            axis(ui, f, false);
            ui.add_space(4.0);
            axis(ui, f, true);
        });
    }
    ui.add_space(10.0);
    group(ui, |ui| {
        let enabled = check(ui, f, "__marginEnabled", tl!("Margin"), true);
        let mut margins = f.get("margin").and_then(Value::as_array).cloned().unwrap_or_else(|| vec![json!(0); 4]);
        ui.add_space(5.0);
        ui.add_enabled_ui(enabled, |ui| {
            let pair = |ui: &mut egui::Ui, value: &mut Value, label: &str| {
                ui.label(label);
                let mut n = value.as_f64().unwrap_or(0.0) as f32;
                if widgets::value_field(ui, &mut n, 0.0..=300_000.0, "px", 64.0).accessible_name(label).changed() {
                    *value = json!(n);
                }
            };
            let labels = [tl!("Top:"), tl!("Left:"), tl!("Bottom:"), tl!("Right:")];
            if crate::direction::is_rtl() {
                // Same row as left-to-right, in reading order. Each pair gets an explicit width:
                // a bare `ui.vertical` scope in a right-to-left row is seeded at the row's full
                // remaining width by the fork, which would balloon the group past the column.
                let label_refs: Vec<&str> = labels.iter().map(|s| &**s).collect();
                let pair_w = label_width(ui, &label_refs).max(64.0);
                let w = 4.0 * pair_w + 3.0 * ui.spacing().item_spacing.x;
                let h = 2.0 * ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
                ui.allocate_ui_with_layout(egui::vec2(w, h), egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    for (value, label) in margins.iter_mut().zip(labels) {
                        ui.allocate_ui_with_layout(egui::vec2(pair_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| pair(ui, value, label));
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    for (value, label) in margins.iter_mut().zip(labels) {
                        ui.vertical(|ui| pair(ui, value, label));
                    }
                });
            }
        });
        f.insert("margin".into(), Value::Array(margins));
    });
    ui.add_space(8.0);
    check(ui, f, "centerColumns", tl!("Center Columns"), false);
    check(ui, f, "clearExisting", tl!("Clear Existing Guides"), false);
    // A typed edit makes a named preset custom; changing target/colour/preview does not.
    if ["columns", "rows", "width", "height", "gutter", "rowGutter", "margin", "centerColumns", "__columnsEnabled", "__rowsEnabled", "__marginEnabled"]
        .iter()
        .any(|k| before.get(*k) != f.get(*k))
        && before.get("__preset") == f.get("__preset")
    {
        f.insert("__preset".into(), json!("custom"));
    }
}

/// The action rail's width: the OK / Cancel buttons are this wide.
const RAIL_W: f32 = 96.0;

/// Own the action rail as well as the body, so OK/Cancel/Preview follow Photoshop's arrangement.
pub fn body(app: &PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, interactive: bool) -> Option<bool> {
    let before = (params(f), on(f, "__preview", true), f.get("color").cloned());
    let mut outcome = None;
    // The host keeps its `top_down(Align::Min)` layout, so the body mirrors itself: in a
    // right-to-left UI its blocks sit at the reading start and its rows read right to left.
    ui.with_layout(crate::direction::body_layout(), |ui| {
        let row = |ui: &mut egui::Ui| {
            // A vertical scope, not `ui.vertical`: the latter pins `Align::Min`, which keeps the
            // rows inside it left-to-right even in a right-to-left UI. The column keeps its
            // natural width in both directions — a wider right-to-left column would push the
            // action rail off its mirror axis.
            column(ui, |ui| {
                main_fields(app, ui, f);
                ui.min_rect()
            });
            ui.add_space(12.0);
            column(ui, |ui| {
                let valid = validate(app, f).is_ok();
                if ui.add_enabled_ui(valid, |ui| widgets::primary_button(ui, tl!("OK"), RAIL_W)).inner.clicked() {
                    outcome = Some(true);
                }
                ui.add_space(4.0);
                if widgets::secondary_button(ui, tl!("Cancel"), RAIL_W).clicked() {
                    outcome = Some(false);
                }
                ui.add_space(6.0);
                check(ui, f, "__preview", tl!("Preview"), true);
                ui.min_rect()
            });
            ui.min_rect()
        };
        if crate::direction::is_rtl() {
            // A natural-width mirrored row anchored at the body's right (reading-start) edge:
            // the fields column keeps its left-to-right width and the rail sits at the
            // reading end exactly where the left-to-right row mirrors it.
            let fields_w = fields_w(ui);
            let w = fields_w + 12.0 + ui.spacing().item_spacing.x + RAIL_W;
            let right = ui.max_rect().right();
            let top = ui.next_widget_position().y;
            let row_rect = egui::Rect::from_min_size(egui::pos2(right - w, top), egui::vec2(w, ui.available_height()));
            ui.scope_builder(egui::UiBuilder::new().max_rect(row_rect).layout(egui::Layout::right_to_left(egui::Align::Min)), |ui| {
                // Cross `Max`: the rows right-align inside the width-capped column, so each row's
                // reading-start edge sits at the column's right edge, mirroring the left-to-right
                // column's left edge. `Min` would left-align them and leave slack at the reading start.
                ui.allocate_ui_with_layout(egui::vec2(fields_w, 0.0), egui::Layout::top_down(egui::Align::Max), |ui| {
                    main_fields(app, ui, f);
                });
                ui.add_space(12.0);
                ui.allocate_ui_with_layout(egui::vec2(RAIL_W, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    let valid = validate(app, f).is_ok();
                    if ui.add_enabled_ui(valid, |ui| widgets::primary_button(ui, tl!("OK"), RAIL_W)).inner.clicked() {
                        outcome = Some(true);
                    }
                    ui.add_space(4.0);
                    if widgets::secondary_button(ui, tl!("Cancel"), RAIL_W).clicked() {
                        outcome = Some(false);
                    }
                    ui.add_space(6.0);
                    check(ui, f, "__preview", tl!("Preview"), true);
                });
            });
        } else {
            ui.horizontal_top(row);
        }
        match calculate(app, f) {
            Ok(guides) => {
                f.insert("__previewGuides".into(), json!(guides));
                f.insert("__previewParams".into(), params(f));
                f.insert("__previewSource".into(), source(app));
            }
            Err(error) => {
                ui.colored_label(Tokens::get(ui.ctx()).text_dim, tl!("Invalid guide layout")).on_hover_text(error);
            }
        }
    });
    if outcome.is_none() && interactive && validate(app, f).is_ok() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        outcome = Some(true);
    }
    // The canvas paints before the dialog. Schedule the frame that displays this draft edit.
    if before != (params(f), on(f, "__preview", true), f.get("color").cloned()) {
        ui.ctx().request_repaint();
    }
    outcome
}

#[cfg(test)]
mod tests;
