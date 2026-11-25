//! The Layers panel's view of the session for craft-layers' `LayersPanel`: `PhotoLayers` (the
//! `LayerTree` over the active document), `PhotoThumbs` (its thumbnails), the settings and filter
//! conversions, and the row menus as entries.

use photocraft_engine::link_watch::LinkWatchExt;
use std::collections::HashSet;
use std::sync::Arc;

use craft::layers::ui::filter::{EffectKind, FilterBy, FilterKind, LayerAttribute, LayerFilter, SmartKind, SmartSource};
use craft::layers::ui::panel::MenuTarget;
use craft::layers::ui::thumbs::{LayerThumb, LayerThumbKind, ThumbContents};
use craft::layers::ui::tree::{
    LayerInfo, LayerKind, LayerLocks, LayerPart, LayerProperties, LayerRowKey, LayerSelection, LayerTree, LayersFooterState, MaskInfo, SubRow, SubRowGroup,
    SubRows, ThumbBadge,
};
use craft::theme::LabelColor as UiLabel;
use craft::ui::contract::{Entry, MemoProps, PlainData, ReorderMove, Tracked};
use craft::ui::list::ThumbSize;
use craft::ui::patterns::blend_keys::{BlendKeyEntry, blend_entries};
use craft::ui::primitives::textures::{Revision, ThumbRender, ThumbRequest, ThumbSource, thumb_image};
use craft::ui::runtime::env::Label;
use photocraft_color::BlendMode;
use photocraft_doc::{Document, Effect, Fill, LabelColor, Layer, LayerContent, LayerId};
use photocraft_engine::DocState;
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The blend mode a panel key names.
pub(crate) fn blend_of(key: &str) -> Option<BlendMode> {
    photocraft_engine::commands::parse_blend(key)
}

/// A document label colour as the framework's (`None` for none), matched by name.
pub(crate) fn ui_label(c: LabelColor) -> Option<UiLabel> {
    Some(match c {
        LabelColor::None => return None,
        LabelColor::Red => UiLabel::Red,
        LabelColor::Orange => UiLabel::Orange,
        LabelColor::Yellow => UiLabel::Yellow,
        LabelColor::Green => UiLabel::Green,
        LabelColor::Blue => UiLabel::Blue,
        LabelColor::Violet => UiLabel::Violet,
        LabelColor::Gray => UiLabel::Gray,
        LabelColor::Seafoam => UiLabel::Seafoam,
        LabelColor::Indigo => UiLabel::Indigo,
        LabelColor::Magenta => UiLabel::Magenta,
        LabelColor::Fuchsia => UiLabel::Fuchsia,
    })
}

/// The document label colour of a framework one.
pub(crate) fn doc_label(c: Option<UiLabel>) -> LabelColor {
    LabelColor::ALL.into_iter().find(|l| ui_label(*l) == c).unwrap_or(LabelColor::None)
}

fn effect_kind(e: &Effect) -> Option<EffectKind> {
    Some(match e {
        Effect::BevelEmboss(_) => EffectKind::BevelEmboss,
        Effect::Stroke(_) => EffectKind::Stroke,
        Effect::InnerShadow(_) => EffectKind::InnerShadow,
        Effect::InnerGlow(_) => EffectKind::InnerGlow,
        Effect::Satin(_) => EffectKind::Satin,
        Effect::ColorOverlay { .. } | Effect::GradientOverlay { .. } | Effect::PatternOverlay { .. } => EffectKind::Overlay,
        Effect::OuterGlow(_) => EffectKind::OuterGlow,
        Effect::DropShadow(_) => EffectKind::DropShadow,
        #[allow(unreachable_patterns)]
        _ => return None,
    })
}

/// The Smart Filter row selected on its own, while its smart object is active and the row is listed in the Layers
/// panel in front: a row nobody sees highlighted (the panel closed, minimized or behind another tab) is not selected.
pub(crate) fn listed_filter(app: &PhotocraftApp) -> Option<crate::smart_ui::FilterDrag> {
    if !super::panel_docking::is_front(app, "layers") {
        return None;
    }
    let st = app.session.active()?;
    let picked = crate::smart_ui::selected(&app.smart_filter_picks, &st.doc, st.active_layer);
    PhotoLayers::new(app, st, &app.ui.layer_filter, false, picked).selected_filter()
}

/// What the Layers panel's model is built from ([`PhotoLayers::new`]): the document as the session tracks it, whose
/// revision moves with every change to it, view state included, and the app's answers for its rows.
#[derive(MemoProps)]
struct LayersFacts<'a> {
    doc: &'a Tracked<DocState>,
    filter: &'a LayerFilter,
    #[memo_props(hash = "craft-layers part: a fieldless enum")]
    target: LayerPart,
    mask_target: bool,
    vector_mask_target: bool,
    link_enabled: bool,
    #[memo_props(hash = "a plain model id")]
    hovered: Option<LayerId>,
    working_text: Option<u64>,
    /// The missing linked files of the open documents (the Smart Object filter).
    missing: craft::ui::contract::Revision,
    foreign_drag: bool,
    picked: Option<crate::smart_ui::FilterDrag>,
}

