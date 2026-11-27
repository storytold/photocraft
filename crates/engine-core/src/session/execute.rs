use super::history::{fix_selection, visibility_changes};
use super::{Arc, Document, EngineError, LayerId, Result, Session, Value, channel_cmds, commands, hidden_target, jobs, smart_cmds};
use crate::color_channels;

impl Session {
    /// Run a command by id with JSON params. Returns a JSON result.
    ///
    /// Any command accepts an optional `"coalesce": "<key>"` param: consecutive edits with the same
    /// key (and no other edit, undo or redo in between) share one history step, like Photoshop's
    /// single "Edit Type Layer" step for a whole typing session or one step per slider drag.
    ///
    /// Runs synchronously, including job-capable commands (see [`jobs`]; [`Session::start`] runs
    /// those in the background).
    pub fn execute(&mut self, id: &str, params: Value) -> Result<Value> {
        let before = self.active_layer_stamp();
        let result = self.dispatch(id, params, false);
        self.note_layer_change(before);
        match result? {
            jobs::Started::Done(v) => Ok(v),
            // Not reached: inline dispatch never starts a job. Waiting is still correct.
            jobs::Started::Job(j) => self.wait_job(j),
        }
    }

    /// Withdraw an offer the command didn't take.
    pub fn clear_live_stroke(&mut self) {
        self.live_commit = None;
    }

    /// The per-command value checks `dispatch` and `precheck` share: the documented keys, then the
    /// `check_params` hooks.
    pub(crate) fn validate_params(&self, id: &str, spec: &crate::registry::CommandSpec, params: &Value) -> Result<()> {
        crate::param_check::check(spec, params)?;
        crate::registry::hooks().check_params.iter().try_for_each(|check| check(self, id, params))
    }

    /// Check a call without running it: the command exists, its params have the shape
    /// [`Self::execute`] requires (New Layer's fully), and it can run now. For a caller that does
    /// something first only when the command will go through: Free Transform applies its box
    /// before a new layer is made.
    ///
    /// Complete for the commands in [`PRECHECKED`]: there it accepts what `execute` accepts, but
    /// for the group nesting limit (a test holds the two together). For others it checks only what they share.
    pub fn precheck(&mut self, id: &str, params: &Value) -> Result<()> {
        let spec = crate::registry::spec(id)?;
        if !(params.is_object() || params.is_null()) {
            return Err(EngineError::BadParams { cmd: id.to_string(), msg: "params must be a JSON object".into() });
        }
        self.validate_params(id, spec, params)?;
        if id == "layer.new.layer" {
            crate::new_layer::parse(params)?;
        }
        if id == "layer.new.group" {
            crate::group_options::below(id, params)?;
        }
        if id == "layer.new.group" || id == "layer.new.groupFromLayers" {
            crate::group_options::GroupOptions::parse(id, params)?;
        }
        // A command that takes `"layer"` (an id or a name, as it resolves it) fails on one the
        // document doesn't have.
        if spec.params.contains("\"layer\":id")
            && let Some(l) = commands::explicit_layer(self, params)?
            && self.active().is_none_or(|st| st.doc.layer(l).is_none())
        {
            return Err(EngineError::BadParams { cmd: id.to_string(), msg: format!("no layer {}", l.0) });
        }
        self.admit(id, spec, params).map(|_| ())
    }

    /// What `dispatch` checks once the params are valid, before a floating selection drops (so a
    /// refused command leaves it floating) and again after: the precondition (on a named layer
    /// for commands that take one), a job running on the document, a hidden target, then the
    /// engine crates' `admit` hooks (the Magic Eraser's colour-mode alert). The run params (the Channels panel's target filled in) when it
    /// may run.
    pub(crate) fn admit(&mut self, id: &str, spec: &commands::CommandSpec, params: &Value) -> Result<Value> {
        let run_params = channel_cmds::inject_target(self, id, commands::inject_kind(id, params.clone()));
        if let Some(why) = color_channels::refusal(self, id, &run_params) {
            return Err(EngineError::Disabled(id.into(), why));
        }
        if let Err(why) = self.explicit_layer_enabled(spec, params)?.unwrap_or_else(|| self.precondition(spec, &run_params)) {
            return Err(EngineError::Disabled(id.to_string(), why));
        }
        if let Some(why) = self.job_conflict(id, spec.journal) {
            return Err(EngineError::Disabled(id.to_string(), why));
        }
        // Painting on or moving a hidden layer is refused, as in Photoshop (#571).
        if let Some(why) = hidden_target::refusal(self, id, params) {
            return Err(EngineError::Other(why.into()));
        }
        for admit in &crate::registry::hooks().admit {
            admit(self, id, params)?;
        }
        Ok(run_params)
    }

