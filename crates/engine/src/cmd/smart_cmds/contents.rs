use super::filters::{smart, smart_mut};
use super::placing::{contain_box, detach_psd, stretch_placement};
use super::rendering::{refresh, refresh_or_fail, refresh_or_fail_opt, source_box, unapplied_filters};
use super::source::{authorize_source, authorized_source_bytes, source_bytes, source_image};
use super::{
    Arc, DocId, Document, EngineError, JobCtx, Layer, LayerContent, LayerId, Metadata, Rect, Result, Session, SmartContentsId, SmartLink, SmartSource, Value,
    bad, base_name, decode_source, encode_source, json, layer_param, other,
};

pub(super) fn path_param<'a>(cmd: &str, p: &'a Value) -> Result<&'a str> {
    p.get("path").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| bad(cmd, "pass `path`"))
}

/// Instances sharing contents in this document, including those inside groups.
fn instances(doc: &Document, contents_id: SmartContentsId) -> Vec<LayerId> {
    doc.walk().into_iter().filter_map(|(_, _, l)| matches!(&l.content, LayerContent::Smart(sm) if sm.contents_id == contents_id).then_some(l.id)).collect()
}

/// Swaps shared contents, keeping each instance's transform and filters, and re-renders.
pub(super) fn set_source(
    s: &mut Session,
    p: &Value,
    label: &str,
    keep_psd: bool,
    make: impl FnOnce(&Metadata, &SmartSource) -> Result<SmartSource>,
) -> Result<Value> {
    let id = layer_param(s, p)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let new = make(&doc.metadata, &smart(doc, id)?.source)?;
    set_shared_source(s, id, label, keep_psd, new)
}

// Keep the shared-instance transaction out of the per-command source-builder instantiations.
fn set_shared_source(s: &mut Session, id: LayerId, label: &str, keep_psd: bool, new: SmartSource) -> Result<Value> {
    set_shared_source_fit(s, id, label, keep_psd, new, None, false)
}

/// Stretching new contents over the old: the new source's box, and each instance's old box (see
/// [`authored_boxes`]). With `contain`, the new box is first grown to each old box's aspect
/// around its centre (see [`contain_box`]), so the new contents keep theirs.
struct Stretch {
    to: [f64; 4],
    from: Vec<(LayerId, [f64; 4])>,
    contain: bool,
}

