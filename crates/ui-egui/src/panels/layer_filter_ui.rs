//! The Layers panel's filter row (#5048): a filter-type dropdown, as in Photoshop, then the
//! controls for that type. Kind toggles layer kinds, Name matches part of the layer name, Effect picks one effect kind,
//! Mode picks one blend mode, Attribute one layer attribute (#10881), Color one layer colour label (#5898), Smart Object toggles linked, missing
//! and embedded smart objects, Artboard lists only the artboards that hold the selected layers. The
//! criteria live per open document; missing links and selected-artboard rows are derived context.

use std::collections::HashSet;

use photocraft_color::BlendMode;
use photocraft_doc::{Document, Fill, LabelColor, Layer, LayerContent, LayerId, SmartSource};
use serde::{Deserialize, Serialize};

use crate::{PhotocraftApp, icons, widgets};

/// What the Layers panel filters by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterBy {
    #[default]
    Kind,
    Name,
    Effect,
    Mode,
    Attribute,
    Color,
    SmartObject,
    Artboard,
}

/// One open document's user criteria. Derived row context is never stored here.
#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    /// Off keeps every criterion but shows the whole tree.
    pub on: bool,
    pub by: FilterBy,
    pub kinds: Vec<String>,
    /// As typed; matched trimmed and ignoring case.
    pub name: String,
    pub mode: BlendMode,
    pub effect: effect::EffectKind,
    pub attribute: attribute::Attribute,
    pub color: LabelColor,
    /// The Smart Object toggles that are on, from [`SMART`].
    pub smart: Vec<String>,
}

/// Derived current-frame information, never part of stored criteria or equality.
#[derive(Default)]
pub struct FilterContext {
    pub missing: HashSet<String>,
    pub board_rows: HashSet<LayerId>,
    /// Empty/Not Empty read this document-derived cache, never stored criteria.
    pub empty: Option<std::sync::Arc<HashSet<LayerId>>>,
}

impl FilterContext {
    pub fn of_app(app: &PhotocraftApp, filter: &Filter) -> Self {
        Self::with_artboard_rows(app, filter, || app.session.active().map_or_else(HashSet::new, |state| board_rows(&state.doc, &state.selected_layers())))
    }

    /// Use the document and selection that the Layers panel already read for this frame.
    pub(super) fn with_selection(app: &PhotocraftApp, filter: &Filter, doc: &Document, selection: &[LayerId]) -> Self {
        Self::with_artboard_rows(app, filter, || board_rows(doc, selection))
    }

    fn with_artboard_rows(app: &PhotocraftApp, filter: &Filter, artboard_rows: impl FnOnce() -> HashSet<LayerId>) -> Self {
        let mut context = Self::default();
        // Off lists every layer: nothing reads the context, so skip the scans (the emptiness
        // scan reruns after every paint stroke).
        if !filter.on {
            return context;
        }
        match filter.by {
            FilterBy::SmartObject => context.missing = app.session.missing_links(),
            FilterBy::Artboard => context.board_rows = artboard_rows(),
            FilterBy::Attribute => context.empty = attribute::empty_layers(app, filter.attribute),
            _ => {}
        }
        context
    }
}

impl Filter {
    pub fn of(app: &PhotocraftApp) -> Self {
        switch::current(app)
    }

    /// Matching without document-derived context (Kind, Name, Mode, Color, Smart toggles).
    pub fn active(&self) -> bool {
        self.active_with(&FilterContext::default())
    }

    pub fn active_with(&self, context: &FilterContext) -> bool {
        self.on
            && match self.by {
                FilterBy::Kind => !self.kinds.is_empty(),
                FilterBy::Name => !self.name.trim().is_empty(),
                FilterBy::Effect | FilterBy::Mode | FilterBy::Attribute | FilterBy::Color => true,
                FilterBy::SmartObject => !self.smart.is_empty(),
                FilterBy::Artboard => !context.board_rows.is_empty(),
            }
    }