/// The id the Layers panel's model is derived under.
pub(crate) fn model_id() -> egui::Id {
    egui::Id::unique("photocraft-layers-model")
}

/// The value the Layers panel's model is derived as: `None` while no document is open.
pub(crate) type Model = Option<Tracked<PhotoLayers>>;

/// The Layers panel's model of the active document `st`, derived from what it is built from: built again only when
/// one of [`LayersFacts`] changed. A new build is a new `Tracked`, so the panel's key moves exactly then. It is always
/// `Some`.
pub(crate) fn model(
    app: &PhotocraftApp,
    st: &Tracked<DocState>,
    filter: &LayerFilter,
    foreign_drag: bool,
    picked: Option<crate::smart_ui::FilterDrag>,
) -> Arc<Model> {
    let facts = LayersFacts {
        doc: st,
        filter,
        target: edit_target(app, st),
        mask_target: app.ui.mask_target,
        vector_mask_target: app.ui.vector_mask_target,
        link_enabled: app.session.is_enabled("layer.linkLayers"),
        hovered: app.canvas_hover_layer,
        working_text: working_text(app),
        missing: app.session.missing_links_revision(),
        foreign_drag,
        picked,
    };
    craft::ui::runtime::derived::derived(app.ctx(), model_id(), &facts, || Some(Tracked::new(PhotoLayers::new(app, st, filter, foreign_drag, picked))))
}

/// Derives the model as `None` while no document is open: the kept model, which holds the document it showed, is
/// replaced in the pass that finds the document gone.
pub(crate) fn no_model(app: &PhotocraftApp) {
    let _ = craft::ui::runtime::derived::derived(app.ctx(), model_id(), &(), || Model::None);
}

/// A Type Mask session's working text, which the panel never lists.
fn working_text(app: &PhotocraftApp) -> Option<u64> {
    app.ui.text_edit.as_ref().filter(|ed| ed.mask).map(|ed| ed.layer)
}

/// The part of the active layer edits go to ([`LayerTree::selection`]).
fn edit_target(app: &PhotocraftApp, st: &DocState) -> LayerPart {
    let ui = &app.ui;
    match st.active_layer.and_then(|id| st.doc.layer(id)) {
        Some(l) if l.frame.is_some() => match crate::frame_tool::target(app, l) {
            crate::frame_tool::Target::Frame => LayerPart::FrameShape,
            crate::frame_tool::Target::Content => LayerPart::FrameContent,
            crate::frame_tool::Target::Both => LayerPart::Pixels,
        },
        Some(l) if ui.filter_mask_target == Some(l.id.0) && app.filter_mask_targeted() => LayerPart::FilterMask,
        Some(l) if ui.vector_mask_target && l.vector_mask.is_some() => LayerPart::VectorMask,
        Some(l) if ui.mask_target && l.mask.is_some() => LayerPart::LayerMask,
        _ => LayerPart::Pixels,
    }
}

/// The Layers panel's model of the active document: the document, the parts of its state the rows show and what the
/// app decides for them (the edit target, the hovered layer, the filter's answers). Plain data: the panel holds it in a
/// `Tracked`, derived from [`LayersFacts`] ([`model`]).
#[derive(Clone, PlainData)]
pub(crate) struct PhotoLayers {
    doc: Arc<Document>,
    active_layer: Option<LayerId>,
    selected: Vec<LayerId>,
    isolated_layers: Vec<LayerId>,
    fx_collapsed: Vec<LayerId>,
    #[plain_data(assume = "craft-layers filter: plain values and strings")]
    filter: LayerFilter,
    /// The part edits go to ([`LayerTree::selection`]).
    target: LayerPart,
    mask_target: bool,
    vector_mask_target: bool,
    link_enabled: bool,
    hovered: Option<LayerId>,
    /// A Type Mask session's working text, which the panel never lists.
    working_text: Option<u64>,
    /// The empty layers, while the filter is Attribute Empty or Not Empty.
    empty: Option<Arc<HashSet<LayerId>>>,
    /// The missing linked files, while the filter is Smart Object.
    missing: HashSet<String>,
    /// A layer drag from another document (a transfer) is under way: rows take no drop.
    foreign_drag: bool,
    /// The Smart Filter row selected on its own, as stored ([`crate::smart_ui::selected`]).
    picked: Option<crate::smart_ui::FilterDrag>,
}