/// [`set_shared_source`]; with `stretch`, re-aiming every instance's placement so the new
/// source's box lands where its old box did; with `clip`, see [`replace_contents_with`].
fn set_shared_source_fit(s: &mut Session, id: LayerId, label: &str, keep_psd: bool, new: SmartSource, stretch: Option<Stretch>, clip: bool) -> Result<Value> {
    let (parent, contents_id, changed, warnings) = s.edit(label, |doc, _| {
        let contents_id = smart(doc, id)?.contents_id;
        let changed = instances(doc, contents_id);
        let mut warnings = std::collections::BTreeSet::new();
        for &instance in &changed {
            if let Some(st) = &stretch {
                let old_box = st.from.iter().find(|(l, _)| *l == instance).map(|(_, b)| *b).ok_or(EngineError::NoLayer(instance))?;
                let to = if st.contain { contain_box(old_box, st.to) } else { st.to };
                stretch_placement(smart_mut(doc, instance)?, old_box, to);
            }
            smart_mut(doc, instance)?.source = new.clone();
            if !keep_psd {
                detach_psd(doc.layer_mut(instance).ok_or(EngineError::NoLayer(instance))?);
            }
            warnings
                .extend(unapplied_filters(smart(doc, instance)?).into_iter().map(|n| format!("the {n} smart filter is not supported, so it was not applied")));
            // A hidden instance (or one in a hidden group) is not in the flattened document: with
            // `clip`, it is left unrendered, like the parts of the others past the canvas.
            let layer = doc.layer(instance).ok_or(EngineError::NoLayer(instance))?;
            if clip && hidden_in(doc, instance) {
                // The id too: instances often share a name, and each must get its own warning.
                warnings.insert(format!(
                    "\"{}\" (layer {}) is hidden, so clipToCanvas left it unrendered: it shows nothing until it is rendered again without clipToCanvas",
                    layer.name, instance.0
                ));
                smart_mut(doc, instance)?.cache = None;
                continue;
            }
            // Effects (the layer's or a group's) follow the edge and bounds of what they cover,
            // and smart filters read past the canvas: such an instance is rendered whole, so the
            // flattened result stays the same.
            let clip_this = (clip && !needs_whole_render(doc, instance)).then(|| visible_clip(layer, doc.bounds()));
            // Only a warped placement is clipped (see `place`): say so, as the layer then holds
            // less than a full render, which a saved document would keep.
            if clip_this.is_some() && smart(doc, instance)?.warp.as_ref().is_some_and(|w| !w.is_identity()) {
                warnings.insert(format!(
                    "\"{}\" (layer {}) was rendered only where it shows in the flattened document (clipToCanvas): the rest of it stays empty until it is rendered again without clipToCanvas",
                    layer.name, instance.0
                ));
            }
            refresh_or_fail_opt(doc, instance, &JobCtx::new(), clip_this)?;
        }
        Ok((doc.id, contents_id, changed, warnings))
    })?;
    // An old editor must not overwrite newly replaced/relinked contents on close. Keep its
    // document open, but the next Edit Contents opens the replacement source.
    s.smart_links.retain(|l| l.parent != parent || l.contents_id != contents_id);
    // Every instance changed, not only `layer`: list them, so a caller doesn't repeat the
    // command once per copy (#4075).
    Ok(json!({"layer": id.0, "instances": changed.iter().map(|l| l.0).collect::<Vec<_>>(), "warnings": warnings}))
}

pub(super) fn replace_contents(s: &mut Session, p: &Value) -> Result<Value> {
    let fit = fit_param(p)?;
    let clip = clip_param(p)?;
    let path = path_param("layer.smartObjects.replaceContents", p)?.to_string();
    let bytes = photocraft_format::read_file(std::path::Path::new(&path)).map_err(|e| other(format!("can't read {path}: {e}")))?;
    let id = layer_param(s, p)?;
    replace_contents_with(s, id, &base_name(&path), bytes, fit, clip)
}

/// Replace Contents with a file the caller already read, for the smart object `p` names (`layer`,
/// else the active layer) and with its `fit`: the body of the command minus its ambient path
/// read (#8061).
pub fn replace_contents_from_bytes(s: &mut Session, p: &Value, file_name: &str, bytes: Vec<u8>) -> Result<Value> {
    let fit = fit_param(p)?;
    let clip = clip_param(p)?;
    let id = layer_param(s, p)?;
    replace_contents_with(s, id, file_name, bytes, fit, clip)
}

/// A request's `clipToCanvas` option: absent or null is off.
fn clip_param(p: &Value) -> Result<bool> {
    match p.get("clipToCanvas") {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(o) => Err(bad("layer.smartObjects.replaceContents", format!("`clipToCanvas` must be true or false, not {o}"))),
    }
}

/// How Replace Contents places a file of another size than the old contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// The new file keeps its own pixel size through the old placement (the default).
    Keep,
    /// The new file fills the old contents' box, whatever its size (what mockup templates expect).
    Stretch,
    /// The new file is scaled to fit inside the old contents' box, keeping its aspect ratio, and
    /// centred in it; the rest of the box stays empty.
    Contain,
}

impl Fit {
    /// A request's `fit` option: absent or null is [`Fit::Keep`]; `None` for anything but
    /// `"keep"`, `"stretch"` or `"contain"`.
    pub fn from_param(v: Option<&Value>) -> Option<Fit> {
        match v {
            None | Some(Value::Null) => Some(Fit::Keep),
            Some(Value::String(f)) if f == "keep" => Some(Fit::Keep),
            Some(Value::String(f)) if f == "stretch" => Some(Fit::Stretch),
            Some(Value::String(f)) if f == "contain" => Some(Fit::Contain),
            Some(_) => None,
        }
    }
}

