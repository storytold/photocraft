//! Selected tool and tool options remembered between launches, as in Photoshop (#2814).
//!
//! `Preferences::tool_options` holds the [`ToolOptions`] fields that differ from their defaults,
//! so a default changed in a later release still reaches users who never touched that option.
//! Saved once the pointer is up (not on every slider step), restored at launch. A hand-edited or
//! old file can't break the options: an unknown key is ignored and a field that doesn't read back
//! (wrong type, a number too large for its field) keeps its default.

use serde_json::{Map, Value};

use crate::PhotocraftApp;
use crate::state::{Tool, ToolOptions};

/// Explicit choices override temporary pen and held-key tools, even when selecting the same tool.
pub(crate) fn select(app: &mut PhotocraftApp, tool: Tool) {
    app.stylus.tool_before_eraser = None;
    app.ui.spring = None;
    app.ui.tool = tool;
}

/// Temporary canvas overrides do not change `ui.tool`. Spring-loaded letters and the pen's
/// eraser end do, so remember the tool they will return to instead. A different explicit choice
/// (or beginning a type edit) owns the tool and stands, just as in `spring_tools::tick`.
fn selected(app: &PhotocraftApp) -> Tool {
    let tool = if app.ui.tool == Tool::Eraser { app.stylus.tool_before_eraser.unwrap_or(app.ui.tool) } else { app.ui.tool };
    if let Some(s) = &app.ui.spring
        && tool == s.tool
        && app.ui.text_edit.is_none()
    {
        return s.previous;
    }
    tool
}

/// The options that differ from the defaults, as JSON (`Null` when none do).
fn changed(o: &ToolOptions) -> Value {
    let (Ok(Value::Object(now)), Ok(Value::Object(def))) = (serde_json::to_value(o), serde_json::to_value(ToolOptions::default())) else {
        return Value::Null;
    };
    let diff: Map<String, Value> = now.into_iter().filter(|(k, v)| def.get(k) != Some(v)).collect();
    if diff.is_empty() { Value::Null } else { Value::Object(diff) }
}

/// Remember the selected tool and its options once the user lets go of the pointer. Run every
/// frame, so choices made through the control channel or a tool preset are remembered too.
pub fn persist(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    let tool = selected(app);
    if app.prefs_rt.selected_tool != Some(tool) {
        app.prefs_rt.selected_tool = Some(tool);
        if let Ok(now) = serde_json::to_value(tool)
            && app.session.prefs().selected_tool != now
        {
            app.session.prefs.edit(|p| p.selected_tool = now);
            // prefs_ui::tick ran before the panels; ensure it can flush this new revision.
            ctx.request_repaint();
        }
    }
    if app.prefs_rt.tool_options.as_ref() == Some(&app.ui.tool_options) {
        return;
    }
    app.prefs_rt.tool_options = Some(app.ui.tool_options.clone());
    let now = changed(&app.ui.tool_options);
    if app.session.prefs().tool_options != now {
        app.session.prefs.edit(|p| p.tool_options = now);
    }
}

/// Flush the final choice on normal exit, even if no further frame ran after the tool changed.
pub(crate) fn save_on_exit(app: &mut PhotocraftApp) -> Result<(), String> {
    persist(app, &egui::Context::default());
    crate::prefs_ui::save_preferences(app)
}

/// Restore the selected tool before the first frame and its options field by field.
pub fn restore(app: &mut PhotocraftApp) {
    let saved = app.session.prefs().selected_tool.clone();
    if !saved.is_null() {
        match serde_json::from_value::<Tool>(saved) {
            Ok(tool) => {
                app.ui.tool = tool;
                app.ui.remember_group_tool();
            }
            Err(err) => log::warn!("Preferences: ignoring the remembered selected tool: {err}"),
        }
    }
    app.prefs_rt.selected_tool = Some(app.ui.tool);
    let saved = app.session.prefs().tool_options.clone();
    if let Some(saved) = saved.as_object()
        && let Ok(Value::Object(mut cur)) = serde_json::to_value(&app.ui.tool_options)
    {
        for (k, v) in saved {
            let Some(old) = cur.get(k).cloned() else { continue };
            cur.insert(k.clone(), v.clone());
            if !reads_back(&cur, k) {
                log::warn!("Preferences: ignoring the remembered tool option `{k}`: {v}");
                cur.insert(k.clone(), old);
            }
        }
        if let Ok(o) = serde_json::from_value(Value::Object(cur)) {
            app.ui.tool_options = o;
        }
    } else if !saved.is_null() {
        log::warn!("Preferences: the remembered tool options are not an object; using the defaults");
    }
    app.prefs_rt.tool_options = Some(app.ui.tool_options.clone());
}

/// Does `opts` deserialize, with field `k` coming back as a value? A number out of an `f32`'s
/// range reads as infinity, which writes back as `null`.
fn reads_back(opts: &Map<String, Value>, k: &str) -> bool {
    let Ok(o) = serde_json::from_value::<ToolOptions>(Value::Object(opts.clone())) else { return false };
    let Ok(Value::Object(back)) = serde_json::to_value(&o) else { return false };
    back.get(k).is_some_and(|v| nulls(v) <= opts.get(k).map_or(0, nulls))
}

fn nulls(v: &Value) -> usize {
    match v {
        Value::Null => 1,
        Value::Array(a) => a.iter().map(nulls).sum(),
        Value::Object(o) => o.values().map(nulls).sum(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_tool_and_non_default_options_roundtrip() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.tool = Tool::Move;
        app.ui.tool_options.tolerance = 42.5;
        persist(&mut app, &egui::Context::default());

        let saved = app.session.prefs();
        assert_eq!(saved.selected_tool, serde_json::to_value(Tool::Move).unwrap());
        assert_eq!(saved.tool_options["tolerance"], Value::from(42.5));
        assert!(saved.tool_options.get("contiguous").is_none(), "unchanged defaults are not written");

        let mut restored = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        restored.session.prefs.edit(|p| {
            p.selected_tool = saved.selected_tool.clone();
            p.tool_options = saved.tool_options.clone();
        });
        restore(&mut restored);
        assert_eq!(restored.ui.tool, Tool::Move);
        assert_eq!(restored.ui.tool_options.tolerance, 42.5);
    }

    #[test]
    fn restore_ignores_unknown_and_malformed_tool_options() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let original_tool = app.ui.tool;
        let default_contiguous = ToolOptions::default().contiguous;
        app.session.prefs.edit(|p| {
            p.selected_tool = Value::from("not-a-tool");
            p.tool_options = serde_json::json!({
                "tolerance": 42.5,
                "contiguous": "not a boolean",
                "future_option": true
            });
        });

        restore(&mut app);
        assert_eq!(app.ui.tool, original_tool, "an invalid selected tool leaves the current tool intact");
        assert_eq!(app.ui.tool_options.tolerance, 42.5, "valid fields still restore");
        assert_eq!(app.ui.tool_options.contiguous, default_contiguous, "a malformed field falls back to its default");
    }
}