impl PhotoLayers {
    pub(crate) fn new(app: &PhotocraftApp, st: &DocState, filter: &LayerFilter, foreign_drag: bool, picked: Option<crate::smart_ui::FilterDrag>) -> Self {
        let empty = match (filter.by, filter.attribute) {
            (FilterBy::Attribute, LayerAttribute::Empty | LayerAttribute::NotEmpty) => crate::layer_filter_ui::attribute::empty_layers(app),
            _ => None,
        };
        let missing = if filter.by == FilterBy::SmartObject { app.session.missing_links() } else { HashSet::new() };
        let ui = &app.ui;
        let target = edit_target(app, st);
        Self {
            doc: Arc::clone(&st.doc),
            active_layer: st.active_layer,
            selected: st.selected_layers().to_vec(),
            isolated_layers: st.isolated_layers.clone(),
            fx_collapsed: st.fx_collapsed.clone(),
            filter: ui.layer_filter.clone(),
            target,
            mask_target: ui.mask_target,
            vector_mask_target: ui.vector_mask_target,
            link_enabled: app.session.is_enabled("layer.linkLayers"),
            hovered: app.canvas_hover_layer,
            working_text: working_text(app),
            empty,
            missing,
            foreign_drag,
            picked,
        }
    }

    fn doc(&self) -> &Document {
        &self.doc
    }

    fn layer(&self, id: &LayerId) -> Option<&Layer> {
        self.doc.layer(*id)
    }

    /// The layers a property edit changes: every selected layer with the active one among them,
    /// the Background left out of a several-layer change (#5856), else the active one.
    pub(crate) fn targets(&self) -> Vec<LayerId> {
        let Some(active) = self.active_layer else { return Vec::new() };
        let selection = self.selected.clone();
        if selection.len() > 1 && selection.contains(&active) {
            selection.iter().copied().filter(|id| *id == active || !self.doc().is_background(*id)).collect()
        } else {
            vec![active]
        }
    }

    /// The selected Smart Filter row, while its smart object is the active layer and its row is
    /// listed: Delete removes it.
    pub(crate) fn selected_filter(&self) -> Option<crate::smart_ui::FilterDrag> {
        let picked = self.picked?;
        let layer = LayerId(picked.0);
        let mut at = Some(layer);
        while let Some(id) = at {
            if !photocraft_engine::select_extra_cmds::isolation_shows(self.doc(), &self.isolated_layers, id) {
                return None;
            }
            at = self.parent(&id);
            if at.is_some_and(|p| !self.is_open(&p)) {
                return None;
            }
        }
        let filter = &self.filter;
        (self.effects_open(&layer) && (!filter.active() || self.listed(&layer, filter))).then_some(picked)
    }

    fn smart_filter_count(&self, id: &LayerId) -> usize {
        smart_filter_count(self.doc(), id)
    }
}

fn smart_filter_count(doc: &Document, id: &LayerId) -> usize {
    match doc.layer(*id).map(|l| &l.content) {
        Some(LayerContent::Smart(sm)) => sm.smart_filters.len(),
        _ => 0,
    }
}

/// The document index of the Smart Filter shown `row`-th from the top of `id`'s rows (the stack
/// lists the last filter first); the same conversion turns an index back into its row.
pub(crate) fn filter_index(doc: &Document, id: &LayerId, row: usize) -> Option<usize> {
    smart_filter_count(doc, id).checked_sub(1)?.checked_sub(row)
}

impl LayerTree for PhotoLayers {
    type Id = LayerId;

    fn children(&self, parent: Option<&LayerId>) -> Vec<LayerId> {
        let list: &[Layer] = match parent {
            None => &self.doc().layers,
            Some(id) => self.layer(id).filter(|l| l.frame.is_none()).and_then(Layer::children).unwrap_or_default(),
        };
        list.iter()
            .rev()
            .filter(|l| Some(l.id.0) != self.working_text)
            .filter(|l| photocraft_engine::select_extra_cmds::isolation_shows(self.doc(), &self.isolated_layers, l.id))
            .map(|l| l.id)
            .collect()
    }

    fn parent(&self, id: &LayerId) -> Option<LayerId> {
        self.doc().layers.index().parent(*id)
    }

    fn is_open(&self, id: &LayerId) -> bool {
        matches!(self.layer(id).map(|l| &l.content), Some(LayerContent::Group(g)) if g.expanded)
    }

    fn effects_open(&self, id: &LayerId) -> bool {
        !self.fx_collapsed.contains(id)
    }