/// The `fit` of command params `p`; anything [`Fit::from_param`] doesn't take is a bad parameter.
fn fit_param(p: &Value) -> Result<Fit> {
    Fit::from_param(p.get("fit"))
        .ok_or_else(|| bad("layer.smartObjects.replaceContents", format!("`fit` must be \"keep\", \"stretch\" or \"contain\", not {}", p["fit"])))
}

/// Replace Contents from a file's name and bytes: what the desktop and web file pickers hand over.
pub fn replace_contents_bytes(s: &mut Session, id: LayerId, file_name: &str, bytes: Vec<u8>) -> Result<Value> {
    replace_contents_fit(s, id, file_name, bytes, Fit::Keep)
}

/// [`replace_contents_bytes`] with a [`Fit`].
pub fn replace_contents_fit(s: &mut Session, id: LayerId, file_name: &str, bytes: Vec<u8>, fit: Fit) -> Result<Value> {
    replace_contents_with(s, id, file_name, bytes, fit, false)
}

/// [`replace_contents_fit`]; with `clip_to_canvas`, warped instances without layer effects (their
/// own or a group's around them) or visible smart filters are rendered only inside the canvas (and
/// their layer mask's revealed area), and hidden instances not at all. What a flattened export of
/// the document shows is the same, at a fraction of the cost when a placement is much larger than
/// the canvas; the parts left out are gone from the layer (a hidden instance shows nothing) until
/// something re-renders it in full (Undo restores the previous contents as before).
pub fn replace_contents_with(s: &mut Session, id: LayerId, file_name: &str, bytes: Vec<u8>, fit: Fit, clip_to_canvas: bool) -> Result<Value> {
    let stretch = match fit {
        Fit::Keep => {
            decode_source(file_name, &bytes)?; // fail before touching the document
            None
        }
        Fit::Stretch | Fit::Contain => {
            let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
            // The old boxes first: without a stored size they decode the old contents.
            let from = authored_boxes(s, doc, id)?;
            // Decode now, failing before touching the document; the re-render finds the image in
            // the source cache.
            let b = source_image(file_name, &bytes, doc.pixel_format())?.bounds;
            let to = [f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)];
            Some(Stretch { to, from, contain: fit == Fit::Contain })
        }
    };
    set_shared_source_fit(
        s,
        id,
        "Replace Contents",
        false,
        SmartSource::Embedded { file_name: file_name.to_string(), bytes: Arc::new(bytes) },
        stretch,
        clip_to_canvas,
    )
}

/// Whether smart object layer `id` must be rendered whole for the flattened document to be right:
/// layer effects, its own or those of a group around it, lay out against the bounds and edges of
/// what they cover (strokes, inner shadows, aligned gradients and patterns), and smart filters
/// sample beyond the canvas.
fn needs_whole_render(doc: &Document, id: LayerId) -> bool {
    let filtered =
        doc.layer(id).is_some_and(|l| matches!(&l.content, LayerContent::Smart(sm) if sm.filters_enabled && sm.smart_filters.iter().any(|f| f.visible)));
    filtered || doc.path_of(id).is_none_or(|p| (1..=p.len()).any(|n| doc.layer_at(&p[..n]).is_some_and(photocraft_compose::effects::has_effects)))
}

/// Whether `id` or a group around it is hidden, so it shows nowhere in the flattened document.
fn hidden_in(doc: &Document, id: LayerId) -> bool {
    doc.path_of(id).is_some_and(|p| (1..=p.len()).any(|n| doc.layer_at(&p[..n]).is_some_and(|l| !l.visible)))
}