    /// Is the command currently runnable? (drives menu enablement)
    pub fn is_enabled(&self, id: &str) -> bool {
        self.is_enabled_with(id, &Value::Null)
    }

    /// [`Self::is_enabled`] for a call with `params`: their `"target"` can enable a command, as a
    /// targeted layer mask enables Image › Adjustments › Invert on an adjustment layer (#780).
    pub fn is_enabled_with(&self, id: &str, params: &Value) -> bool {
        // An unknown id is disabled; its reason would rank every command id for "did you mean".
        commands::find(id).is_some_and(|spec| self.spec_disabled_reason(spec, id, params).is_none())
    }

    /// Why the command can't run now (None = it can): its own precondition, or a background job
    /// running on the active document.
    pub fn disabled_reason(&self, id: &str) -> Option<String> {
        self.disabled_reason_with(id, &Value::Null)
    }

    /// [`Self::disabled_reason`] for a call with `params`.
    pub fn disabled_reason_with(&self, id: &str, params: &Value) -> Option<String> {
        let spec = match crate::registry::spec(id) {
            Ok(spec) => spec,
            Err(e) => return Some(e.to_string()),
        };
        self.spec_disabled_reason(spec, id, params)
    }

    /// [`Self::disabled_reason_with`] for the catalogue entry `spec` of `id`.
    fn spec_disabled_reason(&self, spec: &commands::CommandSpec, id: &str, params: &Value) -> Option<String> {
        self.precondition(spec, &channel_cmds::inject_target(self, id, params.clone())).err().or_else(|| self.job_conflict(id, spec.journal))
    }

    /// The command's own precondition for a call with `params` (their target filled in).
    fn precondition(&self, spec: &commands::CommandSpec, params: &Value) -> std::result::Result<(), String> {
        if let Some(why) = color_channels::refusal(self, spec.id, params) {
            return Err(why);
        }
        if self.names_existing_layer(spec.id, params) {
            return Ok(());
        }
        // A recorded invalid comp index succeeds without an edit, even with no comps.
        // Ordinary Apply and references by id/name retain the command's usual precondition.
        if spec.id == "layerComp.apply"
            && params
                .get("comp")
                .and_then(Value::as_object)
                .is_some_and(|fields| fields.len() == 1 && fields.get("index").is_some_and(|index| index.as_u64().is_some() || index.as_i64().is_some()))
        {
            return self.active().map(|_| ()).ok_or_else(|| "no document open".into());
        }
        // `ifPresent`: with no Background there is nothing to do, which isn't an error.
        if spec.id == "layer.new.layerFromBackground" && params.get("ifPresent").and_then(Value::as_bool) == Some(true) {
            return self.active().map(|_| ()).ok_or_else(|| "no document open".into());
        }
        if spec.id == "layer.smartFilter.deleteFilterMask" && params.get("ifPresent").and_then(Value::as_bool) == Some(true) {
            return smart_cmds::smart_layer_active(self);
        }
        // Recorded Unlink can be repeated on a single, already-unlinked layer.
        // Keep the menu toggle's two-layer requirement when no state is supplied.
        if spec.id == "layer.linkLayers" && params.get("linked").and_then(Value::as_bool) == Some(false) {
            let d = self.active().ok_or_else(|| "no document open".to_string())?;
            return if d.selected_layers().is_empty() { Err("no layer selected".into()) } else { Ok(()) };
        }
        if let Some(why) = bitmap_mode_refusal(self, spec.id, params) {
            return Err(why.into());
        }
        channel_cmds::mask_target_enabled(self, spec.id, params).unwrap_or_else(|| (spec.enabled)(self))
    }

    /// Whether `id` is a layer command whose `"layer"` param names an existing layer, so it needs no
    /// active layer (#2457). Matches how `layer_param` resolves the layer when the command runs.
    fn names_existing_layer(&self, id: &str, params: &Value) -> bool {
        (matches!(id, "layer.hideLayers" | "layer.showLayers" | "layer.renameLayer")
            || (id == "layer.setProps"
                && params.as_object().is_some_and(|o| o.contains_key("visible") && o.keys().all(|k| matches!(k.as_str(), "layer" | "visible" | "coalesce")))))
            && params.get("layer").and_then(Value::as_u64).is_some_and(|l| self.active().is_some_and(|st| st.doc.layer(LayerId(l)).is_some()))
    }

