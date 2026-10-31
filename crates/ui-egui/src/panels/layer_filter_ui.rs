//! The Layers panel's filter row (#5048): a filter-type dropdown, as in Photoshop, then the
//! controls for that type. Kind toggles layer kinds, Name matches part of the layer name, Mode
//! picks one blend mode, Smart Object toggles linked, missing and embedded smart objects. The
//! criteria live in `UiState` (`layer_filter*`), so the control channel can read and set them.

use std::collections::HashSet;

use photocraft_color::BlendMode;
use photocraft_doc::{Layer, LayerContent, SmartSource};
use serde::{Deserialize, Serialize};

use crate::state::UiState;
use crate::{PhotocraftApp, icons, widgets};

/// What the Layers panel filters by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterBy {
    #[default]
    Kind,
    Name,
    Mode,
    SmartObject,
}

/// The filter criteria, copied out of `UiState` for one frame of the Layers panel.
#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    pub by: FilterBy,
    pub kinds: Vec<String>,
    /// As typed; matched trimmed and ignoring case.
    pub name: String,
    pub mode: BlendMode,
    /// The Smart Object toggles that are on, from [`SMART`].
    pub smart: Vec<String>,
    /// The linked files the Smart Object filter's Missing toggle lists
    /// ([`photocraft_engine::Session::missing_links`]).
    pub missing: HashSet<String>,
}

impl Filter {
    pub fn of(ui: &UiState) -> Self {
        Self {
            by: ui.layer_filter_by,
            kinds: ui.layer_filter.clone(),
            name: ui.layer_filter_name.clone(),
            mode: ui.layer_filter_mode,
            smart: ui.layer_filter_smart.clone(),
            missing: HashSet::new(),
        }
    }

    /// [`Filter::of`] `app`'s settings, with the active document's missing linked files while
    /// the panel filters by Smart Object.
    pub fn of_app(app: &PhotocraftApp) -> Self {
        let mut f = Self::of(&app.ui);
        if f.by == FilterBy::SmartObject {
            f.missing = app.session.missing_links();
        }
        f
    }

    /// Whether the panel is filtered. Like Photoshop, Mode always filters (by Normal at first);
    /// Kind, Name and Smart Object only once a toggle is on or some text is typed.
    pub fn active(&self) -> bool {
        match self.by {
            FilterBy::Kind => !self.kinds.is_empty(),
            FilterBy::Name => !self.name.trim().is_empty(),
            FilterBy::Mode => true,
            FilterBy::SmartObject => !self.smart.is_empty(),
        }
    }

    /// Whether `l` is listed under this filter.
    pub fn shows(&self, l: &Layer) -> bool {
        if !self.active() {
            return true;
        }
        match self.by {
            FilterBy::Kind => self.kinds.iter().any(|k| k == kind(l)),
            FilterBy::Name => l.name.to_lowercase().contains(&self.name.trim().to_lowercase()),
            FilterBy::Mode => l.blend == self.mode,
            FilterBy::SmartObject => {
                let LayerContent::Smart(sm) = &l.content else { return false };
                self.smart.iter().any(|k| match (k.as_str(), &sm.source) {
                    ("linked", SmartSource::Linked { .. }) | ("embedded", SmartSource::Embedded { .. }) => true,
                    ("missing", SmartSource::Linked { path }) => self.missing.contains(path),
                    _ => false,
                })
            }
        }
    }
}

/// The kinds the Kind filter toggles.
pub const KINDS: [&str; 5] = ["pixel", "adjustment", "type", "shape", "smart"];

/// The Smart Object filter's toggles: smart objects linked to a file, linked to a file that is
/// not there, and embedded in the document.
pub const SMART: [&str; 3] = ["linked", "missing", "embedded"];