    /// The switch and the type with only the criterion that type reads; the other types' stored
    /// values are dormant and left at their defaults, so changing one changes nothing here.
    pub fn effective(&self) -> Self {
        let mut f = Self { on: self.on, by: self.by, ..Self::default() };
        match self.by {
            FilterBy::Kind => f.kinds = self.kinds.clone(),
            FilterBy::Name => f.name = self.name.trim().to_lowercase(),
            FilterBy::Effect => f.effect = self.effect,
            FilterBy::Mode => f.mode = self.mode,
            FilterBy::Attribute => f.attribute = self.attribute,
            FilterBy::Color => f.color = self.color,
            FilterBy::SmartObject => f.smart = self.smart.clone(),
            FilterBy::Artboard => {}
        }
        f
    }

    /// Whether the criteria pick something: a Kind or Smart Object button on, a Name typed, or a
    /// type that always has a value. Off with criteria set is a manual Off.
    pub fn criteria_set(&self) -> bool {
        match self.by {
            FilterBy::Kind => !self.kinds.is_empty(),
            FilterBy::Name => !self.name.trim().is_empty(),
            FilterBy::Effect | FilterBy::Mode | FilterBy::Attribute | FilterBy::Color | FilterBy::Artboard => true,
            FilterBy::SmartObject => !self.smart.is_empty(),
        }
    }

    /// Only explicit edits of the active criterion update automatic activation; context and
    /// dormant criteria are not.
    pub fn activate_changed(&mut self, before: &Self) {
        let mut old = before.effective();
        old.on = self.on;
        if self.effective() != old {
            // Artboard is armed when chosen, even without eligible selected layers. Only
            // user criteria are compared here; selection context cannot undo a manual Off.
            if self.by == FilterBy::Artboard {
                if before.by != FilterBy::Artboard {
                    self.on = true;
                }
                return;
            }
            self.on = self.criteria_set();
        }
    }

    pub fn shows(&self, layer: &Layer) -> bool {
        self.shows_with(layer, &FilterContext::default())
    }

    pub fn shows_with(&self, l: &Layer, context: &FilterContext) -> bool {
        if !self.active_with(context) {
            return true;
        }
        match self.by {
            FilterBy::Kind => self.kinds.iter().any(|k| k == kind(l)),
            FilterBy::Name => l.name.to_lowercase().contains(&self.name.trim().to_lowercase()),
            FilterBy::Effect => self.effect.shows(l),
            FilterBy::Mode => l.blend == self.mode,
            FilterBy::Attribute => self.attribute.shows(l, context.empty.as_deref()),
            FilterBy::Color => l.label == self.color,
            FilterBy::SmartObject => {
                let LayerContent::Smart(sm) = &l.content else { return false };
                self.smart.iter().any(|k| match (k.as_str(), &sm.source) {
                    ("linked", SmartSource::Linked { .. }) | ("embedded", SmartSource::Embedded { .. }) => true,
                    ("missing", SmartSource::Linked { path }) => context.missing.contains(path),
                    _ => false,
                })
            }
            FilterBy::Artboard => context.board_rows.contains(&l.id),
        }
    }
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            on: false,
            by: FilterBy::Kind,
            kinds: Vec::new(),
            name: String::new(),
            mode: BlendMode::Normal,
            effect: effect::EffectKind::default(),
            attribute: attribute::Attribute::default(),
            color: LabelColor::None,
            smart: Vec::new(),
        }
    }
}

pub mod switch;
pub fn retain_open(app: &mut PhotocraftApp) {
    switch::retain_open(app);
}

/// The Artboard filter's rows for `selection`: the artboards that hold the selected layers and
/// every layer inside them. The set is empty when nothing is selected or a selected layer is in
/// no artboard, and the panel is then not filtered. The panel lists these rows as its tree does: the contents of a closed
/// group stay hidden until the group is opened (`layer_tree_ui::panel_rows` opens every group only
/// for the types whose matches can sit in a closed group).
pub fn board_rows(doc: &Document, selection: &[LayerId]) -> HashSet<LayerId> {
    fn add(l: &Layer, rows: &mut HashSet<LayerId>) {
        let mut pending = vec![l];
        while let Some(layer) = pending.pop() {
            rows.insert(layer.id);
            pending.extend(layer.children().unwrap_or_default());
        }
    }
    let selected: HashSet<LayerId> = selection.iter().copied().collect();
    if selected.is_empty() {
        return HashSet::new();
    }
    // Artboards are top-level groups ([`Document::artboard_of`]): each one's layers are walked once.
    let (mut rows, mut held) = (HashSet::new(), 0);
    for board in doc.layers.iter().filter(|l| l.artboard().is_some()) {
        let mut inside = HashSet::new();
        add(board, &mut inside);
        let n = selected.iter().filter(|id| inside.contains(id)).count();
        if n > 0 {
            held += n;
            rows.extend(inside);
        }
    }
    if held < selected.len() {
        return HashSet::new();
    }
    rows
}

