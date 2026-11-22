use craft::behavior::geometry::BoxAllow;
use craft::behavior::kurbo::Rect;
use craft::draw::ui::transform::{TransformEdit, TransformSession};
use craft::imaging::ui::transform_preview::{PreviewPixels, PreviewTextures, StyledPreview};

use super::commit::{cancel, commit};
#[cfg(test)]
use super::draw::texture_quad;
use super::preview::{Above, AbovePlan, preview_image};
use super::{
    Arc, BezierMesh, CAS_ONLY_SCALES, Color32, DocId, Document, Follow, Homography, LOCKED, LayerContent, LayerId, MadeLayer, OpenPreview, OtherTexels,
    PhotocraftApp, Several, Warp, WarpStyle, default_interpolation, json, settings, swap_mode_texels,
};
use crate::state::{FreeTransform, SessionWarp};

/// The source frame `r` (`[x0, y0, x1, y1]`) as the session's frame.
fn frame_of(r: [f64; 4]) -> Rect {
    Rect::new(r[0], r[1], r[2], r[3])
}

/// What the content allows: editable type refuses distort and perspective, everything else allows all.
fn type_allow(typed: bool) -> BoxAllow {
    if typed { BoxAllow::new(true, true, false) } else { BoxAllow::default() }
}

/// The session over the source frame `rect`: the stored settings, the interpolation default and the nudge pause.
/// `warp_ok` is false where the content cannot warp (a lone mask or channel, several layers, Content-Aware Scale).
fn open_session(app: &PhotocraftApp, rect: [f64; 4], allow: BoxAllow, warp_ok: bool) -> TransformSession<SessionWarp> {
    open_session_at(app, rect, None, allow, warp_ok)
}

/// [`open_session`] with the box starting on `start` instead of on the frame's corners (a smart object's placement).
fn open_session_at(app: &PhotocraftApp, rect: [f64; 4], start: Option<[[f64; 2]; 4]>, allow: BoxAllow, warp_ok: bool) -> TransformSession<SessionWarp> {
    let session = match start {
        Some(q) => {
            let quad = q.map(|p| craft::behavior::kurbo::Point::new(p[0], p[1]));
            let pivot = craft::behavior::kurbo::Point::new(q.iter().map(|p| p[0]).sum::<f64>() / 4.0, q.iter().map(|p| p[1]).sum::<f64>() / 4.0);
            TransformSession::with_state(frame_of(rect), craft::behavior::geometry::quad::BoxState::new(quad, pivot))
        }
        None => TransformSession::new(frame_of(rect)),
    };
    session.allow(allow.with_warp(warp_ok)).settings(&settings(app), default_interpolation(app)).nudge_pause(crate::nudge_bundle::pause(app))
}

