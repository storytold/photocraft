use craft::theme::variant::Tone;
use craft::ui::contract::Show as _;
use craft::ui::notices::{Notice, NoticeLayout};

use super::{BlendMode, LayerId, PhotocraftApp};

// ----------------------------------------------------------------------------- layers

/// The Layers panel's blend modes: Photoshop's, plus the layer's own Paint.NET mode when it has one
/// (an imported `.pdn`), so the menu matches Photoshop's for every other layer.
pub(crate) fn blend_options(groups: bool, current: BlendMode) -> Vec<(BlendMode, &'static str)> {
    let own = Some(current).filter(|m| !m.has_psd_equivalent());
    std::iter::once(BlendMode::PassThrough).filter(|_| groups).chain(BlendMode::LAYER_MODES).chain(own).map(|m| (m, m.label())).collect()
}

/// Files the OS drops on the Layers panel: taken anywhere over the panel, with the pointer.
struct LayersFileDrop(egui::Rect);

impl craft::ui::contract::DropTarget<craft::ui::contract::OsFiles> for LayersFileDrop {
    type Drop = (craft::ui::contract::OsFiles, egui::Pos2);

    fn accept(&self, files: &craft::ui::contract::OsFiles, at: egui::Pos2, _op: craft::ui::contract::DropOperation) -> Option<Self::Drop> {
        (!files.files.is_empty()).then(|| (files.clone(), at))
    }

    fn mark(&self, _drop: &Self::Drop) -> craft::ui::contract::DropMark {
        craft::ui::contract::DropMark::Into(self.0)
    }
}

/// Draw the active document's Layers panel and record where it is for files dropped on it.
pub(super) fn layers(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    use craft::layers::ui::panel::LayersPanel;
    use craft::layers::ui::thumbs::LayerThumb;
    use craft::ui::contract::Bind;
    use craft::ui::primitives::textures::{ThumbnailOptions, Thumbnails};

    use super::layers_tree::{self, PhotoThumbs};

    // A Move-way pick of a layer opens its groups only when the Move option says so (#11381).
    let reveal = crate::layer_reveal::take(app);
    let ctx = ui.ctx().clone();
    // Thumbnail Contents is the clip preference alone, so the panel reads it each pass.
    app.ui.layers_settings.options.thumbnail_contents = crate::layers_panel_ui::thumbnail_contents(app);
    let Some(st) = app.session.active_tracked() else {
        layers_tree::no_model(app);
        let _ = Notice::new(Tone::Neutral, tl!("No document")).layout(NoticeLayout::Centred).show(ui);
        return;
    };
    // While the drag shows another document, its rows take no drop.
    let foreign = crate::layer_transfer::source(app, &ctx).is_some_and(|source| source != st.doc.id);
    let settings = &app.ui.layers_settings;
    // The tree borrows the app for the pass, so the filter is bound through a copy and stored back after it.
    let mut filter = app.ui.layer_filter.clone();
    let before = filter.clone();
    let picked = crate::smart_ui::selected(&app.smart_filter_picks, &st.doc, st.active_layer);
    let model = layers_tree::model(app, st, &filter, foreign, picked);
    let Some(tree) = &*model else { return };
    let thumbs = Thumbnails::<LayerThumb<LayerId>>::in_context(&ctx, "photocraft-layer-thumbs", ThumbnailOptions::new());
    let source = PhotoThumbs::new(app, &st.doc);
    let mut panel = LayersPanel::new(tree)
        .settings(settings)
        .thumbnails(&thumbs, &source)
        .filter(Bind::bound(&mut filter))
        .row_menu(|target| layers_tree::row_menu(app, tree, target))
        .document(st.doc.id);
    if let Some((id, groups)) = reveal {
        panel = panel.reveal(id, groups);
    }
    if let Some(key) = crate::shortcuts::chord(app, "layer.hideLayers") {
        panel = panel.visibility_key(key);
    }
    if app.find_layers_focus {
        panel = panel.focus_filter_name();
    }
    if let Some(id) = app.layers_rename {
        panel = panel.start_rename(id);
    }
    let out = panel.show(ui);
    let doc_id = st.doc.id;
    app.find_layers_focus = false;
    app.layers_rename = None;
    if filter != before {
        app.ui.layer_filter = filter;
    }
    if out.hovered.is_some() {
        app.hover_layer = out.hovered;
    }
    // Preview on Canvas When Reordering Layers: the move the release would make.
    match out.drop_preview.as_ref().filter(|_| app.ui.layers_settings.options.preview_on_canvas_when_reordering) {
        Some(mv) => crate::layers_panel_ui::preview_reorder(app, &super::layers_events::move_params(mv)),
        None => app.reorder_preview = None,
    }
    // Files dropped from the OS are placed beside a row, like a layer dropped from inside the list.
    // The panel reports only the row under the pointer (not each row's rect), so a drop goes
    // directly above that row, or above the active layer when no row is under the pointer.
    if ctx.viewport_id() == egui::ViewportId::ROOT && ui.layer_id().order == egui::Order::Background {
        let list = out.response.rect.intersect(ui.clip_rect());
        let slots = out.hovered.map(|id| vec![(list, id)]).unwrap_or_default();
        app.drop_layers = Some(crate::file_open::DropLayers { doc: doc_id, frame: ctx.cumulative_frame_nr(), list, slots });
        let zone = craft::ui::contract::DropZone::new(ui.layer_id(), list);
        if let Some((files, at)) = craft::ui::runtime::drag::zone_target(ui, egui::Id::unique("layers-file-drop"), zone, &LayersFileDrop(list)) {
            let slot = app.layers_drop_at(&ctx, at).flatten();
            app.open_dropped(&ctx, files.files, crate::file_open::DropTo::Layers(slot));
        }
    }
    super::layers_events::apply_layers_events(app, &ctx, out.events);
}
