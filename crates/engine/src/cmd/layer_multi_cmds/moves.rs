use super::selection::{reselect, same_named_targets, selected, top_level};
use super::structure::{check_group_depth, relink_copies};
use super::{Document, EngineError, Layer, LayerContent, LayerId, Result, Session, Value, bad, int_i32, json};

/// A Layers-panel drag of the selected layer records its drop target by name (and which of
/// several same-named layers it is), as [`recorded_selection`] does: the id means nothing in the
/// document the action plays in later, so playback failed with "no such layer" (#7845). Drags of
/// several layers, or of a layer that isn't the selected one, keep their ids.
pub(super) fn recorded_move(s: &Session, mut p: Value, pre_selected: Option<&[LayerId]>) -> Value {
    if p.get("layers").is_some() {
        return p;
    }
    let Some(st) = s.active() else { return p };
    let moved = match p.get("layer") {
        None => st.active_layer,
        Some(v) => match v.as_u64().map(LayerId) {
            Some(id) if pre_selected.map_or_else(|| st.selected_layers() == [id], |sel| sel == [id]) => Some(id),
            _ => return p,
        },
    };
    if let Some(fields) = p.as_object_mut() {
        fields.remove("layer");
    }
    let Some(id) = p.get("target").and_then(Value::as_u64).map(LayerId) else { return p };
    let Some(layer) = st.doc.layer(id) else { return p };
    // A copy leaves its source where it is (it may even land beside it), so the source stays a
    // candidate; the new copy (the active layer once the step has run, which is when it is
    // recorded) is left out instead, as it doesn't exist yet when the step replays.
    let copy = p.get("copy").and_then(Value::as_bool).unwrap_or(false);
    let skip = if copy { st.active_layer.filter(|a| Some(*a) != moved) } else { moved };
    let matches = same_named_targets(&st.doc, &layer.name, skip.as_slice());
    let Some(occurrence) = matches.iter().position(|candidate| *candidate == id) else { return p };
    if let Some(fields) = p.as_object_mut() {
        fields.insert("target".into(), json!(layer.name));
        if matches.len() > 1 {
            fields.insert("targetOccurrence".into(), json!(occurrence));
        }
    }
    p
}

/// `layer.moveTo`'s drop target: an id, or a name with `targetOccurrence` choosing among layers
/// that share it (in document order, the `moved` layers aside; required when several do).
fn move_target(s: &Session, p: &Value, moved: &[LayerId]) -> Result<LayerId> {
    const CMD: &str = "layer.moveTo";
    match p.get("target") {
        Some(Value::String(name)) => {
            let occurrence = p
                .get("targetOccurrence")
                .map(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| bad(CMD, "`targetOccurrence` must be a non-negative integer")))
                .transpose()?;
            let st = s.active().ok_or(EngineError::NoDocument)?;
            let matches = same_named_targets(&st.doc, name, moved);
            if matches.len() > 1 && occurrence.is_none() {
                return Err(bad(CMD, format!("multiple layers named `{name}`; specify `targetOccurrence`")));
            }
            let index = occurrence.unwrap_or(0);
            matches.get(index).copied().ok_or_else(|| bad(CMD, format!("no layer named `{name}` at occurrence {index}")))
        }
        Some(v) => v.as_u64().map(LayerId).ok_or_else(|| bad(CMD, "`target` must be a layer id or name")),
        None => Err(bad(CMD, "missing `target`")),
    }
}

// ---------- moving layers ----------

pub(super) fn link_groups(doc: &Document, ids: &[LayerId]) -> Vec<u64> {
    let mut g: Vec<u64> = ids.iter().filter_map(|id| doc.layer(*id)?.link_group).collect();
    g.sort_unstable();
    g.dedup();
    g
}

/// `ids` plus every layer linked to one of them.
pub(crate) fn with_links(doc: &Document, ids: &[LayerId]) -> Vec<LayerId> {
    let groups = link_groups(doc, ids);
    let mut out = ids.to_vec();
    if !groups.is_empty() {
        for (_, _, l) in doc.walk() {
            if l.link_group.is_some_and(|g| groups.contains(&g)) && !out.contains(&l.id) {
                out.push(l.id);
            }
        }
    }
    out
}

/// Move layers by whole pixels the way the Move tool does: pixels, linked masks, type and
/// vector content (shapes, linked vector masks) all move together. `before` is `doc` as it was
/// before any of the moves (the edit's starting document), read when type and shapes re-render.
pub(crate) fn move_layers(doc: &mut Document, before: &Document, moves: &[(LayerId, i32, i32)]) -> Result<()> {
    move_layers_wrapped(doc, before, moves, None)
}

/// [`move_layers`]. `wrap` is the canvas when View › Pattern Preview is on: raster pixels and
/// linked masks on those layers that leave it come back on the opposite edge. `dx`/`dy` stay raw.
pub(crate) fn move_layers_wrapped(doc: &mut Document, before: &Document, moves: &[(LayerId, i32, i32)], wrap: Option<photocraft_geom::Rect>) -> Result<()> {
    for &(id, dx, dy) in moves {
        if dx == 0 && dy == 0 {
            continue;
        }
        let locks = doc.effective_locks(id);
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        if locks.position || locks.all {
            return Err(EngineError::Other(format!("layer \"{}\" is position-locked", l.name)));
        }
        crate::commands::translate_layer_wrapped(before, l, dx, dy, wrap);
        crate::vector_cmds::translate_vectors_wrapped(before, l, dx as f64, dy as f64, wrap);
        note_move_phase_tree(l, dx, dy, 0);
    }
    Ok(())
}

/// Does a move aimed at the layer mask (`"target":"mask"`) move `l`'s mask alone? Only an
/// unlinked mask does (the chain off); a linked one moves with its layer, as in Photoshop.
pub fn moves_mask_alone(l: &Layer) -> bool {
    l.mask.as_ref().is_some_and(|m| !m.linked)
}

/// `layer.translate`: the explicit layer, or every selected layer, plus their linked layers.
/// With `"target":"mask"` a layer whose mask is unlinked moves that mask alone.
pub fn translate(s: &mut Session, p: &Value) -> Result<Value> {
    let (dx, dy) = match p.get("units").and_then(Value::as_str) {
        None | Some("px") => (int_i32("layer.translate", p, "dx")?.unwrap_or(0), int_i32("layer.translate", p, "dy")?.unwrap_or(0)),
        // Points (1/72 in), as a Photoshop action records a nudge: whole pixels at this
        // document's resolution.
        Some("pt") => {
            let dpi = s.active().map_or(72.0, |d| f64::from(d.doc.resolution_dpi));
            let to_px = |key: &str| -> Result<i32> {
                let pt = match p.get(key) {
                    None => 0.0,
                    Some(v) => v.as_f64().filter(|f| f.is_finite()).ok_or_else(|| bad("layer.translate", format!("`{key}` must be a number")))?,
                };
                let px = (pt * dpi / 72.0).round();
                if px.abs() > f64::from(i32::MAX) {
                    return Err(bad("layer.translate", format!("`{key}` = {pt} pt is outside the 32-bit coordinate range")));
                }
                Ok(px as i32)
            };
            (to_px("dx")?, to_px("dy")?)
        }
        Some(other) => return Err(bad("layer.translate", format!("unknown units `{other}` (px|pt)"))),
    };
    let flag = |key: &str| match p.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(v) => v.as_bool().ok_or_else(|| bad("layer.translate", format!("`{key}` must be a boolean"))),
    };
    let carry = flag("selection")?;
    let nest = flag("nest")?;
    let wrap = flag("wrap")?;
    if dx == 0 && dy == 0 {
        return Ok(Value::Null);
    }
    let roots = match crate::commands::explicit_layer(s, p)? {
        Some(id) => vec![id],
        None => selected(s),
    };
    if roots.is_empty() {
        return Err(EngineError::Other("no active layer".into()));
    }
    let mask_target = p.get("target").and_then(Value::as_str) == Some("mask");
    let before = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let tile = wrap.then(|| before.bounds());
    // A whole number of canvases changes nothing only when every target is raster samples.
    // Text, shapes and the rest keep the raw delta, so they still move.
    if tile.is_some_and(|tile| wrapped_delta(dx, dy, tile) == (0, 0) && targets_only_wrap(&before, &roots, mask_target, dx, dy)) {
        return Ok(Value::Null);
    }
    let mut shift = (0, 0);
    let nested = s.edit("Move", |doc, _| {
        let (alone, rest): (Vec<LayerId>, Vec<LayerId>) = roots.iter().partition(|id| mask_target && doc.layer(**id).is_some_and(moves_mask_alone));
        let mut ids = move_targets(doc, &rest);
        if ids.is_empty() && alone.is_empty() {
            return Err(EngineError::NoLayer(roots[0]));
        }
        let moves: Vec<_> = ids.iter().map(|&id| (id, dx, dy)).collect();
        move_layers_wrapped(doc, &before, &moves, tile)?;
        // In the same step, so Undo takes the move and the change of board back together.
        let nested = if nest { auto_nest(doc, &ids) } else { Vec::new() };
        for id in alone {
            let locks = doc.effective_locks(id);
            let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
            if locks.position || locks.all {
                return Err(EngineError::Other(format!("layer \"{}\" is position-locked", l.name)));
            }
            if let Some(m) = &mut l.mask {
                m.surface = move_surface(&m.surface, dx, dy, tile);
            }
            ids.push(id);
        }
        // A board moved past any canvas edge grows the canvas, as a new one does (#1531, #3070).
        if ids.iter().any(|id| doc.layer(*id).is_some_and(|l| l.artboard().is_some())) {
            shift = crate::artboard_cmds::fit_canvas(doc);
        }
        // The Move tool carries the selection along when it moves several layers whole, as
        // Photoshop does (#7538): one step, so Undo takes both back. Wrap it only when a
        // wrapping target moves; a selection of shapes would jump a canvas if it folded.
        if carry {
            let sel_wrap = tile.filter(|_| ids.iter().any(|id| doc.layer(*id).is_some_and(|l| subtree_wraps_only(l, 0))));
            doc.selection = doc.selection.as_ref().map(|sel| move_surface(sel, dx, dy, sel_wrap));
        }
        Ok(nested)
    })?;
    crate::artboard_cmds::follow_shift(s, shift);
    if nested.is_empty() {
        return Ok(Value::Null);
    }
    Ok(json!({"nested": nested.iter().map(|id| id.0).collect::<Vec<_>>()}))
}