/// The kinds the Kind filter toggles.
pub const KINDS: [&str; 5] = ["pixel", "adjustment", "type", "shape", "smart"];

/// The Smart Object filter's toggles: smart objects linked to a file, linked to a file that is
/// not there, and embedded in the document.
pub const SMART: [&str; 3] = ["linked", "missing", "embedded"];

/// `ui.set {layerFilter: {on?, by?, kinds?, name?, effect?, mode?, attribute?, color?, smart?}}`:
/// `current` with the given fields replaced, or an error naming the bad field (nothing changes
/// then). `by` is `kind`, `name`, `effect`, `mode`, `attribute`, `color`, `smartObject` or
/// `artboard`; `kinds` a list of [`KINDS`]; `effect` an [`effect::EffectKind`] id; `mode` one of
/// the layer blend modes such as `Multiply` (not Pass Through, which the Mode list doesn't offer,
/// #10878); `attribute` an [`attribute::Attribute`] id; `color` a label colour id (`none`, `red`,
/// … `gray`, [`LabelColor::id`]); `smart` a list of [`SMART`]. Artboard has no criteria of its
/// own: the selected layers pick the artboards.
pub fn patched(current: &Filter, v: &serde_json::Value) -> Result<Filter, String> {
    let Some(o) = v.as_object() else { return Err("layerFilter must be an object".into()) };
    if let Some(k) = o.keys().find(|k| !["on", "by", "kinds", "name", "effect", "mode", "attribute", "color", "smart"].contains(&k.as_str())) {
        return Err(format!("unknown layerFilter field `{k}` (fields: on, by, kinds, name, effect, mode, attribute, color, smart)"));
    }
    let mut f = current.clone();
    if let Some(by) = o.get("by") {
        f.by = serde_json::from_value(by.clone())
            .map_err(|_| "layerFilter.by must be kind, name, effect, mode, attribute, color, smartObject or artboard".to_string())?;
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
        f.mode = photocraft_engine::commands::blend_from_str(name).filter(|m| BlendMode::LAYER_MODES.contains(m)).ok_or_else(|| {
            let modes: Vec<&str> = BlendMode::LAYER_MODES.iter().map(|m| m.label()).collect();
            format!("layerFilter.mode: `{name}` is not a layer blend mode the filter lists (modes: {})", modes.join(", "))
        })?;
    }
    if let Some(e) = o.get("effect") {
        f.effect = effect::parse(e)?;
    }
    if let Some(a) = o.get("attribute") {
        f.attribute = attribute::parse(a)?;
    }
    if let Some(color) = o.get("color") {
        let id = color.as_str().ok_or("layerFilter.color must be a label colour id")?;
        f.color = LabelColor::ALL.into_iter().find(|c| c.id() == id).ok_or_else(|| {
            let ids: Vec<&str> = LabelColor::ALL.iter().map(|c| c.id()).collect();
            format!("layerFilter.color: unknown colour `{id}` (colours: {})", ids.join(", "))
        })?;
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
    f.activate_changed(current);
    if let Some(on) = o.get("on") {
        f.on = on.as_bool().ok_or("layerFilter.on must be a boolean")?;
    }
    Ok(f)
}

/// Store `f` as the Layers panel's filter.
pub fn apply(app: &mut PhotocraftApp, f: Filter) {
    switch::apply(app, f);
}

/// The kind filter's name for `l`'s content ("" for groups, which no kind matches).
fn kind(l: &Layer) -> &'static str {
    match &l.content {
        LayerContent::Raster(_) => "pixel",
        LayerContent::Adjustment(_) => "adjustment",
        LayerContent::Fill(Fill::Solid(_)) if l.vector_mask.is_some() => "shape",
        LayerContent::Fill(_) => "",
        LayerContent::Text(_) => "type",
        LayerContent::Shape(_) => "shape",
        LayerContent::Smart(_) => "smart",
        LayerContent::Group(_) => "",
    }
}

