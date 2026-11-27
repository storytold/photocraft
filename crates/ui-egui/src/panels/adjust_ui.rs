//! Properties-panel editor for the Color Lookup adjustment layer.
//!
//! It commits through `layer.setAdjustment` (which merges the params into the current values) on
//! every change, since its table isn't in the params. The other kinds, Selective Color included,
//! are edited by `adjust_editors`.

use std::path::PathBuf;

use craft::behavior::EditEvent;
// PLAN: `craft::cms` needs the umbrella's `cms` feature in crates/ui-egui/Cargo.toml (not in this set).
use craft::cms::lutfile::{LutError, LutFile, write_cube};
use craft::color_ui::ui::{LutChoice, LutPicker};
use craft::ui::choice::{Radio, RadioGroup, Switch};
use craft::ui::contract::{Bind, Props as _, Show, ShowContainer};
use photocraft_doc::{Adjustment, LayerId};
use serde_json::{Value, json};

use crate::PhotocraftApp;

/// The choice a built-in look id stands for; `none` is no LUT.
fn builtin_choice(id: &str) -> LutChoice {
    photocraft_engine::adjust_cmds::LOOKS.iter().find(|(known, _)| *known == id).map_or(LutChoice::None, |(known, _)| LutChoice::Builtin(known))
}

/// The picker's choice for a Color Lookup layer: none, a built-in look by its title, else the LUT the layer
/// holds (the picker shows it as its folder row when the folder has a file of that name).
fn choice_of(adj: &Adjustment) -> LutChoice {
    let Adjustment::ColorLookup { name, lut, size, .. } = adj else { return LutChoice::None };
    if lut.is_none() {
        return LutChoice::None;
    }
    match photocraft_engine::adjust_cmds::LOOKS.iter().find(|(_, title)| title == name) {
        Some((id, _)) => LutChoice::Builtin(id),
        None => LutChoice::loaded(name.clone(), *size as usize),
    }
}

/// `layer.setAdjustment` params for a picked choice: a folder file is read and embedded like Load 3D LUT…
/// does. `None` for the LUT the layer already holds, which is no pick.
fn params_of(choice: &LutChoice) -> Option<Value> {
    match choice {
        LutChoice::None => Some(json!({"lut": "none"})),
        LutChoice::Builtin(id) => Some(json!({"lut": id})),
        LutChoice::Folder(path) => Some(json!({"file": path.to_string_lossy()})),
        _ => None,
    }
}

/// The built-in look a wheel or arrow step proposes, for the live preview; the picker steps over those only.
fn preview_id(choice: &LutChoice) -> Option<&str> {
    match choice {
        LutChoice::None => Some("none"),
        LutChoice::Builtin(id) => Some(id),
        _ => None,
    }
}

/// Runs `params` on the layer and ends the preview. Errors (an unreadable LUT file) land in the status bar.
fn set_adjustment(app: &mut PhotocraftApp, layer: LayerId, mut params: Value) {
    params["layer"] = json!(layer.0);
    if let Err(e) = app.run("layer.setAdjustment", params) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    app.live_adjust = None;
}

/// Embeds the LUT Load 3D LUT… read in `layer`, or reports why it could not be read and changes nothing.
fn apply_loaded(app: &mut PhotocraftApp, layer: LayerId, loaded: Result<LutFile, LutError>) {
    match loaded {
        Ok(lut) => {
            set_adjustment(app, layer, json!({"fileName": format!("{}.cube", lut.title), "data": write_cube(&lut)}));
        }
        Err(e) => {
            app.ui.status = e.0;
            app.ui.status_error = true;
        }
    }
}

