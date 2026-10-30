//! The Info panel's options (its panel menu › Panel Options…): which colour readouts the panel
//! shows.
//!
//! Photoshop's Panel Options offer two readout slots, each set to one mode. Here every readout
//! is a checkbox and any number can be on at once, so a separator can watch Actual Color,
//! Grayscale and every spot channel's ink together. The Spot Channels and Alpha Channels
//! readouts list every extra channel at the pointer at the same time; Photoshop shows a spot
//! channel's value only while that channel is targeted on its own. The options are kept between
//! launches, apart from the saved workspace.

use serde::{Deserialize, Serialize};

use crate::PhotocraftApp;

/// Where the options are kept between launches (`prefs.dialogs`).
const OPTIONS_PREF: &str = "infoPanel";

/// A readout of the Info panel: Photoshop's Color Readout modes, then the extra channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Readout {
    /// The channels of the document's colour mode: RGB, K for Grayscale, CMYK, Lab, or every
    /// channel of a Multichannel document. Lab is reconstructed from the RGB composite and
    /// cannot recover out-of-gamut native Lab values clipped by RGB compositing.
    ActualColor,
    /// K %: `100 − Rec.601 luma` of the RGB values, as Photoshop reads it.
    Grayscale,
    Rgb,
    /// RGB as hex digits.
    Web,
    Hsb,
    Hsl,
    Cmyk,
    Lab,
    /// The CMYK percentages added up (Photoshop's Total Ink: the process plates).
    TotalInk,
    /// Every spot channel's ink % added up: the separations total.
    TotalSpotInk,
    /// The composite's alpha under the pointer, as Photoshop's Opacity readout.
    Opacity,
    /// Every spot channel's ink %, all at once.
    SpotChannels,
    /// Every alpha channel's masked %, white = 0 %.
    AlphaChannels,
}

impl Readout {
    /// Every readout in menu order: Photoshop's modes, then the channel readouts.
    pub const ALL: [Readout; 13] = [
        Readout::ActualColor,
        Readout::Grayscale,
        Readout::Rgb,
        Readout::Web,
        Readout::Hsb,
        Readout::Hsl,
        Readout::Cmyk,
        Readout::Lab,
        Readout::TotalInk,
        Readout::TotalSpotInk,
        Readout::Opacity,
        Readout::SpotChannels,
        Readout::AlphaChannels,
    ];

    /// The dialog label (Photoshop's names).
    pub fn label(self) -> &'static str {
        match self {
            Readout::ActualColor => "Actual Color",
            Readout::Grayscale => "Grayscale",
            Readout::Rgb => "RGB Color",
            Readout::Web => "Web Color",
            Readout::Hsb => "HSB Color",
            Readout::Hsl => "HSL Color",
            Readout::Cmyk => "CMYK Color",
            Readout::Lab => "Lab Color",
            Readout::TotalInk => "Total Ink",
            Readout::TotalSpotInk => "Total Spot Ink",
            Readout::Opacity => "Composite opacity",
            Readout::SpotChannels => "Spot Channels",
            Readout::AlphaChannels => "Alpha Channels",
        }
    }
}

/// The Info panel options, kept between launches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InfoPanelState {
    /// The readouts shown. Any order and duplicates are accepted (`ui.set`); the panel shows them
    /// in [`Readout::ALL`] order, each once.
    pub readouts: Vec<Readout>,
}

impl Default for InfoPanelState {
    /// Photoshop's defaults: Actual Color and CMYK.
    fn default() -> Self {
        Self { readouts: vec![Readout::ActualColor, Readout::Cmyk] }
    }
}

impl InfoPanelState {
    pub fn shows(&self, r: Readout) -> bool {
        self.readouts.contains(&r)
    }

    /// The readouts shown, in menu order, each once.
    pub fn shown(&self) -> Vec<Readout> {
        Readout::ALL.iter().copied().filter(|r| self.shows(*r)).collect()
    }