/// The artboard a layer at `bounds` is on: the topmost board holding the centre of its bounds
/// ([`crate::layer_place::centre`]), the rule a new layer is placed by. `boards` are top to bottom.
fn board_under(boards: &[(LayerId, photocraft_geom::Rect)], bounds: photocraft_geom::Rect) -> Option<LayerId> {
    let (cx, cy) = crate::layer_place::centre(bounds);
    boards.iter().find(|(_, r)| r.contains(cx, cy)).map(|(id, _)| *id)
}

/// One planned auto-nest move: its order (walk order, then its place above a clip base), the
/// layer, and the boards it leaves and joins.
type Nest = ((usize, usize), LayerId, Option<LayerId>, Option<LayerId>);

/// Move-tool auto-nesting, as in Photoshop: after a Move drag, a layer now on another artboard
/// joins that board, on top of its stack, and a layer moved off every board leaves its board for
/// the top level, directly above the board it left. Artboards share one canvas, so without this a
/// layer dragged onto another board stayed in its old board and was clipped away by it.
///
/// A clipping group moves as one: a base that changes board takes the layers clipped to it along,
/// directly above it and still clipped. Left behind, they were released (their base gone) and
/// covered their old board. A clipped layer never changes board by itself, moved or not: it shows
/// only over its base, so it goes where its base goes (with it, or nowhere). The group stays
/// whole where it is when any of it can't go.
///
/// `ids` are the moved layers ([`move_targets`]), already at their new positions. Left where they
/// are: artboards themselves, the Background, empty layers, layers whose board doesn't change, a
/// group that would nest past [`photocraft_doc::MAX_GROUP_DEPTH`], and layers locked against it
/// with Lock › Prevent Auto-Nesting (on the layer, a group around it, or the board it would join).
/// Returns the layers that changed board, bottom to top.
fn auto_nest(doc: &mut Document, ids: &[LayerId]) -> Vec<LayerId> {
    let boards: Vec<(LayerId, photocraft_geom::Rect)> = doc.artboards().into_iter().rev().map(|(id, _, board)| (id, board.rect)).collect();
    if boards.is_empty() {
        return Vec::new();
    }
    let locked_board = |doc: &Document, id: LayerId| doc.layer(id).is_some_and(|l| l.locks.artboard || l.locks.all);
    // Bottom to top, so layers moving together keep their order in their new parent: by walk
    // order, and a carried clipped layer by its base's and then its place above the base.
    let mut plan: Vec<Nest> = Vec::new();
    // The paths of the layers planned so far: a layer inside one of them goes with it.
    let mut carried: Vec<Vec<usize>> = Vec::new();
    for (order, (path, _, l)) in doc.walk().into_iter().enumerate() {
        if !ids.contains(&l.id) || l.artboard().is_some() || doc.is_background(l.id) || carried.iter().any(|c| path.starts_with(c)) {
            continue;
        }
        if l.clipped && crate::clip_group::base_of(doc, l.id).is_some() {
            continue;
        }
        let bounds = crate::layer_multi_cmds::layer_bounds(l);
        let Some(bounds) = bounds.filter(|b| !b.is_empty()) else { continue };
        let (from, to) = (doc.artboard_of(l.id), board_under(&boards, bounds));
        let clipped = if l.clipped { Vec::new() } else { crate::clip_group::clipped_above(doc, l.id) };
        let group: Vec<&Layer> = std::iter::once(l).chain(clipped.iter().filter_map(|id| doc.layer(*id))).collect();
        if from == to || to.is_some_and(|b| locked_board(doc, b)) || group.iter().any(|m| doc.effective_locks(m.id).artboard) {
            continue;
        }
        if to.is_some() && group.iter().any(|m| 1 + crate::layer_place::subtree_depth(m) > photocraft_doc::MAX_GROUP_DEPTH) {
            continue;
        }
        for (k, m) in group.into_iter().enumerate() {
            plan.push(((order, k), m.id, from, to));
            carried.extend(doc.path_of(m.id));
        }
    }
    if plan.is_empty() {
        return Vec::new();
    }
    plan.sort_by_key(|p| p.0);
    let moving: Vec<LayerId> = plan.iter().map(|p| p.1).collect();
    let before = crate::clip_group::Before::read(doc, &moving);
    let mut nested = Vec::with_capacity(plan.len());
    // How many layers have left each board so far: each goes above the last one.
    let mut left: Vec<(LayerId, usize)> = Vec::new();
    for (_, id, from, to) in plan {
        let Some(layer) = doc.remove(id) else { continue };
        match to {
            Some(board) => match doc.layer_mut(board).and_then(Layer::children_mut) {
                Some(children) => children.push(layer),
                None => doc.layers.push(layer),
            },
            None => {
                let above = from.and_then(|b| doc.layers.iter().position(|t| t.id == b)).map_or(doc.layers.len(), |i| i + 1);
                let count = left.iter_mut().find(|(b, _)| Some(*b) == from);
                let n = match count {
                    Some((_, n)) => {
                        *n += 1;
                        *n - 1
                    }
                    None => {
                        if let Some(b) = from {
                            left.push((b, 1));
                        }
                        0
                    }
                };
                doc.layers.insert((above + n).min(doc.layers.len()), layer);
            }
        }
        nested.push(id);
    }
    crate::clip_group::settle(doc, &nested, &before, false);
    nested
}

/// The layers a move of `roots` takes along: their linked layers, without layers whose group
/// is also moving (the group carries them).
pub fn move_targets(doc: &Document, roots: &[LayerId]) -> Vec<LayerId> {
    top_level(doc, &with_links(doc, roots))
}

/// `doc` with the layers `ids` ([`move_targets`]) moved by whole pixels, as `layer.translate`
/// leaves them: the Move tool's live preview while it drags. Type, shape and smart-object pixels
/// shift as they are instead of re-rendering (the commit re-renders them), so a drag frame costs
/// copying the moving pixels only.
pub fn moved(doc: &Document, ids: &[LayerId], dx: i32, dy: i32) -> Result<Document> {
    moved_target(doc, ids, dx, dy, false)
}

/// #14960: a linked Pattern Overlay with no stored effects reference point tiles from the document
/// origin, and moving the layer has to carry that grid with it. A layer that has no reference point
/// adds each move to its own `effects.move_phase`, which only the overlay reads (a layer moved back
/// to where it was gets the document grid again). The reference point itself is left alone: pattern
/// strokes, glows and bevel textures keep anchoring at it, else at the layer's corner, also when
/// they are added after the move, and the wrap checks of a Pattern Preview move keep reading it as
/// before. A stored reference point shifts in both move paths, as it always did. Only content that
/// moves as a picture gets a phase: an adjustment or fill layer has no position, and a move of one
/// changes nothing about it.
fn note_move_phase(l: &mut Layer, dx: i32, dy: i32) {
    if l.effects.reference.is_none() && (dx != 0 || dy != 0) && matches!(l.content, LayerContent::Raster(_) | LayerContent::Group(_)) {
        let (x, y) = l.effects.move_phase.unwrap_or((0.0, 0.0));
        l.effects.move_phase = Some((x + f64::from(dx), y + f64::from(dy)));
    }
}

/// [`note_move_phase`] over a moved layer and the layers inside it: the commit path shifts the
/// reference point of every layer of a group it moves, so a linked overlay deeper in the group
/// needs its phase there too.
fn note_move_phase_tree(l: &mut Layer, dx: i32, dy: i32, depth: usize) {
    note_move_phase(l, dx, dy);
    if depth >= photocraft_doc::MAX_GROUP_DEPTH {
        return;
    }
    if let LayerContent::Group(g) = &mut l.content {
        g.children.iter_mut().for_each(|c| note_move_phase_tree(c, dx, dy, depth + 1));
    }
}

/// [`moved`] for a drag aimed at the layer mask (`mask_target`): unlinked masks move alone.
pub fn moved_target(doc: &Document, ids: &[LayerId], dx: i32, dy: i32, mask_target: bool) -> Result<Document> {
    moved_wrapping(doc, ids, dx, dy, mask_target, None)
}

/// [`moved_target`] that wraps raster pixels inside `tile` (View › Pattern Preview). `None`
/// is a straight move. `dx` and `dy` stay raw: only [`wrap_surface`] folds them into the tile.
pub fn moved_wrapping(doc: &Document, ids: &[LayerId], dx: i32, dy: i32, mask_target: bool, wrap: Option<photocraft_geom::Rect>) -> Result<Document> {
    let mut out = doc.clone();
    for &id in ids {
        let locks = doc.effective_locks(id);
        let l = out.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        if locks.position || locks.all {
            return Err(EngineError::Other(format!("layer \"{}\" is position-locked", l.name)));
        }
        if mask_target && moves_mask_alone(l) {
            if let Some(m) = &mut l.mask {
                m.surface = move_surface(&m.surface, dx, dy, wrap);
            }
            continue;
        }
        shift_shown(doc, l, dx, dy, wrap);
    }
    Ok(out)
}

/// `dx` and `dy` folded into one `tile`. A tile with no extent leaves the delta unchanged.
/// Raster wrapping uses this; text, shapes and other geometry keep the raw delta.
pub fn wrapped_delta(dx: i32, dy: i32, tile: photocraft_geom::Rect) -> (i32, i32) {
    let Some((w, h)) = tile_period(tile) else { return (dx, dy) };
    (wrap_axis(dx, w), wrap_axis(dy, h))
}

/// `photocraft_algo::resample::translate_surface` with the content scan cached per tile (a Move
/// drag shifts the same layers every frame; scanning them each time cost more than the copy).
pub(crate) fn shift_surface(s: &photocraft_raster::Surface, dx: i32, dy: i32) -> photocraft_raster::Surface {
    s.translated(dx, dy, photocraft_compose::bounds::content_bounds(s))
}

