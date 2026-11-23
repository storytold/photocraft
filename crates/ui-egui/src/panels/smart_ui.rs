//! Layers-panel pieces for smart objects: the thumbnail badge and the Smart Filters sub-rows
//! (eye toggles; double-click a filter to re-open its dialog with the recorded parameters; click
//! to select one; drag onto another smart object to move it there, ⌥-drag to copy it, #4271;
//! drag it up or down its own stack to reorder it).
//! The Smart Filters row carries the filter mask thumbnail: click it to paint the filter mask,
//! ⌥-click to view it, ⇧-click to disable it (FILE-215-3).

use photocraft_doc::{Document, Layer, LayerContent, LayerId, SmartObject};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The display name of a smart filter (its command's label without the ellipsis). A Photoshop
/// filter PhotoCraft doesn't implement shows its own name and is marked as kept as is.
pub(crate) fn filter_label(f: &photocraft_doc::SmartFilter) -> String {
    let command = f.command.as_str();
    if command == photocraft_engine::smart_cmds::UNSUPPORTED_FILTER {
        let name = f.params.get("name").and_then(Value::as_str).unwrap_or("Smart filter");
        return format!("{} (kept, not editable)", name.trim_end_matches("...").trim_end_matches('…'));
    }
    photocraft_engine::commands::find(command)
        .map_or_else(|| command.rsplit('.').next().unwrap_or(command).to_string(), |c| tl!(c.label).trim_end_matches('…').to_string())
}

/// Smart Filters header + one row per filter (top filter first, as in Photoshop) under a smart
/// layer. Edits are queued in `actions` as engine commands.
pub fn has_filters(l: &Layer) -> bool {
    matches!(&l.content, LayerContent::Smart(sm) if !sm.smart_filters.is_empty())
}

/// What a click on the filter mask thumbnail does: target it for painting (selecting the smart
/// object), ⌥ view it alone (⇧⌥ as an overlay), ⇧ disable or enable it.
pub(crate) fn filter_mask_click(l: &Layer, m: egui::Modifiers) -> Vec<(String, Value)> {
    let LayerContent::Smart(sm) = &l.content else { return Vec::new() };
    let layer = l.id.0;
    if m.alt {
        let mode = if m.shift { "toggleOverlay" } else { "toggleGray" };
        return sm
            .filter_mask
            .is_some()
            .then(|| (photocraft_engine::mask_view_cmds::ID.to_string(), json!({"layer": layer, "mode": mode, "mask": "filter"})))
            .into_iter()
            .collect();
    }
    if m.shift {
        return sm
            .filter_mask
            .as_ref()
            .map(|f| ("layer.smartFilter.disableFilterMask".to_string(), json!({"layer": layer, "enabled": !f.enabled})))
            .into_iter()
            .collect();
    }
    vec![("layer.select".into(), json!({"layer": layer})), ("ui.filterMaskTarget".into(), json!(layer))]
}

/// A Smart Filter row selected on its own: the smart object and the filter index (`None` = the
/// Smart Filters row, all of them).
pub(crate) type FilterDrag = (u64, Option<usize>);

/// The Smart Filter row each document has selected on its own.
pub(crate) type FilterPicks = std::collections::HashMap<photocraft_doc::DocId, FilterDrag>;

/// The selected smart filter row, while its smart object is active and the filter still exists.
pub(crate) fn selected(picks: &FilterPicks, doc: &Document, active: Option<LayerId>) -> Option<FilterDrag> {
    let (layer, filter) = *picks.get(&doc.id)?;
    if active != Some(LayerId(layer)) {
        return None;
    }
    let LayerContent::Smart(sm) = &doc.layer(LayerId(layer))?.content else { return None };
    let exists = match filter {
        Some(i) => i < sm.smart_filters.len(),
        None => !sm.smart_filters.is_empty(),
    };
    exists.then_some((layer, filter))
}

/// Selects a Smart Filter row of document `doc` on its own: `(smart object, filter index)`, `None`
/// for the Smart Filters row.
pub(crate) fn pick(picks: &mut FilterPicks, doc: photocraft_doc::DocId, row: FilterDrag) {
    picks.insert(doc, row);
}

/// Forgets the selected smart filter row: its smart object's own row was clicked, the filter was
/// deleted (its index would now name the next one) or the panel's filter hides its row.
pub(crate) fn deselect(picks: &mut FilterPicks, doc: photocraft_doc::DocId) {
    picks.remove(&doc);
}

/// The command for dropping the smart filters of `from` on the smart object `target`: they move
/// there, or with ⌥ held on release are copied (Photoshop).
pub(crate) fn drop_action(from: u64, filter: Option<usize>, target: LayerId, copy: bool) -> (String, Value) {
    let mut payload = json!({"from": from, "to": target.0, "copy": copy});
    if let (Some(i), Some(o)) = (filter, payload.as_object_mut()) {
        o.insert("filter".into(), json!(i));
    }
    ("layer.smartFilter.transfer".into(), payload)
}

/// Smart filters dropped on the footer's Delete button: a filter's row deletes that filter, the
/// Smart Filters row clears them all.
pub(crate) fn trash_action((layer, filter): FilterDrag) -> (String, Value) {
    match filter {
        Some(i) => ("layer.smartFilter.delete".into(), json!({"layer": layer, "index": i})),
        None => ("layer.smartFilter.clearSmartFilters".into(), json!({"layer": layer})),
    }
}

/// The fired Delete command (Delete or Backspace, which the shortcut route has taken from the input) while a filter's
/// row is selected removes that filter (the Smart Filters row: all of them), as the panel's Delete button does
/// (#4751), instead of deleting the smart object or reaching Edit › Clear.
pub(crate) fn keys(app: &mut PhotocraftApp) -> bool {
    // A text edit keeps its keys. Text fields and widgets that delete with these keys themselves
    // take them before the key reaches here (#1534).
    if app.ui.text_edit.is_some() {
        return false;
    }
    let Some(doc) = app.session.active().map(|st| st.doc.id) else { return false };
    let Some(picked) = crate::panels::listed_filter(app) else { return false };
    // Only where the key would delete the smart object: Quick Mask, a single channel, a targeted
    // mask or a selection keep their own Delete.
    if crate::shortcut_dispatch::delete_key_command(app, "edit.clear".into()) != "layer.delete" {
        return false;
    }
    let (command, params) = trash_action(picked);
    deselect(&mut app.smart_filter_picks, doc);
    if let Err(e) = app.run(&command, params) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    true
}

/// Re-opens a smart filter's dialog with its recorded parameters; OK runs
/// `layer.smartFilter.setParams` instead of adding another filter (see [`confirm_command`]).
pub fn open_editor(app: &mut PhotocraftApp, ctx: &egui::Context, layer: u64, sm: &SmartObject, index: usize) {
    let Some(f) = sm.smart_filters.get(index) else { return };
    // Camera Raw has its own full-window dialog rather than a generic parameter form.
    if f.command == photocraft_engine::lens_cmds::RAW {
        if let Err(e) = crate::camera_raw_ui::open_smart_filter(app, ctx, photocraft_doc::LayerId(layer), index) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        return;
    }
    // So has the Filter Gallery.
    if f.command == "filter.filterGallery" {
        if let Err(e) = crate::gallery_ui::open_smart_filter(app, photocraft_doc::LayerId(layer), index) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        return;
    }
    // And Liquify, which re-opens with the filter's recorded strokes.
    if f.command == photocraft_engine::distort_cmds::LIQUIFY {
        if let Err(e) = crate::liquify_ui::open_smart_filter(app, ctx, photocraft_doc::LayerId(layer), index) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        return;
    }
    let target = match crate::smart_filter_dialog::target(app, photocraft_doc::LayerId(layer), index) {
        Ok(target) => target,
        Err(e) => {
            crate::notices::error(app, e);
            return;
        }
    };
    if crate::adjust_dialog::open_smart_filter(app, &f.command, &f.params, target.clone()).is_some() || !crate::filter_dialog::has_dialog(&f.command) {
        return;
    }
    // The hosted filter dialog carries the target as its captured value; the recorded parameters
    // overlay the schema defaults, not the user's last-used values.
    if let Err(e) = crate::filter_dialog::open_smart_edit(app, &f.command, &f.params.as_object().cloned().unwrap_or_default(), target) {
        crate::notices::error(app, e);
    }
}

/// The command a hosted filter dialog's live preview runs ([`confirm_command`]'s): the Smart Filter
/// blending command for the dialog `key` names (its Blending Options dialog),
/// `layer.smartFilter.setParams` while the dialog edits a smart filter (`captured` is a smart-filter
/// target, #6324), else `command`.
pub fn preview_command<'a>(key: &str, command: &'a str, captured: &Value) -> &'a str {
    if crate::smart_filter_dialog::owns(key) {
        crate::smart_filter_dialog::COMMAND
    } else if crate::smart_filter_dialog::is_target(captured) {
        "layer.smartFilter.setParams"
    } else {
        command
    }
}

