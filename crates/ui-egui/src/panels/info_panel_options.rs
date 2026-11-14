//! The Info panel's options (its panel menu › Panel Options…): which colour readouts the panel
//! shows.
//!
//! As in Photoshop, the dialog has two group boxes, First Color Readout and Second Color
//! Readout, each with one Mode dropdown; the panel shows the first readout on the left and the
//! second on the right. The options are kept between launches, apart from the saved workspace.

use serde::{Deserialize, Serialize};

use crate::PhotocraftApp;

/// Where the options are kept between launches (`prefs.dialogs`).
const OPTIONS_PREF: &str = "infoPanel";

/// A Color Readout mode of the Info panel (Photoshop's Panel Options › Mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Readout {
    /// The channels of the document's colour mode: RGB, K for Grayscale, CMYK, Lab, or every
    /// channel of a Multichannel document. Lab is reconstructed from the RGB composite and
    /// cannot recover out-of-gamut native Lab values clipped by RGB compositing.
    ActualColor,
    /// The sample in View › Proof Setup's profile, in that profile's channels.
    ProofColor,
    /// K %: `100 − Rec.601 luma` of the RGB values, as Photoshop reads it.
    Grayscale,
    Rgb,
    /// RGB as hex digits.
    Web,
    Hsb,
    Hsl,
    Cmyk,
    Lab,
    /// The CMYK percentages added up (`Σ:`).
    TotalInk,
    /// The composite's alpha under the pointer (`Op:`).
    Opacity,
}

impl Readout {
    /// Every mode in the Mode dropdown's order.
    pub const ALL: [Readout; 11] = [
        Readout::ActualColor,
        Readout::ProofColor,
        Readout::Grayscale,
        Readout::Rgb,
        Readout::Web,
        Readout::Hsb,
        Readout::Hsl,
        Readout::Cmyk,
        Readout::Lab,
        Readout::TotalInk,
        Readout::Opacity,
    ];

    /// The dropdown label (Photoshop's names).
    pub fn label(self) -> &'static str {
        match self {
            Readout::ActualColor => "Actual Color",
            Readout::ProofColor => "Proof Color",
            Readout::Grayscale => "Grayscale",
            Readout::Rgb => "RGB Color",
            Readout::Web => "Web Color",
            Readout::Hsb => "HSB Color",
            Readout::Hsl => "HSL Color",
            Readout::Cmyk => "CMYK Color",
            Readout::Lab => "Lab Color",
            Readout::TotalInk => "Total Ink",
            Readout::Opacity => "Opacity",
        }
    }

    /// Whether the Mode dropdown draws a separator above this mode: Actual Color, Proof Color |
    /// Grayscale … Lab Color | Total Ink, Opacity.
    fn starts_group(self) -> bool {
        matches!(self, Readout::Grayscale | Readout::TotalInk)
    }
}

/// The Info panel options, kept between launches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InfoPanelState {
    /// First Color Readout: the left readout.
    pub first: Readout,
    /// Second Color Readout: the right readout.
    pub second: Readout,
}

impl Default for InfoPanelState {
    /// Photoshop's defaults: Actual Color and CMYK Color.
    fn default() -> Self {
        Self { first: Readout::ActualColor, second: Readout::Cmyk }
    }
}

impl InfoPanelState {
    /// The two readouts, first then second (the same mode twice shows twice, as in Photoshop).
    pub fn shown(&self) -> [Readout; 2] {
        [self.first, self.second]
    }
}

/// Sets the options and saves them with the preferences.
pub fn set_options(app: &mut PhotocraftApp, state: InfoPanelState) {
    app.ui.info_panel = state;
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
        app.ui.info_panel = state;
    }
}

/// The Info panel's menu, shared by docked and floating panels: Panel Options… opens the dialog.
pub fn panel_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if ui.button(tl!("Panel Options…")).clicked() {
        app.ui.info_panel_dialog = Some(app.ui.info_panel.clone());
        ui.close();
    }
}

/// One Color Readout group box: its title, then Mode: and the dropdown.
fn readout_group(ui: &mut egui::Ui, id: &str, title: &str, mode: &mut Readout) {
    let t = crate::theme::Tokens::get(ui.ctx());
    crate::widgets::section_label(ui, title);
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.field_border)).inner_margin(8).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(tl!("Mode:"));
            let options: Vec<(Readout, &str)> = Readout::ALL.iter().map(|r| (*r, r.label())).collect();
            crate::widgets::dropdown_wheel_grouped(ui, id, tl!(title), mode, &options, 150.0, |r| r.starts_group());
        });
    });
}

/// The body of the Panel Options dialog: the First and Second Color Readout group boxes.
fn options_body(ui: &mut egui::Ui, opts: &mut InfoPanelState) {
    readout_group(ui, "info-first-readout", "First Color Readout", &mut opts.first);
    ui.add_space(8.0);
    readout_group(ui, "info-second-readout", "Second Color Readout", &mut opts.second);
}