/// Draw the filter row: the type dropdown, then that type's controls. The Layers panel runs
/// [`selection::follow`] after it, also while Filter Options › Hide hides the row.
pub fn bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    switch::bar(app, ui);
}

fn controls(filter: &mut Filter, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let by = [
            (FilterBy::Kind, tl!("Kind")),
            (FilterBy::Name, tl!("Name")),
            (FilterBy::Effect, tl!("Effect")),
            (FilterBy::Mode, tl!("Mode")),
            (FilterBy::Attribute, tl!("Attribute")),
            (FilterBy::Color, tl!("Color")),
            (FilterBy::SmartObject, tl!("Smart Object")),
            (FilterBy::Artboard, tl!("Artboard")),
        ];
        widgets::dropdown(ui, "layer-filter-by", tl!("Kind"), &mut filter.by, &by, 64.0);
        ui.add_space(4.0);
        match filter.by {
            FilterBy::Kind => {
                for (kind, icon, tip) in [
                    ("pixel", "image", tl!("Filter for pixel layers")),
                    ("adjustment", "adjustment-layer", tl!("Filter for adjustment layers")),
                    ("type", "type", tl!("Filter for type layers")),
                    ("shape", "square", tl!("Filter for shape layers")),
                    ("smart", "app-window", tl!("Filter for smart objects")),
                ] {
                    let on = filter.kinds.iter().any(|k| k == kind);
                    if icons::button(ui, icon, 22.0, on, tip).clicked() {
                        if on {
                            filter.kinds.retain(|k| k != kind);
                        } else {
                            filter.kinds.push(kind.to_string());
                        }
                    }
                }
            }
            FilterBy::Name => {
                let width = ui.available_width().max(48.0);
                widgets::text_field(ui, ui.scope_id().with("layer-filter-name"), &mut filter.name, tl!("Search"), width, false);
            }
            FilterBy::Effect => effect::controls(&mut filter.effect, ui),
            FilterBy::Mode => {
                let modes: Vec<(BlendMode, &str)> = BlendMode::LAYER_MODES.into_iter().map(|m| (m, m.label())).collect();
                let width = ui.available_width().max(96.0);
                widgets::dropdown(ui, "layer-filter-mode", tl!("Mode"), &mut filter.mode, &modes, width);
            }
            FilterBy::Attribute => attribute::controls(&mut filter.attribute, ui),
            FilterBy::Color => {
                // Photoshop's list: None, then the label colours in the Layers panel menu's order.
                let colors: Vec<(LabelColor, &str)> =
                    LabelColor::ALL.into_iter().map(|c| (c, if c == LabelColor::None { tl!("None") } else { tl!(c.label()) })).collect();
                let width = ui.available_width().max(96.0);
                widgets::dropdown(ui, "layer-filter-color", tl!("Color"), &mut filter.color, &colors, width);
            }
            FilterBy::SmartObject => {
                for (kind, icon, tip) in [
                    ("linked", "link", tl!("Filter for smart objects linked locally")),
                    ("missing", "unlink", tl!("Filter for linked smart objects that are missing")),
                    ("embedded", "file", tl!("Filter for embedded smart objects")),
                ] {
                    let on = filter.smart.iter().any(|k| k == kind);
                    if icons::button(ui, icon, 22.0, on, tip).clicked() {
                        if on {
                            filter.smart.retain(|k| k != kind);
                        } else {
                            filter.smart.push(kind.to_string());
                        }
                    }
                }
            }
            // Artboard has no controls: the selected layers pick the artboards ([`board_rows`]).
            FilterBy::Artboard => {}
        }
    });
}

pub mod effect;

#[cfg(test)]
mod effect_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod switch_tests;

pub mod attribute;
#[cfg(test)]
mod attribute_tests;
#[cfg(test)]
mod kind_mode_tests;
pub mod selection;
#[cfg(test)]
mod selection_tests;