/// The command a confirmed filter dialog runs: smart-filter edits update the recorded filter.
pub fn confirm_command(app: &PhotocraftApp, captured: &Value, command: String, params: Value) -> Result<(String, Value), String> {
    // One builder for the preview and for OK, so the two cannot differ.
    if crate::smart_filter_dialog::is_target(captured) {
        return crate::smart_filter_dialog::params_command(app, captured, &command, params);
    }
    Ok((command, params))
}

#[cfg(test)]
mod editor_tests;

#[cfg(test)]
mod tests {
    use photocraft_engine::Session;

    use super::*;
    use crate::Services;

    fn smart(app: &PhotocraftApp) -> (photocraft_doc::Layer, photocraft_doc::SmartObject) {
        let st = app.session.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
        let LayerContent::Smart(sm) = l.content.clone() else { panic!("not a smart object") };
        (l, sm)
    }

    /// FILE-215-3: clicking the filter mask thumbnail targets it, the Brush paints into it (not
    /// the layer mask, no rasterize prompt), and clicking the layer thumbnail goes back.
    #[test]
    fn the_filter_mask_thumbnail_targets_the_filter_mask() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        app.run("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
        let (l, _) = smart(&app);
        let actions = super::filter_mask_click(&l, egui::Modifiers::NONE);
        assert_eq!(actions.last().unwrap().0, "ui.filterMaskTarget");
        assert!(super::filter_mask_click(&l, egui::Modifiers::ALT).is_empty(), "nothing to view yet");
        app.ui.filter_mask_target = Some(l.id.0);
        app.sync_views();
        assert!(app.filter_mask_targeted());
        assert_eq!(crate::canvas::paint_target(&app), json!("filterMask"));
        assert_eq!(app.with_mask_target("filter.blur.gaussianBlur", json!({}))["target"], "filterMask");
        assert!(crate::rasterize_prompt::needed(&app).is_none(), "painting the filter mask needs no rasterizing");
        app.run(
            "paint.stroke",
            json!({"points": [[4, 0], [4, 31]], "size": 4, "hardness": 1, "color": "#000000", "target": crate::canvas::paint_target(&app)}),
        )
        .unwrap();
        let (l, sm) = smart(&app);
        assert_eq!(sm.filter_mask.as_ref().unwrap().value(4, 16), 0.0);
        assert!(l.mask.is_none(), "no layer mask");
        assert_eq!(super::filter_mask_click(&l, egui::Modifiers::ALT)[0].1["mask"], "filter");
        // Targeting a layer mask (or the pixels, `ui.maskTarget`) ends it.
        app.ui.mask_target = true;
        app.sync_views();
        assert_eq!(app.ui.filter_mask_target, None, "a layer mask target replaces it");
    }