/// Show the pending Info panel Panel Options dialog as a modal; OK applies and saves the
/// options, Cancel or Escape leaves them as they were.
pub fn show_dialog(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(mut opts) = app.ui.info_panel_dialog.take() else { return };
    let mut confirmed = false;
    let mut cancelled = ctx.input_mut(|i| i.consume_key(i.modifiers, egui::Key::Escape));
    egui::Modal::new(egui::Id::unique("info-panel-options-modal")).show(ctx, |ui| {
        ui.set_width(380.0);
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
                ui.set_width(260.0);
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
        assert_eq!(InfoPanelState::default().shown(), [Readout::ActualColor, Readout::Cmyk]);
        let mut value = serde_json::to_value(crate::state::UiState::default()).unwrap();
        value.as_object_mut().unwrap().remove("info_panel");
        let restored: crate::state::UiState = serde_json::from_value(value).unwrap();
        assert_eq!(restored.info_panel, InfoPanelState::default());
        for value in [json!({}), json!({"first": "gray"}), json!({"second": "spotChannels"}), json!({"readouts": ["grayscale"]}), json!(null), json!(7)] {
            let mut session = photocraft_engine::Session::new();
            session.prefs.edit(|p| {
                p.dialogs.insert(OPTIONS_PREF.into(), value);
            });
            let app = PhotocraftApp::new(session, Default::default());
            assert_eq!(app.ui.info_panel, InfoPanelState::default());
        }
    }

    /// The Mode list is Photoshop's, in its order, with separators after Proof Color and Lab
    /// Color; the ids are camelCase.
    #[test]
    fn mode_list_matches_the_reference_application() {
        let labels: Vec<&str> = Readout::ALL.iter().map(|r| r.label()).collect();
        assert_eq!(
            labels,
            ["Actual Color", "Proof Color", "Grayscale", "RGB Color", "Web Color", "HSB Color", "HSL Color", "CMYK Color", "Lab Color", "Total Ink", "Opacity"]
        );
        let groups: Vec<Readout> = Readout::ALL.iter().copied().filter(|r| r.starts_group()).collect();
        assert_eq!(groups, [Readout::Grayscale, Readout::TotalInk]);
        let s = InfoPanelState { first: Readout::ProofColor, second: Readout::TotalInk };
        assert_eq!(serde_json::to_value(&s).unwrap(), json!({"first": "proofColor", "second": "totalInk"}));
    }

    /// The options survive the preferences and a new shell, with workspace saving off; the open
    /// dialog is never saved.
    #[test]
    fn options_survive_restart_independently_of_workspace() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.prefs.edit(|p| p.workspace.remember_workspace_changes = false);
        for (first, second) in [(Readout::Grayscale, Readout::Opacity), (Readout::ProofColor, Readout::ProofColor), (Readout::TotalInk, Readout::Lab)] {
            set_options(&mut app, InfoPanelState { first, second });
            let mut session = photocraft_engine::Session::new();
            session.load_prefs_json(&app.session.prefs_to_json()).unwrap();
            let restored = PhotocraftApp::new(session, Default::default());
            assert_eq!(restored.ui.info_panel.shown(), [first, second]);
        }
        app.ui.info_panel_dialog = Some(InfoPanelState::default());
        let json = serde_json::to_value(&app.ui).unwrap();
        assert!(json.get("info_panel_dialog").is_none(), "transient");
    }

    /// Panel Options… in the panel menu opens the dialog: two group boxes, First and Second
    /// Color Readout, each with a Mode dropdown and no checkboxes; OK applies and saves the
    /// edited copy, Cancel or Escape drops it.
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
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(2);
        assert!(h.query_by_label("Info Panel Options").is_none(), "no dialog until asked");
        h.state_mut().ui.info_panel_dialog = Some(h.state().ui.info_panel.clone());
        h.run_steps(2);
        for group in ["First Color Readout", "Second Color Readout"] {
            assert!(h.query_all_by_label(group).next().is_some(), "{group} group box");
        }
        assert_eq!(h.query_all_by_label("Mode:").count(), 2, "one Mode dropdown per group");
        let values: Vec<String> = h.query_all_by_role(egui::accesskit::Role::ComboBox).filter_map(|n| n.value().map(|v| v.to_string())).collect();
        assert_eq!(values, ["Actual Color", "CMYK Color"], "the dropdowns show the current modes");
        assert!(h.query_all_by_role(egui::accesskit::Role::CheckBox).next().is_none(), "no checkboxes");
        let draft = InfoPanelState { first: Readout::ProofColor, second: Readout::Opacity };
        h.state_mut().ui.info_panel_dialog = Some(draft.clone());
        h.run_steps(2);
        assert_eq!(h.state().ui.info_panel, InfoPanelState::default(), "nothing applied before OK");
        h.get_by_label("Cancel").click();
        h.run_steps(2);
        assert!(h.state().ui.info_panel_dialog.is_none() && h.state().ui.info_panel == InfoPanelState::default(), "Cancel drops the change");
        h.state_mut().ui.info_panel_dialog = Some(draft.clone());
        h.run_steps(2);
        h.get_by_label("OK").click();
        h.run_steps(2);
        assert!(h.state().ui.info_panel_dialog.is_none());
        assert_eq!(h.state().ui.info_panel, draft, "OK applies");
        assert_eq!(h.state().session.prefs().dialogs.get("infoPanel").unwrap(), &json!({"first": "proofColor", "second": "opacity"}), "and saves");
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
        let mut h = Harness::builder().with_size(vec2(640.0, 480.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                crate::shortcuts::handle(app, ui.ctx());
                show_dialog(app, ui.ctx());
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.state_mut().ui.info_panel_dialog = Some(h.state().ui.info_panel.clone());
        h.run_steps(2);
        // A draft change in the open dialog must not let menu shortcuts act on the document behind it.
        h.state_mut().ui.info_panel_dialog = Some(InfoPanelState { first: Readout::Opacity, second: Readout::Cmyk });
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
        assert_eq!(h.state().session.prefs_to_json(), saved, "the draft mode was cancelled");
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
