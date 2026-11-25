//! JPEG Options: the quality and format File › Save, Save As and Save a Copy ask for before writing
//! a JPEG, as Photoshop's JPEG Options does. Quality is Photoshop's 0–12 scale (Low 0–4, Medium
//! 5–7, High 8–9, Maximum 10–12), mapped to the encoder's 1–100 by
//! `photocraft_engine::file_cmds::jpeg_quality`; Format Options are Baseline ("Standard"), Baseline
//! Optimized and Progressive, and the dialog shows the size the file will have. Matte and Scans
//! aren't offered yet. Re-saving an opened JPEG starts at the file's own quality, as File › Save
//! keeps it (#4032). Otherwise the dialog starts at the quality and format it was last answered
//! with, also after a restart, as Photoshop's does (see [`Start::for_active`]).
//!
//! The save is parked (`format_prompts::ParkedSave`) while the hosted dialog is up and written
//! once answered. Cancel drops the save. Scripts skip the prompt by passing `quality` or
//! `jpegFormat`.
//!
//! Any save can ask: check [`wants_prompt`], then [`park`] (which starts where
//! [`Start::for_active`] says) and return. The answer is written by [`finish`] and journalled as
//! the `record` command passed to `park`: with its path, `quality` (0–12, left out when the
//! file's own quality was kept) and `jpegFormat`, or without the path for a plain `file.save`,
//! which writes whichever file an action is played on ([`plain_save_step`]). File › Save over a
//! flat JPEG that still holds the document asks too, as Photoshop does (#2223):
//! `PhotocraftApp::save_flat_in_place`.

use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{ExportOptionsKind, FormatOptions, HostedFormatOptions, JpegFormat as UiJpegFormat, JpegOptions, SizeEstimate};
use craft::ui::runtime::memory::{SaveKey, saved_value, set_saved_value};
use photocraft_codecs::JpegFormat;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::format_prompts::{self, ParkedFormat, ParkedSave};
use crate::{ExportSettings, PhotocraftApp};

mod size_readout;

/// The highest quality on Photoshop's JPEG scale.
pub const MAX_QUALITY: u8 = 12;

/// A JPEG Options answer, from the prompt or from a command's `quality` and `jpegFormat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JpegChoice {
    /// Photoshop's 0–12 scale; `None` keeps the quality the save would use anyway (the opened
    /// file's own, else the encoder's default).
    pub quality: Option<u8>,
    pub format: JpegFormat,
}

impl JpegChoice {
    /// `settings` writing this choice.
    pub fn apply(self, settings: ExportSettings) -> ExportSettings {
        ExportSettings {
            jpeg_quality: self.quality.map(|q| photocraft_engine::file_cmds::jpeg_quality(f64::from(q))).or(settings.jpeg_quality),
            jpeg_format: self.format,
            ..settings
        }
    }

    /// Adds `quality` and `jpegFormat` to a recorded save step, so replaying it doesn't ask again.
    pub fn record(self, step: &mut Value) {
        if let Some(q) = self.quality {
            step["quality"] = q.into();
        }
        step["jpegFormat"] = self.format.name().into();
    }
}

/// The JPEG Options a command or control call gives (`"quality": 0..12`, `"jpegFormat"`), which
/// skip the prompt. Neither given: `None`. A quality that isn't a number from 0 to 12, or a
/// format other than the three names, is an error.
pub fn choice_param(params: &Value, cmd: &str) -> Result<Option<JpegChoice>, String> {
    let format = photocraft_engine::file_cmds::jpeg_format_param(params, cmd).map_err(|e| e.to_string())?;
    let quality = match params.get("quality") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_f64()
                .filter(|q| (0.0..=f64::from(MAX_QUALITY)).contains(q))
                .map(|q| q.round() as u8)
                .ok_or_else(|| format!("{cmd}: `quality` must be a number from 0 to 12 (Photoshop's JPEG scale), got {v}"))?,
        ),
    };
    Ok((quality.is_some() || format.is_some()).then(|| JpegChoice { quality, format: format.unwrap_or_default() }))
}

/// The params of a recorded plain `file.save` (which writes whichever file an action is played
/// on): none, unless JPEG Options changed something, so that a replay writes it too without
/// asking. `quality` is left out when the file's own quality was kept, and `jpegFormat` when it
/// is Baseline, the default.
pub fn plain_save_step(choice: Option<JpegChoice>) -> Value {
    let mut step = Value::Object(Default::default());
    if let Some(c) = choice {
        if let Some(q) = c.quality {
            step["quality"] = q.into();
        }
        if c.format != JpegFormat::Baseline {
            step["jpegFormat"] = c.format.name().into();
        }
    }
    step
}