    fn info(&self, id: &LayerId) -> LayerInfo {
        let Some(l) = self.layer(id) else { return LayerInfo::new(String::new(), LayerKind::Pixel) };
        let doc = self.doc();
        let kind = if l.frame.is_some() {
            LayerKind::Frame
        } else if l.artboard().is_some() {
            LayerKind::Artboard
        } else {
            match &l.content {
                LayerContent::Raster(_) => LayerKind::Pixel,
                LayerContent::Group(_) => LayerKind::Group,
                LayerContent::Adjustment(_) => LayerKind::Adjustment,
                LayerContent::Fill(_) => LayerKind::Fill,
                LayerContent::Text(_) => LayerKind::Text,
                LayerContent::Shape(_) => LayerKind::Shape,
                LayerContent::Smart(_) => LayerKind::SmartObject,
            }
        };
        let mut info = LayerInfo::new(l.name.clone(), kind);
        info.visible = l.visible;
        let index = doc.layers.index();
        info.dimmed = !l.visible || std::iter::successors(index.parent(*id), |p| index.parent(*p)).any(|p| self.layer(&p).is_some_and(|g| !g.visible));
        info.label = ui_label(l.label);
        let mut locks = LayerLocks::default();
        locks.transparency = l.locks.transparency;
        locks.pixels = l.locks.pixels;
        locks.position = l.locks.position;
        locks.artboard = l.locks.artboard;
        locks.all = l.locks.all;
        info.locks = locks;
        info.background = doc.is_background(l.id);
        info.renamable = !info.background;
        info.clipped = l.clipped;
        info.linked = l.link_group.is_some();
        info.blend_key = l.blend.key().to_string();
        info.blend_label = (!matches!(l.blend, BlendMode::Normal | BlendMode::PassThrough)).then(|| tl!(l.blend.label()).to_string());
        info.effect_kinds = l.effects.items.iter().filter(|e| e.enabled()).filter_map(effect_kind).collect();
        info.advanced_blending = l.excluded_channels != 0 || l.advanced != Default::default() || !l.blend_if.is_default();
        if let LayerContent::Smart(sm) = &l.content {
            info.smart = Some(match sm.source {
                photocraft_doc::SmartSource::Linked { .. } => SmartSource::Linked,
                _ => SmartSource::Embedded,
            });
            info.has_smart_filters = !sm.smart_filters.is_empty();
            info.smart_filters_enabled = sm.filters_enabled;
        }
        // Smart objects and shape layers show their pixels as any layer does; only the badge tells them apart (#4584).
        info.thumb_badge = match l.content {
            LayerContent::Smart(_) => Some(ThumbBadge::SmartObject),
            LayerContent::Shape(_) => Some(ThumbBadge::Shape),
            _ => None,
        };
        if let Some(m) = &l.mask {
            info.masks.push(MaskInfo::new(LayerPart::LayerMask, m.enabled, m.linked));
        }
        if let Some(m) = &l.vector_mask {
            info.masks.push(MaskInfo::new(LayerPart::VectorMask, m.enabled, m.linked));
        }
        info.has_effects = !l.effects.items.is_empty();
        info.warning = crate::layer_row_ui::unsupported_key(l)
            .map(|key| crate::i18n::fmt(tl!("Not rendered: this adjustment ({key}) is kept as imported."), &[("key", key)]));
        info.thumb_loads_selection = !info.background;
        info
    }

    fn listed(&self, id: &LayerId, filter: &LayerFilter) -> bool {
        let Some(l) = self.layer(id) else { return false };
        match filter.by {
            // A fill lists under no Kind button, except a solid colour with a vector mask: a shape.
            FilterBy::Kind if matches!(l.content, LayerContent::Fill(_)) => {
                matches!(l.content, LayerContent::Fill(Fill::Solid(_))) && l.vector_mask.is_some() && filter.kinds.contains(&FilterKind::Shape)
            }
            FilterBy::Attribute if matches!(filter.attribute, LayerAttribute::Empty | LayerAttribute::NotEmpty) => {
                let empty = self.empty.as_ref().is_some_and(|set| set.contains(id));
                empty == (filter.attribute == LayerAttribute::Empty)
            }
            FilterBy::SmartObject => match &l.content {
                LayerContent::Smart(sm) => filter.smart.iter().any(|k| match (k, &sm.source) {
                    (SmartKind::Linked, photocraft_doc::SmartSource::Linked { .. }) => true,
                    (SmartKind::Missing, photocraft_doc::SmartSource::Linked { path }) => self.missing.contains(path),
                    (SmartKind::Embedded, photocraft_doc::SmartSource::Embedded { .. }) => true,
                    _ => false,
                }),
                _ => false,
            },
            _ => filter.lists(self, id),
        }
    }

    fn selection(&self) -> LayerSelection<LayerId> {
        let mut selection = LayerSelection::new(self.selected.clone(), self.active_layer, self.target);
        selection.sub_row = self.selected_filter().and_then(|(layer, index)| {
            let id = LayerId(layer);
            let row = match index {
                Some(i) => Some(self.smart_filter_count(&id).checked_sub(1)?.checked_sub(i)?),
                None => None,
            };
            Some(LayerRowKey::Filters(id, row))
        });
        selection
    }

