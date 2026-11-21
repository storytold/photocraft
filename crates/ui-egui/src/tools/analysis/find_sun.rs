//! Image › Analysis › Find Sun… (#15977): the sun in a 2:1 equirectangular image, its direction
//! in the axis convention the user picks, and a matching directional light, each with a Copy
//! button. The dialog only shows what `image.analysis.findSun` returns; it reruns the command when
//! an option or the document changes, and changes nothing itself.
//!
//! While it is the top dialog, a click or drag on the image puts the sun there (the command's
//! `at`), for when the brightest area is not the sun (`DialogPick::Sun`), and the canvas marks the
//! sun the dialog is showing. Find Automatically goes back to the found one.

use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, CanvasGesture, Entry, Props as _, Sampler as _, Show as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::style::TypeRole;
use craft::ui::text::NumberField;
use photocraft_doc::DocId;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;

const COMMAND: &str = "image.analysis.findSun";

fn label(text: &str) -> Label {
    Label::from(text.to_string())
}

/// The axis conventions: the command's `frame` value, its name and where the image centre faces.
const FRAMES: [(&str, &str, &str); 3] = [
    ("yUpLeftHanded", "Y up, left-handed", "+Y is up, the image centre faces +Z and its right +X."),
    ("yUpRightHanded", "Y up, right-handed", "+Y is up, the image centre faces −Z and its right +X."),
    ("zUpRightHanded", "Z up, right-handed", "+Z is up, the image centre faces +Y and its right +X."),
];

/// The options the dialog runs the command with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SunOptions {
    frame: String,
    heading: Option<f64>,
    exposure: Option<f64>,
    /// Where the user put the sun, in document pixels; `None` finds it.
    #[serde(default)]
    at: Option<[f64; 2]>,
}

impl Default for SunOptions {
    fn default() -> Self {
        SunOptions { frame: FRAMES[0].0.to_string(), heading: Some(0.0), exposure: Some(1.0), at: None }
    }
}

impl SunOptions {
    fn params(&self) -> Value {
        json!({"frame": self.frame, "heading": self.heading.unwrap_or(0.0), "exposure": self.exposure.unwrap_or(1.0).max(0.0), "at": self.at})
    }
}

/// The Find Sun dialog.
pub(crate) struct FindSunDialog {
    options: SunOptions,
    /// The command's last answer.
    result: Result<Value, String>,
    /// The options and document state `result` is for.
    computed: Option<(SunOptions, DocId, u64)>,
    /// Text a Copy button asked for, until [`take`] puts it on the clipboard.
    copied: Option<String>,
    /// The active document's canvas, which takes a press through the dialog's modal layer to put the sun.
    canvas: Option<egui::Id>,
}