/// The JPEG Options in a recorded save step, which replays without asking. A `file.saveACopy`
/// step from the engine command keeps its params as they were given, and the engine clamps a
/// `quality` to 0–12 and ignores one that isn't a number, so the replay reads it the same way. A
/// format other than the three names is still an error, as it is for the engine.
pub fn recorded_choice(params: &Value, cmd: &str) -> Result<Option<JpegChoice>, String> {
    let format = photocraft_engine::file_cmds::jpeg_format_param(params, cmd).map_err(|e| e.to_string())?;
    let quality = params.get("quality").and_then(Value::as_f64).filter(|q| q.is_finite()).map(|q| q.clamp(0.0, f64::from(MAX_QUALITY)).round() as u8);
    Ok((quality.is_some() || format.is_some()).then(|| JpegChoice { quality, format: format.unwrap_or_default() }))
}

/// The 0–12 quality nearest an encoder quality (1–100), the inverse of the engine's mapping.
pub fn quality_from_percent(q: u32) -> u8 {
    let q = f64::from(q.clamp(1, 100));
    ((q - 1.0) / 99.0 * f64::from(MAX_QUALITY)).round() as u8
}

/// Where the prompt starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Start {
    /// Quality on the 0–12 scale.
    pub quality: u8,
    /// The opened file's own encoder quality (1–100) that `quality` was rounded from. Left on
    /// that step, OK writes it exactly and records no `quality`, so a replay keeps it too.
    pub own_quality: Option<u8>,
    pub format: JpegFormat,
}

impl Start {
    /// Starts at an opened JPEG's own encoder quality (1–100; out of range is clamped).
    pub fn own(quality: u8) -> Self {
        let q = quality.clamp(1, 100);
        Start { quality: quality_from_percent(u32::from(q)), own_quality: Some(q), format: JpegFormat::Baseline }
    }

    /// Starts at an encoder quality (1–100) that isn't the file's own, such as a preference.
    pub fn percent(quality: u32) -> Self {
        Start { quality: quality_from_percent(quality), own_quality: None, format: JpegFormat::Baseline }
    }

    /// For the active document: the Format Options the dialog was last answered with, else
    /// Baseline; and the quality of its opened JPEG (`Document::file_encoding`, which File › Save
    /// keeps, #4032), else the quality last answered, else the Export preferences' JPEG quality
    /// (the one Quick Export uses).
    pub fn for_active(app: &PhotocraftApp) -> Self {
        let remembered = remembered(app);
        let start = match app.session.active().and_then(|st| st.doc.file_encoding.jpeg_quality) {
            Some(q) => Start::own(q),
            None => match remembered.quality {
                Some(quality) => Start { quality, own_quality: None, format: JpegFormat::Baseline },
                None => Start::percent(app.session.prefs().export.jpeg_quality),
            },
        };
        Start { format: remembered.format.unwrap_or(start.format), ..start }
    }
}

/// The saved value JPEG Options writes its last answer under and starts the next dialog from, as
/// Photoshop does (it keeps it across restarts).
pub(crate) const REMEMBER_KEY: SaveKey = SaveKey::new("photocraft.file.jpegOptions");

/// The saved record of the last answer. A field that no longer fits is left out on its own when
/// read (see [`remembered`]), so one bad value doesn't reset the other.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Remembered {
    /// The quality on the 0–12 scale.
    pub(crate) quality: Option<u8>,
    /// The format's name (`JpegFormat::name`).
    pub(crate) jpeg_format: Option<String>,
}

/// What the dialog was last answered with, each part checked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Last {
    quality: Option<u8>,
    format: Option<JpegFormat>,
}

fn remembered(app: &PhotocraftApp) -> Last {
    let saved: Remembered = saved_value(app.ctx(), &REMEMBER_KEY);
    Last { quality: saved.quality.filter(|&q| q <= MAX_QUALITY), format: saved.jpeg_format.as_deref().and_then(JpegFormat::parse) }
}

/// Records an answered dialog for the next one. A quality left on the opened file's own is not
/// an answer: the earlier one stays, so saving a series of opened JPEGs doesn't forget what a
/// new document was last saved at.
fn remember(app: &PhotocraftApp, choice: JpegChoice) {
    let quality = choice.quality.or(remembered(app).quality);
    set_saved_value(app.ctx(), &REMEMBER_KEY, Remembered { quality, jpeg_format: Some(choice.format.name().into()) });
}

/// What a parked JPEG save keeps besides the hosted dialog's answer.
pub struct Parked {
    /// See [`Start::own_quality`].
    own_quality: Option<u8>,
    /// The sizes worked out for the dialog's footer.
    readout: size_readout::Readout,
    /// The estimate the dialog asked for and has not been answered yet.
    asked: Option<Asked>,
}

impl Parked {
    /// A parked JPEG save whose prompt started at `own_quality` (see [`Start::own_quality`]).
    fn new(own_quality: Option<u8>) -> Self {
        Parked { own_quality, readout: size_readout::Readout::default(), asked: None }
    }
}

