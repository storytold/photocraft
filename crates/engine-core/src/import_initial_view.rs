//! One-time native import view handoff, independent of document preparation or editing.

use std::sync::Arc;

use crate::{EngineError, Result, Session};

impl crate::DocState {
    /// Transfer a just-admitted import's view when the host must defer its tab
    /// behind an earlier Open confirmation or Place hold. This is not a general view snapshot:
    /// an edited document or an ordinary native reopen has no fresh import hint.
    pub fn take_fresh_import_view(&mut self) -> Option<photocraft_io::NativeInitialView> {
        if !std::mem::take(&mut self.import_initial_view_applied)
            || self.revision != 1
            || self.view_revision != 0
            || self.history.past_len() != 0
            || self.history.can_redo()
            || self.revision != self.saved_revision
        {
            return None;
        }
        let target = match self.channel_view.target {
            crate::channel_cmds::ChannelTarget::Composite => photocraft_io::NativeImportTarget::Composite,
            crate::channel_cmds::ChannelTarget::LayerMask => photocraft_io::NativeImportTarget::LayerMask,
            crate::channel_cmds::ChannelTarget::Alpha(index) => photocraft_io::NativeImportTarget::AlphaChannel(index),
            _ => return None, // A later manual color/vector target is not an imported hint.
        };
        let show_layer_mask = self.channel_view.layer_mask.is_some_and(|view| {
            view.display_only && !view.filter && view.mode == crate::mask_view_cmds::MaskViewMode::Gray && Some(view.layer) == self.active_layer()
        });
        Some(photocraft_io::NativeInitialView {
            selected_layers: self.selection().to_vec(),
            active_layer: self.active_layer(),
            target,
            selected_path: self.initial_path_selection.take(),
            selected_path_identity: self.initial_path_identity.take(),
            visible_alpha_channels: self.channel_view.alpha_visible.iter().enumerate().filter_map(|(index, visible)| visible.then_some(index)).collect(),
            show_layer_mask,
            samplers: std::mem::take(&mut self.color_samplers),
        })
    }
}

impl Session {
    /// Insert one prepared document, then apply its optional view.
    /// Color policy is resolved by the caller before admission.
    pub fn admit_import_with_initial_view(
        &mut self,
        document: photocraft_doc::Document,
        path: Option<String>,
        source_read_only: bool,
        view: Option<photocraft_io::NativeInitialView>,
    ) -> (usize, Option<String>) {
        let protected = path.as_ref().filter(|_| source_read_only).cloned();
        let index = self.add_document(document, path.filter(|_| !source_read_only));
        if let Some(state) = self.docs.get_mut(index) {
            state.protected_import_source = protected;
        }
        self.finish_import_admission(index, source_read_only, view)
    }

    fn finish_import_admission(&mut self, index: usize, source_read_only: bool, view: Option<photocraft_io::NativeInitialView>) -> (usize, Option<String>) {
        if let Some(state) = self.docs.get_mut(index) {
            state.source_read_only = source_read_only;
        }
        // Application validates every hint before mutating any view state, so a rejected hint
        // can be dropped as a unit without discarding successfully decoded image content.
        let mut view = view;
        let changed_path = view.as_mut().zip(self.docs.get(index)).is_some_and(|(view, state)| {
            if view.matches_selected_path(state.doc()) {
                return false;
            }
            view.selected_path = None;
            view.selected_path_identity = None;
            true
        });
        let warning = self
            .apply_import_initial_view(view)
            .err()
            .map(|error| format!("Native initial view was not applied; the default view was used: {error}"))
            .or_else(|| changed_path.then(|| "Imported path selection was not applied because the selected path changed during admission".into()));
        self.sync_tools_mask_target();
        (index, warning)
    }

    /// Validate and apply an import's native view state once, without an edit or history step.
    /// Admission callers use [`Self::admit_import_with_initial_view`] to report invalid optional hints.
    pub fn apply_import_initial_view(&mut self, view: Option<photocraft_io::NativeInitialView>) -> Result<()> {
        let Some(view) = view else { return Ok(()) };
        let before = self.active_layer_stamp();
        let st = self.active_mut().ok_or(EngineError::NoDocument)?;
        view.validate(st.doc()).map_err(|e| EngineError::Other(e.to_string()))?;
        if !view.selected_layers.is_empty() {
            st.set_active_layer(view.active_layer);
            *st.selection_mut() = view.selected_layers;
            st.layer_anchor = st.active_layer();
            let target = crate::LayerTarget { active: st.active_layer(), selected: st.selection().to_vec() };
            st.history.set_current_layers(target.clone());
            // The opened snapshot predates this one-time view handoff. Update its target only
            // while it still holds this exact admitted document, never an older edited state.
            if let Some(index) = st.snapshots().iter().position(|snapshot| snapshot.id == 0 && Arc::ptr_eq(&snapshot.doc, st.doc()))
                && let Some(snapshot) = st.snapshots_mut().get_mut(index)
            {
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
            st.active_layer.map(|layer| crate::mask_view_cmds::LayerMaskView {
                layer,
                mode: crate::mask_view_cmds::MaskViewMode::Gray,
                filter: false,
                display_only: true,
            })
        } else {
            None
        };
        st.initial_path_selection = view.selected_path;
        st.initial_path_identity = view.selected_path_identity;
        st.import_initial_view_applied = true;
        self.note_layer_change(before);
        self.sync_tools_mask_target();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