/// [`crate::commands::translate_layer`] plus the vector side, without re-rendering anything.
/// `wrap` matches the commit: raster pixels fold back into the canvas.
fn shift_shown(doc: &Document, l: &mut Layer, dx: i32, dy: i32, wrap: Option<photocraft_geom::Rect>) {
    use self::shift_surface as translate_surface;
    let a = photocraft_geom::Affine::translate(f64::from(dx), f64::from(dy));
    if let Some(r) = &mut l.effects.reference {
        *r = (r.0 + f64::from(dx), r.1 + f64::from(dy));
    } else {
        note_move_phase(l, dx, dy);
    }
    let mask_wrap = wrap.filter(|_| wraps_linked_masks(&l.content));
    if let Some(m) = l.mask.as_mut().filter(|m| m.linked) {
        m.surface = move_surface(&m.surface, dx, dy, mask_wrap);
    }
    if let Some(vm) = l.vector_mask.as_mut().filter(|v| v.linked) {
        vm.path = match mask_wrap {
            Some(tile) => wrap_path(&vm.path, dx, dy, tile),
            None => vm.path.transform(&a),
        };
    }
    match &mut l.content {
        LayerContent::Raster(s) => *s = move_surface(s, dx, dy, wrap),
        LayerContent::Text(t) => {
            t.transform = a.mul(&t.transform);
            if let Some(c) = &mut t.cache {
                *c = translate_surface(c, dx, dy);
            }
        }
        LayerContent::Shape(sh) => crate::vector_cmds::shift_shape(doc, sh, f64::from(dx), f64::from(dy)),
        LayerContent::Smart(sm) => crate::smart_cmds::shift_smart(sm, dx, dy),
        LayerContent::Group(g) => {
            if let Some(ab) = &mut g.artboard {
                ab.rect = ab.rect.translate(dx, dy);
            }
            g.children.iter_mut().for_each(|c| shift_shown(doc, c, dx, dy, wrap));
        }
        LayerContent::Adjustment(_) | LayerContent::Fill(_) => {}
    }
}

/// True when a whole-canvas `dx`/`dy` would leave every targeted layer unchanged: each move is
/// raster samples (or a mask moved alone) folding back onto themselves. A group counts only when
/// every descendant does. Text, shapes, smart objects, artboards and effect reference points do
/// not. A linked vector mask on a wrapping layer folds with it. A linked Pattern Overlay on the
/// content kinds #14960 measured is the exception: the drag preview tiles its grid by the raw
/// delta, so [`overlay_grid_moves`] keeps a move that would shift that grid (review,
/// moves.rs:458).
fn targets_only_wrap(doc: &Document, roots: &[LayerId], mask_target: bool, dx: i32, dy: i32) -> bool {
    let (alone, rest): (Vec<LayerId>, Vec<LayerId>) = roots.iter().copied().partition(|id| mask_target && doc.layer(*id).is_some_and(moves_mask_alone));
    let ids = move_targets(doc, &rest);
    if ids.is_empty() && alone.is_empty() {
        return false;
    }
    ids.iter().all(|id| doc.layer(*id).is_some_and(|l| subtree_wraps(l, 0, true) && !overlay_grid_moves(doc, l, 0, dx, dy)))
}

/// Whether the raw `dx`/`dy` shifts the grid of a linked Pattern Overlay on `l` or on a layer
/// inside it. The drag preview records that delta as the move phase of every raster and group
/// layer ([`note_move_phase`]) and the overlay tiles from it (#14960); when [`targets_only_wrap`]
/// folds a whole-canvas move away the commit keeps no phase, and the pattern snaps back from what
/// the preview showed. The grid only stays where it was when the painted pattern has no scale and
/// no rotation and its period divides the delta; a scaled or rotated pattern is treated as moved
/// (the fold would need the rotated lattice, and the preview shows the phase either way). An
/// overlay that paints nothing, with the pattern missing from the document, cannot diverge.
fn overlay_grid_moves(doc: &Document, l: &Layer, depth: usize, dx: i32, dy: i32) -> bool {
    if depth > photocraft_doc::MAX_GROUP_DEPTH {
        return false;
    }
    // Only the content kinds whose overlay grid reads the move phase can move (overlay_anchor),
    // and only while the style is on; an unlinked overlay tiles from the canvas, not from this layer.
    if l.effects.enabled
        && matches!(l.content, LayerContent::Raster(_) | LayerContent::Group(_))
        && l.effects.items.iter().any(|e| match e {
            photocraft_doc::Effect::PatternOverlay { name, id, scale, angle, link, .. } => {
                *link && (*scale != 1.0 || *angle != 0.0 || pattern_period_moves(doc, id, name, dx, dy))
            }
            _ => false,
        })
    {
        return true;
    }
    if let LayerContent::Group(g) = &l.content {
        return g.children.iter().any(|c| overlay_grid_moves(doc, c, depth + 1, dx, dy));
    }
    false
}

/// Whether the document pattern `id`/`name` tiles with a period that does not fold `dx`/`dy` away.
fn pattern_period_moves(doc: &Document, id: &str, name: &str, dx: i32, dy: i32) -> bool {
    photocraft_doc::pattern::find(&doc.patterns, id, name).is_some_and(|p| {
        p.width > 0 && p.height > 0 && (i64::from(dx).rem_euclid(i64::from(p.width)) != 0 || i64::from(dy).rem_euclid(i64::from(p.height)) != 0)
    })
}

/// Whether `l` and everything inside it wrap as raster samples. The geometry question, which
/// linked masks and the carried selection ask: a layer's #14960 move phase is not geometry and
/// does not change the answer.
fn subtree_wraps_only(l: &Layer, depth: usize) -> bool {
    subtree_wraps(l, depth, false)
}

/// [`subtree_wraps_only`]; with `phase`, a recorded move phase also counts as something a
/// whole-canvas move changes, so [`targets_only_wrap`] does not skip that move (its overlay grid
/// shifts by the raw delta, as the drag preview shows it).
fn subtree_wraps(l: &Layer, depth: usize, phase: bool) -> bool {
    if depth > photocraft_doc::MAX_GROUP_DEPTH || l.effects.reference.is_some() || l.frame.is_some() || (phase && l.effects.move_phase.is_some()) {
        return false;
    }
    if l.vector_mask.as_ref().is_some_and(|m| m.linked) && !wraps_linked_masks(&l.content) {
        return false;
    }
    match &l.content {
        LayerContent::Raster(_) | LayerContent::Adjustment(_) | LayerContent::Fill(_) => true,
        LayerContent::Group(g) => g.artboard.is_none() && g.children.iter().all(|c| subtree_wraps(c, depth + 1, phase)),
        LayerContent::Text(_) | LayerContent::Shape(_) | LayerContent::Smart(_) => false,
    }
}

fn move_surface(s: &photocraft_raster::Surface, dx: i32, dy: i32, wrap: Option<photocraft_geom::Rect>) -> photocraft_raster::Surface {
    match wrap {
        Some(tile) => wrap_surface(s, dx, dy, tile),
        None => shift_surface(s, dx, dy),
    }
}

/// Linked raster and vector masks wrap with the layer when its pixels wrap. A group's mask wraps
/// only when every descendant does; otherwise the mask would slide off a shape that kept the raw
/// delta. Type, shapes and smart objects keep the raw delta, so their masks do too.
pub(crate) fn wraps_linked_masks(content: &LayerContent) -> bool {
    match content {
        LayerContent::Raster(_) | LayerContent::Adjustment(_) | LayerContent::Fill(_) => true,
        LayerContent::Group(g) => g.artboard.is_none() && g.children.iter().all(|c| subtree_wraps_only(c, 0)),
        _ => false,
    }
}

fn tile_period(tile: photocraft_geom::Rect) -> Option<(i32, i32)> {
    let w = i32::try_from(tile.width()).ok()?;
    let h = i32::try_from(tile.height()).ok()?;
    (w > 0 && h > 0).then_some((w, h))
}

fn wrap_axis(delta: i32, period: i32) -> i32 {
    i64::from(delta).rem_euclid(i64::from(period)) as i32
}

/// Move `s` by whole pixels, folding samples that start inside `tile` back into it. Samples
/// already outside the tile stay where they are. A shift that never leaves the tile is an
/// ordinary move. Each wrap piece is a [`shift_surface`]-style tile copy of its source rect, so a
/// live drag never materializes the in-canvas bounding rectangle as one buffer.
pub(crate) fn wrap_surface(s: &photocraft_raster::Surface, dx: i32, dy: i32, tile: photocraft_geom::Rect) -> photocraft_raster::Surface {
    let Some((w, h)) = tile_period(tile) else { return shift_surface(s, dx, dy) };
    let dx = wrap_axis(dx, w);
    let dy = wrap_axis(dy, h);
    if dx == 0 && dy == 0 {
        return s.clone();
    }
    let content = photocraft_compose::bounds::content_bounds(s);
    let inside = content.intersect(&tile);
    if inside.is_empty() {
        return s.clone();
    }
    let moved = inside.translate(dx, dy);
    if tile.contains_rect(&moved) && tile.contains_rect(&content) {
        return shift_surface(s, dx, dy);
    }
    let mut pieces = wrap_pieces(inside, dx, dy, tile, w, h).into_iter();
    let Some((dest, from)) = pieces.next() else { return s.clone() };
    let Some(mut out) = shifted_piece(s, dest, from) else { return shift_surface(s, dx, dy) };
    for (dest, from) in pieces {
        let Some(piece) = shifted_piece(s, dest, from) else { return shift_surface(s, dx, dy) };
        copy_rect(&mut out, &piece, dest);
    }
    if !tile.contains_rect(&content) {
        for (coord, _) in s.tiles() {
            for r in rect_minus(coord.rect(), tile) {
                if r.is_empty() {
                    continue;
                }
                let bytes = s.to_interleaved(r);
                out.write_interleaved(r, &bytes);
            }
        }
    }
    out.prune();
    out
}

fn shifted_piece(s: &photocraft_raster::Surface, dest: photocraft_geom::Rect, from: photocraft_geom::Rect) -> Option<photocraft_raster::Surface> {
    let ox = dest.x0.checked_sub(from.x0)?;
    let oy = dest.y0.checked_sub(from.y0)?;
    let mut piece = s.translated(ox, oy, from);
    clip_to_rect(&mut piece, dest);
    Some(piece)
}

/// Drop samples `translated` kept because a tile-aligned shift clones whole 256×256 tiles.
/// Only stored tiles are visited, so a sparse layer on a large canvas does not allocate the
/// empty space between them.
fn clip_to_rect(s: &mut photocraft_raster::Surface, dest: photocraft_geom::Rect) {
    let coords: Vec<_> = s.tiles().map(|(c, _)| *c).collect();
    if coords.is_empty() {
        return;
    }
    let px = s.default_pixel();
    for c in coords {
        let tr = c.rect();
        let hit = tr.intersect(&dest);
        if hit.is_empty() {
            s.fill_rect(tr, &px);
        } else if hit != tr {
            for r in rect_minus(tr, dest) {
                s.fill_rect(r, &px);
            }
        }
    }
    s.prune();
}

