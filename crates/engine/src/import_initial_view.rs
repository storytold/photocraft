//! One-time native import view handoff, independent of document preparation or editing.

use std::sync::Arc;

use crate::{EngineError, Result, Session};

impl Session {
    /// Admit one imported document with the caller's existing color policy, then apply its view.
    /// The callback must add exactly one document and return its index plus any policy report.
    /// Invalid optional view hints leave the admitted document's default view intact and return
    /// a warning for the host's import notices. Invalid admission callbacks still roll back.
    pub fn admit_import_with_initial_view<T>(
        &mut self,
        source_read_only: bool,
        view: Option<photocraft_io::NativeInitialView>,
        admit: impl FnOnce(&mut Session) -> (usize, T),
    ) -> Result<(usize, T, Option<String>)> {
        let previous = self.active().map(|state| state.doc.id);
        let previous_len = self.docs.len();
        let (index, report) = admit(self);
        if index != previous_len || self.docs.len() != previous_len.saturating_add(1) || self.active != Some(index) {
            // The callback's returned index is untrusted on this path. Remove the actual
            // appended suffix, including multiple admissions, without closing an older tab.
            while self.docs.len() > previous_len {
                self.close(self.docs.len() - 1);
            }
            self.active = previous.and_then(|id| self.docs.iter().position(|state| state.doc.id == id));
            self.note_active_change();
            self.sync_tools_mask_target();
            return Err(EngineError::Other("import admission did not append and activate exactly one document".into()));
        }
        if let Some(state) = self.docs.get_mut(index) {
            state.source_read_only = source_read_only;
        }
        // Application validates every hint before mutating any view state, so a rejected hint
        // can be dropped as a unit without discarding successfully decoded image content.
        let warning =
            self.apply_import_initial_view(view).err().map(|error| format!("Native initial view was not applied; the default view was used: {error}"));
        self.sync_tools_mask_target();
        Ok((index, report, warning))
    }

    /// Validate and apply an import's native view state once, without an edit or history step.
    /// Admission callers use [`Self::admit_import_with_initial_view`] to report invalid optional hints.
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
}

#[cfg(test)]
mod rollback_review_tests;

#[cfg(test)]
mod initial_view_tests {
    use super::*;
    use photocraft_doc::Document;
    use photocraft_io::{NativeImportTarget, NativeInitialView};
    use serde_json::json;

    #[test]
    fn selected_layers_without_active_layer_keep_the_imported_default_view_and_warn() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":8,"height":6})).unwrap();
        session.execute("layer.new.layer", json!({"name":"existing edit"})).unwrap();
        let previous = session.active().unwrap().clone();
        let previous_save = photocraft_format::save_to_bytes(&previous.doc, &Default::default()).unwrap();
        let mut imported = Document::new("retained", photocraft_geom::Size::new(8, 6), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        imported.layers.extend(["first", "last"].map(|name| photocraft_doc::Layer::raster(name, photocraft_color::PixelFormat::RGBA8)));
        let selected = imported.layers[0].id;
        let (index, default_view, warning) = session
            .admit_import_with_initial_view(
                true,
                Some(NativeInitialView { selected_layers: vec![selected], samplers: vec![[2., 3.]], ..Default::default() }),
                |session| {
                    let index = session.add_document(imported, None);
                    (index, session.active().unwrap().clone())
                },
            )
            .unwrap();
        assert!(warning.unwrap().contains("selected layers require an active layer"));
        assert_eq!(session.documents().len(), 2);
        assert_eq!(session.active_index(), Some(index));
        let state = session.active().unwrap();
        assert!(Arc::ptr_eq(&state.doc, &default_view.doc));
        assert!(state.source_read_only);
        assert_eq!(state.active_layer, default_view.active_layer);
        assert!(state.active_layer.is_some());
        assert_eq!(state.selected_layers, default_view.selected_layers);
        assert_eq!(state.layer_anchor, default_view.layer_anchor);
        assert_eq!(state.channel_view, default_view.channel_view);
        assert_eq!(state.color_samplers, default_view.color_samplers);
        assert_eq!((state.revision, state.saved_revision, state.history.past_len()), (default_view.revision, default_view.saved_revision, 0));
        assert_eq!(state.snapshots.first().unwrap().layers, default_view.snapshots.first().unwrap().layers);
        let saved = photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap();
        let reopened = photocraft_io::import("retained.pcraft", &saved).unwrap();
        assert!(reopened.initial_view.is_none());
        assert_eq!(reopened.document.layer_count(), 2);
        let existing = &session.documents()[0];
        assert!(Arc::ptr_eq(&existing.doc, &previous.doc));
        assert_eq!(existing.active_layer, previous.active_layer);
        assert_eq!(existing.selected_layers, previous.selected_layers);
        assert_eq!(existing.layer_anchor, previous.layer_anchor);
        assert_eq!(existing.path, previous.path);
        assert_eq!(existing.source_read_only, previous.source_read_only);
        assert_eq!(
            (existing.revision, existing.saved_revision, existing.history.past_len()),
            (previous.revision, previous.saved_revision, previous.history.past_len())
        );
        assert_eq!(photocraft_format::save_to_bytes(&existing.doc, &Default::default()).unwrap(), previous_save);
    }