    #[test]
    fn selecting_the_filter_mask_replaces_a_layer_mask_view() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        app.run("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        app.run("view.layerMask", json!({"mode": "gray"})).unwrap();
        app.sync_mask_targets();
        assert!(app.layer_mask_targeted());
        let id = app.session.active().unwrap().active_layer.unwrap();
        app.select_filter_mask_target(Some(id.0));
        app.sync_views();
        assert!(app.filter_mask_targeted());
        assert!(!app.layer_mask_targeted());
        assert!(photocraft_engine::mask_view_cmds::current(app.session.active().unwrap()).is_none());
        app.run("paint.stroke", json!({"points": [[8, 8]], "size": 6, "hardness": 1, "color": "#000000"})).unwrap();
        let st = app.session.active().unwrap();
        let layer = st.doc.layer(id).unwrap();
        assert_eq!(layer.mask.as_ref().unwrap().value(8, 8), 1.0);
        let LayerContent::Smart(sm) = &layer.content else { panic!("smart object") };
        assert_eq!(sm.filter_mask.as_ref().unwrap().value(8, 8), 0.0);
        app.run("view.layerMask", json!({"mode": "gray", "mask": "filter"})).unwrap();
        app.select_filter_mask_target(Some(id.0));
        assert!(photocraft_engine::mask_view_cmds::current(app.session.active().unwrap()).unwrap().filter, "a matching filter-mask view stays open");
    }