    fn sub_rows(&self, id: &LayerId, group: SubRowGroup) -> Option<SubRows> {
        let l = self.layer(id)?;
        match group {
            SubRowGroup::Effects => {
                if l.effects.items.is_empty() {
                    return None;
                }
                let mut header = SubRow::new(tl!("Effects"));
                header.on = l.effects.enabled;
                let items = l
                    .effects
                    .items
                    .iter()
                    .map(|e| {
                        let mut row = SubRow::new(tl!(e.label()));
                        row.on = e.enabled();
                        row
                    })
                    .collect();
                Some(SubRows::new(header, items))
            }
            SubRowGroup::SmartFilters => {
                let LayerContent::Smart(sm) = &l.content else { return None };
                if sm.smart_filters.is_empty() {
                    return None;
                }
                let mut header = SubRow::new(tl!("Smart Filters"));
                header.on = sm.filters_enabled;
                header.mask = true;
                let items = sm
                    .smart_filters
                    .iter()
                    .rev()
                    .map(|f| {
                        let mut row = SubRow::new(crate::smart_ui::filter_label(f));
                        row.on = f.visible;
                        row.options = true;
                        row
                    })
                    .collect();
                Some(SubRows::new(header, items))
            }
            _ => None,
        }
    }

    fn properties(&self) -> Option<LayerProperties> {
        let l = self.active_layer.and_then(|id| self.layer(&id))?;
        let mut p = LayerProperties::new(l.blend.key().to_string());
        // Pass Through is a group's mode: offered only when every target is a group.
        p.group = self.targets().iter().all(|id| self.layer(id).is_some_and(Layer::is_group));
        p.opacity = f64::from(l.opacity) * 100.0;
        p.fill = f64::from(l.fill_opacity) * 100.0;
        p.locks = self.info(&l.id).locks;
        p.editable = !self.doc().is_background(l.id);
        Some(p)
    }

    fn blend_modes(&self, group: bool, current: &str) -> Vec<Entry<String>> {
        static CACHE: crate::i18n::LangCache<(bool, BlendMode), Vec<Entry<String>>> = crate::i18n::LangCache::new();
        let current = BlendMode::from_key(current).or_else(|| blend_of(current)).unwrap_or(BlendMode::Normal);
        let current = if current.has_psd_equivalent() { BlendMode::Normal } else { current };
        CACHE.get_or_build((group, current), || {
            let modes: Vec<BlendKeyEntry<BlendMode>> =
                super::layers::blend_options(group, current).into_iter().map(|(m, _)| super::blend_entry(m, m)).collect();
            blend_entries(&modes, |mode| Label::from(tl!(mode.key.label()).to_string()))
                .into_iter()
                .map(|entry| match entry.value() {
                    Some(mode) => Entry::new(mode.key().to_string(), entry.label().clone()),
                    None => Entry::separator(),
                })
                .collect()
        })
    }

    fn footer(&self) -> LayersFooterState {
        let active = self.active_layer.and_then(|id| self.layer(&id));
        let delete = if self.vector_mask_target && active.is_some_and(|l| l.vector_mask.is_some()) {
            "Delete Vector Mask"
        } else if self.mask_target && active.is_some_and(|l| l.mask.is_some()) {
            "Delete Layer Mask"
        } else {
            "Delete layer"
        };
        let mut footer = LayersFooterState::new(tl!(delete).to_string());
        footer.delete_enabled = active.is_some();
        footer.link_enabled = self.link_enabled;
        footer.mask_label = crate::layer_menu_ui::mask_button_command(active, self.doc().selection.is_some(), false).map(|_| {
            if active.is_some_and(|l| l.mask.is_some()) {
                tl!("Add vector mask").to_string()
            } else {
                crate::i18n::fmt(tl!("Add a mask  (from the selection; {key} inverts)"), &[("key", &crate::shortcuts::pretty("Alt"))])
            }
        });
        type FooterMenus = (Vec<Entry<String>>, Vec<Entry<String>>);
        static MENUS: crate::i18n::LangCache<(), FooterMenus> = crate::i18n::LangCache::new();
        let (adjustments, styles) = MENUS.get_or_build((), || {
            let mut adjustments: Vec<Entry<String>> = photocraft_engine::command_specs()
                .iter()
                .filter(|c| c.id.starts_with("layer.newAdjustmentLayer."))
                .map(|c| Entry::new(c.id.to_string(), tl!(c.label).trim_end_matches('…').to_string()))
                .collect();
            adjustments.push(Entry::separator());
            adjustments.push(Entry::new("layer.newFillLayer.solidColor".to_string(), tl!("Solid Color…").to_string()));
            adjustments.push(Entry::new("layer.newFillLayer.gradient".to_string(), tl!("Gradient…").to_string()));
            adjustments.push(Entry::new("layer.newFillLayer.pattern".to_string(), tl!("Pattern…").to_string()));
            let mut styles = vec![Entry::new(String::new(), tl!("Blending Options…").to_string()), Entry::separator()];
            styles.extend(crate::layer_style::KINDS.iter().map(|&(kind, label)| Entry::new(kind.to_string(), format!("{}…", tl!(label)))));
            (adjustments, styles)
        });
        footer.adjustment_menu = adjustments;
        footer.style_menu = styles;
        footer
    }