/// The part of `canvas` where `l` can show anything given its own layer mask: inside the mask's
/// painted area when the mask hides by default at full density without a feather, else the whole
/// canvas (a feather spreads the mask by an amount that depends on its values, which a 32-bit
/// mask can take past 1). Group masks and clipping can only hide more, so this is never too small.
fn visible_clip(l: &Layer, canvas: Rect) -> Rect {
    match &l.mask {
        Some(m) if m.enabled && m.density >= 1.0 && m.feather <= 0.0 && m.surface.default_pixel().first().copied().unwrap_or(1.0) <= 0.0 => {
            m.surface.content_bounds().inflate(1).intersect(&canvas)
        }
        _ => canvas,
    }
}

/// The box (source pixels) each instance of `id`'s contents was authored for: its `soLD` stored
/// size when the layer came from a PSD (cheap), else the box of the decoded contents, which the
/// instances share (decoded once). A linked file's path comes from the document, so the
/// session's gate sees it before it is read, as for Replace Contents' own path.
fn authored_boxes(s: &Session, doc: &Document, id: LayerId) -> Result<Vec<(LayerId, [f64; 4])>> {
    let mut decoded = None;
    instances(doc, smart(doc, id)?.contents_id)
        .into_iter()
        .map(|l| {
            let sm = smart(doc, l)?;
            let stored =
                sm.psd_raw.as_deref().and_then(|d| photocraft_io::smart_map::parse_sold(d)).and_then(|p| photocraft_io::smart_map::stored_size(&p.descriptor));
            let b = match stored {
                Some((w, h)) => Some([0.0, 0.0, w, h]),
                None => {
                    authorize_source(s, "layer.smartObjects.replaceContents", doc, &sm.source)?;
                    *decoded.get_or_insert_with(|| source_box(doc, sm))
                }
            };
            Ok((l, b.ok_or_else(|| other("the smart object's contents are unavailable, so their box is unknown"))?))
        })
        .collect()
}

/// The file name and bytes of smart object `id`'s contents.
pub fn contents_of(doc: &Document, id: LayerId) -> Result<(String, Arc<Vec<u8>>)> {
    source_bytes(&doc.metadata, &smart(doc, id)?.source).ok_or_else(|| other("the smart object's contents are unavailable"))
}

