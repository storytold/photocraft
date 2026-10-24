//! Make native imported data ready for its first render, using the ordinary native renderers.

use std::sync::Arc;

use photocraft_doc::{Document, LayerContent, LayerPath, MAX_GROUP_DEPTH, SmartSource};

use crate::jobs::JobCtx;
use crate::{EngineError, Result, Session, smart_cmds};

#[path = "import_prepare/memo.rs"]
mod memo;

impl Session {
    /// Apply a validated import's native view state once, without an edit or history step.
    /// Callers validate before publishing the pending document; native reopen supplies no hint.
    pub fn apply_import_initial_view(&mut self, view: Option<photocraft_io::NativeInitialView>) -> Result<()> {
        let Some(view) = view else { return Ok(()) };
        let st = self.active_mut().ok_or(EngineError::NoDocument)?;
        view.validate(&st.doc).map_err(|e| EngineError::Other(e.to_string()))?;
        if !view.selected_layers.is_empty() {
            st.active_layer = view.active_layer;
            st.selected_layers = view.selected_layers;
            st.layer_anchor = st.active_layer;
            st.history.set_current_layers(crate::LayerTarget { active: st.active_layer, selected: st.selected_layers.clone() });
        }
        st.channel_view.target = match view.target {
            photocraft_io::NativeImportTarget::Composite => crate::channel_cmds::ChannelTarget::Composite,
            photocraft_io::NativeImportTarget::LayerMask => crate::channel_cmds::ChannelTarget::LayerMask,
            photocraft_io::NativeImportTarget::AlphaChannel(index) => crate::channel_cmds::ChannelTarget::Alpha(index),
        };
        st.color_samplers = view.samplers;
        for index in view.visible_alpha_channels {
            st.channel_view.set_alpha_visible(index, true);
        }
        st.channel_view.layer_mask = if view.show_layer_mask {
            st.active_layer.map(|layer| crate::mask_view_cmds::LayerMaskView { layer, mode: crate::mask_view_cmds::MaskViewMode::Gray })
        } else {
            None
        };
        st.initial_path_selection = view.selected_path;
        self.sync_tools_mask_target();
        Ok(())
    }

    /// Prepare pending data under this session's normal stored-command authority.
    pub fn prepare_imported_document(&self, doc: &mut Document, ctx: &JobCtx) -> Result<()> {
        let _authority = crate::jobs::render_authorization_scope(self.authorize);
        prepare_imported_document(doc, ctx)
    }

    /// The fallible import installation path: final native color policy, native preparation,
    /// then publication. `add_document` remains available for already-ready native documents.
    pub fn open_imported_document(&mut self, mut doc: Document, path: Option<String>, ctx: &JobCtx) -> Result<(usize, serde_json::Value)> {
        ctx.check()?;
        let color = self.color.open_policy(&mut doc, self.prefs().file_handling.ignore_exif_profile_tag);
        self.prepare_imported_document(&mut doc, ctx)?;
        Ok((self.add_document(doc, path), color))
    }
}

const MAX_CONTENT_DEPTH: usize = 64;

const MAX_ITEMS: usize = 65_536;
const MAX_SOURCE_BYTES: usize = 1 << 30;
// Match the native type renderer's working-byte limits. These are import-preparation allowances,
// not claims about whole-process memory or larger file support.
const MAX_PREPARATION_BYTES: u64 = if cfg!(target_pointer_width = "64") { 4 << 30 } else { 1 << 30 };

struct Allowance {
    remaining_bytes: u64,
    source_bytes: usize,
    items: usize,
    memo: memo::Memo,
}

fn limit(what: &str) -> EngineError {
    EngineError::Other(format!("native import preparation exceeds {what}; the document was not opened"))
}

impl Allowance {
    fn charge_bytes(remaining: &mut u64, bytes: u64) -> Result<()> {
        *remaining = remaining.checked_sub(bytes).ok_or_else(|| limit("its working-byte allowance"))?;
        Ok(())
    }
    fn source(&mut self, bytes: usize) -> Result<()> {
        self.source_bytes = self.source_bytes.checked_sub(bytes).ok_or_else(|| limit("1 GiB of embedded source bytes"))?;
        Ok(())
    }