    fn accepts_drop(&self, mv: &ReorderMove<LayerRowKey<LayerId>>, _mods: craft::keymap::Mods) -> bool {
        if self.foreign_drag {
            return false;
        }
        let doc = self.doc();
        let layers: Vec<LayerId> = mv.keys.iter().filter_map(|k| if let LayerRowKey::Layer(id) = k { Some(*id) } else { None }).collect();
        if let Some(LayerRowKey::Layer(parent)) = &mv.parent
            && self.layer(parent).is_some_and(|l| l.frame.is_some())
        {
            // Into a frame: one pixel layer or smart object becomes its content.
            return matches!(layers.as_slice(), [one] if self.layer(one).is_some_and(|l| matches!(l.content, LayerContent::Raster(_) | LayerContent::Smart(_))));
        }
        // Nothing moves the Background off the bottom or lands below it; Alt still copies it above.
        let Some(bg) = doc.layers.first().filter(|b| doc.is_background(b.id)).map(|b| b.id) else { return true };
        if !mv.copy && layers.contains(&bg) {
            return false;
        }
        if mv.parent.is_none() {
            let siblings = self.children(None).into_iter().filter(|id| !layers.contains(id)).count();
            return mv.to < siblings || (mv.copy && layers.contains(&bg));
        }
        true
    }

    fn accepts_mask_drop(&self, from: &LayerId, part: LayerPart, to: &LayerId) -> bool {
        // The engine moves layer masks only.
        part == LayerPart::LayerMask && from != to && self.layer(to).is_some_and(|l| l.mask.is_none())
    }

    fn label_colors_in_menu(&self, target: &MenuTarget<LayerId>) -> bool {
        matches!(menu_kind(self, target), MenuKind::Layer(_))
    }

    fn hovered(&self) -> Option<LayerId> {
        self.hovered
    }
}

/// Which menu a right-click at a target opens.
enum MenuKind {
    /// The empty space: thumbnail size and clipping.
    Empty,
    /// A pixel layer's thumbnail: Select Pixels, then thumbnail size and clipping.
    Thumbnail,
    /// A layer mask's thumbnail.
    Mask(LayerId),
    /// An effects row.
    Effects,
    /// The layer menu.
    Layer(LayerId),
    /// No menu.
    None,
}

fn menu_kind(tree: &PhotoLayers, target: &MenuTarget<LayerId>) -> MenuKind {
    match &target.row {
        None => MenuKind::Empty,
        Some(LayerRowKey::Effects(..)) => MenuKind::Effects,
        Some(LayerRowKey::Layer(id)) => {
            let Some(l) = tree.layer(id) else { return MenuKind::None };
            match target.part {
                Some(LayerPart::LayerMask) if l.mask.is_some() => MenuKind::Mask(*id),
                // A pixel layer's thumbnail; not the Background's, which has no transparency to load.
                Some(LayerPart::Pixels) if matches!(l.content, LayerContent::Raster(_)) && l.frame.is_none() && !tree.doc().is_background(*id) => {
                    MenuKind::Thumbnail
                }
                Some(LayerPart::LayerMask | LayerPart::VectorMask | LayerPart::FilterMask) => MenuKind::None,
                _ => MenuKind::Layer(*id),
            }
        }
        _ => MenuKind::None,
    }
}