    /// The precondition of a command whose params document `"layer":id` (it acts on that layer),
    /// when `params` name an existing layer of the active document: checked on that layer rather
    /// than on the active one, which stays active (#466). A script, action or agent can then
    /// Replace Contents, Rasterize or Merge Down a layer without selecting it first. `None` for
    /// other commands, or without such a layer.
    pub(crate) fn explicit_layer_enabled(&mut self, spec: &commands::CommandSpec, params: &Value) -> Result<Option<std::result::Result<(), String>>> {
        if !spec.params.contains("\"layer\":id") {
            return Ok(None);
        }
        // An ATN group-end index is a valid no-op only for Select. Let that command validate
        // its other parameters and preserve the current selection.
        if spec.id == "layer.select"
            && let Some(selector @ Value::Object(fields)) = params.get("layer")
            && (fields.contains_key("index") || fields.contains_key("ordinal"))
            && commands::recorded_layer(self, selector)?.is_none()
        {
            return Ok(None);
        }
        let target = match commands::explicit_layer(self, params) {
            Ok(Some(target)) => target,
            Ok(None) => return Ok(None),
            // Read-only queries keep their own parameter-error contract; recorded commands
            // must identify a missing named receiver before checking the active layer.
            Err(_) if !spec.journal => return Ok(None),
            Err(error) => return Err(error),
        };
        let Some(d) = self.active_mut() else { return Ok(None) };
        if d.doc.layer(target).is_none() {
            return Ok(None);
        }
        let active = d.active_layer;
        d.set_active_layer(Some(target));
        // The checks see the resolved id: a relative ordinal (`Frwr`, `Bckw`) would otherwise
        // resolve again from the swapped-in active layer and land one layer further.
        let mut resolved = params.clone();
        if let Some(fields) = resolved.as_object_mut() {
            fields.insert("layer".into(), Value::from(target.0));
        }
        let r = self.precondition(spec, &resolved);
        if let Some(d) = self.active_mut() {
            d.set_active_layer(active);
        }
        Ok(Some(r))
    }

    /// Apply an undoable edit to the active document.
    pub fn edit<R>(&mut self, label: &str, f: impl FnOnce(&mut Document, &mut Option<LayerId>) -> Result<R>) -> Result<R> {
        self.edit_with(label, true, f)
    }

    /// Apply a change of layer visibility (and nothing else): a history state when History
    /// Options › Make Layer Visibility Changes Undoable is on (the default, as in Photoshop),
    /// otherwise the document changes without one, so the History panel and Undo skip it.
    pub fn edit_visibility<R>(&mut self, label: &str, f: impl FnOnce(&mut Document, &mut Option<LayerId>) -> Result<R>) -> Result<R> {
        let record = self.visibility_undoable();
        self.edit_with(label, record, f)
    }

    /// History Options › Make Layer Visibility Changes Undoable.
    pub fn visibility_undoable(&self) -> bool {
        self.prefs().history_options.make_layer_visibility_changes_undoable
    }