    pub fn toggle(&mut self, r: Readout) {
        if self.shows(r) {
            self.readouts.retain(|x| *x != r);
        } else {
            self.readouts.push(r);
            self.readouts = self.shown();
        }
    }
}

/// Sets the options and saves them with the preferences.
pub fn set_options(app: &mut PhotocraftApp, state: InfoPanelState) {
    app.ui.info_panel = InfoPanelState { readouts: state.shown() };
    persist_options(app);
}

/// Saves the options with the preferences.
pub fn persist_options(app: &mut PhotocraftApp) {
    if let Ok(value) = serde_json::to_value(&app.ui.info_panel) {
        app.session.prefs.edit(|p| {
            p.dialogs.insert(OPTIONS_PREF.into(), value);
        });
    }
}

/// Restores the saved options; missing or unreadable ones keep the defaults.
pub fn restore_options(app: &mut PhotocraftApp) {
    if let Some(v) = app.session.prefs().dialogs.get(OPTIONS_PREF)
        && let Ok(state) = serde_json::from_value::<InfoPanelState>(v.clone())
    {
        app.ui.info_panel = InfoPanelState { readouts: state.shown() };
    }
}

/// The Info panel's menu, shared by docked and floating panels: Panel Options… opens the dialog.
pub fn panel_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if ui.button(tl!("Panel Options…")).clicked() {
        app.ui.info_panel_dialog = Some(app.ui.info_panel.clone());
        ui.close();
    }
}

/// The checkboxes of the Panel Options dialog, Photoshop's readouts in two columns and the
/// channel readouts below.
fn options_body(ui: &mut egui::Ui, opts: &mut InfoPanelState) {
    ui.label(tl!("Readouts"));
    ui.add_space(4.0);
    let (color, channels): (Vec<Readout>, Vec<Readout>) =
        Readout::ALL.iter().copied().partition(|r| !matches!(r, Readout::SpotChannels | Readout::AlphaChannels));
    let half = color.len().div_ceil(2);
    ui.columns(2, |cols| {
        for (i, r) in color.iter().enumerate() {
            let mut on = opts.shows(*r);
            if cols[i / half].checkbox(&mut on, tl!(r.label())).changed() {
                opts.toggle(*r);
            }
        }
    });
    ui.add_space(8.0);
    for r in channels {
        let mut on = opts.shows(r);
        if ui.checkbox(&mut on, tl!(r.label())).changed() {
            opts.toggle(r);
        }
    }
}

