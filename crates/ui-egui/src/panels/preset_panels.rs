//! The state of the preset panels (Gradients, Patterns, Styles, Shapes, Tool Presets, Clone Source) and the paths their
//! callers use.
//!
//! The panels share one browser, the framework's `PresetBrowser`, in `preset_browser`. Every action is an engine command
//! (`gradient.presets.*`, `pattern.presets.*`, `style.presets.*`, `shape.presets.*`, `tool.presets.*`, `cloneSource.*`);
//! the panel modules only draw and dispatch. The dock draws the bodies from the panel registry.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(test)]
use serde_json::json;

use crate::PhotocraftApp;

pub(crate) mod clone_offsets;
pub(crate) mod clone_source;
pub(crate) mod tool_presets;

pub use super::preset_browser::{finish_custom_shape, gradients_panel, patterns_panel, shape_picker, shapes_panel, styles_panel, tool_presets_panel};
pub use clone_offsets::{clone_mapping_at, clone_sample_point};
pub(crate) use clone_offsets::{clone_offset_edit, clone_offset_readout, remember_clone_cursor};
pub use clone_source::panel as clone_source_panel;
pub use tool_presets::select as select_tool_preset;

/// Panel state (serde, so the control channel can read and drive it).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetUi {
    /// Last clone preview position in its destination document; never restored into another session.
    #[serde(skip)]
    pub clone_cursor: Option<(photocraft_doc::DocId, [f64; 2])>,
    /// Selected preset per panel (by name / pattern id).
    pub selected: BTreeMap<String, String>,
    /// Tool Presets: Current Tool Only.
    pub current_tool_only: bool,
    /// Custom Shape tool: the shape it draws.
    pub custom_shape: String,
    /// Folder/group order per panel, by name (#3658), like the dock's `tabOrder` (#2272). A panel
    /// with no entry keeps the library's order; unknown names are ignored and groups missing from
    /// a list follow it in library order. Saved with the UI state.
    #[serde(rename = "groupOrder", alias = "group_order", deserialize_with = "lenient_group_order", skip_serializing_if = "BTreeMap::is_empty")]
    pub group_order: BTreeMap<String, Vec<String>>,
    /// The view state of the panels on the framework's preset browser, by panel. Read leniently
    /// like `groupOrder`: a panel's entry that no longer parses is dropped, not the whole saved UI state.
    #[serde(deserialize_with = "lenient_browsers", skip_serializing_if = "BTreeMap::is_empty")]
    pub browsers: BTreeMap<String, craft::ui::patterns::presets::PresetState<super::preset_browser::PresetKey>>,
}

/// Read `groupOrder` keeping what makes sense: non-object values, non-list entries and non-string
/// names are dropped instead of failing the whole saved UI state.
const MAX_GROUP_ORDER: usize = 10_000;
fn lenient_group_order<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeMap<String, Vec<String>>, D::Error> {
    let v = Value::deserialize(d)?;
    let mut out = BTreeMap::new();
    for (panel, names) in v.as_object().into_iter().flatten() {
        let Some(names) = names.as_array() else { continue };
        out.insert(panel.clone(), names.iter().filter_map(Value::as_str).take(MAX_GROUP_ORDER).map(str::to_owned).collect());
    }
    Ok(out)
}

fn lenient_browsers<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, craft::ui::patterns::presets::PresetState<super::preset_browser::PresetKey>>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(v.as_object().into_iter().flatten().filter_map(|(panel, state)| Some((panel.clone(), serde_json::from_value(state.clone()).ok()?))).collect())
}

impl PresetUi {
    /// The Custom Shape tool's shape name.
    pub fn shape(&self) -> &str {
        if self.custom_shape.is_empty() { tl!("Heart") } else { &self.custom_shape }
    }

    /// Display order of the folders for `panel`, as library indices: the stored order first
    /// (names that no longer exist are dropped), then groups missing from it in library order.
    /// Mirrors the dock's per-group tab order (#2272, #3658).
    pub(crate) fn display_order(&self, panel: &str, names: &[String]) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::with_capacity(names.len());
        for name in self.group_order.get(panel).into_iter().flatten() {
            if let Some(i) = names.iter().position(|n| n == name)
                && !out.contains(&i)
            {
                out.push(i);
            }
        }
        for i in 0..names.len() {
            if !out.contains(&i) {
                out.push(i);
            }
        }
        out
    }

    /// Move folder `from` so it is drawn before `before` (`None` = last) for `panel`. The
    /// library's own order is stored as no entry, as the dock does for its tabs.
    pub(crate) fn move_group(&mut self, panel: &str, names: &[String], from: &str, before: Option<&str>) {
        if before == Some(from) {
            return;
        }
        let mut order: Vec<String> = self.display_order(panel, names).into_iter().filter_map(|i| names.get(i).cloned()).collect();
        let Some(at) = order.iter().position(|n| n == from) else { return };
        let moved = order.remove(at);
        let to = before.and_then(|b| order.iter().position(|n| n == b)).unwrap_or(order.len());
        order.insert(to, moved);
        if order.as_slice() == names {
            self.group_order.remove(panel);
        } else {
            self.group_order.insert(panel.to_string(), order);
        }
    }
}