    /// [`Self::edit`], recording a history state only when `record` is set. An unrecorded change
    /// still marks the document modified and leaves the redo states alone.
    pub(crate) fn edit_with<R>(&mut self, label: &str, record: bool, f: impl FnOnce(&mut Document, &mut Option<LayerId>) -> Result<R>) -> Result<R> {
        if record {
            crate::actions_cmds::before_action_edit(self);
        }
        // Every mutation path (linked-file refresh, transform, place) re-renders smart objects
        // through this entry; the requesting session's authority must reach those renders.
        let _authorization = crate::jobs::render_authorization_scope(self.authorize);
        let _document = crate::doc_path::scope(self.active().and_then(|d| d.path.as_deref()).map(std::path::Path::new));
        photocraft_compose::discard_unreadable_pixel_caches();
        let scratch_generation = photocraft_raster::spill::read_error_generation();
        let restrict = self.color_restrict;
        let filter_mask_layer = self.filter_mask_layer;
        let sampler_map = self.sampler_map.take();
        let symmetry_map = self.symmetry_map.take();
        let st = self.active_mut().ok_or(EngineError::NoDocument)?;
        let before = st.doc.clone();
        // Only this document's pixels gate its edits: an unreadable tile elsewhere (another
        // open document) does not block it.
        before.scratch_health().map_err(EngineError::Other)?;
        // Selecting layers is not a step, so the state this edit leaves behind targets what was
        // selected just before it (#1356): undoing a stroke keeps the painted layer active.
        let prior = st.layer_target();
        let previous_active_layer = st.active_layer;
        let mut doc = (*before).clone();
        let mut active = st.active_layer;
        let r = f(&mut doc, &mut active)?;
        if let Some(restrict) = restrict {
            channel_cmds::restrict_to_color(&before, &mut doc, restrict.layer, restrict.colors)?;
        }
        if let Some(id) = filter_mask_layer {
            smart_cmds::refresh_after_filter_mask_edit(&mut doc, id)?;
        }
        if let Err(error) = photocraft_raster::spill::check_since(scratch_generation).and_then(|()| before.scratch_health()).and_then(|()| doc.scratch_health())
        {
            st.set_active_layer(previous_active_layer);
            return Err(EngineError::Other(error));
        }
        *st.doc_mut() = Arc::new(doc);
        st.set_active_layer(active);
        fix_selection(st);
        channel_cmds::fix_view(st);
        // A canvas-size change (crop, Canvas Size, Image Size, rotation) moves the Color Sampler
        // points with their pixels and drops the ones that land outside the new canvas. View
        // state: not a history step. A mapped edit (the four above) gives the exact map; any other
        // size change falls back to dropping whatever is now outside.
        let mut samplers_removed = 0;
        // The points sit on pixel centres (#11725), so they follow the geometry map, the one the
        // symmetry gets: `sampler_map` is that map shifted for pixel indices (a flip or rotation).
        if let Some(a) = sampler_map.and(symmetry_map) {
            let bounds = st.doc.bounds();
            let n = st.color_samplers.len();
            st.color_samplers.retain_mut(|p| {
                let q = a.apply(photocraft_geom::Point::new(p[0], p[1]));
                *p = [q.x, q.y];
                bounds.contains(q.x.floor() as i32, q.y.floor() as i32)
            });
            samplers_removed = n - st.color_samplers.len();
        } else if st.doc.size != before.size {
            let bounds = st.doc.bounds();
            let n = st.color_samplers.len();
            st.color_samplers.retain(|p| bounds.contains(p[0].floor() as i32, p[1].floor() as i32));
            samplers_removed = n - st.color_samplers.len();
        }
        // The painting symmetry stays on its pixels too (Photoshop keeps it as a path).
        if let Some(a) = symmetry_map {
            st.symmetry = st.symmetry.take().and_then(|sym| sym.transform(&a));
        }
        // The pixels the edit can have changed, as its undo would report them: nothing for a
        // selection, guides or a layer's name (a new lasso selection used to recomposite a large
        // document, seconds on the CPU path, #1773), a new or deleted layer's bounds (a
        // duplicate or a paste), and everything when that can't be bounded. A command may still
        // report its own, tighter damage after this.
        let damage = crate::damage::step_damage(&before, &st.doc);
        let keep_clean = st.keeps_clean(&before);
        #[cfg(any(test, debug_assertions))]
        let hint_before = before.clone();
        let key = self.coalesce_request.clone();
        let st = self.active_mut().ok_or(EngineError::NoDocument)?;
        let layers = st.layer_target();
        if !record {
            // Nothing to coalesce with: the next recorded step starts a state of its own.
            st.coalesce = None;
        } else if key.is_none() || st.coalesce != key || !st.history.can_undo() {
            st.history.set_current_layers(prior);
            st.history.record(label, before.clone(), layers);
            let changes = visibility_changes(&before, &st.doc);
            st.history.add_current_visibility(&changes);
            st.trim_history();
            st.coalesce = key;
        } else {
            st.history.set_current_layers(layers);
            let changes = visibility_changes(&before, &st.doc);
            st.history.add_current_visibility(&changes);
            st.coalesce = key;
        }
        st.bump_revision(keep_clean);
        st.last_damage = damage;
        st.record_damage();
        self.samplers_removed += samplers_removed;
        #[cfg(any(test, debug_assertions))]
        {
            self.hint_before = Some(hint_before);
        }
        if damage.is_none_or(|d| !d.is_empty()) {
            for edited in &crate::registry::hooks().pixels_edited {
                edited(self);
            }
        }
        Ok(r)
    }

    /// A command's own rect for the pixels its latest [`Session::edit`] changed: sub-tile precise
    /// for a brush dab, where the edit's own damage ([`crate::layer_multi_cmds::step_damage`]) has tile
    /// precision. The redraw area becomes the intersection of the two, so a hint can only narrow
    /// it (or bound a change `step_damage` can't). The hint is first grown as `step_damage` grows
    /// a change (by a mask's feather and by effects, `grow_hint`), so the intersection keeps a
    /// dab's soft ring and shadow. In debug and test builds, a hint that misses a tile the edit
    /// changed is counted in [`Session::damage_misses`] and logged; it never panics.
    pub fn hint_damage(&mut self, rect: photocraft_geom::Rect) {
        #[cfg(any(test, debug_assertions))]
        if let Some(before) = self.hint_before.take()
            && let Some(st) = self.active()
            && st.doc.id == before.id
        {
            let misses = crate::damage::hint_misses(&before, &st.doc, rect);
            if misses > 0 {
                let step = st.history.undo_label().unwrap_or("?");
                eprintln!("warning: `{step}` hinted damage {rect:?} that misses {misses} changed tile(s); the canvas may not redraw them");
                self.damage_misses += misses;
            }
        }
        let Some(st) = self.active_mut() else { return };
        let rect = crate::damage::grow_hint(&st.doc, rect);
        st.last_damage = Some(st.last_damage.map_or(rect, |d| d.intersect(&rect)));
        st.record_damage();
    }
}