/// `ui.set {layerFilter: {by?, kinds?, name?, mode?, smart?}}`: `current` with the given fields
/// replaced, or an error naming the bad field (nothing changes then). `by` is `kind`, `name`,
/// `mode` or `smartObject`; `kinds` a list of [`KINDS`]; `mode` a blend mode name such as
/// `Multiply` or `Pass Through`; `smart` a list of [`SMART`].
pub fn patched(current: &UiState, v: &serde_json::Value) -> Result<Filter, String> {
    let Some(o) = v.as_object() else { return Err("layerFilter must be an object".into()) };
    if let Some(k) = o.keys().find(|k| !["by", "kinds", "name", "mode", "smart"].contains(&k.as_str())) {
        return Err(format!("unknown layerFilter field `{k}` (fields: by, kinds, name, mode, smart)"));
    }
    let mut f = Filter::of(current);
    if let Some(by) = o.get("by") {
        f.by = serde_json::from_value(by.clone()).map_err(|_| "layerFilter.by must be kind, name, mode or smartObject".to_string())?;
    }
    if let Some(kinds) = o.get("kinds") {
        let list = kinds.as_array().ok_or("layerFilter.kinds must be a list")?;
        f.kinds = Vec::new();
        for k in list {
            let Some(k) = k.as_str().filter(|k| KINDS.contains(k)) else {
                return Err(format!("layerFilter.kinds: unknown kind {k} (kinds: {})", KINDS.join(", ")));
            };
            if !f.kinds.iter().any(|have| have == k) {
                f.kinds.push(k.to_string());
            }
        }
    }
    if let Some(name) = o.get("name") {
        f.name = name.as_str().ok_or("layerFilter.name must be a string")?.to_string();
    }
    if let Some(mode) = o.get("mode") {
        let name = mode.as_str().ok_or("layerFilter.mode must be a blend mode name")?;
        f.mode = photocraft_engine::commands::blend_from_str(name)
            .filter(|m| *m == BlendMode::PassThrough || BlendMode::LAYER_MODES.contains(m))
            .ok_or_else(|| format!("layerFilter.mode: unknown blend mode `{name}`"))?;
    }
    if let Some(smart) = o.get("smart") {
        let list = smart.as_array().ok_or("layerFilter.smart must be a list")?;
        f.smart = Vec::new();
        for k in list {
            let Some(k) = k.as_str().filter(|k| SMART.contains(k)) else {
                return Err(format!("layerFilter.smart: unknown toggle {k} (toggles: {})", SMART.join(", ")));
            };
            if !f.smart.iter().any(|have| have == k) {
                f.smart.push(k.to_string());
            }
        }
    }
    Ok(f)
}

/// Store `f` as the Layers panel's filter.
pub fn apply(ui: &mut UiState, f: Filter) {
    ui.layer_filter_by = f.by;
    ui.layer_filter = f.kinds;
    ui.layer_filter_name = f.name;
    ui.layer_filter_mode = f.mode;
    ui.layer_filter_smart = f.smart;
}

/// The kind filter's name for `l`'s content ("" for groups, which no kind matches).
fn kind(l: &Layer) -> &'static str {
    match &l.content {
        LayerContent::Raster(_) => "pixel",
        LayerContent::Adjustment(_) | LayerContent::Fill(_) => "adjustment",
        LayerContent::Text(_) => "type",
        LayerContent::Shape(_) => "shape",
        LayerContent::Smart(_) => "smart",
        LayerContent::Group(_) => "",
    }
}

/// Draw the filter row: the type dropdown, then that type's controls.
pub fn bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let by = [(FilterBy::Kind, tl!("Kind")), (FilterBy::Name, tl!("Name")), (FilterBy::Mode, tl!("Mode")), (FilterBy::SmartObject, tl!("Smart Object"))];
        widgets::dropdown(ui, "layer-filter-by", tl!("Kind"), &mut app.ui.layer_filter_by, &by, 64.0);
        ui.add_space(4.0);
        match app.ui.layer_filter_by {
            FilterBy::Kind => {
                for (kind, icon, tip) in [
                    ("pixel", "image", tl!("Filter for pixel layers")),
                    ("adjustment", "adjustment-layer", tl!("Filter for adjustment layers")),
                    ("type", "type", tl!("Filter for type layers")),
                    ("shape", "square", tl!("Filter for shape layers")),
                    ("smart", "app-window", tl!("Filter for smart objects")),
                ] {
                    let on = app.ui.layer_filter.iter().any(|k| k == kind);
                    if icons::button(ui, icon, 22.0, on, tip).clicked() {
                        if on {
                            app.ui.layer_filter.retain(|k| k != kind);
                        } else {
                            app.ui.layer_filter.push(kind.to_string());
                        }
                    }
                }
            }
            FilterBy::Name => {
                let width = ui.available_width().max(48.0);
                widgets::text_field(ui, ui.scope_id().with("layer-filter-name"), &mut app.ui.layer_filter_name, tl!("Search"), width, false);
            }
            FilterBy::Mode => {
                let modes: Vec<(BlendMode, &str)> = std::iter::once(BlendMode::PassThrough).chain(BlendMode::LAYER_MODES).map(|m| (m, m.label())).collect();
                let width = ui.available_width().max(96.0);
                widgets::dropdown(ui, "layer-filter-mode", tl!("Mode"), &mut app.ui.layer_filter_mode, &modes, width);
            }
            FilterBy::SmartObject => {
                for (kind, icon, tip) in [
                    ("linked", "link", tl!("Filter for smart objects linked locally")),
                    ("missing", "unlink", tl!("Filter for linked smart objects that are missing")),
                    ("embedded", "file", tl!("Filter for embedded smart objects")),
                ] {
                    let on = app.ui.layer_filter_smart.iter().any(|k| k == kind);
                    if icons::button(ui, icon, 22.0, on, tip).clicked() {
                        if on {
                            app.ui.layer_filter_smart.retain(|k| k != kind);
                        } else {
                            app.ui.layer_filter_smart.push(kind.to_string());
                        }
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests;