/// Runs the engine command `id`; a failure goes to the status line.
pub(crate) fn run(app: &mut PhotocraftApp, id: &str, p: Value) -> Option<Value> {
    match app.run(id, p) {
        Ok(v) => Some(v),
        Err(e) => {
            app.ui.status = e;
            app.ui.status_error = true;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #3658: a group/folder drag reorders by name, stores only a real reorder, and falls back to
    /// the library's order when the saved order matches it or names a group that's gone.
    #[test]
    fn moving_a_folder_reorders_like_the_dock_tabs() {
        let names: Vec<String> = ["Grays", "Pastel", "Pure", "Dark"].iter().map(|s| s.to_string()).collect();
        let mut st = PresetUi::default();
        assert_eq!(st.display_order("swatches", &names), vec![0, 1, 2, 3]);
        st.move_group("swatches", &names, "Pastel", None);
        assert_eq!(st.group_order.get("swatches").unwrap(), &["Grays", "Pure", "Dark", "Pastel"]);
        st.move_group("swatches", &names, "Pastel", Some("Grays"));
        assert_eq!(st.group_order.get("swatches").unwrap(), &["Pastel", "Grays", "Pure", "Dark"]);
        assert_eq!(st.display_order("swatches", &names), vec![1, 0, 2, 3]);
        // Back to the library's own order: no entry, as the dock stores its default tab order.
        st.move_group("swatches", &names, "Pastel", Some("Pure"));
        assert!(!st.group_order.contains_key("swatches"), "a no-op order is not stored");
        // Out-of-range names, moves onto themselves and unknown `before` names change nothing bad.
        for (from, before) in [("Nope", None), ("Grays", Some("Grays"))] {
            let mut m = PresetUi::default();
            m.move_group("swatches", &names, from, before);
            assert!(m.group_order.is_empty(), "{from} {before:?}");
            assert_eq!(m.display_order("swatches", &names), vec![0, 1, 2, 3]);
        }
        // A missing group in a saved order follows the rest in library order.
        let mut m = PresetUi::default();
        m.group_order.insert("swatches".into(), vec!["Dark".into(), "Grays".into()]);
        assert_eq!(m.display_order("swatches", &names), vec![3, 0, 1, 2]);
    }

    /// #3658: the folder order persists with the UI state and reads leniently (an old state has no
    /// field, and garbage in one never fails the whole state).
    #[test]
    fn folder_order_persists_and_reads_leniently() {
        let old: PresetUi = serde_json::from_value(json!({"sizes": {"swatches": 22.0}})).unwrap();
        assert!(old.group_order.is_empty(), "an older state without the field still loads");
        let mut st = PresetUi::default();
        st.group_order.insert("swatches".into(), vec!["Dark".into(), "Grays".into()]);
        let back: PresetUi = serde_json::from_value(serde_json::to_value(&st).unwrap()).unwrap();
        assert_eq!(back.group_order.get("swatches").unwrap(), &["Dark", "Grays"]);
        for bad in [json!("x"), json!(7), json!({"swatches": "paths"}), json!({"swatches": [1, null, {"a": 1}]})] {
            let back: PresetUi = serde_json::from_value(json!({"groupOrder": bad})).unwrap();
            assert!(back.group_order.get("swatches").is_none_or(Vec::is_empty), "{bad}");
        }
        let mut st = PresetUi::default();
        st.browsers.insert("patterns".into(), Default::default());
        st.group_order.insert("patterns".into(), vec!["Rocks".into()]);
        st.selected.insert("patterns".into(), "p1".into());
        let mut saved = serde_json::to_value(&st).unwrap();
        saved["browsers"]["styles"] = json!("a state of an older format");
        let back: PresetUi = serde_json::from_value(saved).unwrap();
        assert!(back.browsers.contains_key("patterns") && !back.browsers.contains_key("styles"), "only the entry that fails is dropped");
        assert_eq!((back.group_order, back.selected), (st.group_order, st.selected), "the rest of the state loads");
    }
}
