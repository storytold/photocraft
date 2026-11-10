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
            let target = crate::LayerTarget { active: st.active_layer, selected: st.selected_layers.clone() };
            st.history.set_current_layers(target.clone());
            // The opened snapshot predates this one-time view handoff. Update its target only
            // while it still holds this exact admitted document, never an older edited state.
            if let Some(snapshot) = st.snapshots.iter_mut().find(|snapshot| snapshot.id == 0 && Arc::ptr_eq(&snapshot.doc, &st.doc)) {
                snapshot.layers = target;
            }
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
            st.active_layer.map(|layer| crate::mask_view_cmds::LayerMaskView { layer, mode: crate::mask_view_cmds::MaskViewMode::Gray, filter: false })
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

/// Report preparation limits as open failures without implying a partially prepared document was installed.
fn limit(what: &str) -> EngineError {
    EngineError::Other(format!("native import preparation exceeds {what}; the document was not opened"))
}

impl Allowance {
    /// Debit the shared working allowance with checked subtraction, preserving the counter on failure.
    fn charge_bytes(remaining: &mut u64, bytes: u64) -> Result<()> {
        *remaining = remaining.checked_sub(bytes).ok_or_else(|| limit("its working-byte allowance"))?;
        Ok(())
    }
    /// Charge each embedded archive encounter against the cumulative source-byte allowance.
    fn source(&mut self, bytes: usize) -> Result<()> {
        self.source_bytes = self.source_bytes.checked_sub(bytes).ok_or_else(|| limit("1 GiB of embedded source bytes"))?;
        Ok(())
    }

    /// Admit one preparation item with checked subtraction from the remaining item count.
    fn item(&mut self) -> Result<()> {
        self.items = self.items.checked_sub(1).ok_or_else(|| limit("its preparation item count"))?;
        Ok(())
    }

    /// Charge retained data or an explicitly scoped temporary reservation against the shared byte allowance.
    fn bytes(&mut self, bytes: u64) -> Result<()> {
        Self::charge_bytes(&mut self.remaining_bytes, bytes)
    }

    /// Look up an exact native preparation result and debit each returned Surface clone's metadata before cloning it.
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

    /// Compute the checked full-frame working reservation for native composition and source/result buffers.
    fn render_bytes(doc: &Document) -> Result<u64> {
        // Native composition uses RGBA float pixels; filters also keep source/result buffers.
        doc.size.area().checked_mul(32).ok_or_else(|| limit("its raster bounds"))
    }

    /// Debit a full native render reservation and return its size for loader admission credit.
    fn render(&mut self, doc: &Document) -> Result<u64> {
        let bytes = Self::render_bytes(doc)?;
        self.bytes(bytes)?;
        Ok(bytes)
    }

    /// Keep an item's temporary frame live through nested preparation and memo insertion, then release only that
    /// reservation. Retained cache/source debits made by the work remain charged even when the work fails.
    fn with_render<T>(&mut self, doc: &Document, work: impl FnOnce(&mut Self, u64, &mut u64) -> Result<T>) -> Result<T> {
        let render = self.render(doc)?;
        let mut temporary = render;
        let result = work(self, render, &mut temporary);
        self.remaining_bytes = self.remaining_bytes.checked_add(temporary).ok_or_else(|| limit("temporary render accounting"))?;
        result
    }

    /// Extend the same live temporary frame for a larger source before its recursive preparation begins.
    fn extend_render(&mut self, temporary: &mut u64, extra: u64) -> Result<()> {
        let total = temporary.checked_add(extra).ok_or_else(|| limit("temporary render accounting"))?;
        self.bytes(extra)?;
        *temporary = total;
        Ok(())
    }

    /// Charge the actual retained tile payload and owned Surface metadata separately from temporary render work.
    fn cache(&mut self, surface: &photocraft_raster::Surface) -> Result<()> {
        let bytes = surface.tiles().try_fold(memo::clone_bytes(surface)?, |sum, (_, tile)| {
            sum.checked_add(tile.bytes().len() as u64).ok_or_else(|| limit("retained native cache bytes"))
        })?;
        self.bytes(bytes)
    }