fn number(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn triple(v: &Value, decimals: usize) -> String {
    (0..3).map(|i| format!("{:.decimals$}", number(&v[i]))).collect::<Vec<_>>().join(", ")
}

/// One read-out line: the value and a Copy button that copies `copy`.
fn readout(ui: &mut egui::Ui, text: String, button: &str, tooltip: Option<&str>, copy: String, copied: &mut Option<String>) {
    ui.horizontal(|ui| {
        Text::new(label(&text)).show(ui);
        let mut b = Button::new(label(button));
        if let Some(tip) = tooltip {
            b = b.tooltip(label(tip));
        }
        if b.show(ui).clicked() {
            *copied = Some(copy);
        }
    });
}

fn body(ui: &mut egui::Ui, options: &mut SunOptions, result: &Result<Value, String>, copied: &mut Option<String>) {
    let r = match result {
        Ok(r) => r,
        Err(error) => {
            Notice::new(Tone::Bad, label(error)).layout(NoticeLayout::Inline).show(ui);
            return;
        }
    };
    let note = if r["picked"].as_bool() == Some(true) {
        Some((Tone::Info, tl!("The sun is where you put it.")))
    } else if r["found"].as_bool() != Some(true) {
        Some((Tone::Warn, tl!("No light found in the image.")))
    } else if r["clear"].as_bool() != Some(true) {
        Some((Tone::Warn, tl!("No clear sun (an overcast sky?). This is the brightest area.")))
    } else {
        Some((Tone::Good, tl!("Found the sun.")))
    };
    if let Some((tone, text)) = note {
        Notice::new(tone, label(text)).layout(NoticeLayout::Inline).show(ui);
    }
    if r["equirectangular"].as_bool() == Some(false) {
        Notice::new(Tone::Warn, label(tl!("This image is not 2:1, so the result treats it as a full sphere anyway."))).layout(NoticeLayout::Inline).show(ui);
    }
    let p = &r["position"];
    ui.horizontal(|ui| {
        Text::new(label(&format!("{}: {:.0}, {:.0} px", tl!("Position"), number(&p[0]), number(&p[1])))).show(ui);
        if options.at.is_some() && Button::new(label(tl!("Find Automatically"))).show(ui).clicked() {
            options.at = None;
        }
    });
    Text::new(label(tl!("Wrong spot? Click or drag on the sun in the image."))).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    Text::new(label(&format!("{}: {:.2}°   {}: {:.2}°", tl!("Azimuth"), number(&r["azimuth"]), tl!("Elevation"), number(&r["elevation"])))).show(ui);

    let entries: Vec<Entry<String>> = FRAMES.iter().map(|(key, name, _)| Entry::new((*key).to_string(), label(tl!(name)))).collect();
    if let Some(frame) = Select::new(label(tl!("Axes")), Bind::given(options.frame.clone()), &entries).show(ui).value() {
        options.frame.clone_from(frame);
    }
    if let Some((_, _, about)) = FRAMES.iter().find(|(key, _, _)| *key == options.frame) {
        Text::new(label(tl!(about))).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    }
    ui.horizontal(|ui| {
        Text::new(label(tl!("Sky rotation"))).show(ui);
        NumberField::new(label(tl!("Sky rotation")), Bind::bound(&mut options.heading))
            .range(-360.0..=360.0)
            .unit("°")
            .tooltip(label(tl!("Turn this many degrees to the right if your sky is shown turned, so the image centre faces somewhere else. In Unity: 0 for a cubemap imported from this image, 90 for the Panoramic skybox, plus the skybox material's Rotation.")))
            .show(ui);
    });
    readout(ui, format!("{}: {}", tl!("Direction to the sun"), triple(&r["direction"], 4)), tl!("Copy direction"), None, triple(&r["direction"], 7), copied);
    let rotation = &r["lightRotation"];
    if !rotation.is_null() {
        let q = &rotation["quaternion"];
        let e = &rotation["euler"];
        readout(
            ui,
            format!("{}: X {:.2}°  Y {:.2}°  Z {:.2}°", tl!("Light rotation"), number(&e[0]), number(&e[1]), number(&e[2])),
            tl!("Copy rotation"),
            Some(tl!("Copies the rotation as Quaternion(x,y,z,w), the text Unity's Transform › Paste › Rotation reads.")),
            format!("Quaternion({:.7},{:.7},{:.7},{:.7})", number(&q[0]), number(&q[1]), number(&q[2]), number(&q[3])),
            copied,
        );
    }

    let light = &r["light"];
    let hex = light["colour"].as_str().unwrap_or("#FFFFFF").to_string();
    let kelvin = light["kelvin"].as_f64().map(|k| format!("  ~{:.0} K", (k / 50.0).round() * 50.0)).unwrap_or_default();
    readout(ui, format!("{}: {hex}{kelvin}", tl!("Light colour")), tl!("Copy colour"), None, hex.clone(), copied);
    let measured = light["measured"].as_bool() == Some(true);
    let intensity = number(&light["intensity"]);
    let estimate = if measured { String::new() } else { format!(" ({})", tl!("estimate")) };
    readout(ui, format!("{}: {intensity:.3}{estimate}", tl!("Light intensity")), tl!("Copy intensity"), None, format!("{intensity:.3}"), copied);
    ui.horizontal(|ui| {
        Text::new(label(tl!("Sky exposure"))).show(ui);
        NumberField::new(label(tl!("Sky exposure")), Bind::bound(&mut options.exposure))
            .range(0.0..=1000.0)
            .tooltip(label(tl!("The exposure your sky is shown at. Keep 1 for the light to match the image; the intensity scales with it.")))
            .show(ui);
    });
    let how = if measured {
        tl!(
            "Measured from the sun in this 32-bit image. The sun in the sky also lights a 3D scene through its environment lighting, so a scene can come out brighter than the photo unless the sun is removed from that copy of the sky."
        )
    } else if number(&r["elevation"]) <= 0.0 {
        tl!("The sun is below the horizon, so there is no direct sunlight to match.")
    } else if r["linear"].as_bool() == Some(true) && light["clipped"].as_bool() != Some(true) {
        tl!(
            "Nothing here is much brighter than the sky, so the intensity is estimated from the sky's brightness with a clear-sky model, and the colour from the sun's height."
        )
    } else if light["clipped"].as_bool() == Some(true) {
        tl!(
            "The sun is clipped in this image, so its real brightness is lost. The intensity is estimated from the sky's brightness with a clear-sky model, and the colour from the sun's height."
        )
    } else {
        tl!(
            "An 8 or 16-bit image clips the sun, so its real brightness is lost. The intensity is estimated from the sky's brightness with a clear-sky model, and the colour from the sun's height. A 32-bit image of the sun gives a measured value."
        )
    };
    Text::new(label(how)).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
}

impl StackDialog for FindSunDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id("photocraft.findSun")
    }

    fn show(&mut self, ui: &mut egui::Ui, _stack: &DialogStack) -> Option<DialogResult<()>> {
        let FindSunDialog { options, result, copied, canvas, .. } = self;
        let canvas = *canvas;
        let shown =
            FormDialog::new(label(tl!("Find Sun")), Bind::bound(options)).ok_label(label(tl!("Done"))).alt_resets(false).show(ui, |ui, form, options| {
                if let Some(canvas) = canvas {
                    form.pass_canvas_gesture(canvas, CanvasGesture::PRIMARY);
                    form.keep_view_open(canvas);
                }
                body(ui, options, result, copied);
                true
            });
        match (shown.accepted, shown.closed) {
            (Some(_), _) => Some(DialogResult::Ok(())),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn confirm(&mut self, _ctx: &egui::Context) -> Option<()> {
        Some(())
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({
            "__command": COMMAND,
            "frame": self.options.frame,
            "heading": self.options.heading,
            "exposure": self.options.exposure,
            "at": self.options.at,
            "result": self.result.as_ref().ok(),
            "error": self.result.as_ref().err(),
        }))
    }

    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        match field {
            "frame" => {
                let frame = value.as_str().filter(|f| FRAMES.iter().any(|(key, _, _)| key == f));
                let keys: Vec<&str> = FRAMES.iter().map(|(key, _, _)| *key).collect();
                self.options.frame = frame.ok_or_else(|| format!("field `frame` takes one of {keys:?}, got {value}"))?.to_string();
            }
            "heading" => self.options.heading = Some(value.as_f64().ok_or_else(|| format!("field `heading` takes degrees, got {value}"))?),
            "exposure" => {
                self.options.exposure = Some(value.as_f64().filter(|e| *e >= 0.0).ok_or_else(|| format!("field `exposure` takes a number ≥ 0, got {value}"))?);
            }
            "at" => {
                self.options.at = match &value {
                    Value::Null => None,
                    v => {
                        let xy = v.as_array().filter(|a| a.len() == 2).and_then(|a| Some([a[0].as_f64()?, a[1].as_f64()?]));
                        Some(xy.ok_or_else(|| format!("field `at` takes [x, y] in document pixels or null, got {value}"))?)
                    }
                };
            }
            _ => return Err(format!("the Find Sun dialog has no field `{field}`")),
        }
        ctx.request_repaint();
        Ok(())
    }
}