    fn item(&mut self) -> Result<()> {
        self.items = self.items.checked_sub(1).ok_or_else(|| limit("its preparation item count"))?;
        Ok(())
    }

    fn bytes(&mut self, bytes: u64) -> Result<()> {
        Self::charge_bytes(&mut self.remaining_bytes, bytes)
    }

    fn memo_hit(
        &mut self,
        key: &memo::Key,
        bytes: &Arc<Vec<u8>>,
        profile: &Option<Arc<Vec<u8>>>,
        ctx: &JobCtx,
    ) -> Result<Option<(Arc<Vec<u8>>, photocraft_raster::Surface)>> {
        let Self { remaining_bytes, memo, .. } = self;
        memo.get(key, bytes, profile, ctx, &mut |bytes| Self::charge_bytes(remaining_bytes, bytes))
    }

    fn render_bytes(doc: &Document) -> Result<u64> {
        // Native composition uses RGBA float pixels; filters also keep source/result buffers.
        doc.size.area().checked_mul(32).ok_or_else(|| limit("its raster bounds"))
    }

    fn render(&mut self, doc: &Document) -> Result<u64> {
        let bytes = Self::render_bytes(doc)?;
        self.bytes(bytes)?;
        Ok(bytes)
    }
}

/// Prepare private pending data after final native color policy, before install or export.
/// A failure/cancellation leaves `doc` unchanged. Valid caches (including unavailable native
/// filters' imported pixels) are preserved. Reports and original source locators are not read.
/// Session-scoped hosts use [`Session::prepare_imported_document`] to install their authority.
pub fn prepare_imported_document(doc: &mut Document, ctx: &JobCtx) -> Result<()> {
    ctx.check()?;
    let mut allowance = Allowance { remaining_bytes: MAX_PREPARATION_BYTES, source_bytes: MAX_SOURCE_BYTES, items: MAX_ITEMS, memo: Default::default() };
    prepare_with_allowance(doc, ctx, &mut allowance)
}

fn prepare_with_allowance(doc: &mut Document, ctx: &JobCtx, allowance: &mut Allowance) -> Result<()> {
    ctx.check()?;
    // Check hierarchy before cloning: a malformed API-supplied group tree must not reach a
    // recursive Clone before the ordinary native depth/count bounds are established.
    let ids = missing_caches(doc, ctx, allowance)?;
    if ids.is_empty() {
        return ctx.check();
    }
    let mut pending = doc.clone();
    prepare_layers(&mut pending, ids, ctx, 0, allowance)?;
    ctx.check()?;
    *doc = pending;
    Ok(())
}

fn missing_caches(doc: &Document, ctx: &JobCtx, allowance: &mut Allowance) -> Result<Vec<LayerPath>> {
    // Keep one iterator per group depth, not one pointer per existing layer. Already-ready
    // native documents do not acquire a new layer-count ceiling or a metadata clone.
    let mut pending = vec![(doc.layers.iter().enumerate(), Vec::new())];
    let mut out = Vec::new();
    while let Some((children, parent)) = pending.last_mut() {
        ctx.check()?;
        let Some((index, layer)) = children.next() else {
            pending.pop();
            continue;
        };
        if parent.len() > MAX_GROUP_DEPTH {
            return Err(limit("native group depth"));
        }
        match &layer.content {
            LayerContent::Group(group) => {
                let mut path = parent.clone();
                path.push(index);
                pending.push((group.children.iter().enumerate(), path));
            }
            LayerContent::Text(_) | LayerContent::Shape(_) | LayerContent::Smart(_)
                if layer.surface().is_none_or(|surface| surface.format() != doc.pixel_format()) || crate::import_native_actions::has_pending(layer) =>
            {
                allowance.item()?;
                let mut path = parent.clone();
                path.push(index);
                out.push(path);
            }
            _ => {}
        }
    }
    Ok(out)
}

fn prepare(doc: &mut Document, ctx: &JobCtx, depth: usize, allowance: &mut Allowance) -> Result<bool> {
    if depth > MAX_CONTENT_DEPTH {
        return Err(limit("64 nested native contents"));
    }
    let ids = missing_caches(doc, ctx, allowance)?;
    prepare_layers(doc, ids, ctx, depth, allowance)
}

