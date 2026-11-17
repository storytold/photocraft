//! Tool Presets: choosing a preset applies its tool and options; the + button stores the current tool's own options.

use serde_json::{Value, json};

use super::run;
use crate::PhotocraftApp;
use crate::state::Tool;

/// The engine's id of `t` (the tool's name with a lower-case first letter).
pub(crate) fn tool_id(t: Tool) -> String {
    let s = format!("{t:?}");
    let mut c = s.chars();
    c.next().map(|f| f.to_ascii_lowercase().to_string() + c.as_str()).unwrap_or_default()
}

/// Apply a tool preset: the engine sets the brush/colour, the shell switches tool and options.
pub fn select(app: &mut PhotocraftApp, name: &str) {
    // PLAN: assumes `ToolOptions::crop` is `Clone + PartialEq` (tools-w45).
    let crop_before = app.ui.tool_options.crop.clone();
    let Some(r) = run(app, "tool.presets.select", json!({"preset": name})) else { return };
    if let Some(tool) = r["tool"].as_str().and_then(Tool::from_name) {
        crate::tool_memory::select(app, tool);
        // Each tool keeps its own brush (#218), so switch it in before the preset's brush lands
        // on it, and re-apply the brush the command just set for the old tool.
        crate::paint_mouse::sync_tool_brush(app);
        if let Some(b) = r["options"].get("brush").filter(|b| b.is_object()) {
            let _ = app.run("tools.setBrush", json!({"brush": b}));
        }
    }
    if let Some(stored) = r["options"].get("toolOptions").filter(|o| o.is_object()) {
        // A preset saved by an older build holds the old option keys: read them once, here.
        let mut stored = stored.clone();
        // PLAN: `tool_memory::upgrade(&mut Value)` per cx.P6 (tools-w71) does not exist at the base.
        crate::tool_memory::upgrade(&mut stored);
        let mut cur = serde_json::to_value(&app.ui.tool_options).unwrap_or_default();
        if let (Some(c), Some(o)) = (cur.as_object_mut(), stored.as_object()) {
            for (k, v) in o {
                // An object option (`sample_all`, one per tool) takes only the entries the
                // preset stored, so the other tools' keep theirs (#5524).
                match (c.get_mut(k), v) {
                    (Some(Value::Object(have)), Value::Object(set)) => have.extend(set.clone()),
                    _ => {
                        c.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        if let Ok(opts) = serde_json::from_value(cur) {
            app.ui.tool_options = opts;
        }
    }
    // As the options bar does: a crop preset's ratio (or W x H) fits the frame at once.
    if app.ui.tool_options.crop != crop_before {
        // PLAN: `crop_ui::refit(app, editing: bool)` per tl.P1 (tools-w45) does not exist at the base.
        crate::crop_ui::refit(app, false);
    }
}

/// The `tool.presets.new` params for the + button: `tool`'s own options only, so choosing the
/// preset later leaves the other tools' settings alone.
pub(crate) fn new_params(app: &PhotocraftApp, tool: Tool) -> Value {
    let all = serde_json::to_value(&app.ui.tool_options).unwrap_or_default();
    let mut own: serde_json::Map<String, Value> =
        crate::tool_reset::option_keys(tool).iter().filter_map(|k| Some(((*k).to_string(), all.get(*k)?.clone()))).collect();
    // Sample All Layers is each tool's own (#5524): store `tool`'s alone.
    if let Some(k) = crate::tool_reset::sample_all_key(tool)
        && let Some(v) = all.get("sample_all").and_then(|m| m.get(&k))
    {
        own.insert("sample_all".into(), json!({ k: v }));
    }
    json!({"name": tool.label(), "tool": tool_id(tool), "options": {"toolOptions": own}})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.sync_views();
        app
    }

    #[test]
    fn tool_preset_switches_tool_and_options() {
        let mut app = app();
        app.ui.tool = Tool::Brush;
        select(&mut app, "Radial Gradient");
        assert_eq!(app.ui.tool, Tool::Gradient);
        assert_eq!(app.ui.tool_options.gradient_style, "radial");
        select(&mut app, "Soft Eraser 60 px");
        assert_eq!(app.ui.tool, Tool::Eraser);
        assert_eq!(app.session.tools.brush.size, 60.0);
        assert_eq!(tool_id(Tool::CloneStamp), "cloneStamp");
        assert_eq!(tool_id(Tool::PatternStamp), "patternStamp");
        assert_eq!(Tool::from_name(&tool_id(Tool::CustomShape)), Some(Tool::CustomShape));
    }

    /// A crop preset picked in Tool Presets fits the frame to its ratio at once, as the options
    /// bar's dropdown does.
    #[test]
    fn a_crop_tool_preset_fits_the_frame_to_its_ratio() {
        let mut app = app();
        app.ui.tool = Tool::Crop;
        // The tool draws its default frame in its first pass.
        crate::canvas::testing::frame(&mut app);
        let size = |app: &PhotocraftApp| crate::crop_ui::frame(app).and_then(|frame| frame.crop_size());
        assert_eq!(size(&app), Some((64.0, 48.0)));
        app.run("tool.presets.new", json!({"name": "Square", "tool": "crop", "options": {"toolOptions": {"crop_ratio": "1:1"}}})).unwrap();
        select(&mut app, "Square");
        crate::canvas::testing::frame(&mut app);
        assert_eq!(size(&app), Some((48.0, 48.0)));
    }

    /// Every bool of `value` turned over, so a test sees which options a preset left alone.
    fn flip_bools(value: &mut Value) {
        match value {
            Value::Bool(b) => *b = !*b,
            Value::Object(map) => map.values_mut().for_each(flip_bools),
            _ => {}
        }
    }

    /// #7356: a preset made with + keeps only its own tool's options, so choosing it doesn't
    /// reset the Crop tool's Delete Cropped Pixels or the Move tool's Auto-Select.
    #[test]
    fn new_tool_preset_stores_only_its_tools_options() {
        let mut app = app();
        app.ui.tool = Tool::RectMarquee;
        app.ui.tool_options.feather = 5.0;
        let p = new_params(&app, Tool::RectMarquee);
        let r = app.run("tool.presets.new", p).unwrap();
        let name = r["name"].as_str().unwrap().to_string();
        let stored = app.session.presets.tool_presets.iter().find(|t| t.name == name).unwrap().options["toolOptions"].as_object().unwrap().clone();
        assert_eq!(stored["feather"], json!(5.0));
        assert!(stored.keys().all(|k| !k.starts_with("crop") && !k.starts_with("move")), "{stored:?}");
        let mut flipped = serde_json::to_value(&app.ui.tool_options).unwrap();
        flip_bools(&mut flipped);
        flipped["feather"] = json!(0.0);
        app.ui.tool_options = serde_json::from_value(flipped.clone()).unwrap();
        select(&mut app, &name);
        let after = serde_json::to_value(&app.ui.tool_options).unwrap();
        assert_eq!(after["feather"], json!(5.0));
        for (key, value) in flipped.as_object().unwrap().iter().filter(|(key, _)| !stored.contains_key(*key)) {
            assert_eq!(&after[key], value, "{key} is not this preset's option");
        }
    }

    /// #5524 with #7356: a Blur preset keeps Blur's Sample All Layers and leaves the Magic Wand's
    /// alone, both when it is made and when it is chosen.
    #[test]
    fn new_tool_preset_stores_only_its_tools_sample_all_layers() {
        let mut app = app();
        app.ui.tool = Tool::Blur;
        app.ui.tool_options.sample_all.blur = true;
        let p = new_params(&app, Tool::Blur);
        assert_eq!(p["options"]["toolOptions"]["sample_all"], json!({"blur": true}));
        let name = app.run("tool.presets.new", p).unwrap()["name"].as_str().unwrap().to_string();
        app.ui.tool_options.sample_all.blur = false;
        app.ui.tool_options.sample_all.magic_wand = true;
        select(&mut app, &name);
        assert!(app.ui.tool_options.sample_all.blur);
        assert!(app.ui.tool_options.sample_all.magic_wand, "the Magic Wand's Sample All Layers is not Blur's");
    }
}