/// Copy `r` from stored source tiles (at most one 256×256 buffer at a time). Empty source tiles stay default.
fn copy_rect(dst: &mut photocraft_raster::Surface, src: &photocraft_raster::Surface, r: photocraft_geom::Rect) {
    if r.is_empty() {
        return;
    }
    for (tc, _) in src.tiles() {
        let tr = tc.rect().intersect(&r);
        if tr.is_empty() {
            continue;
        }
        let bytes = src.to_interleaved(tr);
        dst.write_interleaved(tr, &bytes);
    }
}

/// Fold a linked vector mask the way [`wrap_surface`] folds samples. Subpaths that start outside
/// the canvas stay put. Intersect/subtract masks keep the raw delta: flattening those copies
/// would change the Boolean. Simple combine masks wrap as at most four zone copies of the
/// in-canvas subpaths.
pub(crate) fn wrap_path(path: &photocraft_doc::Path, dx: i32, dy: i32, tile: photocraft_geom::Rect) -> photocraft_doc::Path {
    let a = photocraft_geom::Affine::translate(f64::from(dx), f64::from(dy));
    let Some((w, h)) = tile_period(tile) else { return path.transform(&a) };
    let dx = wrap_axis(dx, w);
    let dy = wrap_axis(dy, h);
    if dx == 0 && dy == 0 {
        return path.clone();
    }
    // Intersect/subtract/exclude copies would flatten into one Boolean and hide a seam. Keep
    // the raw delta; only Combine/Join can be unioned as wrap-zone copies.
    let simple = !path.inverted && path.subpaths.iter().all(|s| matches!(s.op, photocraft_doc::PathOp::Combine | photocraft_doc::PathOp::Join));
    if !simple {
        return path.transform(&a);
    }
    let mut outside = Vec::new();
    let mut inside = Vec::new();
    for sub in &path.subpaths {
        let one = photocraft_doc::Path { subpaths: vec![sub.clone()], fill_rule: path.fill_rule, inverted: false };
        if path_overlaps_tile(&one, tile) {
            inside.push(sub.clone());
        } else {
            outside.push(sub.clone());
        }
    }
    if inside.is_empty() {
        return path.clone();
    }
    let inside_path = photocraft_doc::Path { subpaths: inside, fill_rule: path.fill_rule, inverted: false };
    let mut wrapped = wrap_simple_inside(&inside_path, dx, dy, tile, w, h);
    wrapped.subpaths.extend(outside);
    wrapped
}

fn wrap_simple_inside(path: &photocraft_doc::Path, dx: i32, dy: i32, tile: photocraft_geom::Rect, w: i32, h: i32) -> photocraft_doc::Path {
    let moved = path.transform(&photocraft_geom::Affine::translate(f64::from(dx), f64::from(dy)));
    if path_inside_tile(&moved, tile) {
        return moved;
    }
    // Already a handful of wrap copies: another pass would duplicate them. Keep the raw in-canvas
    // shift; the next wrap that still fits will fold again.
    if path.subpaths.len() > 4 {
        return moved;
    }
    let mut out = photocraft_doc::Path { subpaths: Vec::new(), fill_rule: moved.fill_rule, inverted: moved.inverted };
    for (sx, sy) in [(0, 0), (-w, 0), (0, -h), (-w, -h)] {
        let copy = moved.transform(&photocraft_geom::Affine::translate(f64::from(sx), f64::from(sy)));
        if !path_overlaps_tile(&copy, tile) {
            continue;
        }
        for (i, mut sub) in copy.subpaths.into_iter().enumerate() {
            if !out.subpaths.is_empty() && i == 0 {
                sub.op = photocraft_doc::PathOp::Combine;
            }
            out.subpaths.push(sub);
        }
    }
    if out.subpaths.is_empty() { moved } else { out }
}

fn path_inside_tile(path: &photocraft_doc::Path, tile: photocraft_geom::Rect) -> bool {
    path.control_bounds()
        .is_some_and(|(x0, y0, x1, y1)| x0 >= f64::from(tile.x0) && y0 >= f64::from(tile.y0) && x1 <= f64::from(tile.x1) && y1 <= f64::from(tile.y1))
}

fn path_overlaps_tile(path: &photocraft_doc::Path, tile: photocraft_geom::Rect) -> bool {
    path.control_bounds()
        .is_some_and(|(x0, y0, x1, y1)| x1 > f64::from(tile.x0) && x0 < f64::from(tile.x1) && y1 > f64::from(tile.y0) && y0 < f64::from(tile.y1))
}

/// `area` minus `hole`, as at most four rectangles.
fn rect_minus(area: photocraft_geom::Rect, hole: photocraft_geom::Rect) -> Vec<photocraft_geom::Rect> {
    let hit = area.intersect(&hole);
    if hit.is_empty() {
        return vec![area];
    }
    if hole.contains_rect(&area) {
        return Vec::new();
    }
    let mut out = Vec::new();
    if area.x0 < hit.x0 {
        out.push(photocraft_geom::Rect::new(area.x0, area.y0, hit.x0, area.y1));
    }
    if hit.x1 < area.x1 {
        out.push(photocraft_geom::Rect::new(hit.x1, area.y0, area.x1, area.y1));
    }
    if area.y0 < hit.y0 {
        out.push(photocraft_geom::Rect::new(hit.x0, area.y0, hit.x1, hit.y0));
    }
    if hit.y1 < area.y1 {
        out.push(photocraft_geom::Rect::new(hit.x0, hit.y1, hit.x1, area.y1));
    }
    out.retain(|r| !r.is_empty());
    out
}

/// `(destination inside the tile, source rect inside `src`)` for a non-negative `dx`/`dy`
/// smaller than the tile.
fn wrap_pieces(
    src: photocraft_geom::Rect,
    dx: i32,
    dy: i32,
    tile: photocraft_geom::Rect,
    w: i32,
    h: i32,
) -> Vec<(photocraft_geom::Rect, photocraft_geom::Rect)> {
    use photocraft_geom::Rect;
    let moved = src.translate(dx, dy);
    let zones = [
        (tile, 0, 0),
        (Rect::new(tile.x1, tile.y0, tile.x1.saturating_add(w), tile.y1), -w, 0),
        (Rect::new(tile.x0, tile.y1, tile.x1, tile.y1.saturating_add(h)), 0, -h),
        (Rect::new(tile.x1, tile.y1, tile.x1.saturating_add(w), tile.y1.saturating_add(h)), -w, -h),
    ];
    let mut out = Vec::new();
    for (zone, sx, sy) in zones {
        let part = moved.intersect(&zone);
        if part.is_empty() {
            continue;
        }
        let from = part.translate(-dx, -dy);
        let dest = part.translate(sx, sy);
        if from.is_empty() || dest.is_empty() {
            continue;
        }
        out.push((dest, from));
    }
    out
}

/// Reorder one or multiple layers to a single destination in one undoable edit.
///
/// Explicit `layers` are ordered according to the source document, not caller ordering.
/// A transient snapshot for the Layers panel's "Preview on canvas when reordering layers".
/// Uses the same source ordering and insertion semantics as `move_to`, but never edits history.
/// Invalid/self-referential hover targets return None (the committed image stays visible).
pub fn preview_move(doc: &Document, p: &Value, add_copy: bool) -> Option<Document> {
    let target = LayerId(p.get("target")?.as_u64()?);
    let position = p.get("position")?.as_str()?;
    if !matches!(position, "above" | "below" | "into") {
        return None;
    }
    let copy = p.get("copy").and_then(Value::as_bool).unwrap_or(false);
    let ids = if let Some(ids) = p.get("layers") {
        ids.as_array()?.iter().map(|v| v.as_u64().map(LayerId)).collect::<Option<Vec<_>>>()?
    } else {
        vec![LayerId(p.get("layer")?.as_u64()?)]
    };
    if ids.is_empty() || doc.layer(target).is_none() {
        return None;
    }
    let moved_ids = top_level(doc, &ids);
    if moved_ids.is_empty() || moved_ids.iter().any(|id| doc.layer(*id).is_none()) {
        return None;
    }
    if moved_ids.len() == 1 && moved_ids[0] == target && !copy {
        return None;
    }
    let target_path = doc.path_of(target)?;
    for id in &moved_ids {
        let path = doc.path_of(*id)?;
        if target_path.starts_with(&path) && !(copy && *id == target && position != "into") {
            return None;
        }
    }
    if position == "into" && !doc.layer(target)?.is_group() {
        return None;
    }
    // The very structural move the drop commits, so the preview shows the same clip groups,
    // Background handling and group-depth refusals; a drop `move_to` would refuse has no preview.
    let mut preview = doc.clone();
    apply_move(&mut preview, &moved_ids, target, position, copy, add_copy).ok()?;
    Some(preview)
}

/// The rect of the artboard layer `id` sits in, or `None` when it sits in none (or is itself a
/// board: boards are never nested, and moving one beside another carries its own rect).
fn board_rect_of(doc: &Document, id: LayerId) -> Option<photocraft_geom::Rect> {
    if doc.layer(id)?.artboard().is_some() {
        return None;
    }
    doc.layer(doc.artboard_of(id)?)?.artboard().map(|board| board.rect)
}

/// Where a dropped layer came from: the rect of the board it sat on, and whether its position was
/// locked there (by its own lock or one on a group or board around it).
#[derive(Clone, Copy)]
struct Origin {
    board: Option<photocraft_geom::Rect>,
    pinned: bool,
}

/// The moves that keep each of `ids` where it sat on its board after a drop into another board.
/// Boards share one canvas, so a layer re-parented without moving keeps its canvas position and
/// lands outside (and is clipped by) the board it was dropped on. `from[i]` is where `ids[i]` was
/// before the drop; the layer shifts by the offset between the two boards' origins. A layer that
/// stays on its board, joins one from outside any board, leaves for the top level, or has its
/// position locked stays where it is. The lock is read on both sides: a lock the layer had from a
/// group or board it left no longer shows on it once it has left (#10489 review).
fn board_offsets(doc: &Document, ids: &[LayerId], from: &[Origin]) -> Result<Vec<(LayerId, i32, i32)>> {
    let mut shifts = Vec::new();
    for (&id, src) in ids.iter().zip(from) {
        let (Some(board), Some(dst)) = (src.board.as_ref(), board_rect_of(doc, id)) else { continue };
        let locks = doc.effective_locks(id);
        if src.pinned || locks.position || locks.all || (board.x0, board.y0) == (dst.x0, dst.y0) {
            continue;
        }
        let (Some(dx), Some(dy)) = (dst.x0.checked_sub(board.x0), dst.y0.checked_sub(board.y0)) else {
            let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
            // Plain adjustment/fill content has no position; only its linked masks or effects
            // reference would move. Keep accepting those no-shift reparentings at any distance.
            if matches!(layer.content, LayerContent::Adjustment(_) | LayerContent::Fill(_))
                && layer.effects.reference.is_none()
                && layer.mask.as_ref().is_none_or(|mask| !mask.linked)
                && layer.vector_mask.as_ref().is_none_or(|mask| !mask.linked)
            {
                continue;
            }
            return Err(bad("layer.moveTo", "these artboards are too far apart to preserve the layer's position"));
        };
        shifts.push((id, dx, dy));
    }
    Ok(shifts)
}

