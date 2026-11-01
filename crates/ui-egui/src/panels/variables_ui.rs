//! Front end for Image › Variables (Define… / Data Sets…) and Apply Data Set — Photoshop's
//! data-driven graphics dialog. All editing drives the headless engine commands
//! (`image.variables.define`, `image.variables.dataSets`, `image.applyDataSet`), so the control
//! channel can drive this with `ui.dialog.set` / `confirm` exactly like a human.
//!
//! The dialog keeps its working state as JSON in the `__variables` field, shaped like the engine
//! command params, so `confirm` passes it straight through. As in Photoshop, the Define page edits
//! one layer at a time (`__layer`); each property reads a variable (which other layers may share)
//! or computes its value with a rule.

use serde_json::{Map, Value, json};

use photocraft_engine::variable_rule::{Rule, Val};

use crate::PhotocraftApp;
use crate::state::DialogKind;
use crate::theme::Tokens;

/// This dialog owns a field set when it carries `__variables`.
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key("__variables")
}

/// Alt+Cancel (Reset): restore `__variables` from the open-time snapshot.
pub fn reset(fields: &mut Map<String, Value>) {
    if !owns(fields) {
        return;
    }
    if let Some(orig) = fields.get("__variablesOrig").cloned() {
        fields.insert("__variables".into(), orig);
    }
}

/// Layers available to bind, top to bottom: (id, display name, is-text).
fn layer_options(app: &PhotocraftApp) -> Vec<(u64, String, bool)> {
    let Some(st) = app.session.active() else { return Vec::new() };
    st.doc.walk().iter().map(|(_, _, l)| (l.id.0, l.name.clone(), matches!(l.content, photocraft_doc::LayerContent::Text(_)))).collect()
}

/// Menu entry points. Returns None for ids this module doesn't own.
pub fn menu(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let has_params = params.as_object().is_some_and(|o| !o.is_empty());
    // Apply Data Set with no data set named opens the Data Sets page to pick one.
    let page = match id {
        "image.variables.define" if !has_params => "define",
        "image.variables.dataSets" | "image.applyDataSet" if !has_params => "dataSets",
        _ => return None,
    };
    let mut f = Map::new();
    let state = app.session.active().map(|st| to_state(&st.doc.variables)).unwrap_or_else(|| json!({"defs": [], "names": [], "dataSets": []}));
    // Photoshop's Define page starts on the active layer.
    if let Some(layer) = app.session.active().and_then(|st| st.active_layer) {
        f.insert("__layer".into(), json!(layer.0));
    }
    // Snapshot at open so Alt+Reset restores the document's variables.
    f.insert("__variablesOrig".into(), state.clone());
    f.insert("__variables".into(), state);
    f.insert("__page".into(), json!(page));
    f.insert("__label".into(), json!("Variables"));
    Some(Ok(json!({"dialog": app.ui.open_dialog(DialogKind::Command, f)})))
}