/// An estimate request: the dialog's revision and the settings it wants the size of.
#[derive(Clone, Copy)]
struct Asked {
    revision: u64,
    /// Quality on the 0–12 scale.
    quality: u8,
    format: JpegFormat,
}

/// The framework's format for the codec's.
fn ui_format(format: JpegFormat) -> UiJpegFormat {
    match format {
        JpegFormat::Optimized => UiJpegFormat::BaselineOptimized,
        JpegFormat::Progressive => UiJpegFormat::Progressive,
        JpegFormat::Baseline => UiJpegFormat::Baseline,
    }
}

/// The codec's format for the framework's.
fn codec_format(format: UiJpegFormat) -> JpegFormat {
    match format {
        UiJpegFormat::BaselineOptimized => JpegFormat::Optimized,
        UiJpegFormat::Progressive => JpegFormat::Progressive,
        _ => JpegFormat::Baseline,
    }
}

/// `true` when `path` names a JPEG (`.jpg`, `.jpeg`, `.jpe`, …).
pub fn is_jpeg_path(path: &str) -> bool {
    photocraft_engine::file_cmds::extension(path).and_then(|e| photocraft_formats::from_extension(&e)) == Some(photocraft_formats::Format::Jpeg)
}

/// Whether saving the active document to `path` asks for JPEG Options.
pub fn wants_prompt(app: &PhotocraftApp, path: &str) -> bool {
    is_jpeg_path(path) && app.session.active().is_some()
}

/// Parks the save of the active document to `path` behind the prompt, starting at
/// [`Start::for_active`]. A prompt already waiting is never replaced.
pub fn park(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str) -> Result<(), String> {
    park_dialog(app, path, copy, record).map(drop)
}

/// [`park`], returning the id of the dialog it opened.
pub fn park_dialog(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str) -> Result<DialogId, String> {
    let doc = app.active_doc_id()?;
    let Start { quality, own_quality, format } = Start::for_active(app);
    let mut options = JpegOptions::new(quality.min(MAX_QUALITY));
    options.format = ui_format(format);
    let dialog = HostedFormatOptions::new(app.ctx(), ExportOptionsKind::Jpeg).initial(FormatOptions::Jpeg(options));
    format_prompts::park(app, ParkedSave { doc, path, copy, record, format: ParkedFormat::Jpeg(Parked::new(own_quality)) }, dialog)
}

/// Answers the hosted dialog's request for the size of its current settings, once the worker has
/// it. A pass applies only its own state: a second pass in a frame finds no new request, and the
/// request still waiting for its encode is answered by whichever pass sees the size, once.
pub(crate) fn poll_estimate(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !matches!(&app.parked_save, Some(ParkedSave { format: ParkedFormat::Jpeg(_), .. })) {
        return;
    }
    let request = app.framework.dialogs.get_mut::<HostedFormatOptions>().and_then(|dialog| dialog.wanted_estimate());
    if let Some(request) = request
        && let (Some(ParkedSave { format: ParkedFormat::Jpeg(parked), .. }), FormatOptions::Jpeg(options)) = (&mut app.parked_save, request.options)
    {
        parked.asked = Some(Asked { revision: request.revision, quality: options.quality, format: codec_format(options.format) });
    }
    let asked = match &app.parked_save {
        Some(ParkedSave { format: ParkedFormat::Jpeg(parked), .. }) => parked.asked,
        _ => None,
    };
    let Some(asked) = asked else { return };
    let source = size_readout::Source::of(app);
    let Some(ParkedSave { format: ParkedFormat::Jpeg(parked), .. }) = &mut app.parked_save else { return };
    // A document that can't be estimated (too large, or gone) answers "no estimate" at once.
    let answer = match source.filter(size_readout::Source::estimable) {
        None => Some(None),
        Some(source) => {
            let repaint = ctx.clone();
            parked.readout.update(source.key(asked.quality, asked.format), || source.job(asked.quality, asked.format), move || repaint.request_repaint())
        }
    };
    if let Some(size) = answer {
        parked.asked = None;
        if let Some(dialog) = app.framework.dialogs.get_mut::<HostedFormatOptions>() {
            dialog.set_estimate(asked.revision, size.map(SizeEstimate::Exact));
        }
    }
}