/// The four thumbnail sizes, in menu order.
fn thumbnail_sizes() -> [(ThumbSize, &'static str); 4] {
    [
        (ThumbSize::None, tl!("No Thumbnails")),
        (ThumbSize::Small, tl!("Small Thumbnails")),
        (ThumbSize::Medium, tl!("Medium Thumbnails")),
        (ThumbSize::Large, tl!("Large Thumbnails")),
    ]
}

/// Menu values that are no engine command: the panel handles them (`layers_events`).
pub(crate) const RENAME: &str = "layersPanel:rename";
pub(crate) const THUMB_SIZE: &str = "layersPanel:thumbs:";
pub(crate) const CLIP_THUMBS: &str = "layersPanel:clip:";
pub(crate) const SELECT_PIXELS: &str = "layersPanel:selectPixels";
pub(crate) const LOAD_MASK: &str = "layersPanel:loadMask:";
pub(crate) const MASK_REFINE: &str = "layersPanel:selectAndMask";
pub(crate) const MASK_OPTIONS: &str = "layersPanel:maskOptions";

/// The row menu at `target` as entries: the layer menu (Photoshop's order, trimmed to the kind),
/// a mask thumbnail's menu, a pixel thumbnail's, an effects row's, or the empty space's.
pub(crate) fn row_menu(app: &PhotocraftApp, tree: &PhotoLayers, target: &MenuTarget<LayerId>) -> Vec<Entry<String>> {
    let has_selection = tree.doc().selection.is_some();
    let commands = |entries: Vec<crate::layer_menu_ui::Entry>, on_set: bool| -> Vec<Entry<String>> {
        let mut out: Vec<Entry<String>> = Vec::new();
        for e in entries {
            match e {
                None => {
                    if out.last().is_some_and(Entry::is_item) {
                        out.push(Entry::separator());
                    }
                }
                Some((label, id)) => {
                    if photocraft_engine::commands::find(id).is_none() && !crate::menu_catalog::CATALOG.iter().any(|m| m.3 == id) {
                        continue;
                    }
                    let enabled = !(on_set && crate::layer_menu_ui::single_layer_only(id)) && crate::menus::is_enabled(app, id);
                    out.push(Entry::new(id.to_string(), tl!(label).to_string()).enabled(enabled));
                }
            }
        }
        while out.last().is_some_and(Entry::is_separator) {
            out.pop();
        }
        out
    };
    let thumbnail_items = |out: &mut Vec<Entry<String>>| {
        let current = app.ui.layers_settings.options.thumbnail_size;
        for (size, label) in thumbnail_sizes() {
            let value = crate::layers_panel_ui::size_key(size);
            out.push(Entry::new(format!("{THUMB_SIZE}{value}"), label.to_string()).checked(current == size));
        }
        out.push(Entry::separator());
        let clip = app.session.prefs().interface.clip_thumbnails_to_layer_bounds;
        for (to_layer, label) in [(true, tl!("Clip Thumbnails to Layer Bounds")), (false, tl!("Clip Thumbnails to Document Bounds"))] {
            out.push(Entry::new(format!("{CLIP_THUMBS}{to_layer}"), label.to_string()).checked(clip == to_layer));
        }
    };
    match menu_kind(tree, target) {
        MenuKind::Empty => {
            let mut out = Vec::new();
            thumbnail_items(&mut out);
            out
        }
        MenuKind::Thumbnail => {
            let mut out = vec![Entry::new(SELECT_PIXELS.to_string(), tl!("Select Pixels").to_string()), Entry::separator()];
            thumbnail_items(&mut out);
            out
        }
        MenuKind::Mask(id) => {
            let Some(mask) = tree.layer(&id).and_then(|l| l.mask.as_ref()) else { return Vec::new() };
            let mut out = commands(
                vec![
                    Some((crate::layer_menu_ui::mask_toggle_label(mask.enabled), "layer.layerMask.enabled")),
                    Some((tl!("Delete Layer Mask"), "layer.layerMask.delete")),
                    Some((tl!("Apply Layer Mask"), "layer.layerMask.apply")),
                ],
                false,
            );
            out.push(Entry::separator());
            for (label, op) in
                [(tl!("Add Mask to Selection"), "add"), (tl!("Subtract Mask from Selection"), "subtract"), (tl!("Intersect Mask with Selection"), "intersect")]
            {
                out.push(Entry::new(format!("{LOAD_MASK}{op}"), label.to_string()));
            }
            out.push(Entry::separator());
            out.push(Entry::new(MASK_REFINE.to_string(), tl!("Select and Mask…").to_string()).enabled(crate::menus::is_enabled(app, "select.selectAndMask")));
            out.push(Entry::new(MASK_OPTIONS.to_string(), tl!("Mask Options…").to_string()));
            out
        }
        MenuKind::Effects => commands(crate::layer_menu_ui::effects_entries(), false),
        MenuKind::Layer(id) => {
            let Some(l) = tree.layer(&id) else { return Vec::new() };
            let mut out = commands(crate::layer_menu_ui::entries(l, target.on_selection, has_selection), target.on_selection);
            out.push(Entry::separator());
            out.push(Entry::new(RENAME.to_string(), tl!("Rename Layer…").to_string()));
            out
        }
        MenuKind::None => Vec::new(),
    }
}

/// The thumbnails of the active document's rows.
pub(crate) struct PhotoThumbs<'a> {
    app: &'a PhotocraftApp,
    doc: &'a Document,
}

impl<'a> PhotoThumbs<'a> {
    pub(crate) fn new(app: &'a PhotocraftApp, doc: &'a Document) -> Self {
        Self { app, doc }
    }

    /// The layer whose pixels a key shows: a frame's content is its first child.
    fn source(&self, key: &LayerThumb<LayerId>) -> Option<&'a Layer> {
        let l = self.doc.layer(key.layer)?;
        match key.kind {
            LayerThumbKind::FrameContent => l.children().and_then(|c| c.first()),
            _ => Some(l),
        }
    }
}

fn rgba8(c: [f32; 4]) -> [u8; 4] {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
}