/// The Color Lookup layer's controls: the LUT picker, its interpolation and Dither.
pub fn color_lookup_editor(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: LayerId, adj: &Adjustment) {
    let Adjustment::ColorLookup { tetrahedral, dither, .. } = adj else { return };
    let mut shown = choice_of(adj);
    if let Some((layer, params)) = &app.live_adjust
        && *layer == id
        && let Some(preview) = params.get("lut").and_then(Value::as_str)
    {
        shown = builtin_choice(preview);
    }
    let mut picker = LutPicker::new(Bind::given(shown));
    let folder = app.session.prefs().file_handling.lut_folder.clone();
    if !folder.is_empty() {
        picker = picker.folder(PathBuf::from(folder));
    }
    let out = picker.show(ui);
    for event in &out.edit.events {
        match event {
            EditEvent::Update { value, .. } => {
                if let Some(look) = preview_id(value) {
                    app.live_adjust = Some((id, json!({"lut": look})));
                }
            }
            EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => {
                if let Some(params) = params_of(after) {
                    set_adjustment(app, id, params);
                }
            }
            EditEvent::Cancel { .. } => app.live_adjust = None,
            _ => {}
        }
    }
    if let Some(loaded) = out.loaded {
        apply_loaded(app, id, loaded);
    }
    let mode = RadioGroup::new(tl!("Interpolation"), Bind::given(*tetrahedral)).show(ui, |ui, _| {
        Radio::new(false).label(tl!("Trilinear")).show(ui);
        Radio::new(true).label(tl!("Tetrahedral")).show(ui);
    });
    if let Some(on) = mode.output.events.committed().copied() {
        set_adjustment(app, id, json!({"tetrahedral": on}));
    }
    let dithered = Switch::new(Bind::given(*dither)).label(tl!("Dither")).show(ui);
    if let Some(on) = dithered.events.committed().copied() {
        set_adjustment(app, id, json!({"dither": on}));
    }
}

#[cfg(test)]
mod lookup_picker_tests {
    use super::*;

    const CUBE: &str = "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";

    fn color_lookup_layer(app: &mut PhotocraftApp) -> LayerId {
        app.run("file.new", json!({"width": 16, "height": 12})).unwrap();
        LayerId(app.run("layer.newAdjustmentLayer.colorLookup", json!({})).unwrap()["layer"].as_u64().unwrap())
    }

    fn adjustment(app: &PhotocraftApp, layer: LayerId) -> Adjustment {
        match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
            photocraft_doc::LayerContent::Adjustment(adj) => adj.clone(),
            other => panic!("not an adjustment: {}", other.kind_name()),
        }
    }

    #[test]
    fn chosen_lut_is_embedded_into_the_requested_adjustment_layer() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let layer = color_lookup_layer(&mut app);
        let mut lut = craft::cms::lutfile::parse("custom.cube", CUBE.as_bytes()).unwrap();
        lut.title = "custom".into();
        apply_loaded(&mut app, layer, Ok(lut));
        let adj = adjustment(&app, layer);
        assert!(matches!(&adj, Adjustment::ColorLookup { name, lut: Some(_), size: 2, .. } if name.contains("custom")), "{adj:?}");
    }

    /// #6558: a LUT folder file is picked by its path and embedded; the layer then holds a LUT named after the file.
    #[test]
    fn lut_folder_files_are_offered_and_embedded() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("film.cube");
        std::fs::write(&path, CUBE).unwrap();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let layer = color_lookup_layer(&mut app);
        assert_eq!(choice_of(&adjustment(&app, layer)), LutChoice::None);
        assert_eq!(params_of(&LutChoice::None), Some(json!({"lut": "none"})));
        assert_eq!(params_of(&LutChoice::loaded("x", 2)), None, "the held LUT is no pick");
        let (id, label) = photocraft_engine::adjust_cmds::LOOKS[0];
        set_adjustment(&mut app, layer, params_of(&LutChoice::Builtin(id)).unwrap());
        assert_eq!(choice_of(&adjustment(&app, layer)), LutChoice::Builtin(id), "{label}");
        let pick = LutChoice::Folder(path.clone());
        assert_eq!(params_of(&pick), Some(json!({"file": path.to_str().unwrap()})));
        set_adjustment(&mut app, layer, params_of(&pick).unwrap());
        assert_eq!(choice_of(&adjustment(&app, layer)), LutChoice::loaded("film.cube", 2));
    }

    #[test]
    fn invalid_lut_does_not_modify_the_existing_adjustment() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let layer = color_lookup_layer(&mut app);
        let before = adjustment(&app, layer);
        apply_loaded(&mut app, layer, Err(LutError("broken.cube: invalid LUT".into())));
        assert_eq!(adjustment(&app, layer), before);
        assert!(app.ui.status_error && app.ui.status.contains("broken.cube"));
    }
}