/// The document-only part of `move_to`, shared with [`preview_move`] so the Layers panel's hover
/// preview cannot drift from what the drop commits. Moves (or, with `copy`, duplicates) the
/// top-level `moved_ids` to `pos` (`above`, `below` or `into`) `target`, then applies the
/// group-depth cap, the Background-stays-at-the-bottom rule and the clip-group rules
/// (`clip_group::settle`). A layer dropped on a different artboard keeps its place on its board
/// (see [`board_offsets`]). Returns the ids of the new copies (empty for a move). On `Err` `doc`
/// is half edited: run it inside `Session::edit` or on a scratch copy.
fn apply_move(doc: &mut Document, moved_ids: &[LayerId], target: LayerId, pos: &str, copy: bool, add_copy: bool) -> Result<Vec<LayerId>> {
    const CMD: &str = "layer.moveTo";
    let background = crate::background_cmds::background_id(doc);
    let before = if copy { Default::default() } else { crate::clip_group::Before::read(doc, moved_ids) };
    // Read before the layers are lifted out: afterwards the source board, and any lock the layer
    // had from the groups around it, are out of reach. A copy is a new layer, so only its own
    // locks (which it keeps) can pin it.
    let from: Vec<Origin> = moved_ids
        .iter()
        .map(|&id| {
            let locks = doc.effective_locks(id);
            Origin { board: board_rect_of(doc, id), pinned: !copy && (locks.position || locks.all) }
        })
        .collect();
    let mut moved = Vec::with_capacity(moved_ids.len());
    for &id in moved_ids {
        moved.push(if copy { crate::commands::layer_copy(doc, id, add_copy)? } else { doc.remove(id).ok_or(EngineError::NoLayer(id))? });
    }
    let copies: Vec<LayerId> = if copy { moved.iter().map(|l| l.id).collect() } else { Vec::new() };
    // Look up the destination AFTER removing every source: its indices may have shifted.
    let path = doc.path_of(target).ok_or(EngineError::NoLayer(target))?;
    if pos == "into" {
        let children = doc.layer_at_mut(&path).and_then(Layer::children_mut).ok_or_else(|| bad(CMD, "target is not a group"))?;
        children.extend(moved);
    } else {
        let (&idx, parent) = path.split_last().ok_or(EngineError::NoLayer(target))?;
        let siblings = if parent.is_empty() {
            &mut doc.layers
        } else {
            doc.layer_at_mut(parent).and_then(Layer::children_mut).ok_or_else(|| bad(CMD, "target's parent is not a group"))?
        };
        let at = if pos == "below" { idx } else { idx + 1 };
        siblings.splice(at..at, moved);
    }
    check_group_depth(doc, "Reorder Layer")?;
    // A copy is a new layer: it starts unlinked from its original, as a Duplicate does
    // (#11217); copies of a whole linked set stay linked to each other under a fresh group.
    if copy {
        relink_copies(doc, &copies)?;
    }
    // The layers now sit in their new parents; carry each across to its new board. Copies keep the
    // order of `moved_ids`, so `from` lines up with either list.
    let landed: &[LayerId] = if copy { &copies } else { moved_ids };
    let shifts = board_offsets(doc, landed, &from)?;
    if !shifts.is_empty() {
        let snapshot = doc.clone();
        move_layers(doc, &snapshot, &shifts)?;
    }
    crate::background_cmds::keep_background_at_bottom(background, doc)?;
    crate::clip_group::settle(doc, landed, &before, false);
    Ok(copies)
}

/// Selected descendants of a moved group remain inside their parent; only top-level
/// selected nodes are detached, and every reference is validated before editing.
pub fn move_to(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.moveTo";
    let indexed = p.get("target").is_some_and(Value::is_object);
    let resolved;
    let p = if let Some(Value::Object(selector)) = p.get("target") {
        let index = selector
            .get("index")
            .filter(|_| selector.len() == 1)
            .and_then(Value::as_u64)
            .ok_or_else(|| bad(CMD, "target reference needs one non-negative index"))?;
        if p.get("position").is_some_and(|value| !value.is_null()) {
            return Err(bad(CMD, "an indexed target already defines its drop position"));
        }
        let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
        let (target, position) = crate::commands::recorded_drop(doc, index)?;
        let mut fields = p.as_object().cloned().ok_or_else(|| bad(CMD, "params must be an object"))?;
        fields.insert("target".into(), json!(target.0));
        fields.insert("position".into(), json!(position));
        if !fields.contains_key("layer") && !fields.contains_key("layers") {
            match selected(s).as_slice() {
                [layer] => {
                    fields.insert("layer".into(), json!(layer.0));
                }
                layers if !layers.is_empty() => {
                    fields.insert("layers".into(), json!(layers.iter().map(|layer| layer.0).collect::<Vec<_>>()));
                }
                _ => {}
            }
        }
        resolved = Value::Object(fields);
        &resolved
    } else {
        p
    };
    let pos = p.get("position").and_then(Value::as_str).unwrap_or("above");
    if !matches!(pos, "above" | "below" | "into") {
        return Err(bad(CMD, "position must be above, below, or into"));
    }
    // ⌥-drag in the Layers panel: duplicates land at the target and the layers stay put.
    let copy = p.get("copy").and_then(Value::as_bool).unwrap_or(false);

    let batch = p.get("layers");
    if batch.is_some() && p.get("layer").is_some() {
        return Err(bad(CMD, "give either `layer` or `layers`, not both"));
    }
    let requested = if let Some(raw) = batch {
        let arr = raw.as_array().filter(|a| !a.is_empty()).ok_or_else(|| bad(CMD, "`layers` must be a nonempty array of ids"))?;
        let mut ids = Vec::with_capacity(arr.len());
        for item in arr {
            let id = LayerId(item.as_u64().ok_or_else(|| bad(CMD, "every layer id must be an unsigned integer"))?);
            if ids.contains(&id) {
                return Err(bad(CMD, "duplicate layer id"));
            }
            ids.push(id);
        }
        ids
    } else {
        vec![crate::commands::layer_param(s, p)?]
    };
    // A copy moves nothing, so its source stays a candidate (⌥-dragging a layer beside itself).
    let target = move_target(s, p, if copy { &[] } else { &requested })?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = &st.doc;
    let target_path = doc.path_of(target).ok_or(EngineError::NoLayer(target))?;
    for &id in &requested {
        if doc.layer(id).is_none() {
            return Err(EngineError::NoLayer(id));
        }
    }
    let moved_ids = top_level(doc, &requested);
    if moved_ids.len() == 1 && moved_ids[0] == target && (batch.is_none() || indexed) && !copy {
        // Preserve single-layer drag-to-self semantics.
        return Ok(Value::Null);
    }
    // Also rejects dragging a selected group onto any of its descendants.
    for id in &moved_ids {
        let path = doc.path_of(*id).ok_or(EngineError::NoLayer(*id))?;
        // A copy may land beside its own source (not inside it).
        let beside_itself = copy && *id == target && pos != "into";
        if target_path.starts_with(&path) && !beside_itself {
            return Err(bad(CMD, "target cannot be one of the moved layers or its descendant"));
        }
    }
    if pos == "into" && doc.layer(target).is_none_or(|l| !l.is_group()) {
        return Err(bad(CMD, "target is not a group"));
    }

    if indexed
        && !copy
        && let [source] = moved_ids.as_slice()
    {
        let source_path = doc.path_of(*source).ok_or(EngineError::NoLayer(*source))?;
        if let Some((&at, parent)) = source_path.split_last() {
            let unchanged = match pos {
                "below" => target_path.split_last().is_some_and(|(&to, target_parent)| parent == target_parent && at.checked_add(1) == Some(to)),
                "into" => {
                    parent == target_path.as_slice()
                        && doc.layer(target).and_then(Layer::children).is_some_and(|children| at.checked_add(1) == Some(children.len()))
                }
                _ => false,
            };
            if unchanged {
                return Ok(Value::Null);
            }
        }
    }

    let original_active = st.active_layer;
    let add_copy = crate::commands::layer_panel_option(s, "addCopyToCopiedLayersAndGroups", true);
    let (count, copies) = s.edit(if copy { "Duplicate Layer" } else { "Reorder Layers" }, |doc, active| {
        let copies = apply_move(doc, &moved_ids, target, pos, copy, add_copy)?;
        *active = if copy {
            copies.last().copied()
        } else if batch.is_some() {
            original_active.filter(|id| requested.contains(id)).or_else(|| requested.last().copied())
        } else {
            moved_ids.first().copied()
        };
        Ok((moved_ids.len(), copies))
    })?;
    if copy {
        let ids: Vec<u64> = copies.iter().map(|id| id.0).collect();
        if batch.is_some() {
            let active = copies.last().copied();
            reselect(s, copies, active);
            return Ok(json!({"moved": count, "layers": ids}));
        }
        return Ok(json!({"layer": ids.first()}));
    }
    if batch.is_some() {
        let active = original_active.filter(|id| requested.contains(id)).or_else(|| requested.last().copied());
        reselect(s, requested, active);
        Ok(json!({"moved": count}))
    } else {
        Ok(Value::Null)
    }
}