impl ThumbSource<LayerThumb<LayerId>> for PhotoThumbs<'_> {
    fn revision(&self, key: &LayerThumb<LayerId>, _request: &ThumbRequest) -> Revision {
        let Some(l) = self.source(key) else { return Revision::new(0) };
        let clip = key.contents == ThumbContents::LayerBounds;
        let generation = |s: &photocraft_raster::Surface| s.generation();
        let shown = match key.kind {
            LayerThumbKind::Pixels | LayerThumbKind::FrameContent => match &l.content {
                LayerContent::Fill(Fill::Solid(color)) => {
                    let shown = crate::solid_fill_ui::thumbnail_color(self.app, self.doc.id, l.id, *color);
                    Revision::of(&(shown.to_rgba8(), 1u8)).get()
                }
                LayerContent::Fill(f) => Revision::of(f).get(),
                _ => l.surface().map_or(0, generation),
            },
            LayerThumbKind::Mask => l.mask.as_ref().map_or(0, |m| m.surface.generation()),
            LayerThumbKind::VectorMask => l.vector_mask.as_ref().map_or(0, crate::mask_thumbs_ui::vector_fingerprint),
            LayerThumbKind::SmartFilterMask => match &l.content {
                LayerContent::Smart(sm) => sm.filter_mask.as_ref().map_or(1, |m| m.surface.generation()),
                _ => 0,
            },
            _ => 0,
        };
        // A tile that fails to read shows zeros, and its pixels again once it reads, without moving its surface's
        // generation: the scratch read-error generation moves with both (#16252).
        let scratch = crate::scratch_reads::generation();
        Revision::of(&(shown, clip, self.doc.size.width, self.doc.size.height, scratch))
    }

    fn render(&self, key: &LayerThumb<LayerId>, request: &ThumbRequest) -> ThumbRender {
        let Some(l) = self.source(key) else { return ThumbRender::Blank };
        let canvas = self.doc.bounds();
        let size = |r: photocraft_geom::Rect| craft::behavior::PixelSize::new(r.width().max(1), r.height().max(1));
        let gray = |surface: &photocraft_raster::Surface| {
            let mut v = [0.0f32; 1];
            thumb_image(size(canvas), request.side, |x, y| {
                surface.read_pixel(canvas.x0 + x as i32, canvas.y0 + y as i32, &mut v);
                let g = (v[0].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                [g, g, g, 255]
            })
        };
        match key.kind {
            LayerThumbKind::Mask => l.mask.as_ref().map_or(ThumbRender::Blank, |m| ThumbRender::Ready(gray(&m.surface))),
            LayerThumbKind::SmartFilterMask => match &l.content {
                LayerContent::Smart(sm) => match &sm.filter_mask {
                    Some(m) => ThumbRender::Ready(gray(&m.surface)),
                    None => ThumbRender::Ready(thumb_image(size(canvas), request.side, |_, _| [255, 255, 255, 255])),
                },
                _ => ThumbRender::Blank,
            },
            LayerThumbKind::VectorMask => l
                .vector_mask
                .as_ref()
                .map_or(ThumbRender::Blank, |m| ThumbRender::Ready(crate::mask_thumbs_ui::vector_thumb_image(self.doc, m, request.side as usize))),
            _ => match &l.content {
                LayerContent::Group(_) | LayerContent::Adjustment(_) | LayerContent::Text(_) if key.kind == LayerThumbKind::Pixels => ThumbRender::Blank,
                LayerContent::Fill(Fill::Solid(color)) => {
                    let rgba = crate::solid_fill_ui::thumbnail_color(self.app, self.doc.id, l.id, *color).to_rgba8();
                    ThumbRender::Ready(thumb_image(size(canvas), request.side, |_, _| rgba))
                }
                LayerContent::Fill(f @ Fill::Gradient { .. }) => {
                    crate::gradient_ui::thumbnail_image(f, request.side).map_or(ThumbRender::Blank, ThumbRender::Ready)
                }
                LayerContent::Fill(_) => ThumbRender::Ready(thumb_image(size(canvas), request.side, |_, _| [128, 128, 128, 255])),
                _ => {
                    let Some(surface) = l.surface() else { return ThumbRender::Blank };
                    let frame = crate::app::layer_thumb_frame(l, canvas, key.contents == ThumbContents::LayerBounds);
                    let n = surface.channels();
                    let mut px = [0.0f32; 8];
                    ThumbRender::Ready(thumb_image(size(frame), request.side, |x, y| {
                        let Some(px) = px.get_mut(..n) else { return [0; 4] };
                        surface.read_pixel(frame.x0 + x as i32, frame.y0 + y as i32, px);
                        rgba8(photocraft_raster::to_rgba(&surface.format(), px))
                    }))
                }
            },
        }
    }
}

/// The `select.loadSelection` params of a Select Pixels pick on `layer`'s thumbnail.
pub(crate) fn select_pixels(layer: LayerId) -> Value {
    json!({"channel": "transparency", "layer": layer.0, "operation": "new"})
}