    #[test]
    fn invalid_optional_view_keeps_admitted_document_and_existing_documents_unchanged() {
        for (existing, active) in [(0, None), (3, None), (3, Some(0)), (3, Some(1))] {
            let mut session = Session::new();
            for index in 0..existing {
                session.execute("file.new", json!({"width": 8, "height": 6, "name": format!("existing {index}")})).unwrap();
                session.execute("layer.layerMask.revealAll", json!({})).unwrap();
                let layer = session.active().unwrap().active_layer.unwrap();
                session
                    .apply_import_initial_view(Some(NativeInitialView {
                        selected_layers: vec![layer],
                        active_layer: Some(layer),
                        target: NativeImportTarget::LayerMask,
                        ..Default::default()
                    }))
                    .unwrap();
            }
            session.active = active;
            session.note_active_change();
            session.sync_tools_mask_target();
            let before = session.documents().to_vec();
            let order = session.open_document_order.clone();
            let mut imported = Document::new("retained", photocraft_geom::Size::new(8, 6), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
            imported.layers.extend(["first", "last"].map(|name| photocraft_doc::Layer::raster(name, photocraft_color::PixelFormat::RGBA8)));
            let default_layer = imported.top_layer();
            let hinted_layer = imported.layers.iter().find(|layer| Some(layer.id) != default_layer).unwrap().id;
            imported.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 8, 6), &[0.2, 0.4, 0.6, 1.]);
            let expected_pixels = imported.layers[0].surface().unwrap().clone();
            if let Some(state) = before.first() {
                imported.id = state.doc.id; // Admission must remap this collision without changing the prior identity.
            }
            let (index, (), warning) = session
                .admit_import_with_initial_view(
                    true,
                    Some(NativeInitialView {
                        selected_layers: vec![hinted_layer],
                        active_layer: Some(hinted_layer),
                        samplers: vec![[2., 3.]],
                        selected_path: Some(0),
                        ..Default::default()
                    }),
                    |session| (session.add_document(imported, None), ()),
                )
                .unwrap();
            assert!(warning.unwrap().contains("selected path is absent"));
            assert_eq!(session.active_index(), Some(index));
            assert_eq!(session.documents().len(), before.len() + 1);
            assert_eq!(&session.open_document_order[..order.len()], order.as_slice());
            let admitted = session.active().unwrap();
            assert!(admitted.source_read_only);
            assert!(admitted.path.is_none());
            assert_eq!(admitted.active_layer, default_layer);
            assert_eq!(admitted.selected_layers, default_layer.into_iter().collect::<Vec<_>>());
            assert!(admitted.color_samplers.is_empty());
            assert!(admitted.initial_path_selection.is_none());
            assert_eq!(admitted.channel_view.target, crate::channel_cmds::ChannelTarget::Composite);
            assert!(!session.tools.mask_targeted);
            assert_eq!(admitted.history.past_len(), 0);
            assert_eq!(admitted.snapshots.first().unwrap().layers.active, default_layer);
            assert_eq!(admitted.snapshots.first().unwrap().layers.selected, admitted.selected_layers);
            assert_eq!(admitted.doc.layers[0].surface().unwrap().pixel(0, 0), expected_pixels.pixel(0, 0));
            for (state, previous) in session.documents().iter().zip(before) {
                assert!(Arc::ptr_eq(&state.doc, &previous.doc));
                assert_eq!(state.path, previous.path);
                assert_eq!(state.source_read_only, previous.source_read_only);
                assert_eq!(state.active_layer, previous.active_layer);
                assert_eq!(state.selected_layers, previous.selected_layers);
                assert_eq!(state.channel_view, previous.channel_view);
                assert_eq!(state.color_samplers, previous.color_samplers);
                assert_eq!(
                    (state.revision, state.saved_revision, state.history.past_len()),
                    (previous.revision, previous.saved_revision, previous.history.past_len())
                );
            }
        }
    }

    #[test]
    fn import_admission_rejects_a_callback_returning_an_existing_document() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
        let original = session.active().unwrap().doc.clone();
        assert!(session.admit_import_with_initial_view(true, Some(NativeInitialView::default()), |_| (0, ())).is_err());
        assert_eq!(session.documents().len(), 1);
        assert!(Arc::ptr_eq(&session.active().unwrap().doc, &original));
        assert!(!session.active().unwrap().source_read_only);
    }

    #[test]
    fn invalid_admission_callback_restores_prior_active_identity_including_none() {
        for active in [None, Some(0)] {
            let mut session = Session::new();
            session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
            let original = session.active().unwrap().doc.clone();
            session.active = active;
            session.note_active_change();
            let imported = Document::new("not activated", photocraft_geom::Size::new(8, 6), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
            let result = session.admit_import_with_initial_view(true, None, |session| {
                let index = session.add_document(imported, None);
                session.active = active; // The callback failed to activate its newly appended document.
                (index, ())
            });
            assert!(result.unwrap_err().to_string().contains("import admission"));
            assert_eq!(session.documents().len(), 1);
            assert_eq!(session.active_index(), active);
            assert!(Arc::ptr_eq(&session.documents()[0].doc, &original));
            assert!(!session.documents()[0].source_read_only);
            assert!(!session.tools.mask_targeted);
        }
    }

    #[test]
    fn import_view_is_validated_against_the_admitted_document_and_warns_on_late_rejection() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
        let original = session.active().unwrap().doc.clone();
        let mut imported = Document::new("pending", photocraft_geom::Size::new(8, 6), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        imported.paths.push(photocraft_doc::vector::NamedPath { name: "Selected saved path".into(), ..Default::default() });
        let view = NativeInitialView { selected_path: Some(0), ..Default::default() };
        view.validate(&imported).unwrap();
        let result = session.admit_import_with_initial_view(true, Some(view), |session| {
            let index = session.add_document(imported, None);
            // Admission hooks can change the pending data after a decoder-time check.
            Arc::make_mut(&mut session.active_mut().unwrap().doc).paths.clear();
            (index, ())
        });
        let (index, (), warning) = result.unwrap();
        assert!(warning.unwrap().contains("selected path is absent"));
        assert_eq!(session.documents().len(), 2);
        assert_eq!(session.active_index(), Some(index));
        assert!(session.active().unwrap().initial_path_selection.is_none());
        assert!(Arc::ptr_eq(&session.documents()[0].doc, &original));
    }

    #[test]
    fn import_admission_preserves_policy_report_read_only_and_native_view() {
        let mut session = Session::new();
        let document = Document::new("accepted", photocraft_geom::Size::new(8, 6), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let (index, color, warning) = session
            .admit_import_with_initial_view(true, Some(NativeInitialView { samplers: vec![[2., 3.]], ..Default::default() }), |session| {
                session.open_document(document, None)
            })
            .unwrap();
        assert!(warning.is_none());
        assert!(color.is_object());
        assert_eq!(session.active_index(), Some(index));
        let state = session.active().unwrap();
        assert!(state.source_read_only);
        assert!(state.path.is_none());
        assert_eq!(state.color_samplers, vec![[2., 3.]]);
        assert_eq!(state.history.past_len(), 0);
    }

    #[test]
    fn imported_mask_and_channel_views_target_existing_native_items_without_edits() {
        let mut session = Session::new();
        session.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
        session.execute("layer.layerMask.revealAll", json!({})).unwrap();
        session.execute("channel.new", json!({})).unwrap();
        session.execute("channel.new", json!({})).unwrap();
        let state = session.active().unwrap();
        let layer = state.active_layer.unwrap();
        let before = (state.revision, state.history.past_len(), state.doc.clone());
        session
            .apply_import_initial_view(Some(NativeInitialView {
                selected_layers: vec![layer],
                active_layer: Some(layer),
                target: NativeImportTarget::LayerMask,
                show_layer_mask: true,
                visible_alpha_channels: vec![1],
                samplers: vec![[2.5, 3.5]],
                ..Default::default()
            }))
            .unwrap();
        let state = session.active().unwrap();
        assert_eq!(state.channel_view.target, crate::channel_cmds::ChannelTarget::LayerMask);
        assert_eq!(state.channel_view.layer_mask.as_ref().unwrap().layer, layer);
        assert!(state.channel_view.alpha_shown(1));
        assert!(session.tools.mask_targeted);
        assert_eq!((state.revision, state.history.past_len()), (before.0, before.1));
        assert!(Arc::ptr_eq(&state.doc, &before.2));
        session.apply_import_initial_view(Some(NativeInitialView { target: NativeImportTarget::AlphaChannel(0), ..Default::default() })).unwrap();
        assert_eq!(session.active().unwrap().channel_view.target, crate::channel_cmds::ChannelTarget::Alpha(0));
        assert_eq!(session.active().unwrap().active_layer, Some(layer), "an empty layer hint preserves the current layer target");
        assert_eq!(session.active().unwrap().selected_layers, vec![layer]);
        assert!(!session.tools.mask_targeted);
    }

    /// Original XCFv3 image properties: one modern sampler at (2,3), no drawable layers.
    fn sampler_xcf() -> Vec<u8> {
        let mut wire = b"gimp xcf v003\0".to_vec();
        for word in [8u32, 6, 0, 39, 20, 2, 3, 0, 0, 0, 0, 0, 0, 0] {
            wire.extend_from_slice(&word.to_be_bytes());
        }
        wire
    }

    #[test]
    fn foreground_and_background_open_apply_the_import_hint_once_and_keep_source_protection() {
        let bytes = sampler_xcf();
        assert_eq!(photocraft_io::import("samplers.xcf", &bytes).unwrap().initial_view.unwrap().samplers, vec![[2., 3.]]);
        for background in [false, true] {
            let mut session = Session::new();
            if background {
                let result = match session.start_open("samplers.xcf", crate::jobs::OpenSource::Bytes(Arc::new(bytes.clone()))).unwrap() {
                    crate::jobs::Started::Done(result) => result,
                    crate::jobs::Started::Job(job) => session.wait_job(job).unwrap(),
                };
                assert_eq!(result["hasInitialView"], true, "a requeue must retain the original handoff's presence");
            } else {
                crate::file_cmds::open_bytes_as(&mut session, "samplers.xcf", &bytes, None, Some("samplers.xcf".into())).unwrap();
            }
            let state = session.active().unwrap();
            assert_eq!(state.color_samplers, vec![[2., 3.]], "background={background}");
            assert!(state.source_read_only);
            assert!(state.path.is_none());
            assert_eq!((state.revision, state.history.past_len()), (1, 0));
            let native = photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap();
            let mut reopened = Session::new();
            crate::file_cmds::open_bytes_as(&mut reopened, "reopened.pcraft", &native, None, None).unwrap();
            assert!(reopened.active().unwrap().color_samplers.is_empty(), "native reopen must not replay historical hints");
        }
    }

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
            NativeInitialView { selected_layers: before.selected_layers.clone(), samplers: vec![[2., 3.]], ..Default::default() },
            NativeInitialView { samplers: vec![[f64::NAN, 0.0]], ..Default::default() },
            NativeInitialView { target: NativeImportTarget::AlphaChannel(0), ..Default::default() },
            NativeInitialView { selected_path: Some(0), ..Default::default() },
        ] {
            assert!(session.apply_import_initial_view(Some(view)).is_err());
            let st = session.active().unwrap();
            assert!(Arc::ptr_eq(&before.doc, &st.doc));
            assert_eq!(st.active_layer, before.active_layer);
            assert_eq!(st.selected_layers, before.selected_layers);
            assert_eq!(st.layer_anchor, before.layer_anchor);
            assert_eq!(st.channel_view, before.channel_view);
            assert_eq!(st.color_samplers, before.color_samplers);
            assert_eq!(st.revision, before.revision);
            assert_eq!(st.saved_revision, before.saved_revision);
            assert_eq!(st.history.past_len(), before.history.past_len());
            assert_eq!(st.snapshots.first().unwrap().layers, before.snapshots.first().unwrap().layers);
        }
    }

    #[test]
    /// Keep the source view with a held import and verify taking it consumes the handoff exactly once without
    /// installing a tab.
    fn held_native_initial_view_is_kept_until_taken_once() {
        let doc = Document::new("held", photocraft_geom::Size::new(8, 6), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        let mut session = Session::new();
        let view = NativeInitialView { samplers: vec![[1.0, 2.0]], ..Default::default() };
        let id = session.hold_open_with_view(doc, true, Some(view.clone()));
        let (_, read_only, pending) = session.take_held_open_with_view(id).unwrap();
        assert!(read_only);
        assert_eq!(pending, Some(view));
        assert!(session.take_held_open_with_view(id).is_none());
        assert!(session.documents().is_empty());
    }
}