    /// #5689: an Image › Adjustments recorded on a smart object (Brightness/Contrast…) opens its
    /// own editor from the Smart Filters row, with the recorded settings, and OK updates that
    /// filter instead of stacking another.
    #[test]
    fn an_adjustment_smart_filter_opens_its_editor_and_updates_it() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"contents": "black"})).unwrap();
        let id = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("image.adjustments.brightnessContrast", json!({"brightness": 40, "contrast": -10})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        let sm = match &app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
            LayerContent::Smart(sm) => sm.clone(),
            other => panic!("expected a smart object, got {other:?}"),
        };
        assert_eq!(sm.smart_filters.len(), 1);
        let app_ctx = app.ctx().clone();
        open_editor(&mut app, &app_ctx, id, &sm, 0);
        crate::dialogs::testing::show(&mut app);
        let d = app.framework.dialogs.open_dialogs().pop().expect("the adjustment editor opened");
        // An adjustment dialog shows its engine parameters, the form its command takes.
        let fields = crate::adjust_dialog::command_params(&app, d.id);
        assert_eq!(fields["brightness"].as_f64(), Some(40.0), "the recorded settings");
        assert_eq!(fields["contrast"].as_f64(), Some(-10.0));
        let dialog = d.id;
        // OK updates the recorded filter (one history step), it does not add a second filter.
        crate::dialogs::testing::confirm(&mut app, dialog).unwrap();
        let LayerContent::Smart(sm) = &app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content else {
            panic!("the layer stays a smart object");
        };
        assert_eq!(sm.smart_filters.len(), 1, "no filter was stacked");
        assert_eq!(sm.smart_filters[0].params.get("brightness").and_then(Value::as_f64), Some(40.0));
        assert_eq!(sm.smart_filters[0].params.get("contrast").and_then(Value::as_f64), Some(-10.0));
    }

    /// #11463: a Selective Color smart filter recorded in the single-colour form, re-edited. Its
    /// old `cyan` and `relative` must not survive replacement and override the edit when the filter
    /// renders and reopens.
    #[test]
    fn a_selective_color_smart_filter_in_the_old_form_keeps_its_edit() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#2030c0"})).unwrap();
        let id = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("image.adjustments.selectiveColor", json!({"colors": "blues", "cyan": 30, "relative": false})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.background_jobs = false;
        let recorded = |app: &PhotocraftApp| match &app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
            LayerContent::Smart(sm) => sm.clone(),
            other => panic!("expected a smart object, got {other:?}"),
        };
        let sm = recorded(&app);
        assert_eq!(sm.smart_filters[0].params.get("cyan").and_then(Value::as_f64), Some(30.0), "recorded in the old form");
        let mut h = egui_kittest::Harness::builder()
            .with_context(app.ctx().clone())
            .with_size(egui::vec2(1200.0, 900.0))
            .build_ui_state(|ui, app| crate::dialogs::run(app, ui), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::AppTheme::pro());
        let ctx = h.ctx.clone();
        // The open dialog's fields, whatever its type.
        let fields_of = |app: &PhotocraftApp, dialog| match crate::adjust_dialog::hosted(app, dialog) {
            Some(_) => crate::adjust_dialog::command_params(app, dialog),
            None => app.framework.dialogs.open_dialogs().into_iter().find(|d| d.id == dialog).and_then(|d| d.fields).unwrap(),
        };
        open_editor(h.state_mut(), &ctx, id, &sm, 0);
        h.run_steps(2);
        let dialog = h.state().framework.dialogs.open_dialogs().pop().unwrap().id;
        h.run_steps(3);
        let fields = fields_of(h.state(), dialog);
        assert!(fields.get("cyan").is_none() && fields.get("relative").is_none(), "folded: {fields:?}");
        assert_eq!(fields["colors"], json!("blues"), "normalization keeps the selected colour");
        assert_eq!(fields["blues"][0].as_f64(), Some(30.0));
        assert_eq!(fields["method"], json!("absolute"));
        // Edit the row and the Method, then OK.
        let mut edited = crate::adjust_dialog::command_params(h.state(), dialog);
        edited["blues"] = json!([10.0, 0.0, 0.0, 0.0]);
        edited["method"] = json!("relative");
        assert!(crate::adjust_dialog::apply_params(h.state_mut(), dialog, &edited), "the dialog takes the edit");
        h.run_steps(2);
        crate::dialogs::testing::confirm(h.state_mut(), dialog).unwrap();
        let sm = recorded(h.state());
        assert_eq!(sm.smart_filters.len(), 1, "no filter was stacked");
        let photocraft_doc::Adjustment::SelectiveColor { relative, adjustments } =
            photocraft_engine::adjust_cmds::selective_from_params(&sm.smart_filters[0].params, None).unwrap()
        else {
            panic!("Selective Color params");
        };
        assert!(relative, "the edited Method renders");
        assert_eq!(adjustments[4], [10.0, 0.0, 0.0, 0.0], "the edited row renders");
        // And reopens as edited.
        open_editor(h.state_mut(), &ctx, id, &sm, 0);
        h.run_steps(2);
        let dialog = h.state().framework.dialogs.open_dialogs().pop().unwrap().id;
        h.run_steps(3);
        let fields = fields_of(h.state(), dialog);
        assert_eq!(fields["colors"], json!("blues"), "reopening keeps the selected colour");
        assert_eq!(fields["blues"][0].as_f64(), Some(10.0));
        assert_eq!(fields["method"], json!("relative"));
    }

    /// #6324: a smart filter opened for editing previews, and its preview replaces the filter in
    /// the stack (as OK does) instead of stacking a second one.
    #[test]
    fn an_edited_smart_filter_previews_what_ok_gives() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48, "background": "#ffffff"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.all", json!({})).unwrap();
        s.execute("edit.stroke", json!({"width": 4, "color": "#000000", "location": "inside"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let id = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("filter.blur.gaussianBlur", json!({"radius": 1.0})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.background_jobs = false;
        let smart = |app: &PhotocraftApp, doc: &Document| match &doc.layer(LayerId(id)).unwrap().content {
            LayerContent::Smart(sm) => sm.clone(),
            other => panic!("expected a smart object, got {other:?} ({})", app.ui.status),
        };
        let sm = smart(&app, &app.session.active().unwrap().doc);
        let ctx = app.ctx().clone();
        open_editor(&mut app, &ctx, id, &sm, 0);
        crate::dialogs::testing::show(&mut app);
        let open = app.framework.dialogs.open_dialogs().pop().expect("the filter's dialog opened");
        let (dialog, key) = (open.id, open.key.unwrap_or_default());
        app.framework.dialogs.set_json_field(&ctx, dialog, "radius", json!(6.0)).unwrap();

        crate::canvas::ensure_filter_preview(&mut app, 0, &ctx).expect("a preview");
        let captured = crate::smart_filter_dialog::target(&app, LayerId(id), 0).unwrap();
        let preview = app.filter_preview.as_ref().unwrap();
        assert_eq!(preview.key.command, "layer.smartFilter.setParams");
        // The dialog's own preview window matches it to the dialog by that command.
        assert_eq!(preview_command(&key, "filter.blur.gaussianBlur", &captured), preview.key.command);
        assert_eq!(preview_command(&key, "filter.blur.gaussianBlur", &Value::Null), "filter.blur.gaussianBlur");
        let shown = preview.result.clone().expect("the preview's document");
        let filters = smart(&app, &shown).smart_filters;
        assert_eq!(filters.len(), 1, "the edited filter is replaced, not stacked");
        assert_eq!(filters[0].params.get("radius").and_then(Value::as_f64), Some(6.0));
        assert_eq!(smart(&app, &app.session.active().unwrap().doc).smart_filters[0].params["radius"], json!(1.0), "the document is untouched");

        crate::dialogs::testing::confirm(&mut app, dialog).unwrap();
        let doc = &app.session.active().unwrap().doc;
        assert_eq!(smart(&app, doc).smart_filters.len(), 1);
        assert!(photocraft_compose::flatten(&shown).px == photocraft_compose::flatten(doc).px, "OK gives what the preview showed");
    }

    /// FILE-215-3: with the filter mask targeted, Move and Edit › Transform act on the smart object
    /// (no filter-mask target, so a move doesn't re-render the filters), and painting and filters
    /// still edit the filter mask.
    #[test]
    fn only_filter_mask_edits_carry_the_filter_mask_target() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        app.run("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
        let id = app.session.active().unwrap().active_layer.unwrap();
        app.select_filter_mask_target(Some(id.0));
        assert!(app.filter_mask_targeted());
        for cmd in [
            "layer.translate",
            "select.float",
            "edit.transform.rotate180",
            "filter.convertForSmartFilters",
            "image.applyImage",
            "filter.liquify",
            "filter.lastFilter",
            "image.adjustments.desaturate",
            "filter.render.flame",
        ] {
            assert!(app.with_mask_target(cmd, json!({"dx": 1})).get("target").is_none(), "{cmd}");
        }
        for cmd in [
            "paint.stroke",
            "filter.blur.gaussianBlur",
            "edit.fill",
            "edit.paste",
            "image.adjustments.invert",
            "image.adjustments.equalize",
            "filter.blur.average",
        ] {
            assert_eq!(app.with_mask_target(cmd, json!({}))["target"], "filterMask", "{cmd}");
        }
    }
}