/// The Frame tool's entry: [`begin`] with the session marked to commit when the press that opened it is released.
pub fn begin_from_frame(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<(), String> {
    begin(app, ctx)?;
    app.ui.frame_tool.auto_transform = true;
    Ok(())
}

/// Start Free Transform on the active layer (or its selected pixels), or on the targeted unlinked
/// layer mask, alpha channel or Quick Mask.
pub fn begin(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<(), String> {
    if app.transform_preparing.is_some() {
        return Err("Free Transform is still preparing the layer".into());
    }
    if let Some((params, path)) = crate::vector_ui::free_transform_path(app) {
        return begin_path(app, params, path);
    }
    if crate::canvas::multiple_color_target(app) {
        return Err("Move and transforms on several colour channels are not implemented yet".into());
    }
    if app.session.active().is_some_and(|st| photocraft_engine::float_cmds::floating(st).is_some()) {
        app.run("select.drop", json!({}))?;
    }
    let target = crate::canvas::paint_target(app);
    let st = app.session.active().ok_or("no document")?;
    let (doc, revision) = (st.doc.clone(), st.revision);
    let lone = photocraft_engine::transform_cmds::lone_target(&doc, st.active_layer, &json!({ "target": target })).map_err(|e| e.to_string())?.cloned();
    if let Some(surf) = lone {
        return begin_lone(app, ctx, doc, surf, target);
    }
    let id = st.active_layer.ok_or("no active layer")?;
    // Several layers selected: one box moves them all, as in Photoshop (#6501).
    let targets = photocraft_engine::transform_cmds::selection_targets(st);
    // The selection takes its linked layers along, as `edit.transform` does: in Photoshop the box
    // over one linked layer frames and moves the whole link group (#11217), but not a linked
    // Background (`box_targets`). Unlinked, unchanged.
    let targets = photocraft_engine::transform_cmds::box_targets(&doc, &targets);
    if targets != [id] {
        return begin_several(app, ctx, doc, revision, id, targets);
    }
    let layer = doc.layer(id).ok_or("no layer")?;
    let frame_only =
        app.ui.tool == crate::state::Tool::Frame && layer.frame.is_some() && crate::frame_tool::target(app, layer) == crate::frame_tool::Target::Frame;
    if photocraft_engine::transform_cmds::locked_for_transform(&doc, id) {
        // Photoshop refuses before the box opens rather than when it is committed (#2585).
        crate::dialogs::alerts::alert(app, tl!(LOCKED));
        return Err(LOCKED.into());
    }
    if matches!(layer.content, LayerContent::Adjustment(_)) && layer.mask.is_none() {
        return Err("Adjustment layers have nothing to transform".into());
    }
    // With a selection the box is the selection's (#5297).
    let b = photocraft_engine::transform_cmds::transform_frame(&doc, layer);
    if b.is_empty() {
        return Err("Could not transform: the layer is empty".into());
    }
    crate::type_tool::commit(app);
    let rect = [b.x0 as f64, b.y0 as f64, b.x1 as f64, b.y1 as f64];
    let session = app.ui.alloc_id();
    // Preview document: the moving pixels removed (layer hidden, or the selection lifted out).
    let mut pd = (*doc).clone();
    let mut lifted = None;
    // Type, shapes and smart objects move whole, selection or not (#2630): only pixels are lifted.
    if let Some(sel) = doc.selection.clone().filter(|_| !photocraft_engine::transform_cmds::moves_whole(layer) && layer.surface().is_some()) {
        if let Some(surf) = pd.layer_mut(id).and_then(|l| l.surface_mut()) {
            let (l, rest) = photocraft_engine::transform_cmds::split_selected(surf, &sel).map_err(|e| e.to_string())?;
            *surf = rest;
            lifted = Some(l);
        }
    } else if let Some(l) = pd.layer_mut(id) {
        l.visible = false;
    }
    // egui reports the renderer's limit in the app; offscreen harnesses only through the device.
    let max_side = ctx.input(|i| i.max_texture_side).max(app.gpu.as_ref().map_or(1, |g| g.max_texture_side()));
    // The layers covering the moving pixels draw over them instead of under them.
    let plan = AbovePlan::new(&doc, id);
    plan.hide_in(&mut pd.layers);
    // What the preview document changes: the moving layer and the covering layers it leaves
    // out, so the canvas redraws only there (`transform_tool::damage`), not the whole document.
    let canvas = doc.bounds();
    let area = std::iter::once(id)
        .chain(plan.ids.iter().copied())
        .map(|l| doc.layer(l).map_or(Some(canvas), |l| photocraft_compose::change_bounds(l, canvas)))
        .try_fold(photocraft_geom::Rect::EMPTY, |r, b| {
            let b = b?;
            Some(if r.is_empty() {
                b
            } else if b.is_empty() {
                r
            } else {
                r.union(&b)
            })
        });
    // A style is drawn again from the pixels as the box moves them, so they are read without it.
    // Not for the selected part of a layer: its style belongs to that part and the rest together.
    let styled = lifted.is_none() && photocraft_compose::effects::has_effects(layer);
    let opacity = layer.opacity * layer.fill_opacity;
    let whole = lifted.is_none();
    let allow = type_allow(matches!(layer.content, LayerContent::Text(_)));
    // A rotated, skewed or distorted smart object: the box opens on its placement (#3649), and the
    // preview's texels (its pixels as they are, over `rect`) move with the box from there.
    let opened = if frame_only { None } else { smart_start(&doc, layer, rect) };
    // Reading the moving pixels and compositing the layers over them is the heavy part: hundreds
    // of megapixels on a large canvas. Done on the UI thread, the window stopped answering for
    // long enough that a Wayland compositor dropped it, so a large layer prepares as a job
    // (with the jobs window's progress) and the box opens when it lands (`on_prepared`).
    let big = plan_pixels(&plan).saturating_add(b.size().area()) >= PREPARE_IN_BACKGROUND_PX;
    let source = doc.clone();
    let doc_id = doc.id;
    let selected = selected_now(app);
    let finish = move |app: &mut PhotocraftApp, prepared: Prepared| -> Result<(), String> {
        // The document is locked while preparing; still, never open a box on another state (or
        // in another tab). The revision is read again: the job itself may have moved it on.
        let Some(revision) = app.session.active().filter(|st| Arc::ptr_eq(&st.doc, &doc) && st.active_layer == Some(id)).map(|st| st.revision) else {
            return Err("Free Transform: the document changed while it was preparing".into());
        };
        // Nor over a layer the Layers panel no longer has selected.
        if selected_now(app) != selected {
            return Err("Free Transform: the layer selection changed while it was preparing".into());
        }
        let Prepared { plan, above, image: (image, uv) } = prepared;
        let above = Above::from_plan(max_side, plan, above);
        app.transform_area = area.map(|area| super::OpenArea { doc: doc.id, key: super::preview_key(session), revision, area });
        let textures = PreviewTextures::new(format!("transform-{session}"), PreviewPixels::new(image, uv));
        let shown = if frame_only { doc.clone() } else { Arc::new(pd) };
        app.transform_preview = Some(OpenPreview {
            opacity,
            above: (!frame_only).then_some(above),
            styled: (styled && !frame_only).then(|| StyledPreview::new(format!("transform-{session}-styled"))),
            follow: Some(Follow { revision, hidden: if whole && !frame_only { vec![id] } else { Vec::new() }, selected: selected_now(app) }),
            placed: opened.map(|(start, frame)| super::Placed { start, rect, frame }),
            ..OpenPreview::new(session, shown, textures, app.ui.tool)
        });
        app.ui.transform = Some(FreeTransform {
            session: match opened {
                Some((start, frame)) => open_session_at(app, frame, Some(start), allow, !frame_only),
                None => open_session(app, rect, allow, !frame_only),
            },
            id: session,
            layer: id.0,
            layers: Vec::new(),
            selection: false,
            target: frame_only.then(|| json!({"frame": id.0})),
            path: None,
            made: None,
            content_aware: None,
        });
        if !frame_only && whole {
            let start = opened.map_or([[rect[0], rect[1]], [rect[2], rect[1]], [rect[2], rect[3]], [rect[0], rect[3]]], |(q, _)| q);
            let warped = warped_preview(app, &doc, id, start, session);
            if let Some(pv) = app.transform_preview.as_mut() {
                pv.warped = warped;
            }
        }
        Ok(())
    };
    // `canceled` is asked between the two images, so a canceled job doesn't make the second.
    let prepare = move |canceled: &dyn Fn() -> bool| -> Option<Prepared> {
        let above = plan.image(max_side);
        if canceled() {
            return None;
        }
        Some(Prepared { above, image: preview_image(&source, id, lifted.as_ref(), b, max_side, styled), plan })
    };
    if !(big && app.background_jobs) {
        let prepared = prepare(&|| false).ok_or("Free Transform: nothing was prepared")?;
        return finish(app, prepared);
    }
    // The worker leaves its result here; the job's own result is only the jobs window's.
    let out = std::sync::Arc::new(std::sync::Mutex::new(None));
    let slot = out.clone();
    let started = app
        .session
        .start_job(
            "edit.freeTransform",
            json!({}),
            tl!("Preparing Free Transform"),
            true,
            move |job| {
                job.progress(0.0, "Reading the layer");
                let prepared = prepare(&|| job.cancelled());
                job.check()?;
                if let Ok(mut s) = slot.lock() {
                    *s = prepared;
                }
                Ok(())
            },
            |_, ()| Ok(json!({})),
        )
        .map_err(|e| e.to_string())?;
    match started {
        photocraft_engine::jobs::Started::Job(job) => {
            app.transform_preparing = Some(super::Preparing { job, out, finish: Box::new(finish), layer: (doc_id, id), made: None });
            Ok(())
        }
        // Ran inline after all (a session without workers).
        photocraft_engine::jobs::Started::Done(_) => {
            let prepared = out.lock().ok().and_then(|mut o| o.take()).ok_or("Free Transform: nothing was prepared")?;
            finish(app, prepared)
        }
    }
}

/// A layer box whose moving and covering pixels add up to this many prepares in the background
/// (16 MP: about a 4096 × 4096 layer, a few tenths of a second to read and composite).
const PREPARE_IN_BACKGROUND_PX: u64 = 16 << 20;

/// What a box's preview images are made from and the images themselves: the worker's result.
pub(crate) struct Prepared {
    plan: AbovePlan,
    above: Option<(egui::ColorImage, [f32; 2])>,
    image: (egui::ColorImage, [f32; 2]),
}

fn plan_pixels(plan: &AbovePlan) -> u64 {
    plan.area()
}

/// The preparing job of a large layer's box ended: open the box, or drop it (canceled, failed).
/// `true` when the event was that job's.
pub(crate) fn on_prepared(app: &mut PhotocraftApp, e: &photocraft_engine::jobs::JobEvent) -> bool {
    if app.transform_preparing.as_ref().is_none_or(|p| p.job != e.id) {
        return false;
    }
    let Some(p) = app.transform_preparing.take() else { return true };
    let (made, (document, layer)) = (p.made, p.layer);
    let prepared = if matches!(e.outcome, photocraft_engine::jobs::JobOutcome::Done(_)) { p.out.lock().ok().and_then(|mut o| o.take()) } else { None };
    let opened = match prepared.map(|prepared| (p.finish)(app, prepared)) {
        Some(Ok(())) => true,
        Some(Err(err)) => {
            crate::notices::error(app, err);
            false
        }
        None => false,
    };
    // As `mark_made` would have done at once, had the box opened inline.
    if let Some(made) = made {
        if opened && let Some(t) = app.ui.transform.as_mut() {
            t.made = Some(made);
        } else {
            take_back_made(app, document, layer);
        }
    }
    true
}

/// Mark the box `begin` opened, or the one it is still preparing, as made for `made`.
fn mark_made(app: &mut PhotocraftApp, made: MadeLayer) {
    if let Some(t) = app.ui.transform.as_mut() {
        t.made = Some(made);
    } else if let Some(p) = app.transform_preparing.as_mut() {
        p.made = Some(made);
    }
}

/// Free Transform of the selected layers `ids` together (#6501): the box frames them all, and the
/// preview moves their composite, drawn over the layers they cover. `active` stays the session's
/// `layer`, so picking another layer still applies the box. The selected pixels of each pixel
/// layer move when there is a selection; type, shapes, smart objects and groups move whole.
fn begin_several(app: &mut PhotocraftApp, ctx: &egui::Context, doc: Arc<Document>, revision: u64, active: LayerId, ids: Vec<LayerId>) -> Result<(), String> {
    use photocraft_engine::transform_cmds as tc;
    // The Background among them refuses the same way, rather than being left out (#11101).
    if tc::background_among(&doc, &ids).is_some() || ids.iter().any(|id| tc::locked_for_transform(&doc, *id)) {
        crate::dialogs::alerts::alert(app, tl!(LOCKED));
        return Err(LOCKED.into());
    }
    let b = tc::transform_frame_of(&doc, &ids);
    let top = *ids.last().ok_or("no layer")?;
    if b.is_empty() {
        return Err("Could not transform: the layers are empty".into());
    }
    crate::type_tool::commit(app);
    let rect = [b.x0 as f64, b.y0 as f64, b.x1 as f64, b.y1 as f64];
    let session = app.ui.alloc_id();
    // `under`: the canvas under the box, without the moving pixels. `moving`: those pixels alone.
    let mut under = (*doc).clone();
    let mut lifted = Vec::new();
    let mut hidden = Vec::new();
    for &id in &ids {
        let Some(l) = doc.layer(id) else { continue };
        match (doc.selection.as_ref().filter(|_| !tc::moves_whole(l)), l.surface()) {
            (Some(sel), Some(surf)) => {
                let (part, rest) = tc::split_selected(surf, sel).map_err(|e| e.to_string())?;
                if let Some(s) = under.layer_mut(id).and_then(|l| l.surface_mut()) {
                    *s = rest;
                }
                lifted.push((id, part));
            }
            _ => {
                if let Some(l) = under.layer_mut(id) {
                    l.visible = false;
                }
                hidden.push(id);
            }
        }
    }
    let max_side = ctx.input(|i| i.max_texture_side).max(app.gpu.as_ref().map_or(1, |g| g.max_texture_side()));
    let above = Above::new(&doc, top, max_side);
    above.hide_in(&mut under.layers);
    // Opacity, fill, masks and styles are in the composite already.
    let mut several =
        Several { name: format!("transform-{session}"), max_side, moving: Several::shown(&doc, &ids, &lifted), lifted, ids: ids.clone(), frame: b, rect: b };
    let textures = several.render();
    app.transform_preview = Some(OpenPreview {
        above: Some(above),
        follow: Some(Follow { revision, hidden, selected: selected_now(app) }),
        several: Some(several),
        ..OpenPreview::new(session, Arc::new(under), textures, app.ui.tool)
    });
    let typed = ids.iter().any(|id| doc.layer(*id).is_some_and(|l| matches!(l.content, LayerContent::Text(_))));
    // One box over several layers doesn't warp (#6501).
    app.ui.transform = Some(FreeTransform {
        session: open_session(app, rect, type_allow(typed), false),
        id: session,
        layer: active.0,
        layers: ids.iter().map(|id| id.0).collect(),
        selection: false,
        target: None,
        path: None,
        made: None,
        content_aware: None,
    });
    Ok(())
}

/// The moving texture's texel at document point `p` as the box opened (full resolution), and
/// where the texture's corners are drawn now: what a test of a box over several layers sees.
#[cfg(test)]
pub(crate) fn moving_texel(app: &PhotocraftApp, p: [i32; 2]) -> (Option<Color32>, [[f64; 2]; 4]) {
    let (Some(t), Some(pv)) = (&app.ui.transform, &app.transform_preview) else { return (None, [[0.0; 2]; 4]) };
    let a = t.rect_array();
    let r = pv.several.as_ref().map_or(photocraft_geom::Rect::new(a[0] as i32, a[1] as i32, a[2] as i32, a[3] as i32), |sv| sv.rect);
    let texel = pv.textures.level_image(0).and_then(|(image, _)| {
        let (x, y) = (usize::try_from(p[0] - r.x0).ok()?, usize::try_from(p[1] - r.y0).ok()?);
        (x < image.size[0] && y < image.size[1]).then(|| image.pixels[y * image.size[0] + x])
    });
    (texel, texture_quad(t, pv).map(|p| [p.x, p.y]))
}

/// The Layers panel's selection, sorted, as a layer's box records it when it opens (#11101).
pub(super) fn selected_now(app: &PhotocraftApp) -> Option<Vec<LayerId>> {
    let mut ids = app.session.active()?.selected_layers().to_vec();
    ids.sort_unstable();
    Some(ids)
}

/// The layers the box moves: every layer of a several-layer session (#6501), else its one layer.
pub(crate) fn moving_layers(t: &FreeTransform) -> Vec<LayerId> {
    if t.layers.is_empty() { vec![LayerId(t.layer)] } else { t.layers.iter().copied().map(LayerId).collect() }
}

/// Is any of the layers the box moves of this kind?
pub(super) fn moves_kind(app: &PhotocraftApp, t: &FreeTransform, kind: fn(&LayerContent) -> bool) -> bool {
    app.session.active().is_some_and(|st| moving_layers(t).iter().any(|id| st.doc.layer(*id).is_some_and(|l| kind(&l.content))))
}

/// Why Warp refuses with several layers selected.
pub const WARP_ONE_LAYER: &str = "Warp works on one layer at a time: select a single layer";

/// Free Transform on a copy (⌥⌘T, no menu item): duplicates the active layer (with a selection,
/// its selected pixels, as Layer via Copy) and transforms the copy (#352).
pub fn begin_copy(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<(), String> {
    let selection = app.session.active().is_some_and(|d| d.doc.selection.is_some());
    // A linked layer without a selection: the reference application makes no copy and frames the
    // whole link group, as Free Transform does (measured, #11217). Unlinked layers get the copy,
    // and so does a targeted mask (a lone target).
    let target = crate::canvas::paint_target(app);
    let linked = !selection
        && app.session.active().is_some_and(|st| {
            let lone = photocraft_engine::transform_cmds::lone_target(&st.doc, st.active_layer, &json!({ "target": target }));
            let targets = photocraft_engine::transform_cmds::selection_targets(st);
            matches!(lone, Ok(None)) && photocraft_engine::transform_cmds::box_targets(&st.doc, &targets) != targets
        });
    if linked {
        return begin(app, ctx);
    }
    let params = if selection { json!({}) } else { json!({"inPlace": true}) };
    app.run(if selection { "layer.new.layerViaCopy" } else { "layer.duplicate" }, params)?;
    let made = app.session.active().and_then(|st| st.active_layer.map(|layer| (st.doc.id, layer)));
    if let Err(e) = begin(app, ctx) {
        if let Some((document, layer)) = made {
            take_back_made(app, document, layer);
        }
        return Err(e);
    }
    mark_made(app, MadeLayer::Copy);
    Ok(())
}

/// Free Transform on the layer a file dropped on the canvas just placed, like the reference app's
/// Place: Esc takes the place back, ↩ makes the place and the transform one Place Embedded step.
/// With Preferences ▸ General ▸ Skip Transform when Placing on, the place stands as it is.
pub fn begin_placed(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<(), String> {
    if app.session.prefs().general.skip_transform_when_placing {
        return Ok(());
    }
    begin(app, ctx)?;
    mark_made(app, MadeLayer::Place);
    Ok(())
}

/// Edit › Content-Aware Scale (#3096): a scale-only box over `region` of the active pixel layer
/// (the selection's bounds, else the layer's content), cleared under the box as the command
/// clears it. OK runs `edit.contentAwareScale` (cas_transform.rs) instead of `edit.transform`.
pub(crate) fn begin_content_aware(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    region: photocraft_geom::Rect,
    live: crate::cas_transform::Live,
) -> Result<(), String> {
    let st = app.session.active().ok_or("no document")?;
    let (doc, revision) = (st.doc.clone(), st.revision);
    let id = st.active_layer.ok_or("no active layer")?;
    let layer = doc.layer(id).ok_or("no layer")?;
    let surf = layer.surface().ok_or("Content-Aware Scale needs a pixel layer")?;
    if region.is_empty() {
        return Err("Could not scale: the layer is empty".into());
    }
    crate::type_tool::commit(app);
    let rect = [region.x0 as f64, region.y0 as f64, region.x1 as f64, region.y1 as f64];
    let session = app.ui.alloc_id();
    // The command vacates the area before writing the result, as Delete does: transparent, or on
    // the Background the background colour (a layer without transparency otherwise keeps it).
    let background = doc.is_background(id).then_some(app.session.tools.background);
    let mut pd = (*doc).clone();
    if let Some(s) = pd.layer_mut(id).and_then(|l| l.surface_mut()).filter(|s| background.is_some() || s.format().alpha) {
        let fmt = s.format();
        let mut fill = vec![0.0; fmt.channels()];
        if let Some([r, g, b, _]) = background {
            let mut enc = [0.0f32; 8];
            let n = photocraft_raster::from_rgba_into(&fmt, [r, g, b, 1.0], &mut enc);
            if let (Some(dst), Some(src)) = (fill.get_mut(..n), enc.get(..n)) {
                dst.copy_from_slice(src);
            }
        }
        s.fill_rect(region, &fill);
        s.prune();
    }
    let max_side = ctx.input(|i| i.max_texture_side).max(app.gpu.as_ref().map_or(1, |g| g.max_texture_side()));
    let above = Above::new(&doc, id, max_side);
    above.hide_in(&mut pd.layers);
    let textures = PreviewTextures::new(format!("transform-{session}"), super::preview::read_surface(surf, region, max_side));
    app.transform_preview = Some(OpenPreview {
        opacity: layer.opacity * layer.fill_opacity,
        above: Some(above),
        follow: Some(Follow { revision, hidden: Vec::new(), selected: None }),
        content_aware: Some(live),
        ..OpenPreview::new(session, Arc::new(pd), textures, app.ui.tool)
    });
    // Content-Aware Scale only scales: no rotating, skewing, distorting or warping.
    app.ui.transform = Some(FreeTransform {
        session: open_session(app, rect, BoxAllow::new(false, false, false), false),
        id: session,
        layer: id.0,
        layers: Vec::new(),
        selection: false,
        target: None,
        path: None,
        made: None,
        content_aware: Some(Default::default()),
    });
    Ok(())
}

/// Undoes the layer a cancelled or failed session made (⌥⌘T's copy, a placed file), leaving
/// nothing to redo.
pub(super) fn take_back_made(app: &mut PhotocraftApp, document: DocId, layer: LayerId) {
    // Tabs can move or close while transforming. Only undo the creation in its own document,
    // and only while it is still the latest step (it may already have been undone).
    let Some(index) = app.session.documents().iter().position(|st| {
        st.doc.id == document
            && st.doc.layer(layer).is_some()
            && st
                .history
                .recorded_before()
                .or_else(|| st.history.past_len().checked_sub(1).and_then(|i| st.history.state(i)))
                .is_some_and(|before| before.layer(layer).is_none())
    }) else {
        return;
    };
    let active = app.session.active_index();
    app.session.set_active(index);
    // Exactly, also in non-linear history (where an undo would land on a later state).
    app.session.rollback();
    if let Some(active) = active {
        app.session.set_active(active);
    }
    app.sync_views();
}

/// After the transform: the step that made the layer and the transform become one history step,
/// the transform's for a copy and Place Embedded for a placed file.
pub(super) fn fold_made(app: &mut PhotocraftApp, document: DocId, layer: LayerId, made: MadeLayer) {
    // A background commit lands in its own document, which need not be the active one by then.
    let Some(index) = app.session.documents().iter().position(|st| st.doc.id == document) else { return };
    let active = app.session.active_index();
    app.session.set_active(index);
    // Only when the step before the transform made the layer: a step taken while the box was
    // open (New Layer, which also ends the box, #5667) stays its own, as does the made layer's.
    if let Some(st) = app.session.active_mut()
        && st.history.past_len().checked_sub(2).and_then(|i| st.history.state(i)).is_some_and(|before| before.layer(layer).is_none())
    {
        st.history.purge_last();
        if made == MadeLayer::Place {
            st.history.set_current_label(photocraft_engine::file_cmds::PLACE_EMBEDDED);
        }
    }
    if let Some(active) = active {
        app.session.set_active(active);
    }
}

/// Free Transform of a targeted unlinked layer mask, alpha channel or Quick Mask by itself: the
/// box frames its content (or the selection) and previews the moving values in grey over the
/// document with them vacated.
fn begin_lone(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    doc: Arc<Document>,
    surf: photocraft_raster::Surface,
    target: serde_json::Value,
) -> Result<(), String> {
    use photocraft_engine::transform_cmds as tc;
    let b = tc::target_frame(&doc, &surf);
    if b.is_empty() {
        return Err("Could not transform: nothing is selected".into());
    }
    let whole;
    let sel = match &doc.selection {
        Some(s) => s,
        None => {
            let mut s = photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8);
            s.fill_rect(b, &[1.0]);
            whole = s;
            &whole
        }
    };
    let (lifted, rest) = tc::split_gray_selected(&surf, sel).map_err(|e| e.to_string())?.ok_or("this channel can't be transformed")?;
    crate::type_tool::commit(app);
    let layer = app.session.active().and_then(|st| st.active_layer).map_or(0, |l| l.0);
    let mut pd = (*doc).clone();
    let tp = json!({ "target": target });
    if let Ok(Some(s)) = tc::lone_target_mut(&mut pd, Some(LayerId(layer)), &tp) {
        *s = rest;
    }
    let rect = [b.x0 as f64, b.y0 as f64, b.x1 as f64, b.y1 as f64];
    let session = app.ui.alloc_id();
    let max_side = ctx.input(|i| i.max_texture_side).max(app.gpu.as_ref().map_or(1, |g| g.max_texture_side()));
    let textures = PreviewTextures::new(format!("transform-{session}"), super::preview::read_surface(&lifted, b, max_side));
    app.transform_preview = Some(OpenPreview { opacity: 0.6, ..OpenPreview::new(session, Arc::new(pd), textures, app.ui.tool) });
    // A lone mask or channel keeps the box: it does not warp.
    app.ui.transform = Some(FreeTransform {
        session: open_session(app, rect, BoxAllow::default(), false),
        id: session,
        layer,
        layers: Vec::new(),
        selection: false,
        target: Some(target),
        path: None,
        made: None,
        content_aware: None,
    });
    Ok(())
}

/// Edit › Free Transform Path: the box frames the path's anchors and handles and draws the
/// path through it; OK commits one `path.transform` step.
fn begin_path(app: &mut PhotocraftApp, params: serde_json::Value, path: photocraft_doc::vector::Path) -> Result<(), String> {
    let (x0, y0, x1, y1) = path.control_bounds().ok_or("the path has no points to transform")?;
    let st = app.session.active().ok_or("no document")?;
    let doc = st.doc.clone();
    let layer = st.active_layer.or_else(|| doc.layers.first().map(|l| l.id)).ok_or("no layer")?;
    crate::type_tool::commit(app);
    // A straight horizontal or vertical path still gets a box to turn.
    let rect = [x0, y0, x1.max(x0 + 1.0), y1.max(y0 + 1.0)];
    let session = app.ui.alloc_id();
    let textures =
        PreviewTextures::new(format!("transform-{session}"), PreviewPixels::new(egui::ColorImage::new([1, 1], vec![Color32::TRANSPARENT]), [1.0, 1.0]));
    app.transform_preview = Some(OpenPreview { path: Some(path), ..OpenPreview::new(session, doc, textures, app.ui.tool) });
    app.ui.transform = Some(FreeTransform {
        session: open_session(app, rect, BoxAllow::default(), true),
        id: session,
        layer: layer.0,
        layers: Vec::new(),
        selection: false,
        target: None,
        path: Some(params),
        made: None,
        content_aware: None,
    });
    Ok(())
}

/// The affine map taking `rect`'s corners to `quad`'s; `None` when the box was distorted out of a
/// parallelogram (Distort, Perspective): the path then goes as a `quad`.
pub(super) fn rect_to_quad_affine(r: [f64; 4], q: [[f64; 2]; 4]) -> Option<photocraft_geom::Affine> {
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let tol = 1e-3 * (1.0 + w.max(h));
    if (q[0][0] + q[2][0] - q[1][0] - q[3][0]).abs() > tol || (q[0][1] + q[2][1] - q[1][1] - q[3][1]).abs() > tol {
        return None;
    }
    let (a, b) = ((q[1][0] - q[0][0]) / w, (q[1][1] - q[0][1]) / w);
    let (c, d) = ((q[3][0] - q[0][0]) / h, (q[3][1] - q[0][1]) / h);
    let m = [a, b, c, d, q[0][0] - a * r[0] - c * r[1], q[0][1] - b * r[0] - d * r[1]];
    m.iter().all(|v| v.is_finite()).then_some(photocraft_geom::Affine { m })
}

/// Start Select › Transform Selection: the same box over the selection's bounds, previewing the
/// selection mask (the marching ants are hidden meanwhile); commits `select.transformSelection`.
pub fn begin_selection(app: &mut PhotocraftApp, _ctx: &egui::Context) -> Result<(), String> {
    let st = app.session.active().ok_or("no document")?;
    let doc = st.doc.clone();
    let sel = doc.selection.clone().ok_or("no selection")?;
    let b = sel.content_bounds();
    if b.is_empty() {
        return Err("no selection".into());
    }
    let layer = st.active_layer.or_else(|| doc.layers.first().map(|l| l.id)).ok_or("no layer")?;
    crate::type_tool::commit(app);
    let rect = [b.x0 as f64, b.y0 as f64, b.x1 as f64, b.y1 as f64];
    let session = app.ui.alloc_id();
    let (w, h) = (b.width() as usize, b.height() as usize);
    let k = w.max(h).div_ceil(2048).max(1);
    let (tw, th) = (w.div_ceil(k), h.div_ceil(k));
    let mut px = vec![Color32::TRANSPARENT; tw * th];
    for ty in 0..th {
        for tx in 0..tw {
            let m = sel.sample_channel(b.x0 + (tx * k) as i32, b.y0 + (ty * k) as i32, 0).clamp(0.0, 1.0);
            px[ty * tw + tx] = Color32::from_rgba_unmultiplied(120, 170, 255, (m * 110.0) as u8);
        }
    }
    let uv = [w as f32 / (tw * k) as f32, h as f32 / (th * k) as f32];
    let textures = PreviewTextures::new(format!("transform-{session}"), PreviewPixels::new(egui::ColorImage::new([tw, th], px), uv));
    let mut pd = (*doc).clone();
    pd.selection = None;
    app.transform_preview = Some(OpenPreview::new(session, Arc::new(pd), textures, app.ui.tool));
    app.ui.transform = Some(FreeTransform {
        session: open_session(app, rect, BoxAllow::default(), true),
        id: session,
        layer: layer.0,
        layers: Vec::new(),
        selection: true,
        target: None,
        path: None,
        made: None,
        content_aware: None,
    });
    Ok(())
}

/// Reports `warp` to the open session: it enters the Warp phase as one undo step with the box. The session reads
/// `now` only for a box nudge, so the app's latest frame time is the one clock.
fn report_warp(app: &mut PhotocraftApp, warp: Warp, on_type: bool, stored: bool) {
    let now = app.nudge_bundle.now;
    let placed = app.transform_preview.as_ref().and_then(|pv| pv.placed_warp);
    if let Some(t) = app.ui.transform.as_mut() {
        let mut warp = SessionWarp::new(warp, on_type);
        warp.placed = placed;
        t.session.apply(TransformEdit::Warp(if stored { warp.stored() } else { warp }), now);
    }
}

/// Start (or switch an active Free Transform into) Warp mode.
pub fn begin_warp(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<(), String> {
    if app.ui.transform.as_ref().is_some_and(|t| t.content_aware.is_some()) {
        return Err(CAS_ONLY_SCALES.into());
    }
    if app.ui.transform.as_ref().is_some_and(|t| !t.layers.is_empty()) {
        return Err(WARP_ONE_LAYER.into());
    }
    if app.ui.transform.is_none() {
        begin(app, ctx)?;
        if app.ui.transform.as_ref().is_some_and(|t| !t.layers.is_empty()) {
            cancel(app);
            return Err(WARP_ONE_LAYER.into());
        }
    }
    enter_warp(app, ctx);
    Ok(())
}

/// Switch to Warp mode: a smart object's existing warp (when the box is untouched), else a
/// custom mesh following the current box (so a transform made first carries over). Editable type
/// gets its own Warp Text style instead ([`enter_type_warp`]). This answers
/// `TransformRequest::NeedWarp` of the session's `WarpRequested`.
pub fn enter_warp(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(t) = app.ui.transform.as_ref() else { return };
    // Warp moves layers and paths (#3027): a lone mask or channel keeps the box, and so do
    // several layers (#6501) and Content-Aware Scale; their content allows no warp.
    if !t.session.allows().warp || t.session.warp().is_some() {
        return;
    }
    // Back from a box mode with the warp held: carry on with it as the box has moved it.
    if let Some(held) = app.transform_preview.as_mut().and_then(|pv| pv.held.take()) {
        let quad = app.ui.transform.as_ref().map_or(held.start, |t| t.quad_array());
        // A smart object's warp held in the document (its box was upright when Warp began) goes on in its contents
        // once the box has moved, through its placement and the move, as any moved smart object's does.
        let held = match (held.placed, contents_of_held(app, &held, quad)) {
            (None, Some(placed)) => placed,
            _ => held,
        };
        let (warp, placed) = held.given_way(quad);
        if let Some(pv) = app.transform_preview.as_mut() {
            if let Some(texture) = held.texture {
                pv.other_texels = Some(OtherTexels { texture: std::mem::replace(&mut pv.textures, texture), placed_warp: None });
            }
            pv.placed_warp = placed;
        }
        report_warp(app, warp, false, false);
        return;
    }
    let Some(t) = app.ui.transform.as_ref() else { return };
    let (layer, untouched) = (t.layer, t.session.is_untouched() && !t.selection && t.path.is_none());
    if warps_type(app) {
        return enter_type_warp(app, ctx);
    }
    let existing = app.session.active().and_then(|d| d.doc.layer(LayerId(layer)).and_then(photocraft_engine::warp_cmds::smart_warp_doc_space));
    // Texels kept for the Redo of an undone Warp on a placed smart object (its map from the contents with them) are
    // dead once Warp begins afresh: this is a new step, and the box may have moved since.
    if let Some(pv) = app.transform_preview.as_mut()
        && pv.other_texels.as_ref().is_some_and(|o| o.placed_warp.is_some())
    {
        pv.other_texels = None;
    }
    // Back into Warp mode after leaving it: the texels it showed are kept.
    if untouched
        && let Some(w) = &existing
        && app.transform_preview.as_ref().is_some_and(|pv| pv.other_texels.is_some())
    {
        if let Some(pv) = app.transform_preview.as_mut() {
            swap_mode_texels(pv);
        }
        report_warp(app, w.clone(), false, true);
        return;
    }
    // The stored warp is edited only when the source can be previewed without it: the texels
    // the box opened with have the warp in them, and the mesh would warp them again.
    if untouched
        && let Some(w) = existing
        && preview_without_smart_warp(app, ctx, w.bounds)
    {
        report_warp(app, w, false, true);
        return;
    }
    if enter_placed_warp(app, ctx) {
        return;
    }
    // A warp of the box as it is: the texels shown are the box's own, so nothing an earlier Warp mode left for an Undo
    // across it (its texels, its map from the contents) belongs to this one.
    if let Some(pv) = app.transform_preview.as_mut() {
        pv.other_texels = None;
        pv.placed_warp = None;
    }
    let Some(t) = app.ui.transform.as_ref() else { return };
    let (rect, quad) = (t.rect_array(), t.quad_array());
    let h = Homography::rect_to_quad(rect, quad).unwrap_or(Homography::IDENTITY);
    let mesh = BezierMesh::identity(rect, 1, 1).map_points(|p| {
        let (x, y) = h.apply(p[0], p[1]);
        [x, y]
    });
    report_warp(app, Warp::custom(mesh, rect), false, false);
}

/// Whether the active layer is warped: a Warp box is open on it, or it is a smart object with a
/// stored warp, which Edit › Transform › Warp then opens for editing (its menu item is checked).
pub fn warps_active_layer(app: &PhotocraftApp) -> bool {
    if app.ui.transform.as_ref().is_some_and(|t| t.session.warp().is_some()) {
        return true;
    }
    let Some(st) = app.session.active() else { return false };
    st.active_layer
        .and_then(|id| st.doc.layer(id))
        .is_some_and(|l| matches!(&l.content, LayerContent::Smart(sm) if sm.warp.as_ref().is_some_and(|w| !w.is_identity())))
}

/// The preview texels for editing a smart object's stored warp: its source placed without the
/// warp, over the warp box `r`, so the box's mesh warps it once. The texels read when the box
/// opened are the layer's render, which already has the warp: warping that again drew the
/// picture well inside the grid (#9798). Those texels are kept as the other mode's
/// ([`OtherTexels`]), for leaving Warp mode. False when the source can't be re-rendered (a
/// missing linked file): the texels keep the warp, so the caller must not adopt it.
fn preview_without_smart_warp(app: &mut PhotocraftApp, ctx: &egui::Context, r: [f64; 4]) -> bool {
    let (Some(t), Some(st)) = (app.ui.transform.as_ref(), app.session.active()) else { return false };
    let (id, session) = (LayerId(t.layer), t.id);
    let doc = st.doc.clone();
    let Some(LayerContent::Smart(sm)) = doc.layer(id).map(|l| &l.content) else { return false };
    let mut flat = sm.clone();
    flat.warp = None;
    let Ok(Some(surface)) = photocraft_engine::smart_cmds::render(&doc, &flat) else { return false };
    flat.cache = Some(surface);
    let mut plain = (*doc).clone();
    if let Some(l) = plain.layer_mut(id) {
        l.content = LayerContent::Smart(flat);
    }
    let b = photocraft_geom::Rect::new(r[0].round() as i32, r[1].round() as i32, r[2].round() as i32, r[3].round() as i32);
    if b.is_empty() {
        return false;
    }
    let max_side = ctx.input(|i| i.max_texture_side).max(app.gpu.as_ref().map_or(1, |g| g.max_texture_side()));
    let Some(pv) = app.transform_preview.as_mut() else { return false };
    let (image, uv) = preview_image(&plain, id, None, b, max_side, pv.styled.is_some());
    let unwarped = PreviewTextures::new(format!("transform-{session}-unwarped"), PreviewPixels::new(image, uv));
    pv.other_texels = Some(OtherTexels { texture: std::mem::replace(&mut pv.textures, unwarped), placed_warp: None });
    true
}

/// The open transform moves an editable type layer as a whole (not a lone mask, channel, path or
/// the selection). Its Warp is Warp Text: only the styles, and the type stays editable (#3004).
pub fn warps_type(app: &PhotocraftApp) -> bool {
    let Some(t) = &app.ui.transform else { return false };
    if t.selection || t.target.is_some() || t.path.is_some() {
        return false;
    }
    app.session.active().and_then(|st| st.doc.layer(LayerId(t.layer))).is_some_and(|l| matches!(l.content, LayerContent::Text(_)))
}

/// Warp on editable type, as in Photoshop: no rasterizing, no mesh to drag, just the warp styles
/// of the options bar (Warp Text's). A box scaled or rotated first is applied first (that keeps
/// type editable). The preview shows the type without its current Warp Text, under that style.
fn enter_type_warp(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.ui.transform.as_ref().is_some_and(|t| !t.session.is_untouched()) {
        commit(app);
        if app.ui.transform.is_some() {
            return; // refused: the box stays as it was
        }
        if let Err(e) = begin(app, ctx) {
            app.ui.status = e;
            return;
        }
    }
    let (Some(t), Some(st)) = (app.ui.transform.as_ref(), app.session.active()) else { return };
    let (id, session) = (LayerId(t.layer), t.id);
    let doc = st.doc.clone();
    let Some(LayerContent::Text(text)) = doc.layer(id).map(|l| &l.content) else { return };
    let current = text.warp.clone();
    let mut plain = (*doc).clone();
    if let Some(LayerContent::Text(tt)) = plain.layer_mut(id).map(|l| &mut l.content)
        && current.is_some()
    {
        tt.warp = None;
        photocraft_engine::type_cmds::refresh(&doc, tt);
    }
    let Some(b) = plain.layer(id).map(|l| photocraft_engine::transform_cmds::transform_bounds(&plain, l)).filter(|b| !b.is_empty()) else { return };
    let rect = [b.x0 as f64, b.y0 as f64, b.x1 as f64, b.y1 as f64];
    if current.is_some()
        && let Some(pv) = app.transform_preview.as_mut()
    {
        let max_side = ctx.input(|i| i.max_texture_side).max(app.gpu.as_ref().map_or(1, |g| g.max_texture_side()));
        // Read as the session's texels were: without the style when the box draws it itself.
        let (image, uv) = preview_image(&plain, id, None, b, max_side, pv.styled.is_some());
        pv.textures = PreviewTextures::new(format!("transform-{session}"), PreviewPixels::new(image, uv));
    }
    let warp =
        current.as_ref().and_then(|w| WarpStyle::parse(&w.style).filter(|s| s.is_preset()).map(|s| (w, s))).map_or(Warp::none(rect), |(w, style)| Warp {
            style,
            bend: f64::from(w.value),
            h_distort: f64::from(w.horizontal_distortion),
            v_distort: f64::from(w.vertical_distortion),
            vertical: !w.horizontal,
            ..Warp::none(rect)
        });
    // PLAN: the session's frame stays the box's; the warp's bounds carry the frame without the old Warp Text, and
    // the preview and the commit read `warp.bounds`.
    report_warp(app, warp, true, false);
}

/// Leave Warp mode (back to the box; warp edits are dropped, like Photoshop's mode toggle on an
/// untouched warp).
pub fn leave_warp(app: &mut PhotocraftApp) {
    let now = app.nudge_bundle.now;
    if let Some(t) = app.ui.transform.as_mut()
        && t.session.warp().is_some()
    {
        t.session.apply(TransformEdit::LeaveWarp, now);
        // A stored warp's edit showed the source without it: the box shows the render with it.
        if let Some(pv) = app.transform_preview.as_mut() {
            swap_mode_texels(pv);
            pv.placed_warp = None;
        }
    }
    if let Some(pv) = app.transform_preview.as_mut() {
        pv.split = None;
    }
}

/// Where Free Transform's box opens on smart object `layer`, and the frame it measures from: its
/// placement's corners, unless they are (to the pixel) the upright `rect` the box would open on
/// anyway at the contents' own size. The frame is the contents' own size, centred on the
/// placement, so W and H read the placement's total scale against the original contents and a
/// typed 100 % brings them back to it (as the reference does); a warped smart object's frame is
/// its box at 100 % ([`upright_frame`]). Not with a selection, whose box is the selection's (#5297).
fn smart_start(doc: &Document, layer: &photocraft_doc::Layer, rect: [f64; 4]) -> Option<([[f64; 2]; 4], [f64; 4])> {
    if doc.selection.is_some() {
        return None;
    }
    let q = photocraft_engine::warp_cmds::smart_box(doc, layer)?;
    Homography::quad_to_quad(q, q)?;
    let warped = matches!(&layer.content, LayerContent::Smart(sm) if sm.warp.as_ref().is_some_and(|w| !w.is_identity()));
    let size = photocraft_engine::warp_cmds::smart_contents_size(doc, layer).filter(|_| !warped);
    let frame = match size {
        Some([w, h]) => {
            let (cx, cy) = (q.iter().map(|p| p[0]).sum::<f64>() / 4.0, q.iter().map(|p| p[1]).sum::<f64>() / 4.0);
            [cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0]
        }
        None => upright_frame(q),
    };
    let corners = |r: [f64; 4]| [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]];
    let upright = q.iter().zip(corners(rect)).all(|(a, b)| (a[0] - b[0]).abs() <= 1.0 && (a[1] - b[1]).abs() <= 1.0);
    let full_size = q.iter().zip(corners(frame)).all(|(a, b)| (a[0] - b[0]).abs() <= 0.5 && (a[1] - b[1]).abs() <= 0.5);
    (!(upright && full_size)).then_some((q, frame))
}

/// An upright frame for a box opening on quad `q`: as wide as its top edge and as tall as its
/// height across it, centred on it, so `q` reads as W and H 100 % with the quad's own angle and
/// skew (the reference: a smart object's earlier rotation shows; its size is the 100 % one).
fn upright_frame(q: [[f64; 2]; 4]) -> [f64; 4] {
    let (ex, ey) = (q[1][0] - q[0][0], q[1][1] - q[0][1]);
    let w = ex.hypot(ey);
    let h = if w > 1e-9 { ((q[3][0] - q[0][0]) * ey - (q[3][1] - q[0][1]) * ex).abs() / w } else { 0.0 };
    let (cx, cy) = (q.iter().map(|p| p[0]).sum::<f64>() / 4.0, q.iter().map(|p| p[1]).sum::<f64>() / 4.0);
    let (w, h) = (w.max(1e-3), h.max(1e-3));
    [cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0]
}

/// Warp mode on a box that opened on a smart object's placement (#3649): the smart object's warp (its stored one,
/// else none over its contents) is edited in the contents' own space and shown through the placement and the box's
/// move so far (#8927). Its control points are carried into the document, which is exact under an affine placement
/// and the reference's own rule under a projective one; the preview shows the contents unplaced, which the mesh
/// places once. The commit stores the warp from those points over the contents (`edit.transform.warp`). False when
/// the box didn't open on a placement or the contents can't be read: the caller warps the box as it is.
fn enter_placed_warp(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    let (Some(t), Some(pv)) = (app.ui.transform.as_ref(), app.transform_preview.as_ref()) else { return false };
    if t.selection || t.path.is_some() || !t.layers.is_empty() || t.target.is_some() {
        return false;
    }
    // Where the box started: on the placement, or on the frame of an upright smart object. An upright one whose box
    // hasn't moved has its warp edited in the document, where it is the same warp ([`enter_warp`]'s stored paths).
    let quad = t.quad_array();
    let (start, opened) = match pv.placed {
        Some(placed) => (placed.start, true),
        None => {
            let r = t.rect_array();
            ([[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]], false)
        }
    };
    let Some(doc) = app.session.active().map(|st| st.doc.clone()) else { return false };
    let Some(layer) = doc.layer(LayerId(t.layer)).filter(|l| matches!(l.content, LayerContent::Smart(_))) else { return false };
    if !opened && quad == start {
        return false;
    }
    let Some((source, place)) = photocraft_engine::warp_cmds::smart_warp_source(&doc, layer) else { return false };
    let Some(moved) = Homography::quad_to_quad(start, quad) else { return false };
    let to_doc = moved.mul(&place);
    let Some((contents, k)) = photocraft_engine::warp_cmds::smart_unplaced_contents(&doc, layer) else { return false };
    let d = source.bounds.map(|v| v * k);
    let b = photocraft_geom::Rect::new(d[0].floor() as i32, d[1].floor() as i32, d[2].ceil() as i32, d[3].ceil() as i32);
    if b.is_empty() {
        return false;
    }
    let max_side = ctx.input(|i| i.max_texture_side).max(app.gpu.as_ref().map_or(1, |g| g.max_texture_side()));
    let texture = PreviewTextures::new(format!("transform-{}-contents", t.id), super::preview::read_surface(&contents, b, max_side));
    let mesh = source.to_mesh(1, 1).map_points(|p| {
        let (x, y) = to_doc.apply(p[0], p[1]);
        [x, y]
    });
    // The stored warp, not yet changed: committing it as it is changes nothing.
    let stored = quad == start && matches!(&layer.content, LayerContent::Smart(sm) if sm.warp.is_some());
    if let Some(pv) = app.transform_preview.as_mut() {
        pv.other_texels = Some(OtherTexels { texture: std::mem::replace(&mut pv.textures, texture), placed_warp: None });
        pv.placed_warp = Some((to_doc.0, quad != start));
    }
    report_warp(app, Warp::custom(mesh, source.bounds), false, stored);
    true
}

/// Edit › Transform's box modes chosen in Warp mode: the warp is held ([`super::HeldWarp`]) and the box comes back
/// where it was before Warp, as the reference does; the warp then shows moving with the box, Warp mode carries on
/// with it and the commit applies both. Warp Text has no box of its own to hold it under: it leaves Warp as before.
pub fn hold_warp(app: &mut PhotocraftApp) {
    let now = app.nudge_bundle.now;
    let placed_before = app.transform_preview.as_ref().and_then(|pv| pv.placed_warp);
    let doc = app.session.active().map(|st| st.doc.clone());
    let Some(t) = app.ui.transform.as_mut() else { return };
    let Some(w) = t.session.warp() else { return };
    let (warp, on_type, stored) = (w.warp.clone(), w.on_type, w.stored);
    t.session.apply(TransformEdit::LeaveWarp, now);
    let start = t.quad_array();
    // An untouched warp is no warp: the reference shows and applies the box mode's picture exactly as without it, also
    // when Warp mode comes back later. It is left as Warp's own toggle leaves it, not held.
    let flat = !on_type
        && !stored
        && match placed_before {
            Some(_) => doc.as_deref().is_some_and(|doc| super::warp::flat_placed_warp(doc, t.layer, &warp, placed_before).is_some()),
            None => super::warp::flat_in_box(&warp, t.rect_array(), start),
        };
    let Some(pv) = app.transform_preview.as_mut() else { return };
    pv.split = None;
    if flat {
        swap_mode_texels(pv);
        pv.placed_warp = None;
        return;
    }
    // The box shows the layer's own texels again; the warp keeps the ones it maps (a smart object's source).
    let texture = pv.other_texels.take().map(|o| std::mem::replace(&mut pv.textures, o.texture));
    let placed = pv.placed_warp.take();
    if !on_type {
        pv.held = Some(super::HeldWarp { warp, start, placed, texture });
    }
}

/// `held`, a smart object's warp held in the document, as one held in its contents: its points taken back through the
/// smart object's placement, which becomes the held map to the document. `None` when the box hasn't moved from where
/// the warp was held, the layer is no smart object, or the placement can't be inverted.
fn contents_of_held(app: &PhotocraftApp, held: &super::HeldWarp, quad: [[f64; 2]; 4]) -> Option<super::HeldWarp> {
    if quad == held.start {
        return None;
    }
    let t = app.ui.transform.as_ref()?;
    if t.selection || t.path.is_some() || !t.layers.is_empty() || t.target.is_some() {
        return None;
    }
    let doc = app.session.active()?.doc.clone();
    let layer = doc.layer(LayerId(t.layer)).filter(|l| matches!(l.content, LayerContent::Smart(_)))?;
    let (source, place) = photocraft_engine::warp_cmds::smart_warp_source(&doc, layer)?;
    let back = place.inverse()?;
    let mesh = held.warp.to_mesh(1, 1).map_points(|p| {
        let (x, y) = back.apply(p[0], p[1]);
        [x, y]
    });
    // The points as the session holds them: in the document (carried through the placement), over the contents' box.
    let doc_mesh = mesh.map_points(|p| {
        let (x, y) = place.apply(p[0], p[1]);
        [x, y]
    });
    Some(super::HeldWarp {
        warp: Warp::custom(doc_mesh, source.bounds),
        start: held.start,
        placed: Some((place.0, false)),
        texture: Some(contents_texture(app, &doc, layer, &source, t.id)?),
    })
}

/// The preview texels of a smart object's contents, unplaced, over its warp's box `source`.
fn contents_texture(app: &PhotocraftApp, doc: &Document, layer: &photocraft_doc::Layer, source: &Warp, session: u64) -> Option<PreviewTextures> {
    let (contents, k) = photocraft_engine::warp_cmds::smart_unplaced_contents(doc, layer)?;
    let d = source.bounds.map(|v| v * k);
    let b = photocraft_geom::Rect::new(d[0].floor() as i32, d[1].floor() as i32, d[2].ceil() as i32, d[3].ceil() as i32);
    if b.is_empty() {
        return None;
    }
    let max_side = app.ctx().input(|i| i.max_texture_side).max(app.gpu.as_ref().map_or(1, |g| g.max_texture_side()));
    Some(PreviewTextures::new(format!("transform-{session}-contents"), super::preview::read_surface(&contents, b, max_side)))
}

/// The stored warp of smart object `id` for the box's preview under a projective move ([`super::WarpedPreview`]);
/// `None` for any other layer, an identity warp, or contents that can't be rendered.
fn warped_preview(app: &PhotocraftApp, doc: &Document, id: LayerId, start: [[f64; 2]; 4], session: u64) -> Option<super::WarpedPreview> {
    let layer = doc.layer(id)?;
    let LayerContent::Smart(sm) = &layer.content else { return None };
    if sm.warp.as_ref().is_none_or(|w| w.is_identity()) {
        return None;
    }
    let (source, place) = photocraft_engine::warp_cmds::smart_warp_source(doc, layer)?;
    let texture = contents_texture(app, doc, layer, &source, session)?;
    Some(super::WarpedPreview { mesh: source.to_mesh(1, 1), bounds: source.bounds, place: place.0, start, texture })
}

/// Keeps what the preview holds beside the session in step with the session's own undo steps. A warp held under a box
/// mode ([`hold_warp`]) lives outside the session, so an Undo back into Warp mode brings the session's warp back while
/// the preview still holds it: the hold is undone (the contents' map and texels come back). A Redo out of Warp mode
/// again drops the session's warp, as Warp's own toggle does ([`leave_warp`]): the contents' map goes and the box's
/// texels come back.
pub(crate) fn reconcile_held(app: &mut PhotocraftApp) {
    let Some(warping) = app.ui.transform.as_ref().map(|t| t.session.warp().is_some()) else { return };
    let Some(pv) = app.transform_preview.as_mut() else { return };
    if warping {
        if let Some(held) = pv.held.take() {
            if let Some(texture) = held.texture {
                pv.other_texels = Some(OtherTexels { texture: std::mem::replace(&mut pv.textures, texture), placed_warp: None });
            }
            pv.placed_warp = held.placed;
        }
    } else if pv.held.is_none() && pv.placed_warp.take().is_some() {
        swap_mode_texels(pv);
    }
}