/// Show the pending Info panel Panel Options dialog as a modal; OK applies and saves the
/// options, Cancel or Escape leaves them as they were.
pub fn show_dialog(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(mut opts) = app.ui.info_panel_dialog.take() else { return };
    let mut confirmed = false;
    let mut cancelled = ctx.input_mut(|i| i.consume_key(i.modifiers, egui::Key::Escape));
    egui::Modal::new(egui::Id::unique("info-panel-options-modal")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.horizontal(|ui| {
            ui.heading(tl!("Info Panel Options"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("×").on_hover_text(tl!("Cancel")).clicked() {
                    cancelled = true;
                }
            });
        });
        ui.separator();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(300.0);
                options_body(ui, &mut opts);
            });
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.set_min_width(74.0);
                if ui.add_sized([74.0, 29.0], egui::Button::new(tl!("OK"))).clicked() {
                    confirmed = true;
                }
                ui.add_space(6.0);
                if ui.add_sized([74.0, 29.0], egui::Button::new(tl!("Cancel"))).clicked() {
                    cancelled = true;
                }
            });
        });
    });
    if confirmed && !cancelled {
        set_options(app, opts);
    } else if !cancelled {
        app.ui.info_panel_dialog = Some(opts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;
    use egui_kittest::{Harness, kittest::Queryable};
    use serde_json::json;

    /// The defaults are Photoshop's; old or malformed saved options keep them.
    #[test]
    fn defaults_and_bad_saved_options() {
        assert_eq!(InfoPanelState::default().shown(), vec![Readout::ActualColor, Readout::Cmyk]);
        let mut value = serde_json::to_value(crate::state::UiState::default()).unwrap();
        value.as_object_mut().unwrap().remove("info_panel");
        let restored: crate::state::UiState = serde_json::from_value(value).unwrap();
        assert_eq!(restored.info_panel, InfoPanelState::default());
        for value in [json!({}), json!({"readouts": "gray"}), json!({"readouts": ["bogus"]}), json!(null), json!(7)] {
            let mut session = photocraft_engine::Session::new();
            session.prefs.edit(|p| {
                p.dialogs.insert(OPTIONS_PREF.into(), value);
            });
            let app = PhotocraftApp::new(session, Default::default());
            assert_eq!(app.ui.info_panel, InfoPanelState::default());
        }
    }

    /// Toggling keeps menu order and drops duplicates; a `ui.set` list in any order reads back
    /// normalised.
    #[test]
    fn readouts_keep_menu_order_once_each() {
        let mut s = InfoPanelState::default();
        s.toggle(Readout::SpotChannels);
        s.toggle(Readout::Grayscale);
        assert_eq!(s.shown(), vec![Readout::ActualColor, Readout::Grayscale, Readout::Cmyk, Readout::SpotChannels]);
        s.toggle(Readout::ActualColor);
        assert!(!s.shows(Readout::ActualColor));
        let loose = InfoPanelState { readouts: vec![Readout::Lab, Readout::Rgb, Readout::Lab] };
        assert_eq!(loose.shown(), vec![Readout::Rgb, Readout::Lab]);
        assert_eq!(serde_json::to_value(&loose).unwrap(), json!({"readouts": ["lab", "rgb", "lab"]}), "camelCase ids");
    }

    /// The options survive the preferences and a new shell, with workspace saving off; the open
    /// dialog is never saved.
    #[test]
    fn options_survive_restart_independently_of_workspace() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.prefs.edit(|p| p.workspace.remember_workspace_changes = false);
        for readouts in [vec![Readout::Grayscale, Readout::SpotChannels], vec![], Readout::ALL.to_vec()] {
            set_options(&mut app, InfoPanelState { readouts: readouts.clone() });
            let mut session = photocraft_engine::Session::new();
            session.load_prefs_json(&app.session.prefs_to_json()).unwrap();
            let restored = PhotocraftApp::new(session, Default::default());
            assert_eq!(restored.ui.info_panel.readouts, readouts);
        }
        app.ui.info_panel_dialog = Some(InfoPanelState::default());
        let json = serde_json::to_value(&app.ui).unwrap();
        assert!(json.get("info_panel_dialog").is_none(), "transient");
    }

    /// Panel Options… in the panel menu opens the dialog; its checkboxes edit a copy that OK
    /// applies and saves, and Cancel or Escape drops.
    #[test]
    fn panel_options_dialog_applies_on_ok_and_drops_on_cancel() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let mut h = Harness::builder().with_size(vec2(260.0, 200.0)).build_ui_state(|ui, app: &mut PhotocraftApp| panel_menu(app, ui), app);
        h.run_steps(2);
        h.get_by_label("Panel Options…").click();
        h.run_steps(2);
        assert_eq!(h.state().ui.info_panel_dialog, Some(InfoPanelState::default()), "the menu opens the dialog");
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let mut h = Harness::builder().with_size(vec2(640.0, 480.0)).build_ui_state(|ui, app: &mut PhotocraftApp| show_dialog(app, ui.ctx()), app);
        h.run_steps(2);
        assert!(h.query_by_label("Info Panel Options").is_none(), "no dialog until asked");
        h.state_mut().ui.info_panel_dialog = Some(h.state().ui.info_panel.clone());
        h.run_steps(2);
        for r in Readout::ALL {
            assert!(h.query_by_label(r.label()).is_some(), "{} checkbox", r.label());
        }
        h.get_by_label("Spot Channels").click();
        h.run_steps(2);
        assert_eq!(h.state().ui.info_panel.readouts, vec![Readout::ActualColor, Readout::Cmyk], "nothing applied before OK");
        h.get_by_label("Cancel").click();
        h.run_steps(2);
        assert!(h.state().ui.info_panel_dialog.is_none() && !h.state().ui.info_panel.shows(Readout::SpotChannels), "Cancel drops the change");
        h.state_mut().ui.info_panel_dialog = Some(h.state().ui.info_panel.clone());
        h.run_steps(2);
        h.get_by_label("Spot Channels").click();
        h.get_by_label("Actual Color").click();
        h.run_steps(1);
        h.get_by_label("OK").click();
        h.run_steps(2);
        assert!(h.state().ui.info_panel_dialog.is_none());
        assert_eq!(h.state().ui.info_panel.readouts, vec![Readout::Cmyk, Readout::SpotChannels], "OK applies");
        assert_eq!(h.state().session.prefs().dialogs.get("infoPanel").unwrap()["readouts"], json!(["cmyk", "spotChannels"]), "and saves");
        h.state_mut().ui.info_panel_dialog = Some(h.state().ui.info_panel.clone());
        h.run_steps(1);
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(h.state().ui.info_panel_dialog.is_none(), "Escape closes it");
    }

    /// Follow the app's dispatch order: the options modal owns Escape and document shortcuts.
    #[test]
    fn panel_options_owns_document_shortcuts_and_escape() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 32, "height": 32, "background": "transparent"})).unwrap();
        app.run("select.rect", json!({"x": 2, "y": 2, "width": 10, "height": 10})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let before = app.run("document.inspect", json!({})).unwrap();
        let revision = app.session.active().unwrap().revision;
        let journal = app.session.journal.len();
        let saved = app.session.prefs_to_json();
        app.ui.info_panel_dialog = Some(app.ui.info_panel.clone());
        let mut h = Harness::builder().with_size(vec2(640.0, 480.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                crate::shortcuts::handle(app, ui.ctx());
                show_dialog(app, ui.ctx());
            },
            app,
        );
        h.run_steps(2);
        // A focused checkbox must not let menu shortcuts act on the document behind it.
        h.get_by_label("Composite opacity").click();
        h.run_steps(2);
        for (key, modifiers) in [
            (egui::Key::Z, egui::Modifiers::COMMAND),
            (egui::Key::Delete, egui::Modifiers::NONE),
            (egui::Key::N, egui::Modifiers { shift: true, ..egui::Modifiers::COMMAND }),
        ] {
            h.event(egui::Event::ModifiersChanged(modifiers));
            h.event(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
            h.run_steps(1);
            h.event(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
            h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
            h.run_steps(2);
            assert!(h.state().ui.info_panel_dialog.is_some(), "{key:?} keeps the options open");
            assert_eq!(h.state_mut().run("document.inspect", json!({})).unwrap(), before, "{key:?} changed the document/history");
            assert_eq!(h.state().session.active().unwrap().revision, revision);
            assert_eq!(h.state().session.journal.len(), journal);
            assert_eq!(h.state().session.prefs_to_json(), saved);
        }
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(h.state().ui.info_panel_dialog.is_none(), "Escape closes the options");
        assert_eq!(h.state_mut().run("document.inspect", json!({})).unwrap(), before, "Escape must not deselect");
        assert_eq!(h.state().session.active().unwrap().revision, revision);
        assert_eq!(h.state().session.journal.len(), journal);
        assert_eq!(h.state().session.prefs_to_json(), saved, "the draft checkbox was cancelled");
        // Positive controls: normal document Escape and Undo still work after closing.
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(h.state().session.active().unwrap().doc.selection.is_none());
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::COMMAND));
        h.event(egui::Event::Key { key: egui::Key::Z, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND });
        h.run_steps(2);
        assert!(h.state().session.active().unwrap().doc.selection.is_some(), "Undo still restores the selection");
    }
}