#[cfg(test)]
mod style_phase_tests {
    use super::*;
    use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, SampleType};
    use photocraft_doc::{Adjustment, Effect, Fill, FxCommon};
    use photocraft_geom::{Rect, Size};

    /// A linked Pattern Overlay with the measured defaults: Normal, 100 %, scale 100, angle 0,
    /// Link with Layer on, phase 0 / 0.
    fn linked_overlay() -> Effect {
        Effect::PatternOverlay {
            common: FxCommon::new(BlendMode::Normal, 1.0),
            name: "quadrants".into(),
            id: String::new(),
            scale: 1.0,
            angle: 0.0,
            link: true,
            phase: (0.0, 0.0),
        }
    }

    /// A raster layer holding a filled rectangle at `rect`, as the measured "Shape" layer.
    fn shape(rect: Rect) -> LayerContent {
        let mut s = photocraft_raster::Surface::new(PixelFormat::RGBA8);
        s.fill_rect(rect, &[1.0, 0.5, 0.0, 1.0]);
        LayerContent::Raster(s)
    }

    /// A document holding one layer of `content` whose effects are `items`, and that layer's id.
    fn doc(content: LayerContent, items: Vec<Effect>) -> (Document, LayerId) {
        let mut d = Document::new("origin", Size::new(40, 40), ColorMode::Rgb, SampleType::U8);
        let mut l = Layer::new("Shape", content);
        l.effects.items = items;
        let id = l.id;
        d.layers.push(l);
        (d, id)
    }

    /// The reference point `id` carries in `doc`.
    fn reference(doc: &Document, id: LayerId) -> Option<(f64, f64)> {
        doc.layer(id).expect("the layer is there").effects.reference
    }

    /// The move phase `id` carries in `doc` (`Effects::move_phase`).
    fn phase(doc: &Document, id: LayerId) -> Option<(f64, f64)> {
        doc.layer(id).expect("the layer is there").effects.move_phase
    }

    /// Done when 2 and 3: a Move records the layer's moves as its move phase, which is what the
    /// linked Pattern Overlay's grid anchors at (#14960); the effects reference point stays unset and
    /// the effect's own `phase` stays 0 / 0 as measured. The commit and the drag preview record the
    /// same thing.
    #[test]
    fn a_move_records_the_move_phase_a_linked_overlay_grid_needs() {
        for (dx, dy) in [(7, 4), (5, 3), (-9, -2)] {
            let (before, id) = doc(shape(Rect::new(10, 10, 20, 20)), vec![linked_overlay()]);
            let preview = moved(&before, &[id], dx, dy).expect("the layer moves");
            assert_eq!(phase(&preview, id), Some((f64::from(dx), f64::from(dy))), "the Move preview of +{dx},+{dy}");
            assert_eq!(reference(&preview, id), None, "the preview leaves the reference point unset");
            let mut committed = before.clone();
            move_layers(&mut committed, &before, &[(id, dx, dy)]).expect("the command moves the layer");
            assert_eq!(phase(&committed, id), Some((f64::from(dx), f64::from(dy))), "the committed move of +{dx},+{dy} matches the preview");
            assert_eq!(reference(&committed, id), None, "the commit leaves the reference point unset");
            match &committed.layer(id).expect("the layer").effects.items[0] {
                Effect::PatternOverlay { phase, .. } => assert_eq!(*phase, (0.0, 0.0), "the stored phase stays 0 / 0"),
                other => panic!("the overlay is still there, got {other:?}"),
            }
        }
    }

    /// Done when 3: the overlay can be added after the layer has already moved, so the phase is
    /// recorded by the move itself: a layer with no effects at all carries the sum of its moves.
    #[test]
    fn a_move_records_the_phase_on_a_layer_that_has_no_effects_yet() {
        let (before, id) = doc(shape(Rect::new(10, 10, 20, 20)), Vec::new());
        let after = moved(&before, &[id], 7, 4).expect("the layer moves");
        assert_eq!(phase(&after, id), Some((7.0, 4.0)), "the grid of an overlay added later starts from this phase");
    }

    /// Done when 2: moving the layer back puts the grid on the document origin again. The phase is
    /// the sum of the layer's moves, not something read off its bounds, which came back too.
    #[test]
    fn moving_the_layer_back_puts_the_move_phase_on_the_document_origin() {
        let (before, id) = doc(shape(Rect::new(10, 10, 20, 20)), vec![linked_overlay()]);
        let there = moved(&before, &[id], 7, 4).expect("the layer moves");
        let back = moved(&there, &[id], -7, -4).expect("the layer moves back");
        let l = back.layer(id).expect("the layer");
        assert_eq!(l.effects.move_phase, Some((0.0, 0.0)), "the sum of the moves, with the layer back where it was");
        assert_eq!(crate::layer_multi_cmds::layer_bounds(l), Some(Rect::new(10, 10, 20, 20)), "and its bounds came back with it");
    }

    /// Done when 4: with the overlay on the group, only the group's own moves carry its grid. A
    /// child moving alone leaves the group without a move phase, so the group keeps tiling on the
    /// document grid.
    #[test]
    fn a_group_carries_the_move_phase_for_its_own_moves_only() {
        let child = Layer::new("Shape", shape(Rect::new(10, 10, 20, 20)));
        let child_id = child.id;
        let mut group = Layer::group("Group", vec![child]);
        group.effects.items = vec![linked_overlay()];
        let group_id = group.id;
        let mut d = Document::new("origin", Size::new(40, 40), ColorMode::Rgb, SampleType::U8);
        d.layers.push(group);
        let only_child = moved(&d, &[child_id], 7, 4).expect("the child moves");
        assert_eq!(phase(&only_child, group_id), None, "a child's move leaves the group's own grid alone");
        assert_eq!(phase(&only_child, child_id), Some((7.0, 4.0)), "the moved child records its own phase");
        let group_moved = moved(&d, &[group_id], 7, 4).expect("the group moves");
        assert_eq!(phase(&group_moved, group_id), Some((7.0, 4.0)), "the group's own move records its phase");
        assert_eq!(phase(&group_moved, child_id), Some((7.0, 4.0)), "and the child moved with it");
    }

    /// The unmeasured content types keep today's behaviour: an adjustment or fill layer has no
    /// position, and a move of one still changes nothing about it.
    #[test]
    fn a_move_records_no_reference_point_on_content_that_has_no_position() {
        let cases = [LayerContent::Adjustment(Adjustment::Invert), LayerContent::Fill(Fill::Solid(Color::rgb(1.0, 0.0, 0.0)))];
        for content in cases {
            let what = format!("{content:?}");
            let (before, id) = doc(content, vec![linked_overlay()]);
            let after = moved(&before, &[id], 7, 4).expect("the layer moves");
            assert_eq!(after, before, "the whole document is unchanged for {what}");
            assert_eq!(reference(&after, id), None, "no reference point for {what}");
        }
    }

    /// The measured 20 x 20 pattern of #14960: quadrants red / green / blue / black, and one white
    /// marker pixel at tile (0, 0).
    fn quad_pattern() -> photocraft_doc::Pattern {
        let mut tile = photocraft_raster::Surface::new(PixelFormat::RGBA8);
        tile.fill_rect(Rect::new(0, 0, 10, 10), &[1.0, 0.0, 0.0, 1.0]);
        tile.fill_rect(Rect::new(10, 0, 20, 10), &[0.0, 1.0, 0.0, 1.0]);
        tile.fill_rect(Rect::new(0, 10, 10, 20), &[0.0, 0.0, 1.0, 1.0]);
        tile.fill_rect(Rect::new(10, 10, 20, 20), &[0.0, 0.0, 0.0, 1.0]);
        tile.fill_rect(Rect::new(0, 0, 1, 1), &[1.0, 1.0, 1.0, 1.0]);
        photocraft_doc::Pattern::new("quadrants", tile, 20, 20)
    }

    /// A document with the #14960 pattern registered and `layer` in it.
    fn fixture(layer: Layer) -> Document {
        let mut d = Document::new("origin", Size::new(80, 80), ColorMode::Rgb, SampleType::U8);
        d.patterns.push(quad_pattern());
        d.layers.push(layer);
        d
    }

    /// A linked PatternOverlay naming `name`, looked up by name (empty id) like [`linked_overlay`].
    fn linked_overlay_named(name: &str) -> Effect {
        Effect::PatternOverlay {
            common: FxCommon::new(BlendMode::Normal, 1.0),
            name: name.into(),
            id: String::new(),
            scale: 1.0,
            angle: 0.0,
            link: true,
            phase: (0.0, 0.0),
        }
    }

    /// A pattern with a 15 px period in both axes: red in columns 0..7 of each tile, blue in
    /// 8..15. 15 divides neither 40 nor 80, so a whole-canvas move shifts this grid (the review
    /// input at moves.rs:458).
    fn stripes15() -> photocraft_doc::Pattern {
        let mut tile = photocraft_raster::Surface::new(PixelFormat::RGBA8);
        tile.fill_rect(Rect::new(0, 0, 7, 15), &[1.0, 0.0, 0.0, 1.0]);
        tile.fill_rect(Rect::new(7, 0, 15, 15), &[0.0, 0.0, 1.0, 1.0]);
        photocraft_doc::Pattern::new("stripes", tile, 15, 15)
    }

    /// A session on a 40 x 40 RGB8 document with `stripes15` registered and `layer` above the
    /// Background, and that layer's id.
    fn stripes_session(layer: Layer) -> (Session, LayerId) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 40, "depth": 8, "mode": "rgb", "background": "transparent"})).expect("a document opens");
        let id = layer.id;
        s.edit("setup", |doc, _| {
            doc.patterns.push(stripes15());
            doc.layers.push(layer);
            Ok(())
        })
        .expect("the layer is set up");
        (s, id)
    }

    /// The flattened `rect` as one letter per pixel, row-major (W white, R red, G green, B blue,
    /// K black, `?` for anything else), so a test can compare what the compositor painted.
    fn letters(doc: &Document, rect: Rect) -> Vec<char> {
        let q = |v: f32| v > 0.5;
        photocraft_compose::render(doc, rect)
            .px
            .iter()
            .map(|p| match (q(p[0]), q(p[1]), q(p[2])) {
                (true, false, false) => 'R',
                (false, true, false) => 'G',
                (false, false, true) => 'B',
                (false, false, false) => 'K',
                (true, true, true) => 'W',
                _ => '?',
            })
            .collect()
    }

    /// Review point 2 (#14960): a pattern stroke is not measured by this issue, and today it
    /// anchors at the layer's corner, so it stays exactly where it was relative to its layer across
    /// a move. Recording the move phase must not become the anchor of a layer whose style carries a
    /// pattern paint other than the overlay.
    #[test]
    fn a_pattern_stroke_stays_where_it_is_relative_to_its_layer_across_a_move() {
        let pat = quad_pattern();
        let mut l = Layer::new("Shape", shape(Rect::new(20, 20, 40, 40)));
        l.effects.items = vec![Effect::Stroke(photocraft_doc::StrokeFx {
            common: FxCommon::new(BlendMode::Normal, 1.0),
            size: 3.0,
            position: photocraft_doc::StrokePosition::Outside,
            paint: photocraft_doc::FxPaint::Pattern { name: pat.name.clone(), id: pat.id.clone(), scale: 1.0 },
            overprint: false,
        })];
        let id = l.id;
        let d = fixture(l);
        let before = letters(&d, Rect::new(14, 22, 46, 28));
        let after = moved(&d, &[id], 5, 3).expect("the layer moves");
        assert_eq!(reference(&after, id), None, "the move leaves the reference point the stroke anchors at unset");
        // The same window of pixels, followed by the move: the stroke must read identically.
        assert_eq!(before, letters(&after, Rect::new(19, 25, 51, 31)), "the pattern stroke moved with the layer, unchanged");
    }

    /// Review point 1 (#14960): a shape layer is one of the kinds the issue did not measure, so a
    /// linked Pattern Overlay on it keeps today's anchor at the layer's corner and moves with the
    /// layer instead of staying on the document grid.
    #[test]
    fn a_linked_overlay_on_a_shape_layer_moves_with_the_layer() {
        let path = photocraft_doc::Path::new(vec![photocraft_doc::Subpath::polygon(&[(20.0, 20.0), (40.0, 20.0), (40.0, 40.0), (20.0, 40.0)])]);
        let sh = photocraft_doc::ShapeLayer { path, fill: Some(Fill::Solid(Color::rgb(1.0, 0.5, 0.0))), ..Default::default() };
        let mut l = Layer::new("Shape", LayerContent::Shape(sh));
        l.effects.items = vec![linked_overlay()];
        let id = l.id;
        let d = fixture(l);
        let before = letters(&d, Rect::new(22, 22, 38, 38));
        let after = moved(&d, &[id], 7, 4).expect("the shape layer moves");
        assert_eq!(reference(&after, id), None, "a shape layer records no move phase");
        assert_eq!(before, letters(&after, Rect::new(29, 26, 45, 42)), "the overlay moved with the shape layer, unchanged");
    }

    /// A 3 px outside stroke painted with the #14960 pattern (not measured by the issue: it keeps
    /// today's anchor, the effects reference point, else the layer's corner).
    fn pattern_stroke() -> Effect {
        let pat = quad_pattern();
        Effect::Stroke(photocraft_doc::StrokeFx {
            common: FxCommon::new(BlendMode::Normal, 1.0),
            size: 3.0,
            position: photocraft_doc::StrokePosition::Outside,
            paint: photocraft_doc::FxPaint::Pattern { name: pat.name.clone(), id: pat.id.clone(), scale: 1.0 },
            overprint: false,
        })
    }

    /// Review (CodeRabbit, moves.rs:357): a layer with no effects is moved from (23, 21) by +5,+3,
    /// and only then gets a pattern stroke. The stroke tiles from the layer's corner (28, 24), as it
    /// does on `upstream/main`, so it reads exactly like the same stroke on a layer that was drawn
    /// at (28, 24) and never moved. The move phase the overlay needs must not become its anchor.
    #[test]
    fn a_pattern_stroke_added_after_a_move_tiles_from_the_layer_corner() {
        let window = Rect::new(20, 16, 56, 52);
        let mut never = Layer::new("Shape", shape(Rect::new(28, 24, 48, 44)));
        never.effects.items = vec![pattern_stroke()];
        let expected = letters(&fixture(never), window);
        let l = Layer::new("Shape", shape(Rect::new(23, 21, 43, 41)));
        let id = l.id;
        let d = fixture(l);
        let preview = moved(&d, &[id], 5, 3).expect("the layer moves");
        let mut committed = d.clone();
        move_layers(&mut committed, &d, &[(id, 5, 3)]).expect("the command moves the layer");
        for (what, mut after) in [("the drag preview", preview), ("the committed move", committed)] {
            after.layer_mut(id).expect("the layer").effects.items = vec![pattern_stroke()];
            assert_eq!(letters(&after, window), expected, "{what}: the stroke added after the move tiles from the layer corner (28, 24)");
        }
    }

    /// Review (CodeRabbit, moves.rs:369): on an 80 px wide canvas, a group with a linked mask over
    /// a raster child at x 70..75 with a linked overlay. The group moves +1 (no wrap), then a
    /// View > Pattern Preview move of +10 wraps the child to x 1..6; its group's mask has to wrap
    /// with it, as it did on `upstream/main`, whatever phase the first move recorded.
    #[test]
    fn a_group_mask_still_wraps_with_its_child_after_a_move_recorded_the_phase() {
        let mut child = Layer::new("Shape", shape(Rect::new(70, 10, 75, 20)));
        child.effects.items = vec![linked_overlay()];
        let child_id = child.id;
        let mut group = Layer::group("Group", vec![child]);
        let mut mask = photocraft_doc::LayerMask::hide_all();
        mask.surface.fill_rect(Rect::new(70, 10, 75, 20), &[1.0]);
        group.mask = Some(mask);
        let group_id = group.id;
        let d = fixture(group);
        let mut once = d.clone();
        move_layers(&mut once, &d, &[(group_id, 1, 0)]).expect("the group moves");
        let tile = Some(once.bounds());
        let preview = moved_wrapping(&once, &[group_id], 10, 0, false, tile).expect("the wrapping preview");
        let mut committed = once.clone();
        move_layers_wrapped(&mut committed, &once, &[(group_id, 10, 0)], tile).expect("the wrapping move");
        for (what, after) in [("the drag preview", preview), ("the committed move", committed)] {
            let LayerContent::Raster(px) = &after.layer(child_id).expect("the child").content else { panic!("the child is a raster layer") };
            assert_eq!(px.content_bounds(), Rect::new(1, 10, 6, 20), "{what}: the child wrapped to x 1..6");
            let m = after.layer(group_id).and_then(|g| g.mask.as_ref()).expect("the group mask");
            assert_eq!(m.surface.content_bounds(), Rect::new(1, 10, 6, 20), "{what}: the group mask wrapped with its child");
        }
    }

    /// Not measured by #14960, kept as on `upstream/main`: a child with a stored effects reference
    /// point (read from a file) still keeps its group's linked mask on the raw delta in a Pattern
    /// Preview move, and a whole-canvas move of it is still not skipped.
    #[test]
    fn a_stored_reference_point_keeps_the_wrap_checks_as_they_were() {
        let mut child = Layer::new("Shape", shape(Rect::new(70, 10, 75, 20)));
        child.effects.items = vec![linked_overlay()];
        child.effects.reference = Some((3.0, 0.0));
        let mut group = Layer::group("Group", vec![child]);
        let mut mask = photocraft_doc::LayerMask::hide_all();
        mask.surface.fill_rect(Rect::new(70, 10, 75, 20), &[1.0]);
        group.mask = Some(mask);
        let group_id = group.id;
        let d = fixture(group);
        let after = moved_wrapping(&d, &[group_id], 10, 0, false, Some(d.bounds())).expect("the wrapping preview");
        let m = after.layer(group_id).and_then(|g| g.mask.as_ref()).expect("the group mask");
        assert_eq!(m.surface.content_bounds(), Rect::new(80, 10, 85, 20), "the group mask keeps the raw delta, as before");
        assert!(!targets_only_wrap(&d, &[group_id], false, 80, 0), "a whole-canvas move still changes the reference point");
    }

    /// A whole-canvas Pattern Preview move of raster layers changes nothing and is skipped, as on
    /// `upstream/main`, until it moves something the drag preview shows: a recorded move phase, or
    /// the grid of a linked Pattern Overlay whose period the raw delta does not fold back onto it.
    /// The wrap of a linked mask does not depend on that phase.
    #[test]
    fn only_the_skip_check_reads_the_move_phase() {
        let (before, id) = doc(shape(Rect::new(10, 10, 20, 20)), Vec::new());
        assert!(targets_only_wrap(&before, &[id], false, 40, 0), "never moved, no overlay: a whole-canvas move is a no-op");
        let after = moved(&before, &[id], 1, 0).expect("the layer moves");
        let l = after.layer(id).expect("the layer");
        assert!(!targets_only_wrap(&after, &[id], false, 40, 0), "with a move phase the whole-canvas move is not skipped");
        assert!(subtree_wraps_only(l, 0), "but the layer still wraps as raster samples");
        // Review (CodeRabbit, moves.rs:458): the FIRST whole-canvas move of a linked overlay is
        // not skipped either when it shifts the grid: a 15 px period on this 40 px canvas moves
        // by 10 px. The end-to-end preview-vs-commit check is
        // `the_first_whole_canvas_wrapping_move_keeps_the_overlay_grid_the_preview_showed`.
        let (mut grid, gid) = doc(shape(Rect::new(10, 10, 20, 20)), vec![linked_overlay_named("stripes")]);
        grid.patterns.push(stripes15());
        assert!(!targets_only_wrap(&grid, &[gid], false, 40, 0), "a linked overlay whose grid shifts keeps the move");
        let (mut exact, eid) = doc(shape(Rect::new(10, 10, 20, 20)), vec![linked_overlay()]);
        exact.patterns.push(quad_pattern());
        assert!(targets_only_wrap(&exact, &[eid], false, 40, 0), "a 20 px grid repeating exactly over the 40 px canvas keeps the skip");
        let (missing, mid) = doc(shape(Rect::new(10, 10, 20, 20)), vec![linked_overlay_named("stripes")]);
        assert!(targets_only_wrap(&missing, &[mid], false, 40, 0), "an overlay whose pattern is not in the document paints nothing: the skip stays");
    }

    /// Review (CodeRabbit, moves.rs:458): a 40 px canvas and a linked pattern with a 15 px
    /// period. The drag preview of the FIRST whole-canvas move records the raw delta and tiles
    /// the grid from (40, 0); the commit used to fold that move away and snap the pattern back
    /// to (0, 0). For a pixel layer and for a group carrying the overlay.
    #[test]
    fn the_first_whole_canvas_wrapping_move_keeps_the_overlay_grid_the_preview_showed() {
        let striped = |l: Layer| {
            let mut l = l;
            l.effects.items = vec![linked_overlay_named("stripes")];
            l
        };
        let cases = [
            ("a pixel layer", striped(Layer::new("Shape", shape(Rect::new(10, 10, 30, 30))))),
            ("a group", striped(Layer::group("Group", vec![Layer::new("Shape", shape(Rect::new(10, 10, 30, 30)))]))),
        ];
        for (what, l) in cases {
            let (mut s, id) = stripes_session(l);
            let window = Rect::new(12, 12, 28, 13);
            let before = open_doc(&s).clone();
            let preview = moved_wrapping(&before, &[id], 40, 0, false, Some(before.bounds())).expect("the wrapping preview");
            assert_ne!(letters(&preview, window), letters(&before, window), "{what}: the preview shifts the 15 px grid by the raw 40 px delta");
            let steps = s.active().expect("a document").history.past_len();
            s.execute("layer.translate", json!({"layer": id.0, "dx": 40, "dy": 0, "wrap": true})).expect("the wrapping move commits");
            assert_eq!(phase(open_doc(&s), id), Some((40.0, 0.0)), "{what}: the commit records the phase the preview showed");
            assert_eq!(s.active().expect("a document").history.past_len(), steps + 1, "{what}: the moved grid is one history step");
            assert_eq!(letters(open_doc(&s), window), letters(&preview, window), "{what}: the committed pattern is what the preview showed");
        }
    }

    /// The other side of the review point: a whole-canvas move of a grid that repeats EXACTLY
    /// over the canvas (the 20 px #14960 pattern on the 80 px fixture) paints the same picture,
    /// so it stays the no-op skip it was on `upstream/main` and records no phase. Guard test,
    /// passes before and after the fix.
    #[test]
    fn a_whole_canvas_wrapping_move_of_a_grid_that_repeats_exactly_stays_skipped() {
        let mut l = Layer::new("Shape", shape(Rect::new(20, 20, 40, 40)));
        l.effects.items = vec![linked_overlay()];
        let (mut s, id) = pattern_session(l);
        let before = open_doc(&s).clone();
        let steps = s.active().expect("a document").history.past_len();
        s.execute("layer.translate", json!({"layer": id.0, "dx": 80, "dy": 0, "wrap": true})).expect("the wrapping move");
        assert_eq!(s.active().expect("a document").history.past_len(), steps, "the grid did not change, so the move stays skipped");
        assert_eq!(phase(open_doc(&s), id), None, "and it records no phase");
        assert_eq!(letters(open_doc(&s), Rect::new(22, 22, 38, 23)), letters(&before, Rect::new(22, 22, 38, 23)), "the picture is unchanged");
    }

    /// A session on an 80 x 80 RGB8 document with the #14960 pattern registered and `layer` above
    /// the Background, and that layer's id.
    fn pattern_session(layer: Layer) -> (Session, LayerId) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 80, "height": 80, "depth": 8, "mode": "rgb", "background": "transparent"})).expect("a document opens");
        let id = layer.id;
        s.edit("setup", |doc, _| {
            doc.patterns.push(quad_pattern());
            doc.layers.push(layer);
            Ok(())
        })
        .expect("the layer is set up");
        (s, id)
    }

    /// Not measured by #14960, pinned as it now is: Layer > Rasterize > Layer on a shape layer with
    /// a linked overlay keeps the style live but turns the content into pixels, and a pixel layer is
    /// one of the kinds the issue measured, so the grid jumps by the layer's own top-left, from its
    /// corner to the document origin, at that transition. Nothing records the old corner as a move
    /// phase; the issue measures no kind transition, and this test keeps the jump from changing by
    /// accident.
    #[test]
    fn rasterize_layer_moves_a_shape_layer_overlay_grid_to_the_document_origin() {
        let path = photocraft_doc::Path::new(vec![photocraft_doc::Subpath::polygon(&[(25.0, 21.0), (45.0, 21.0), (45.0, 41.0), (25.0, 41.0)])]);
        let sh = photocraft_doc::ShapeLayer { path, fill: Some(Fill::Solid(Color::rgb(1.0, 0.5, 0.0))), ..Default::default() };
        let mut l = Layer::new("Shape", LayerContent::Shape(sh));
        l.effects.items = vec![linked_overlay()];
        let (mut s, id) = pattern_session(l);
        // 5 px inside the shape: red-blue on the grid that starts at the layer's corner (25, 21).
        assert_eq!(letters(open_doc(&s), Rect::new(30, 31, 31, 32)), vec!['B'], "a shape layer keeps the corner anchor");
        s.execute("layer.rasterize.layer", json!({"layer": id.0})).expect("the layer rasterizes");
        let d = open_doc(&s);
        assert!(matches!(d.layer(id).expect("the layer").content, LayerContent::Raster(_)), "the content is pixels now");
        assert_eq!(phase(d, id), None, "the transition records no move phase");
        assert_eq!(letters(d, Rect::new(30, 31, 31, 32)), vec!['K'], "the same pixel reads the document-origin grid: the grid jumped with the content kind");
    }

    /// Not measured by #14960, pinned as it now is: Rasterize > Layer Style bakes the style and
    /// clears the recorded phase together with the reference point, so an overlay added after that
    /// bake tiles from the document origin even though the layer physically moved +5,+3. The
    /// issue's "an overlay added after a move tiles from that move" rule does not reach the state a
    /// bake leaves behind, and this test pins that limit instead of guessing a rule for it.
    #[test]
    fn a_new_overlay_after_a_style_bake_starts_at_the_document_origin() {
        let mut l = Layer::new("Shape", shape(Rect::new(20, 16, 40, 36)));
        l.effects.items = vec![linked_overlay()];
        let (mut s, id) = pattern_session(l);
        s.execute("layer.translate", json!({"layer": id.0, "dx": 5, "dy": 3})).expect("the layer moves");
        assert_eq!(phase(open_doc(&s), id), Some((5.0, 3.0)), "the move records the phase the overlay reads");
        // 5 px inside the moved shape: on the grid that starts at (5, 3) the tile is (25, 26) -> red.
        assert_eq!(letters(open_doc(&s), Rect::new(30, 29, 31, 30)), vec!['R'], "the overlay the bake bakes in follows the phase");
        s.execute("layer.rasterize.layerStyle", json!({"layer": id.0})).expect("the style bakes");
        assert_eq!(phase(open_doc(&s), id), None, "the bake clears the phase with the style");
        s.edit("a new overlay", |doc, _| {
            doc.layer_mut(id).expect("the layer").effects.items = vec![linked_overlay()];
            Ok(())
        })
        .expect("the new overlay lands");
        assert_eq!(
            letters(open_doc(&s), Rect::new(30, 29, 31, 30)),
            vec!['G'],
            "the new overlay starts on the document grid, not on the phase the moved layer carried"
        );
    }

    /// A session on a 40 x 40 RGB8 document with two positioned raster layers: `src` carries a
    /// linked Pattern Overlay and has never moved, `dst` has no style. Returns both layer ids.
    fn style_pair() -> (Session, LayerId, LayerId) {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 40, "depth": 8, "mode": "rgb", "background": "transparent"})).expect("a document opens");
        let (src, dst) = s
            .edit("setup", |doc, _| {
                let mut a = Layer::new("A", shape(Rect::new(5, 5, 15, 15)));
                a.effects.items = vec![linked_overlay()];
                let b = Layer::new("B", shape(Rect::new(20, 20, 30, 30)));
                let (ia, ib) = (a.id, b.id);
                doc.layers.push(a);
                doc.layers.push(b);
                Ok((ia, ib))
            })
            .expect("the two layers are set up");
        (s, src, dst)
    }

    /// The open document (the plain fixture above already uses the name `doc`).
    fn open_doc(s: &Session) -> &Document {
        &s.active().expect("a document is open").doc
    }

    /// Carry `src`'s style onto `dst` the way the Layers panel does it: Layer Style > Copy and
    /// Paste, or `through_clipboard` false: dragging the fx row onto the other layer (a copy, so
    /// the source keeps its own style and phase).
    fn carry_style(s: &mut Session, through_clipboard: bool, src: LayerId, dst: LayerId) {
        if through_clipboard {
            s.execute("layer.layerStyle.copyLayerStyle", json!({"layer": src.0})).expect("the style is copied");
            s.execute("layer.layerStyle.pasteLayerStyle", json!({"layer": dst.0})).expect("the style pastes");
        } else {
            s.execute("layer.layerStyle.transferEffects", json!({"from": src.0, "to": dst.0, "copy": true})).expect("the fx row lands");
        }
    }

    /// Review (#14960): `move_phase` records where THIS layer's Move commands took it, so it belongs
    /// to the layer and not to its style, like the `psd_raw` block and the reference point. Carrying
    /// a style from a layer that moved +5,+3 onto a layer that never moved must leave the second
    /// layer's overlay grid on the document origin.
    #[test]
    fn a_carried_style_does_not_move_the_destination_grid() {
        for through_clipboard in [true, false] {
            let (mut s, src, dst) = style_pair();
            s.execute("layer.translate", json!({"layer": src.0, "dx": 5, "dy": 3})).expect("the source layer moves");
            assert_eq!(phase(open_doc(&s), src), Some((5.0, 3.0)), "the move records the source's phase");
            assert_eq!(phase(open_doc(&s), dst), None, "the target has never moved");
            carry_style(&mut s, through_clipboard, src, dst);
            let d = open_doc(&s);
            let l = d.layer(dst).expect("the target layer");
            assert_eq!(l.effects.items.len(), 1, "the style itself arrived");
            assert_eq!(phase(d, dst), None, "clipboard {through_clipboard}: the carried style leaves the target's grid on the document origin");
            assert_eq!(phase(d, src), Some((5.0, 3.0)), "the source keeps its own phase");
        }
    }

    /// The same rule from the other side: pasting a style from a layer that never moved onto a layer
    /// that moved +2,+2 must not take that layer's grid back to the document origin.
    #[test]
    fn a_carried_style_does_not_erase_the_destination_move_phase() {
        for through_clipboard in [true, false] {
            let (mut s, src, dst) = style_pair();
            s.execute("layer.translate", json!({"layer": dst.0, "dx": 2, "dy": 2})).expect("the target layer moves");
            assert_eq!(phase(open_doc(&s), dst), Some((2.0, 2.0)), "the target records its own phase");
            assert_eq!(phase(open_doc(&s), src), None, "the source has never moved");
            carry_style(&mut s, through_clipboard, src, dst);
            let d = open_doc(&s);
            let l = d.layer(dst).expect("the target layer");
            assert_eq!(l.effects.items.len(), 1, "the style itself arrived");
            assert_eq!(phase(d, dst), Some((2.0, 2.0)), "clipboard {through_clipboard}: the target keeps the phase its own move recorded");
        }
    }
}