/// A Bitmap document holds only black and white, so Photoshop greys out its filters and its
/// Image › Adjustments, all but Invert (#10484), filter plug-ins included (`plugin.run`; the
/// other `plugin.*` commands manage plug-ins). Catalogue queries still answer.
fn bitmap_mode_refusal(s: &Session, id: &str, params: &Value) -> Option<&'static str> {
    let edits = (id.starts_with("filter.") || id.starts_with("image.adjustments.") || id == "plugin.run")
        && id != "image.adjustments.invert"
        && !id.ends_with(".list")
        // Only the Filter Gallery answers `{"list": true}` with its catalogue; other commands ignore it.
        && !(id == "filter.filterGallery" && params.get("list").and_then(Value::as_bool) == Some(true));
    (edits && s.active().is_some_and(|st| st.doc.mode == photocraft_color::ColorMode::Bitmap)).then_some("not available in Bitmap mode")
}

#[cfg(test)]
mod bitmap_mode_tests {
    use serde_json::json;

    /// #10484: Photoshop 2025 on a Bitmap document refuses Gaussian Blur, Add Noise, Levels and
    /// Brightness/Contrast, and runs Invert. They all ran.
    #[test]
    fn bitmap_documents_refuse_filters_and_adjustments_but_invert() {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 16, "height": 16, "mode": "grayscale"})).unwrap();
        s.edit("mode", |doc, _| {
            doc.mode = photocraft_color::ColorMode::Bitmap;
            Ok(())
        })
        .unwrap();
        for id in ["filter.blur.gaussianBlur", "filter.noise.addNoise", "image.adjustments.levels", "image.adjustments.brightnessContrast"] {
            assert!(!s.is_enabled(id), "{id} is greyed out");
            let e = s.execute(id, json!({})).unwrap_err().to_string();
            assert!(e.contains("Bitmap mode"), "{id}: {e}");
        }
        assert!(s.is_enabled("image.adjustments.invert"));
        s.execute("image.adjustments.invert", json!({})).unwrap();
        assert!(s.execute("filter.filterGallery", json!({"list": true})).is_ok(), "a catalogue query still answers");
        // Grayscale again: the filters are back.
        s.edit("mode", |doc, _| {
            doc.mode = photocraft_color::ColorMode::Grayscale;
            Ok(())
        })
        .unwrap();
        assert!(s.is_enabled("filter.blur.gaussianBlur"));
    }

    /// `{"list": true}` answers a catalogue only where the command has one (the Filter Gallery);
    /// HDR Toning ignored it and edited the Bitmap document.
    #[test]
    fn bitmap_documents_refuse_a_list_param_on_commands_without_a_catalogue() {
        let mut s = crate::session();
        s.execute("file.new", json!({"width": 16, "height": 16, "mode": "grayscale", "background": "white"})).unwrap();
        s.execute("image.mode.bitmap", json!({"method": "threshold"})).unwrap();
        let steps = s.active().unwrap().history.past_len();
        let list = json!({"list": true});
        for spec in crate::commands::command_specs() {
            let id = spec.id;
            let edits = (id.starts_with("filter.") || id.starts_with("image.adjustments."))
                && !matches!(id, "image.adjustments.invert" | "filter.filterGallery")
                && !id.ends_with(".list");
            if edits {
                let why = s.disabled_reason_with(id, &list);
                assert!(why.as_deref().is_some_and(|w| w.contains("Bitmap mode")), "{id} with {{\"list\": true}}: {why:?}");
            }
        }
        let e = s.execute("image.adjustments.hdrToning", json!({"list": true, "exposure": 1})).unwrap_err().to_string();
        assert!(e.contains("Bitmap mode"), "{e}");
        assert_eq!(s.active().unwrap().history.past_len(), steps, "no edit");
        assert!(s.execute("filter.filterGallery", list).unwrap()["filters"].is_array(), "the gallery catalogue still answers");
        assert!(s.execute("image.adjustments.colorLookup.list", json!({})).is_ok());
    }
}