fn prepare_layers(doc: &mut Document, ids: Vec<LayerPath>, ctx: &JobCtx, depth: usize, allowance: &mut Allowance) -> Result<bool> {
    let changed = !ids.is_empty();
    for path in ids {
        ctx.check()?;
        let mut layer = doc.layer_at(&path).cloned().ok_or_else(|| EngineError::Other("native preparation layer is missing".into()))?;
        match &mut layer.content {
            LayerContent::Text(text) => {
                allowance.render(doc)?;
                crate::type_cmds::refresh(doc, text);
            }
            LayerContent::Shape(shape) => {
                allowance.render(doc)?;
                crate::vector_cmds::refresh_shape(doc, shape);
            }
            LayerContent::Smart(object) => {
                // Only in-memory source resolution here. Ordinary explicit relink/refresh owns
                // external reads; an imported locator alone cannot authorize them.
                let (name, bytes) = smart_cmds::embedded_source_bytes(&doc.metadata, &object.source)
                    .ok_or_else(|| EngineError::Other("native smart cache is missing and its source needs explicit resource resolution".into()))?;
                allowance.source(bytes.len())?;
                if photocraft_format::is_pcraft(&bytes) && depth >= MAX_CONTENT_DEPTH {
                    return Err(limit("64 nested native contents"));
                }
                let memo_key = memo::Key::new(doc, object, &bytes, ctx)?;
                if let Some(key) = &memo_key
                    && let Some((prepared_bytes, cache)) = allowance.memo_hit(key, &bytes, &doc.icc_profile, ctx)?
                {
                    if let SmartSource::Embedded { bytes, .. } = &mut object.source {
                        *bytes = prepared_bytes;
                    }
                    object.cache = Some(cache);
                    ctx.check()?;
                    let destination = doc.layer_at_mut(&path).ok_or_else(|| EngineError::Other("native preparation layer is missing".into()))?;
                    destination.content = layer.content;
                    continue;
                }
                let render_reservation = allowance.render(doc)?;
                let source_was_embedded = matches!(object.source, SmartSource::Embedded { .. });
                let original_source = object.source.clone();
                let original_bytes = bytes.clone();
                let mut leaf_raster = false;
                let prepared_bytes = if photocraft_format::is_pcraft(&bytes) {
                    let opts = photocraft_format::LoadOptions {
                        max_total_bytes: allowance.remaining_bytes,
                        max_blob_bytes: usize::try_from(allowance.remaining_bytes).unwrap_or(usize::MAX).min(MAX_SOURCE_BYTES),
                        // This render frame has already been debited. Its canvas
                        // admission must not reserve the same frame a second time.
                        // Actual decoded payload still fits only remaining_bytes.
                        max_canvas_pixels: allowance.remaining_bytes.checked_add(render_reservation).ok_or_else(|| limit("its raster bounds"))? / 32,
                        ..Default::default()
                    };
                    let (mut source, decoded_bytes) = photocraft_format::load_from_bytes_measured(&bytes, &opts)
                        .map_err(|e| EngineError::Other(format!("cannot prepare embedded native contents: {e}")))?;
                    allowance.bytes(decoded_bytes)?;
                    // A source larger than the parent cannot borrow a free frame:
                    // debit its extra extent before recursive work or composition.
                    allowance.bytes(Allowance::render_bytes(&source)?.saturating_sub(render_reservation))?;
                    // Reuse only original raster-only native bundles. A nested
                    // Smart/filter/resource must retain its own preparation and
                    // authorization traversal, even if its outer Smart is plain.
                    leaf_raster = source.layers.iter().all(|layer| matches!(layer.content, LayerContent::Raster(_)) && layer.video.is_none());
                    if prepare(&mut source, ctx, depth + 1, allowance)? { Arc::new(smart_cmds::encode_source(&source)?) } else { bytes }
                } else {
                    bytes
                };
                // Embedded PSD UUIDs render from memory without enabling a filesystem link.
                object.source = SmartSource::Embedded { file_name: name, bytes: prepared_bytes };
                crate::import_native_actions::materialize(doc, &mut layer, ctx, &mut |bytes| allowance.bytes(bytes))?;
                if !smart_cmds::prepare_layer(doc, &mut layer, ctx)? {
                    return Err(EngineError::Other("native smart contents could not be prepared".into()));
                }
                if !source_was_embedded && let LayerContent::Smart(object) = &mut layer.content {
                    object.source = original_source;
                }
                ctx.check()?;
                if leaf_raster
                    && let Some(key) = memo_key
                    && let Some(surface) = layer.surface()
                    && let Some(bytes) = allowance.memo.vacancy(&key, surface, allowance.remaining_bytes)
                {
                    allowance.bytes(bytes)?;
                    allowance.memo.insert(key, original_bytes, doc.icc_profile.clone(), surface);
                }
            }
            _ => {
                allowance.render(doc)?;
            }
        }
        ctx.check()?;
        let destination = doc.layer_at_mut(&path).ok_or_else(|| EngineError::Other("native preparation layer is missing".into()))?;
        destination.content = layer.content;
    }
    Ok(changed)
}