/// Opens the dialog for the active document.
pub(crate) fn open(app: &mut PhotocraftApp) -> Result<DialogId, String> {
    app.session.active().ok_or_else(|| tl!("No document is open.").to_string())?;
    // A sun put by hand belongs to the image it was put on.
    let options = SunOptions { at: None, ..app.find_sun_options.clone() };
    let result = app.run(COMMAND, options.params());
    let computed = app.session.active().map(|st| (options.clone(), st.doc.id, st.revision));
    let canvas = app.session.active_index().map(|idx| crate::canvas::document_display::CanvasHost::primary(app, idx).canvas_id());
    Ok(app.framework.dialogs.open(FindSunDialog { options, result, computed, copied: None, canvas }))
}

/// The dialog's result pump (a `TAKERS` row): puts a Copy button's text on the clipboard, and
/// reruns the command when an option or the active document changed. The options are kept for the
/// next open.
pub(crate) fn take(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.framework.dialogs.take::<FindSunDialog>().is_some() {
        return;
    }
    let Some(dialog) = app.framework.dialogs.get_mut::<FindSunDialog>() else { return };
    if let Some(text) = dialog.copied.take() {
        ctx.copy_text(text.clone());
        app.ui.status = format!("{} {text}", tl!("Copied"));
        app.ui.status_error = false;
    }
    let Some(dialog) = app.framework.dialogs.get::<FindSunDialog>() else { return };
    let options = dialog.options.clone();
    let Some(now) = app.session.active().map(|st| (options.clone(), st.doc.id, st.revision)) else { return };
    if dialog.computed.as_ref() == Some(&now) {
        return;
    }
    app.find_sun_options = options.clone();
    let result = app.run(COMMAND, options.params());
    if let Some(dialog) = app.framework.dialogs.get_mut::<FindSunDialog>() {
        dialog.result = result;
        dialog.computed = Some(now);
        ctx.request_repaint();
    }
}

/// A press or drag step at `at` (document pixels) on the image with the dialog on top: the sun goes
/// there, and the next pump measures it. A point off the document is the dialog's press but moves
/// nothing.
pub(crate) fn place(app: &mut PhotocraftApp, at: [f64; 2]) -> bool {
    let Some(size) = app.session.active().map(|st| st.doc.bounds()) else { return false };
    let Some(dialog) = app.framework.dialogs.get_mut::<FindSunDialog>() else { return false };
    let inside = (0.0..=f64::from(size.width())).contains(&at[0]) && (0.0..=f64::from(size.height())).contains(&at[1]);
    if inside {
        dialog.options.at = Some(at);
        app.ctx().request_repaint();
    }
    true
}

/// The sun the canvas marks: where the user put it, or else the dialog's last result.
pub(crate) fn marker(app: &PhotocraftApp) -> Option<[f64; 2]> {
    let dialog = app.framework.dialogs.get::<FindSunDialog>()?;
    dialog.options.at.or_else(|| {
        let p = &dialog.result.as_ref().ok()?["position"];
        Some([p[0].as_f64()?, p[1].as_f64()?])
    })
}