/// OK: writes the parked save with the dialog's quality and format, and journals it.
pub(crate) fn finish(app: &mut PhotocraftApp, ctx: &egui::Context, save: ParkedSave, options: JpegOptions) {
    let own_quality = match &save.format {
        ParkedFormat::Jpeg(parked) => parked.own_quality,
        _ => None,
    };
    let (quality, format) = (options.quality, codec_format(options.format));
    // Left where it started, an opened JPEG keeps its own quality exactly rather than the nearest
    // step on the 0–12 scale (95 would be written as 92). The recorded step then leaves the
    // quality out, so a replay keeps the file's own quality too, as File › Save does (#4032).
    let own = own_quality.filter(|&q| quality_from_percent(u32::from(q)) == quality);
    let choice = JpegChoice { quality: own.is_none().then_some(quality), format };
    remember(app, choice);
    let step = if save.record == "file.save" {
        // A plain Save writes whichever file the action is played on (#3531), with the answer.
        plain_save_step(Some(choice))
    } else {
        let mut step = save.step();
        choice.record(&mut step);
        step
    };
    let mut settings = choice.apply(save.settings(app));
    if own.is_some() {
        settings.jpeg_quality = own;
    }
    let ParkedSave { doc, path, copy, record, .. } = save;
    app.write_answered_save(ctx, doc, path, &settings, copy, (record, step));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn choice_params() {
        let base = JpegFormat::Baseline;
        assert_eq!(choice_param(&json!({}), "file.saveAs"), Ok(None));
        assert_eq!(choice_param(&json!({"quality": null, "jpegFormat": null}), "file.saveAs"), Ok(None));
        assert_eq!(choice_param(&json!({"quality": 12}), "file.saveAs"), Ok(Some(JpegChoice { quality: Some(12), format: base })));
        assert_eq!(choice_param(&json!({"quality": 7.6}), "file.saveAs"), Ok(Some(JpegChoice { quality: Some(8), format: base })));
        assert_eq!(choice_param(&json!({"jpegFormat": "progressive"}), "file.saveAs"), Ok(Some(JpegChoice { quality: None, format: JpegFormat::Progressive })));
        for bad in [
            json!({"quality": 13}),
            json!({"quality": -1}),
            json!({"quality": 1e300}),
            json!({"quality": "8"}),
            json!({"quality": true}),
            json!({"quality": [8]}),
            json!({"jpegFormat": "Optimized"}),
            json!({"jpegFormat": 1}),
            json!({"quality": 8, "jpegFormat": ""}),
        ] {
            assert!(choice_param(&bad, "file.saveAs").is_err(), "{bad}");
        }
    }

    #[test]
    fn recorded_steps_read_like_the_engine() {
        let base = JpegFormat::Baseline;
        assert_eq!(recorded_choice(&json!({"path": "a.jpg"}), "file.saveACopy"), Ok(None));
        assert_eq!(recorded_choice(&json!({"quality": 90}), "file.saveACopy"), Ok(Some(JpegChoice { quality: Some(12), format: base })));
        assert_eq!(recorded_choice(&json!({"quality": -3}), "file.saveACopy"), Ok(Some(JpegChoice { quality: Some(0), format: base })));
        assert_eq!(recorded_choice(&json!({"quality": "8"}), "file.saveACopy"), Ok(None), "a quality that isn't a number is ignored");
        assert_eq!(
            recorded_choice(&json!({"quality": 7.6, "jpegFormat": "optimized"}), "file.saveAs"),
            Ok(Some(JpegChoice { quality: Some(8), format: JpegFormat::Optimized }))
        );
        assert!(recorded_choice(&json!({"jpegFormat": "fast"}), "file.saveACopy").is_err());
    }

    #[test]
    fn quality_scale() {
        // The prompt's starting quality maps back to the encoder quality it came from, near enough.
        for q in 0..=MAX_QUALITY {
            assert_eq!(quality_from_percent(u32::from(photocraft_engine::file_cmds::jpeg_quality(f64::from(q)))), q);
        }
        assert_eq!((quality_from_percent(0), quality_from_percent(85), quality_from_percent(1000)), (0, 10, 12));
    }

    /// The desktop and web exporters write through `ExportSettings::io_options`: the format picked
    /// in JPEG Options reaches the encoder there, not just the settings.
    #[test]
    fn the_answer_reaches_the_encoder() {
        let doc = photocraft_doc::Document::with_background(
            "x",
            photocraft_doc::Size::new(16, 16),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U8,
            photocraft_doc::Color::WHITE,
        );
        let write = |format| {
            let settings = JpegChoice { quality: Some(10), format }.apply(ExportSettings::default());
            photocraft_io::export(&doc, "a.jpg", &settings.io_options()).unwrap().bytes
        };
        // Start of frame: 0xFFC0 baseline, 0xFFC2 progressive. Scan data stuffs its 0xFF bytes, and
        // this small white image's tables and headers hold neither pair, so only the frame marker matches.
        let sof = |b: &[u8], m: u8| b.windows(2).any(|w| w == [0xFF, m]);
        let (base, prog) = (write(JpegFormat::Baseline), write(JpegFormat::Progressive));
        assert!(sof(&base, 0xC0) && !sof(&base, 0xC2), "baseline");
        assert!(sof(&prog, 0xC2) && !sof(&prog, 0xC0), "progressive");
    }
}