#[cfg(test)]
#[path = "import_prepare/resource_tests.rs"]
mod resource_tests;

#[cfg(test)]
mod initial_view_tests {
    use super::*;
    use photocraft_io::{NativeImportTarget, NativeInitialView};
    use serde_json::json;

    #[test]
    fn native_initial_view_is_atomic_view_state_and_not_restored_by_reopen() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
        let first = session.active().unwrap().active_layer.unwrap();
        session.execute("layer.new.layer", json!({"name": "second"})).unwrap();
        let before = session.active().unwrap().revision;
        let view = NativeInitialView { selected_layers: vec![first], active_layer: Some(first), samplers: vec![[2.5, 3.5]], ..Default::default() };
        session.apply_import_initial_view(Some(view)).unwrap();
        let st = session.active().unwrap();
        assert_eq!(st.active_layer, Some(first));
        assert_eq!(st.color_samplers, vec![[2.5, 3.5]]);
        assert_eq!(st.revision, before);
        let bytes = photocraft_format::save_to_bytes(&st.doc, &Default::default()).unwrap();
        let imported = photocraft_io::import("view.pcraft", &bytes).unwrap();
        assert!(imported.initial_view.is_none());
        let mut reopened = Session::new();
        reopened.add_document(imported.document, None);
        assert!(reopened.active().unwrap().color_samplers.is_empty());
        assert!(reopened.active().unwrap().initial_path_selection.is_none());
    }

    #[test]
    fn invalid_native_initial_view_leaves_all_existing_state_unchanged() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
        let before = session.active().unwrap().clone();
        for view in [
            NativeInitialView { samplers: vec![[f64::NAN, 0.0]], ..Default::default() },
            NativeInitialView { target: NativeImportTarget::AlphaChannel(0), ..Default::default() },
            NativeInitialView { selected_path: Some(0), ..Default::default() },
        ] {
            assert!(session.apply_import_initial_view(Some(view)).is_err());
            let st = session.active().unwrap();
            assert!(Arc::ptr_eq(&before.doc, &st.doc));
            assert_eq!(st.active_layer, before.active_layer);
            assert_eq!(st.selected_layers, before.selected_layers);
            assert_eq!(st.channel_view, before.channel_view);
            assert_eq!(st.color_samplers, before.color_samplers);
            assert_eq!(st.revision, before.revision);
        }
    }

    #[test]
    fn held_native_initial_view_is_kept_until_taken_once() {
        let doc = Document::new("held", photocraft_geom::Size::new(8, 6), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let mut session = Session::new();
        let view = NativeInitialView { samplers: vec![[1.0, 2.0]], ..Default::default() };
        let id = session.hold_open(doc, true, Some(view.clone()));
        let (_, read_only, pending) = session.take_held_open_with_view(id).unwrap();
        assert!(read_only);
        assert_eq!(pending, Some(view));
        assert!(session.take_held_open_with_view(id).is_none());
        assert!(session.documents().is_empty());
    }
}