    /// Debit actual decoded non-native raster payload before recursive preparation or composition.
    /// Format-specific decoders retain their own allocation limits; this is the shared preparation ledger.
    fn decoded_surfaces(&mut self, doc: &Document, ctx: &JobCtx) -> Result<()> {
        let mut groups = vec![doc.layers.iter()];
        while let Some(children) = groups.last_mut() {
            ctx.check()?;
            let Some(layer) = children.next() else {
                groups.pop();
                continue;
            };
            if groups.len() > MAX_GROUP_DEPTH + 1 {
                return Err(limit("native group depth"));
            }
            if let Some(surface) = layer.surface() {
                self.cache(surface)?;
            }
            if let Some(mask) = &layer.mask {
                self.cache(&mask.surface)?;
            }
            if let LayerContent::Smart(object) = &layer.content
                && let Some(mask) = &object.filter_mask
            {
                self.cache(&mask.surface)?;
            }
            if let Some(children) = layer.children() {
                groups.push(children.iter());
            }
        }
        for channel in &doc.channels {
            ctx.check()?;
            self.cache(&channel.surface)?;
        }
        if let Some(channel) = &doc.quick_mask {
            self.cache(&channel.surface)?;
        }
        if let Some(selection) = &doc.selection {
            self.cache(selection)?;
        }
        for pattern in &doc.patterns {
            ctx.check()?;
            self.cache(&pattern.surface)?;
        }
        ctx.check()
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

/// Preflight hierarchy before cloning and publish the prepared clone only after all work and the final cancellation
/// check succeed.
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

/// Walk bounded group depth iteratively and collect only missing or stale native caches, leaving ready documents
/// outside the work-item ceiling.
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

/// Bound embedded-content recursion before discovering and preparing missing native caches.
fn prepare(doc: &mut Document, ctx: &JobCtx, depth: usize, allowance: &mut Allowance) -> Result<bool> {
    if depth > MAX_CONTENT_DEPTH {
        return Err(limit("64 nested native contents"));
    }
    let ids = missing_caches(doc, ctx, allowance)?;
    prepare_layers(doc, ids, ctx, depth, allowance)
}

/// Prepare text, shape and Smart caches under shared work/source limits; resolve only in-memory sources and retain
/// stored-filter authorization.
fn prepare_layers(doc: &mut Document, ids: Vec<LayerPath>, ctx: &JobCtx, depth: usize, allowance: &mut Allowance) -> Result<bool> {
    let changed = !ids.is_empty();
    for path in ids {
        ctx.check()?;
        let mut layer = doc.layer_at(&path).cloned().ok_or_else(|| EngineError::Other("native preparation layer is missing".into()))?;
        match &mut layer.content {
            LayerContent::Text(text) => {
                allowance.with_render(doc, |allowance, _, _| {
                    crate::type_cmds::refresh(doc, text);
                    if let Some(cache) = &text.cache {
                        allowance.cache(cache)?;
                    }
                    if let Some(raw) = &text.psd_raw {
                        allowance.bytes(raw.len() as u64)?;
                    }
                    ctx.check()
                })?;
            }
            LayerContent::Shape(shape) => {
                allowance.with_render(doc, |allowance, _, _| {
                    crate::vector_cmds::refresh_shape(doc, shape);
                    if let Some(cache) = &shape.cache {
                        allowance.cache(cache)?;
                    }
                    ctx.check()
                })?;
            }
            LayerContent::Smart(object) => {
                // Only in-memory source resolution here. Ordinary explicit relink/refresh owns
                // external reads; an imported locator alone cannot authorize them.
                let (name, bytes) = smart_cmds::embedded_source_bytes(&doc.metadata, &object.source)
                    .ok_or_else(|| EngineError::Other("native smart cache is missing and its source needs explicit resource resolution".into()))?;
                allowance.source(bytes.len())?;
                if depth >= MAX_CONTENT_DEPTH {
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
                allowance.with_render(doc, |allowance, render_reservation, temporary| {
                    let LayerContent::Smart(object) = &mut layer.content else {
                        return Err(EngineError::Other("native preparation Smart content changed before rendering".into()));
                    };
                    let source_was_embedded = matches!(object.source, SmartSource::Embedded { .. });
                    let original_source = object.source.clone();
                    let original_bytes = bytes.clone();
                    let mut leaf_raster = false;
                    let native_source = photocraft_format::is_pcraft(&bytes);
                    let direct_svg = photocraft_io::svg::is_svg(&bytes) && object.stack_mode.is_none() && object.warp.is_none();
                    let prepared_source;
                    let prepared_bytes = if direct_svg {
                        // The ordinary scale-aware renderer validates and rasterizes
                        // this source once. Importing native shapes first would build
                        // caches that this direct vector route immediately discards.
                        prepared_source = None;
                        bytes
                    } else if native_source {
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
                        allowance.extend_render(temporary, Allowance::render_bytes(&source)?.saturating_sub(render_reservation))?;
                        // Reuse only original raster-only native bundles. A nested
                        // Smart/filter/resource must retain its own preparation and
                        // authorization traversal, even if its outer Smart is plain.
                        leaf_raster = source.layers.iter().all(|layer| matches!(layer.content, LayerContent::Raster(_)) && layer.video.is_none());
                        let changed = prepare(&mut source, ctx, depth + 1, allowance)?;
                        let encoded = if changed { Arc::new(smart_cmds::encode_source(&source)?) } else { bytes };
                        prepared_source = Some(source);
                        encoded
                    } else {
                        let imported = ctx
                            .stage(0.0, 0.5, "Decoding embedded contents", |ctl| {
                                photocraft_io::import_with_options_and_interrupt(&name, &bytes, &Default::default(), ctl)
                            })
                            .map_err(|error| {
                                if ctx.cancelled() { EngineError::Cancelled } else { EngineError::Other(format!("cannot prepare embedded contents: {error}")) }
                            })?;
                        if imported.preview_only {
                            return Err(EngineError::Other("preview-only embedded contents cannot supply a missing native cache".into()));
                        }
                        let mut source = imported.document;
                        allowance.decoded_surfaces(&source, ctx)?;
                        allowance.extend_render(temporary, Allowance::render_bytes(&source)?.saturating_sub(render_reservation))?;
                        prepare(&mut source, ctx, depth + 1, allowance)?;
                        prepared_source = Some(source);
                        bytes
                    };
                    // Supply the prepared source directly: neither serialization nor a fresh
                    // recursive decoder/preparation allowance is needed to render this layer.
                    let image = prepared_source
                        .as_ref()
                        .map(|source| smart_cmds::prepared_source_image(source, doc.pixel_format(), object.stack_mode, ctx))
                        .transpose()?;
                    // Embedded PSD UUIDs render from memory without enabling a filesystem link.
                    object.source = SmartSource::Embedded { file_name: name, bytes: prepared_bytes };
                    crate::import_native_actions::materialize_with_source(
                        doc,
                        &mut layer,
                        ctx,
                        render_reservation,
                        &mut |bytes| allowance.bytes(bytes),
                        image.as_ref(),
                    )?;
                    if !smart_cmds::prepare_layer_with_source(doc, &mut layer, ctx, image.as_ref())? {
                        return Err(EngineError::Other("native smart contents could not be prepared".into()));
                    }
                    if (!source_was_embedded || !native_source)
                        && let LayerContent::Smart(object) = &mut layer.content
                    {
                        object.source = original_source;
                    }
                    ctx.check()?;
                    if let Some(cache) = layer.surface() {
                        allowance.cache(cache)?;
                    }
                    if leaf_raster
                        && let Some(key) = memo_key
                        && let Some(surface) = layer.surface()
                        && let Some(bytes) = allowance.memo.vacancy(&key, surface, allowance.remaining_bytes)
                    {
                        allowance.bytes(bytes)?;
                        allowance.memo.insert(key, original_bytes, doc.icc_profile.clone(), surface);
                    }
                    Ok(())
                })?;
            }
            _ => {}
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
    fn opened_snapshot_restores_the_imported_layer_target() {
        let mut source = Session::new();
        source.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
        let first = source.active().unwrap().active_layer.unwrap();
        source.execute("layer.new.layer", json!({"name": "second"})).unwrap();
        let second = source.active().unwrap().active_layer.unwrap();
        let mut session = Session::new();
        session.add_document((*source.active().unwrap().doc).clone(), None);
        let before = session.active().unwrap().revision;
        session
            .apply_import_initial_view(Some(NativeInitialView { selected_layers: vec![first, second], active_layer: Some(first), ..Default::default() }))
            .unwrap();
        assert_eq!(session.active().unwrap().revision, before);
        session.execute("layer.select", json!({"layer": second.0})).unwrap();
        session.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
        session.execute("history.selectSnapshot", json!({"snapshot": 0})).unwrap();
        let state = session.active().unwrap();
        assert_eq!(state.active_layer, Some(first));
        assert_eq!(state.selected_layers, vec![first, second]);
    }

    #[test]
    fn initial_view_does_not_retarget_an_older_snapshot() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
        let original = session.active().unwrap().active_layer.unwrap();
        session.execute("layer.new.layer", json!({"name": "later"})).unwrap();
        let later = session.active().unwrap().active_layer.unwrap();
        session.apply_import_initial_view(Some(NativeInitialView { selected_layers: vec![later], active_layer: Some(later), ..Default::default() })).unwrap();
        session.execute("history.selectSnapshot", json!({"snapshot": 0})).unwrap();
        let state = session.active().unwrap();
        assert_eq!(state.active_layer, Some(original));
        assert_eq!(state.selected_layers, vec![original]);
        assert!(state.doc.layer(later).is_none());
    }

    #[test]
    /// Apply imported view hints without history or revision changes and verify native reopen does not replay them.
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
    /// Reject invalid view references and nonfinite samplers without changing document identity, selection or revision.
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
    /// Keep the source view with a held import and verify taking it consumes the handoff exactly once without
    /// installing a tab.
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