/// Serialise the document's variables into the dialog's editable JSON shape.
pub fn to_state(v: &photocraft_doc::Variables) -> Value {
    json!({
        "defs": v.defs.iter().map(|d| {
            let (ty, method, align, clip) = match &d.kind {
                photocraft_doc::VarKind::Visibility => ("visibility", "fit", "center", false),
                photocraft_doc::VarKind::TextReplacement => ("textReplacement", "fit", "center", false),
                photocraft_doc::VarKind::PixelReplacement { method, align, clip } => {
                    (_pixel(), _m(method), _a(align), *clip)
                }
            };
            let mut def = json!({"name": d.name, "layer": d.layer.0, "type": ty, "method": method, "align": align, "clip": clip});
            if let Some(rule) = &d.rule {
                def["rule"] = json!(rule);
            }
            def
        }).collect::<Vec<_>>(),
        "names": v.names,
        "dataSets": v.data_sets.iter().map(|s| json!({
            "name": s.name,
            "values": s.values.iter().map(|dv| {
                let (kind, value) = match &dv.value {
                    photocraft_doc::variables::Value::Visibility(b) => ("visibility", json!(b)),
                    photocraft_doc::variables::Value::Text(t) => ("text", json!(t)),
                    photocraft_doc::variables::Value::Pixels(p) => ("pixels", json!(p)),
                };
                json!({"variable": dv.variable, "kind": kind, "value": value})
            }).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn _pixel() -> &'static str {
    "pixelReplacement"
}
fn _m(m: &photocraft_doc::PixelMethod) -> &'static str {
    use photocraft_doc::PixelMethod::*;
    match m {
        Fit => "fit",
        Fill => "fill",
        AsIs => "asIs",
        Conform => "conform",
    }
}
fn _a(a: &photocraft_doc::PixelAlign) -> &'static str {
    use photocraft_doc::PixelAlign::*;
    match a {
        TopLeft => "topLeft",
        TopCenter => "topCenter",
        TopRight => "topRight",
        CenterLeft => "centerLeft",
        Center => "center",
        CenterRight => "centerRight",
        BottomLeft => "bottomLeft",
        BottomCenter => "bottomCenter",
        BottomRight => "bottomRight",
    }
}

const TYPES: &[(&str, &str)] = &[("visibility", "Visibility"), ("textReplacement", "Text Replacement"), ("pixelReplacement", "Pixel Replacement")];
/// Height of the Bindings list before it scrolls (about ten rows).
const BINDINGS_MAX_H: f32 = 200.0;
/// The Bindings filter appears once the list has more rows than this.
const BINDINGS_FILTER_AT: usize = 8;
const METHODS: &[(&str, &str)] =&[("fit", "Fit"), ("fill", "Fill"), ("conform", "Conform"), ("asIs", "As Is")];

/// Render the dialog. Mutates `fields["__variables"]`, `fields["__page"]` and `fields["__layer"]`.
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, fields: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let layers = layer_options(app);
    let mut state = normalize(fields.get("__variables").unwrap_or(&Value::Null));
    let mut page = fields.get("__page").and_then(Value::as_str).unwrap_or("define").to_string();

    ui.set_min_width(460.0);
    // Page switcher.
    ui.horizontal(|ui| {
        for (id, label) in [("define", tl!("Define")), ("dataSets", tl!("Data Sets"))] {
            let sel = page == id;
            if ui.selectable_label(sel, tl!(&label)).clicked() {
                page = id.to_string();
            }
        }
    });
    ui.separator();

    if layers.is_empty() {
        ui.weak(tl!("Open a document to define variables."));
    } else if page == "define" {
        define_page(ui, &t, &layers, &mut state, fields);
    } else {
        data_sets_page(ui, &t, &mut state, fields);
    }

    fields.insert("__variables".into(), state);
    fields.insert("__page".into(), json!(page));
}

/// The dialog state with only object entries in `defs`, `dataSets` and each set's `values`, and
/// only strings in `names`. `ui.dialog.set` can store any JSON in `__variables`, and the pages
/// write fields into entries (`d["name"] = …`), which panics on a number or a string.
fn normalize(state: &Value) -> Value {
    let objects =
        |v: Option<&Value>| -> Vec<Value> { v.and_then(Value::as_array).map(|a| a.iter().filter(|e| e.is_object()).cloned().collect()).unwrap_or_default() };
    let sets: Vec<Value> = objects(state.get("dataSets"))
        .into_iter()
        .map(|mut set| {
            set["values"] = Value::Array(objects(set.get("values")));
            set
        })
        .collect();
    let names: Vec<Value> = state.get("names").and_then(Value::as_array).map(|a| a.iter().filter(|n| n.is_string()).cloned().collect()).unwrap_or_default();
    json!({"defs": objects(state.get("defs")), "names": names, "dataSets": sets})
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// The rule of a dialog def, when it computes its value with one.
fn rule_of(d: &Value) -> Option<&str> {
    d.get("rule").and_then(Value::as_str)
}

/// Every variable in the dialog state, without duplicates: declared names, then names bindings
/// read directly, then names rules read. Each with how the Data Sets page edits it
/// (`visibility`, `pixelReplacement` or `text`), from the bindings that read it directly.
fn columns(state: &Value) -> Vec<(String, &'static str)> {
    let defs: &[Value] = state.get("defs").and_then(Value::as_array).map_or(&[], Vec::as_slice);
    let mut names: Vec<String> = state.get("names").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    names.extend(defs.iter().filter(|d| rule_of(d).is_none()).map(|d| str_of(d, "name").to_string()));
    for d in defs {
        if let Some(rule) = rule_of(d).and_then(|r| Rule::parse(r).ok()) {
            names.extend(rule.variables());
        }
    }
    let mut out: Vec<(String, &'static str)> = Vec::new();
    for n in names {
        if n.trim().is_empty() || out.iter().any(|(o, _)| *o == n) {
            continue;
        }
        let kinds: Vec<&str> = defs.iter().filter(|d| rule_of(d).is_none() && str_of(d, "name") == n).map(|d| str_of(d, "type")).collect();
        let kind = if !kinds.is_empty() && kinds.iter().all(|k| *k == "visibility" || k.is_empty()) {
            "visibility"
        } else if kinds.contains(&"pixelReplacement") && !kinds.contains(&"textReplacement") {
            "pixelReplacement"
        } else {
            "text"
        };
        out.push((n, kind));
    }
    out
}

/// The layer the Define page shows: `__layer` when it names a layer, else the first bound layer,
/// else the top layer.
fn selected_layer(fields: &Map<String, Value>, layers: &[(u64, String, bool)], defs: &[Value]) -> Option<u64> {
    let known = |id: u64| layers.iter().any(|(l, ..)| *l == id);
    fields
        .get("__layer")
        .and_then(Value::as_u64)
        .filter(|&id| known(id))
        .or_else(|| defs.iter().filter_map(|d| d.get("layer").and_then(Value::as_u64)).find(|&id| known(id)))
        .or_else(|| layers.first().map(|(id, ..)| *id))
}

/// The layers with bindings, in the order their first binding appears.
fn bound_layers(defs: &[Value]) -> Vec<u64> {
    let mut out = Vec::new();
    for id in defs.iter().filter_map(|d| d.get("layer").and_then(Value::as_u64)) {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// Check a rule against the data set the Data Sets page shows: its result, or what is wrong.
fn preview(rule: &str, state: &Value, fields: &Map<String, Value>) -> Result<Option<String>, String> {
    let parsed = Rule::parse(rule)?;
    let sets: &[Value] = state.get("dataSets").and_then(Value::as_array).map_or(&[], Vec::as_slice);
    let Some(set) = sets.get(data_set_index(fields, sets.len())) else { return Ok(None) };
    let values: &[Value] = set.get("values").and_then(Value::as_array).map_or(&[], Vec::as_slice);
    let value = |e: &Value| match e.get("value") {
        Some(Value::Bool(b)) => Some(Val::Bool(*b)),
        Some(Value::String(s)) => Some(Val::Text(s.clone())),
        _ => None,
    };
    let lookup = |n: &str| {
        let exact = values.iter().find(|e| str_of(e, "variable") == n);
        exact.or_else(|| values.iter().find(|e| str_of(e, "variable").to_lowercase() == n.to_lowercase())).and_then(value)
    };
    let out = parsed.eval(&lookup)?;
    Ok(Some(format!("{} → {}", str_of(set, "name"), match out {
        Val::Bool(b) => b.to_string(),
        other => format!("\"{}\"", other.text()),
    })))
}

fn define_page(ui: &mut egui::Ui, t: &Tokens, layers: &[(u64, String, bool)], state: &mut Value, fields: &mut Map<String, Value>) {
    let snapshot = state.clone();
    let known_names: Vec<String> = columns(&snapshot).into_iter().map(|(n, _)| n).collect();
    let Some(defs) = state.get_mut("defs").and_then(Value::as_array_mut) else { return };
    let Some(mut layer) = selected_layer(fields, layers, defs) else { return };

    ui.label(tl!("Bind this layer's visibility, text or pixels to a variable, or compute them with a rule. Several layers can use the same variable."));
    ui.add_space(4.0);
    // Layers with bindings are marked, so a template's bound layers are easy to find.
    let labels: Vec<(u64, String)> = layers
        .iter()
        .map(|(id, n, _)| (*id, if defs.iter().any(|d| d.get("layer").and_then(Value::as_u64) == Some(*id)) { format!("{n}  •") } else { n.clone() }))
        .collect();
    let opts: Vec<(u64, &str)> = labels.iter().map(|(id, n)| (*id, n.as_str())).collect();
    ui.horizontal(|ui| {
        ui.label(tl!("Layer:"));
        crate::widgets::dropdown(ui, "var-layer", tl!("Layer"), &mut layer, &opts, 220.0);
    });
    fields.insert("__layer".into(), json!(layer));
    let is_text = layers.iter().any(|(id, _, text)| *id == layer && *text);
    ui.add_space(6.0);

    for (ty, label) in TYPES {
        // Photoshop offers Text Replacement on type layers and Pixel Replacement on the others.
        if (*ty == "textReplacement" && !is_text) || (*ty == "pixelReplacement" && is_text) {
            continue;
        }
        let at = defs.iter().position(|d| d.get("layer").and_then(Value::as_u64) == Some(layer) && str_of(d, "type") == *ty);
        let mut on = at.is_some();
        if crate::widgets::checkbox(ui, &mut on, tl!(label)).changed() {
            match (on, at) {
                (true, None) => {
                    let n = known_names.first().cloned().unwrap_or_else(|| format!("var{}", defs.len() + 1));
                    defs.push(json!({"name": n, "layer": layer, "type": ty, "method": "fit", "align": "center", "clip": false}));
                }
                (false, Some(i)) => {
                    defs.remove(i);
                }
                _ => {}
            }
        }
        let Some(i) = defs.iter().position(|d| d.get("layer").and_then(Value::as_u64) == Some(layer) && str_of(d, "type") == *ty) else { continue };
        let Some(d) = defs.get_mut(i) else { continue };
        ui.indent(format!("var-{ty}"), |ui| binding_editor(ui, t, ty, d, &known_names, &snapshot, fields));
        ui.add_space(4.0);
    }

    // Every binding in the document, grouped by layer, so one variable's layers can be seen
    // together; × unbinds a layer entirely. The list scrolls inside a fixed-height box so a
    // template with many bindings doesn't push the rest of the dialog off screen, and a filter
    // appears once there are enough rows to need one.
    if !defs.is_empty() {
        ui.separator();
        let bound = bound_layers(defs);
        ui.label(egui::RichText::new(format!("{} ({})", tl!("Bindings"), defs.len())).strong());
        let mut filter = fields.get("__bindFilter").and_then(Value::as_str).unwrap_or("").to_string();
        if defs.len() > BINDINGS_FILTER_AT || !filter.is_empty() {
            let id = ui.scope_id().with("var-bind-filter");
            if crate::widgets::text_field(ui, id, &mut filter, tl!("Filter by layer, variable or rule"), 260.0, false).changed() {
                fields.insert("__bindFilter".into(), json!(filter));
            }
        }
        let needle = filter.trim().to_lowercase();
        let mut pick = None;
        let mut unbind = None;
        let mut shown = 0usize;
        egui::ScrollArea::vertical().id_salt("var-bindings").max_height(BINDINGS_MAX_H).auto_shrink([false, true]).show(ui, |ui| {
            for &id in &bound {
                let lname = layers.iter().find(|(l, ..)| *l == id).map_or("?", |(_, n, _)| n.as_str());
                let rows: Vec<String> = defs
                    .iter()
                    .filter(|d| d.get("layer").and_then(Value::as_u64) == Some(id))
                    .map(|d| {
                        let ty = TYPES.iter().find(|(k, _)| *k == str_of(d, "type")).map_or("", |(_, l)| *l);
                        let source = rule_of(d).map_or_else(|| format!("← {}", str_of(d, "name")), |r| format!("= {r}"));
                        format!("{lname} — {} {source}", tl!(ty))
                    })
                    .filter(|row| needle.is_empty() || row.to_lowercase().contains(&needle))
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                shown += rows.len();
                let row = ui.horizontal(|ui| {
                    let remove = egui::Button::new(egui::RichText::new("×").color(t.text_dim)).frame(false);
                    if ui.add(remove).on_hover_text(tl!("Remove this layer's bindings")).clicked() {
                        unbind = Some(id);
                    }
                    ui.vertical(|ui| {
                        for text in rows {
                            if ui.selectable_label(id == layer, text).clicked() {
                                pick = Some(id);
                            }
                        }
                    });
                });
                // Keep the layer being edited in view when it is picked from the dropdown.
                if id == layer && fields.get("__bindScrolled").and_then(Value::as_u64) != Some(id) {
                    row.response.scroll_to_me(Some(egui::Align::Center));
                    fields.insert("__bindScrolled".into(), json!(id));
                }
            }
            if shown == 0 {
                ui.weak(tl!("No bindings match the filter."));
            }
        });
        if let Some(id) = unbind {
            defs.retain(|d| d.get("layer").and_then(Value::as_u64) != Some(id));
        }
        if let Some(id) = pick {
            fields.insert("__layer".into(), json!(id));
        }
    }
    ui.add_space(4.0);
    ui.collapsing(tl!("Rule help"), |ui| {
        ui.weak(tl!("Text compares ignore case and spaces; numbers compare as numbers. Write {Card Name} for names with spaces."));
        for ex in [
            "Type == \"gear\"",
            "Type == \"gear\" and (Rarity == \"epic\" or Level >= 50)",
            "not empty(Badge)",
            "\"{Name} - Lv {Level}\"",
            "if(Rarity == \"epic\", \"★ {Name}\", Name)",
            "switch(Type, \"gear\", \"icons/gear.png\", \"icons/other.png\")",
        ] {
            ui.monospace(ex);
        }
        ui.weak(tl!("Also: != < <= > >= or and not, + & - * /, contains, startsWith, endsWith, upper, lower, trim."));
    });
}

/// One property's source: a variable (typed, or picked from the existing ones) or a rule.
fn binding_editor(ui: &mut egui::Ui, t: &Tokens, ty: &str, d: &mut Value, known: &[String], state: &Value, fields: &Map<String, Value>) {
    let mut use_rule = rule_of(d).is_some();
    ui.horizontal(|ui| {
        let was = use_rule;
        ui.radio_value(&mut use_rule, false, tl!("Variable"));
        ui.radio_value(&mut use_rule, true, tl!("Rule"));
        if use_rule != was {
            // Keep the other source in `__rule` / the name, so toggling back loses nothing.
            if use_rule {
                let stash = str_of(d, "__rule").to_string();
                d["rule"] = json!(stash);
            } else if let Some(obj) = d.as_object_mut() {
                let r = obj.remove("rule").unwrap_or(Value::Null);
                obj.insert("__rule".into(), r);
            }
        }
    });
    if use_rule {
        let mut rule = rule_of(d).unwrap_or("").to_string();
        let hint = if ty == "visibility" { "Type == \"gear\"" } else { "\"{Name} - Lv {Level}\"" };
        if ui.add(egui::TextEdit::singleline(&mut rule).hint_text(hint).desired_width(380.0).font(egui::TextStyle::Monospace)).changed() {
            d["rule"] = json!(rule);
        }
        match preview(&rule, state, fields) {
            Ok(Some(result)) => {
                ui.label(egui::RichText::new(result).color(t.text_dim));
            }
            Ok(None) => {}
            Err(_) if rule.trim().is_empty() => {}
            Err(why) => {
                ui.label(egui::RichText::new(why).color(t.danger));
            }
        }
    } else {
        ui.horizontal(|ui| {
            let mut name = str_of(d, "name").to_string();
            if ui.add(egui::TextEdit::singleline(&mut name).hint_text(tl!("name")).desired_width(180.0)).changed() {
                d["name"] = json!(name);
            }
            // Pick an existing variable to share it with this layer.
            if !known.is_empty() {
                let opts: Vec<(String, &str)> = known.iter().map(|n| (n.clone(), n.as_str())).collect();
                let mut pick = name.clone();
                if crate::widgets::dropdown_literal(ui, &format!("var-pick-{ty}"), tl!("Use an existing variable"), &mut pick, &opts, 150.0) {
                    d["name"] = json!(pick);
                }
            }
        });
    }
    if ty == "pixelReplacement" {
        ui.horizontal(|ui| {
            let mut m = str_of(d, "method").to_string();
            if m.is_empty() {
                m = "fit".into();
            }
            if crate::widgets::dropdown(ui, "var-method", tl!("Method"), &mut m, &METHODS.iter().map(|(a, b)| (a.to_string(), *b)).collect::<Vec<_>>(), 90.0) {
                d["method"] = json!(m);
            }
            let mut clip = d.get("clip").and_then(Value::as_bool).unwrap_or(false);
            if crate::widgets::checkbox(ui, &mut clip, tl!("Clip to Bounding Box")).changed() {
                d["clip"] = json!(clip);
            }
        });
    }
}

fn data_set_index(fields: &Map<String, Value>, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    fields.get("__cur").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).filter(|&i| i < len).unwrap_or(0)
}

fn data_sets_page(ui: &mut egui::Ui, _t: &Tokens, state: &mut Value, fields: &mut Map<String, Value>) {
    let def_meta = columns(state);
    if def_meta.is_empty() {
        ui.weak(tl!("Define at least one variable or rule first."));
        return;
    }
    let Some(sets) = state.get_mut("dataSets").and_then(Value::as_array_mut) else { return };
    let mut cur = data_set_index(fields, sets.len());

    ui.horizontal(|ui| {
        if ui.button("◀").clicked() && cur > 0 {
            cur -= 1;
        }
        let name = sets
            .get(cur)
            .and_then(|s| s.get("name"))
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                let empty = "(none)";
                tl!(empty)
            })
            .to_string();
        ui.label({
            let template = "Data Set: {name}  ({index}/{total})";
            crate::i18n::fmt(
                tl!(template),
                &[("name", &name), ("index", &(if sets.is_empty() { 0 } else { cur + 1 }).to_string()), ("total", &sets.len().to_string())],
            )
        });
        if ui.button("▶").clicked() && cur + 1 < sets.len() {
            cur += 1;
        }
        if ui.button(tl!("New")).clicked() {
            let n = sets.len() + 1;
            sets.push(json!({"name": format!("Data Set {n}"), "values": []}));
            cur = sets.len() - 1;
        }
        if !sets.is_empty() && ui.button(tl!("Delete")).clicked() {
            sets.remove(cur);
            cur = cur.saturating_sub(1);
        }
    });
    ui.separator();

    if let Some(set) = sets.get_mut(cur) {
        // name
        let mut sname = set.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        ui.horizontal(|ui| {
            let name_label = ui.label(tl!("Name:"));
            if ui.add(egui::TextEdit::singleline(&mut sname).desired_width(200.0)).labelled_by(name_label.id).changed() {
                set["name"] = json!(sname);
            }
        });
        ui.add_space(4.0);
        let Some(values) = set.get_mut("values").and_then(Value::as_array_mut) else { return };
        for (vname, vty) in def_meta.iter().map(|(n, k)| (n, *k)) {
            // find or create a value entry for this variable
            let idx = values.iter().position(|v| v.get("variable").and_then(Value::as_str) == Some(vname));
            let i = match idx {
                Some(i) => i,
                None => {
                    let default = match vty {
                        "visibility" => json!({"variable": vname, "kind": "visibility", "value": true}),
                        "pixelReplacement" => json!({"variable": vname, "kind": "pixels", "value": ""}),
                        _ => json!({"variable": vname, "kind": "text", "value": ""}),
                    };
                    values.push(default);
                    values.len() - 1
                }
            };
            ui.horizontal(|ui| {
                ui.add_sized([130.0, 18.0], egui::Label::new(vname.as_str()).truncate());
                match vty {
                    "visibility" => {
                        let mut on = match values[i].get("value") {
                    Some(Value::Bool(b)) => *b,
                    Some(Value::String(s)) => Val::Text(s.clone()).truthy(),
                    _ => true,
                };
                        if ui.checkbox(&mut on, tl!("Visible")).changed() {
                            values[i]["value"] = json!(on);
                        }
                    }
                    "pixelReplacement" => {
                        let mut p = values[i].get("value").and_then(Value::as_str).unwrap_or("").to_string();
                        if ui.add(egui::TextEdit::singleline(&mut p).hint_text(tl!("image file path")).desired_width(240.0)).changed() {
                            values[i]["value"] = json!(p);
                        }
                    }
                    _ => {
                        let mut txt = values[i].get("value").and_then(Value::as_str).unwrap_or("").to_string();
                        if ui.add(egui::TextEdit::singleline(&mut txt).desired_width(240.0)).accessible_name(vname.clone()).changed() {
                            values[i]["value"] = json!(txt);
                        }
                    }
                }
            });
        }
        ui.add_space(6.0);
        if crate::widgets::secondary_button(ui, tl!("Apply"), 84.0).clicked() {
            fields.insert("__apply".into(), json!(sname));
        }
    }
    fields.insert("__cur".into(), json!(cur));
}

/// OK: save the definitions and data sets, and apply one if the Apply button set `__apply`.
pub fn confirm(app: &mut PhotocraftApp, fields: &Map<String, Value>) -> Result<Value, String> {
    let state = fields.get("__variables").cloned().unwrap_or_else(|| json!({}));
    let defs = state.get("defs").cloned().unwrap_or_else(|| json!([]));
    let names = state.get("names").cloned().unwrap_or_else(|| json!([]));
    let sets = state.get("dataSets").cloned().unwrap_or_else(|| json!([]));
    // An empty define clears; the engine adds the names rules read to `names`.
    app.run("image.variables.define", json!({"defs": defs, "names": names}))?;
    app.run("image.variables.dataSets", json!({"dataSets": sets}))?;
    if let Some(name) = fields.get("__apply").and_then(Value::as_str) {
        return app.run("image.applyDataSet", json!({"name": name}));
    }
    Ok(json!({"defined": true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{DataSet, DataValue, LayerId, VarKind, VariableDef, Variables, variables::Value as VV};

    #[test]
    fn to_state_mirrors_the_command_shape() {
        let v = Variables {
            defs: vec![
                VariableDef::direct("show", LayerId(1), VarKind::Visibility),
                VariableDef::direct("title", LayerId(2), VarKind::TextReplacement),
                VariableDef::ruled("Type == \"gear\"", LayerId(3), VarKind::Visibility),
            ],
            names: vec!["Type".into()],
            data_sets: vec![DataSet { name: "A".into(), values: vec![DataValue { variable: "show".into(), value: VV::Visibility(false) }] }],
            active: None,
        };
        let s = to_state(&v);
        assert_eq!(s["defs"][0]["name"], "show");
        assert_eq!(s["defs"][0]["type"], "visibility");
        assert_eq!(s["defs"][1]["type"], "textReplacement");
        assert!(s["defs"][1].get("rule").is_none());
        assert_eq!(s["defs"][2]["rule"], "Type == \"gear\"");
        assert_eq!(s["names"], json!(["Type"]));
        assert_eq!(s["dataSets"][0]["name"], "A");
        assert_eq!(s["dataSets"][0]["values"][0]["kind"], "visibility");
        assert_eq!(s["dataSets"][0]["values"][0]["value"], false);
    }

    #[test]
    fn data_set_index_rejects_out_of_range_control_values() {
        let mut fields = Map::new();
        assert_eq!(data_set_index(&fields, 1), 0);
        fields.insert("__cur".into(), json!(999));
        assert_eq!(data_set_index(&fields, 1), 0);
        fields.insert("__cur".into(), json!(u64::MAX));
        assert_eq!(data_set_index(&fields, 1), 0);
        fields.insert("__cur".into(), json!(1));
        assert_eq!(data_set_index(&fields, 3), 1);
        assert_eq!(data_set_index(&fields, 0), 0);
    }

    #[test]
    fn normalize_keeps_only_object_entries() {
        let s = normalize(&json!({"defs": [1, {"name": "a"}], "dataSets": ["x", {"name": "A", "values": [true, {"variable": "a"}]}, {"values": 3}]}));
        assert_eq!(s, json!({"defs": [{"name": "a"}], "names": [], "dataSets": [{"name": "A", "values": [{"variable": "a"}]}, {"values": []}]}));
        assert_eq!(normalize(&json!({"names": ["a", 1, null, "b"]}))["names"], json!(["a", "b"]));
        for bad in [Value::Null, json!(5), json!("s"), json!([1]), json!({"defs": "x", "names": "y", "dataSets": {}})] {
            assert_eq!(normalize(&bad), json!({"defs": [], "names": [], "dataSets": []}), "{bad}");
        }
    }

    #[test]
    fn columns_cover_shared_variables_and_rule_inputs() {
        let state = json!({
            "names": ["Level"],
            "defs": [
                {"name": "Name", "layer": 1, "type": "textReplacement"},
                {"name": "Name", "layer": 2, "type": "textReplacement"},
                {"name": "Show", "layer": 3, "type": "visibility"},
                {"name": "Img", "layer": 4, "type": "pixelReplacement"},
                {"rule": "Type == \"gear\" and {Card Name} != Name", "layer": 5, "type": "visibility"},
                {"rule": "not valid (", "layer": 6, "type": "visibility"},
            ],
        });
        let cols = columns(&state);
        let want = [("Level", "text"), ("Name", "text"), ("Show", "visibility"), ("Img", "pixelReplacement"), ("Type", "text"), ("Card Name", "text")];
        assert_eq!(cols, want.map(|(n, k)| (n.to_string(), k)).to_vec());
    }

    #[test]
    fn selected_layer_falls_back_to_a_bound_then_the_top_layer() {
        let layers = vec![(1, "a".to_string(), false), (2, "b".to_string(), true)];
        let defs = vec![json!({"layer": 2})];
        let mut f = Map::new();
        assert_eq!(selected_layer(&f, &layers, &defs), Some(2));
        assert_eq!(selected_layer(&f, &layers, &[]), Some(1));
        f.insert("__layer".into(), json!(1));
        assert_eq!(selected_layer(&f, &layers, &defs), Some(1));
        f.insert("__layer".into(), json!(99));
        assert_eq!(selected_layer(&f, &layers, &defs), Some(2), "a stale layer id is ignored");
        assert_eq!(selected_layer(&f, &[], &defs), None);
    }

    #[test]
    fn rule_preview_uses_the_shown_data_set() {
        let state = json!({"dataSets": [
            {"name": "A", "values": [{"variable": "Type", "kind": "text", "value": "Gear"}]},
            {"name": "B", "values": [{"variable": "type", "kind": "text", "value": "rock"}, {"variable": "Show", "kind": "visibility", "value": false}]},
        ]});
        let mut f = Map::new();
        assert_eq!(preview("Type == \"gear\"", &state, &f), Ok(Some("A → true".into())));
        f.insert("__cur".into(), json!(1));
        assert_eq!(preview("Type == \"gear\"", &state, &f), Ok(Some("B → false".into())), "names match ignoring case");
        assert_eq!(preview("if(Show, \"y\", \"n\")", &state, &f), Ok(Some("B → \"n\"".into())));
        assert!(preview("Type ==", &state, &f).is_err());
        assert_eq!(preview("1", &json!({}), &f), Ok(None), "no data set: nothing to preview");
    }

    #[test]
    fn bound_layers_lists_each_layer_once_in_order() {
        let defs = vec![json!({"layer": 3}), json!({"layer": 1}), json!({"layer": 3}), json!({"name": "no layer"})];
        assert_eq!(bound_layers(&defs), vec![3, 1]);
    }

    #[test]
    fn alt_reset_restores_open_snapshot() {
        let mut f = Map::new();
        let orig = json!({"defs": [{"name": "a"}], "dataSets": []});
        f.insert("__variables".into(), json!({"defs": [{"name": "edited"}], "dataSets": []}));
        f.insert("__variablesOrig".into(), orig.clone());
        reset(&mut f);
        assert_eq!(f["__variables"], orig);
    }

    /// Render the Define page once, headless, and return its height.
    fn define_page_height(state: &mut Value, fields: &mut Map<String, Value>, layers: &[(u64, String, bool)]) -> f32 {
        let ctx = egui::Context::default();
        let t = Tokens::get(&ctx);
        let mut h = 0.0;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let r = ui.vertical(|ui| define_page(ui, &t, layers, state, fields)).response;
            h = r.rect.height();
        });
        out.textures_delta.clear();
        h
    }

    #[test]
    fn many_bindings_scroll_instead_of_growing_the_dialog() {
        let few: Vec<(u64, String, bool)> = (1..=2).map(|i| (i, format!("Layer {i}"), false)).collect();
        let many: Vec<(u64, String, bool)> = (1..=40).map(|i| (i, format!("Layer {i}"), false)).collect();
        let defs = |n: u64| json!({"defs": (1..=n).map(|i| json!({"name": format!("v{i}"), "layer": i, "type": "visibility"})).collect::<Vec<_>>(), "names": [], "dataSets": []});
        let small = define_page_height(&mut defs(2), &mut Map::new(), &few);
        let mut fields = Map::new();
        let big = define_page_height(&mut defs(40), &mut fields, &many);
        // 38 more rows (~20 px each) would add ~760 px unbounded; the scroll box caps the growth.
        assert!(big - small < BINDINGS_MAX_H + 60.0, "small {small}, big {big}");
        assert_eq!(fields.get("__bindScrolled"), Some(&json!(1)), "the edited layer is scrolled into view");
    }

    #[test]
    fn bindings_filter_survives_and_matches_nothing_gracefully() {
        let layers: Vec<(u64, String, bool)> = (1..=12).map(|i| (i, format!("Layer {i}"), false)).collect();
        let mut state = json!({"defs": (1..=12).map(|i| json!({"name": format!("v{i}"), "layer": i, "type": "visibility"})).collect::<Vec<_>>()});
        let mut fields = Map::new();
        fields.insert("__bindFilter".into(), json!("no such thing"));
        define_page_height(&mut state, &mut fields, &layers);
        assert_eq!(fields["__bindFilter"], "no such thing");
        assert_eq!(state["defs"].as_array().map(Vec::len), Some(12), "filtering never removes bindings");
    }

    #[test]
    fn owns_detects_the_field() {
        let mut f = Map::new();
        assert!(!owns(&f));
        f.insert("__variables".into(), json!({}));
        assert!(owns(&f));
    }
}