/// Writes the smart object's file as it is. To a path with another extension (a `.png` for
/// `.psb` or `.pcraft` contents) the contents are opened and exported in that format instead.
pub(super) fn export_contents(s: &mut Session, p: &Value) -> Result<Value> {
    let path = path_param("layer.smartObjects.exportContents", p)?;
    let id = layer_param(s, p)?;
    let (name, bytes) = contents_of(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?;
    let ext = |f: &str| {
        let e = std::path::Path::new(f).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
        e.map(|e| match e.as_str() {
            "jpeg" => "jpg".to_string(),
            "tiff" => "tif".to_string(),
            _ => e,
        })
    };
    if ext(path).is_none() || ext(path) == ext(&name) {
        crate::file_cmds::write_file(path, &bytes)?;
        return Ok(json!({"path": path, "fileName": name, "bytes": bytes.len(), "converted": false}));
    }
    let contents = decode_source(&name, &bytes)?;
    let r = photocraft_io::export(&contents, path, &Default::default()).map_err(|e| other(format!("can't export the contents as {}: {e}", base_name(path))))?;
    crate::file_cmds::write_file(path, &r.bytes)?;
    Ok(json!({"path": path, "fileName": name, "bytes": r.bytes.len(), "converted": true, "warnings": r.warnings}))
}

pub(super) fn edit_contents(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let parent = st.doc.id;
    let contents_id = smart(&st.doc, id)?.contents_id;
    // Already open for editing: switch to that document rather than opening another copy.
    if let Some(index) =
        s.smart_links.iter().find(|l| l.parent == parent && l.contents_id == contents_id).and_then(|l| s.documents().iter().position(|d| d.doc.id == l.child))
    {
        s.set_active(index);
        return Ok(json!({"document": index, "parentLayer": id.0}));
    }
    let source = &smart(&st.doc, id)?.source;
    let (name, bytes) = authorized_source_bytes(s, "layer.smartObjects.editContents", &st.doc.metadata, source)?
        .ok_or_else(|| other("the smart object's contents are unavailable"))?;
    let mut child = decode_source(&name, &bytes)?;
    // Bundles keep their document id; each open copy needs its own.
    child.id = DocId::fresh();
    child.name = name;
    let index = s.add_document(child, None);
    // Admission may replace an ID already owned by another open document.
    let child_id = s.documents().get(index).ok_or(EngineError::NoDocument)?.doc.id;
    s.smart_links.retain(|l| l.child != child_id);
    s.smart_links.push(SmartLink { child: child_id, parent, layer: id, contents_id });
    Ok(json!({"document": index, "parentLayer": id.0}))
}

/// Writes an Edit Contents document back into all its parent instances (one undoable step in the
/// parent) and marks it saved. Returns false if `index` isn't an Edit Contents document.
pub fn commit_child(s: &mut Session, index: usize) -> Result<bool> {
    let Some(st) = s.docs.get(index) else { return Ok(false) };
    let child = st.doc.clone();
    let Some(link) = s.smart_links.iter().find(|l| l.child == child.id).copied() else { return Ok(false) };
    let parent = s.docs.iter().position(|d| d.doc.id == link.parent).ok_or_else(|| other("the smart object's document was closed"))?;
    let bytes = Arc::new(encode_source(&child)?);
    let prev = s.active;
    s.active = Some(parent);
    s.note_active_change();
    let r = s.edit("Edit Contents", |doc, _| {
        let ids = instances(doc, link.contents_id);
        if ids.is_empty() {
            return Err(other("the smart object's instances were removed"));
        }
        for id in ids {
            let sm = smart_mut(doc, id)?;
            let name = match &sm.source {
                SmartSource::Embedded { file_name, .. } => file_name.as_str(),
                SmartSource::Linked { .. } => child.name.as_str(),
            };
            let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
            sm.source = SmartSource::Embedded { file_name: format!("{stem}.pcraft"), bytes: bytes.clone() };
            detach_psd(doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?);
            refresh_or_fail(doc, id)?;
        }
        Ok(())
    });
    s.active = prev;
    s.note_active_change();
    r?;
    if let Some(st) = s.docs.get_mut(index) {
        st.saved_revision = st.revision;
    }
    Ok(true)
}

/// Called before a document closes: an edited Edit Contents document updates its parent.
pub(crate) fn on_close(s: &mut Session, index: usize) {
    let Some(st) = s.docs.get(index) else { return };
    let id = st.doc.id;
    if st.is_dirty() && s.smart_links.iter().any(|l| l.child == id) {
        let _ = commit_child(s, index);
    }
    s.smart_links.retain(|l| l.child != id && l.parent != id);
    // Decoded sources are shared by content hash across documents, so the closing document's
    // can't be told apart: drop them all. Other documents decode again on their next re-render.
    if s.docs.get(index).is_some_and(|st| st.doc.walk().iter().any(|(_, _, l)| matches!(l.content, LayerContent::Smart(_)))) {
        super::purge_source_cache();
    }
}

pub(super) fn is_smart_child(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if s.smart_links.iter().any(|l| l.child == d.doc.id) { Ok(()) } else { Err("this document isn't smart object contents".into()) }
}

pub(super) fn update_all(s: &mut Session) -> Result<Value> {
    let ids: Vec<LayerId> = s
        .active()
        .ok_or(EngineError::NoDocument)?
        .doc
        .walk()
        .into_iter()
        .filter(|(_, _, l)| matches!(&l.content, LayerContent::Smart(sm) if matches!(sm.source, SmartSource::Linked { .. })))
        .map(|(_, _, l)| l.id)
        .collect();
    let mut updated = Vec::new();
    s.edit("Update All Modified Content", |doc, _| {
        for id in &ids {
            if refresh(doc, *id)? {
                updated.push(id.0);
            }
        }
        Ok(())
    })?;
    Ok(json!({"updated": updated}))
}
