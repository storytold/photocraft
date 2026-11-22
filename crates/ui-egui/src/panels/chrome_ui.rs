//! Window chrome details: what the status bar shows and remembers, the render-path note, the Home
//! screen state and the options bar's crop and marquee rules.

use craft::behavior::PixelSize;
use craft::ui::bars::{StatusItem, StatusValues};
use craft::ui::dialog::{Alert, HostedAlert};
use craft::ui::runtime::env::Env;
use craft::ui::shell::{RecentSort, RecentView};
use craft::ui::text::InvalidZoom;
use photocraft_doc::{Document, LayerContent};
use serde::{Deserialize, Serialize};

use crate::PhotocraftApp;
use crate::render_path::CpuReason;

/// Shell-only chrome state (serialised with the UI state, so `ui.inspect` reports it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ChromeState {
    /// What the status bar shows next to the zoom field: one of the keys [`status_item`] reads.
    pub status_info: String,
    /// Home screen shown over the open documents (Options bar › Home); holds the document count
    /// when it was opened, so opening or creating a document leaves it.
    pub home: Option<usize>,
    /// How the Home screen lists recent files: thumbnail cards or a text list.
    #[serde(default)]
    pub home_recent_view: RecentView,
    /// The order of the recent files on the Home screen.
    #[serde(default)]
    pub home_recent_sort: RecentSort,
}

impl ChromeState {
    /// Is the Home screen up, given the current number of open documents? With no documents it
    /// shows by itself when `auto_show` (Preferences › General › Auto show the Home Screen).
    ///
    /// `home` stores the document count when Home was opened so File › New / Open can leave it.
    /// Closing a tab that returns to that count must not bring Home back (#3981): once the
    /// document set has changed, the overlay is gone until the user opens Home again.
    pub fn shows_home(&mut self, documents: usize, auto_show: bool) -> bool {
        if self.home.is_some_and(|n| n != documents) {
            self.home = None;
        }
        (documents == 0 && auto_show) || self.home == Some(documents)
    }
}

impl Default for ChromeState {
    fn default() -> Self {
        Self { status_info: "dimensions".into(), home: None, home_recent_view: RecentView::default(), home_recent_sort: RecentSort::default() }
    }
}

/// The saved key of each status bar item. The keys are what saved UI state holds.
const STATUS_KEYS: [(&str, StatusItem); 9] = [
    ("sizes", StatusItem::DocumentSizes),
    ("profile", StatusItem::DocumentProfile),
    ("dimensions", StatusItem::DocumentDimensions),
    ("measurementScale", StatusItem::MeasurementScale),
    ("scratch", StatusItem::ScratchSizes),
    ("efficiency", StatusItem::Efficiency),
    ("timing", StatusItem::Timing),
    ("tool", StatusItem::CurrentTool),
    ("layers", StatusItem::LayerCount),
];

/// The status bar item saved as `key`; `None` for a key that names no item.
pub(crate) fn status_item_of(key: &str) -> Option<StatusItem> {
    STATUS_KEYS.iter().find(|(saved, _)| *saved == key).map(|(_, item)| *item)
}

/// The status bar item the chrome state remembers; an unknown key shows the dimensions.
pub(crate) fn status_item(chrome: &ChromeState) -> StatusItem {
    status_item_of(&chrome.status_info).unwrap_or(StatusItem::DocumentDimensions)
}

/// Remembers the status bar item the user picked.
pub(crate) fn set_status_item(chrome: &mut ChromeState, item: StatusItem) {
    if let Some((key, _)) = STATUS_KEYS.iter().find(|(_, shown)| *shown == item) {
        chrome.status_info = (*key).to_owned();
    }
}

/// What the status bar readout knows about the active document. Items without a value here show a dash.
/// The document's values are computed once per document revision and language.
pub(crate) fn status_values(app: &PhotocraftApp) -> StatusValues {
    let Some(st) = app.session.active() else { return StatusValues::default() };
    let mut values = app.doc_queries.status(&app.session, st, || {
        let doc: &Document = &st.doc;
        let mut values = StatusValues::default();
        values.document_sizes = Some(document_sizes(doc));
        values.document_profile = Some(format!("{} ({}bpc)", profile_name(doc), doc.depth.bits()));
        values.document_dimensions = Some(PixelSize::new(doc.size.width, doc.size.height));
        values.document_resolution = Some(f64::from(doc.resolution_dpi));
        values.layer_count = Some(doc.layer_count());
        values
    });
    values.current_tool = Some(super::tool_commands::command_of(app.ui.tool).into());
    values
}

/// Opens the Invalid zoom alert for a zoom entry the field cut to its range. The status bar, the options bar and the
/// Navigator open it here; the field reports the entry once, in the pass that committed it.
pub(crate) fn zoom_alert(app: &mut PhotocraftApp, invalid: Option<InvalidZoom>) {
    let Some(invalid) = invalid else { return };
    let message = invalid.message(&Env::of_root(app.ctx()));
    app.framework.dialogs.open(HostedAlert::new(Alert::new(tl!("Invalid Zoom"), message)));
}

/// Photoshop's compact byte format: "10.3M", "412.5K".
pub fn fmt_bytes(b: u64) -> String {
    let b = b as f64;
    if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1}G", b / (1024.0 * 1024.0 * 1024.0))
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.1}M", b / (1024.0 * 1024.0))
    } else {
        format!("{:.1}K", b / 1024.0)
    }
}

/// Flattened size and layered size in bytes ("Doc: flat/layered").
pub fn document_sizes(doc: &Document) -> (u64, u64) {
    let bpc = (doc.depth.bits() / 8).max(1) as u64;
    let channels = doc.mode.color_channels().max(1) as u64;
    let flat = doc.size.width as u64 * doc.size.height as u64 * channels * bpc;
    let mut layered = 0u64;
    for (_, _, l) in doc.walk() {
        if let LayerContent::Raster(s) = &l.content {
            // Layers carry transparency on top of the colour channels; clip to the canvas.
            let r = s.tile_bounds().intersect(&doc.bounds());
            layered += r.width() as u64 * r.height() as u64 * (channels + 1) * bpc;
        }
    }
    (flat, layered.max(flat))
}

/// The document's colour profile as the status bar names it.
pub(crate) fn profile_name(doc: &Document) -> String {
    let mode = crate::canvas::mode_label(doc);
    let Some(bytes) = doc.icc_profile.as_ref() else {
        return crate::i18n::fmt(tl!("Untagged {mode}"), &[("mode", tl!(mode))]);
    };
    let Ok(profile) = photocraft_engine::color_cmds::profile_from_bytes(bytes) else {
        return crate::i18n::fmt(tl!("Invalid {mode} profile"), &[("mode", tl!(mode))]);
    };
    if profile.color_space != photocraft_engine::color_cmds::mode_space(doc.mode) {
        return crate::i18n::fmt(tl!("Invalid {mode} profile"), &[("mode", tl!(mode))]);
    }
    let name: String = profile.description.chars().filter(|c| !c.is_control()).take(128).collect();
    let name = name.trim();
    if name.is_empty() {
        return crate::i18n::fmt(tl!("Unnamed {mode} profile"), &[("mode", tl!(mode))]);
    }
    name.to_owned()
}

/// What the status bar says about the active document's render path: its label, tooltip and
/// the command a click runs, or None on the GPU.
pub(crate) fn render_path_label(app: &PhotocraftApp) -> Option<(&'static str, String, &'static str)> {
    let doc = app.session.active().map(|st| st.doc.id.0)?;
    let reason = app.render_path.current(doc);
    // The measured fit (also on a switch to a document whose canvas is current) or the reason
    // of the last full refresh: edits that redraw the whole document do it on the CPU, so say
    // so until they fit again; GPU Memory in Preferences › Performance raises the budget.
    let fit = app.perf.gpu_layer_fit.filter(|f| f.doc == doc && !f.fits()).map(|f| (f.needed, f.budget));
    // Another reason (a flipped view, Advanced Blending…) is the cause: show it, not the fit.
    let over = match reason {
        Some(&CpuReason::OverBudget { needed, budget }) => fit.or(Some((needed, budget))),
        Some(_) => None,
        None => fit,
    };
    if let Some((needed, budget)) = over {
        let tip = crate::i18n::fmt(
            tl!(
                "The layers need {needed} of GPU memory and may hold {budget}, so edits that redraw the whole document do it on the CPU. Click to set GPU Memory."
            ),
            &[("needed", &fmt_bytes(needed)), ("budget", &fmt_bytes(budget))],
        );
        return Some((tl!("GPU Memory Full"), tip, "edit.preferences.performance"));
    }
    let reason = reason?;
    let tip = crate::i18n::fmt(tl!("Drawn on the CPU: {reason}. Click for System Info."), &[("reason", &reason.to_string())]);
    Some((tl!("CPU Rendering"), tip, "help.systemInfo"))
}

/// Marquee drag end point under the options-bar Style (Normal, Fixed Ratio, Fixed Size) and Shift
/// (square/circle while drawing with Normal style), Photoshop behaviour.
pub fn marquee_end(style: &str, w: f64, h: f64, shift: bool, start: [f64; 2], end: [f64; 2]) -> [f64; 2] {
    let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
    let (sx, sy) = (if dx < 0.0 { -1.0 } else { 1.0 }, if dy < 0.0 { -1.0 } else { 1.0 });
    match style {
        "fixedSize" => [start[0] + sx * w.max(1.0), start[1] + sy * h.max(1.0)],
        "fixedRatio" if w > 0.0 && h > 0.0 => {
            // The larger drag extent wins; the other follows the ratio.
            let k = (dx.abs() / w).max(dy.abs() / h);
            [start[0] + sx * k * w, start[1] + sy * k * h]
        }
        _ if shift => {
            let m = dx.abs().max(dy.abs());
            [start[0] + sx * m, start[1] + sy * m]
        }
        _ => end,
    }
}

/// Crop options bar ratio presets: (key, label).
pub const CROP_RATIOS: &[(&str, &str)] = &[
    ("", "Ratio"),
    ("original", "Original Ratio"),
    ("1:1", "1 : 1 (Square)"),
    ("4:5", "4 : 5 (8 : 10)"),
    ("5:7", "5 : 7"),
    ("2:3", "2 : 3 (4 : 6)"),
    ("16:9", "16 : 9"),
];

/// Width/height of a crop ratio key (`original` uses the document size). The sides are the numbers the app writes
/// into the key.
pub fn crop_ratio(key: &str, doc_w: f64, doc_h: f64) -> Option<(f64, f64)> {
    if key == "original" {
        return Some((doc_w, doc_h));
    }
    let (a, b) = key.split_once(':')?;
    Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc() -> Document {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 2400, "height": 1500, "resolution": 72, "background": "white"})).unwrap();
        (*s.active().unwrap().doc).clone()
    }

    /// The label the status bar shows for the active document, given a measured fit (the document id's offset from
    /// the active document, bytes needed, budget) and an optional CPU reason.
    fn label(fit: Option<(i64, u64, u64)>, reason: Option<CpuReason>) -> Option<(&'static str, String, &'static str)> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
        let doc = app.session.active().unwrap().doc.id.0;
        if let Some((offset, needed, budget)) = fit {
            let doc = doc.wrapping_add_signed(offset);
            app.perf.gpu_layer_fit = Some(crate::gpu_canvas::LayerFit { doc, needed, budget });
        }
        if let Some(reason) = reason {
            crate::render_path::note(&mut app, doc, crate::render_path::Path::Cpu(reason));
        }
        render_path_label(&app)
    }

    #[test]
    fn the_render_path_note_follows_the_fit_and_the_cpu_reason() {
        let gb = 1u64 << 30;
        assert!(label(None, None).is_none(), "nothing measured");
        assert!(label(Some((1, 3 * gb, 2 * gb)), None).is_none(), "another document's layers");
        assert!(label(Some((0, 2 * gb, 4 * gb)), None).is_none(), "they fit");
        let (text, _, command) = label(Some((0, 3 * gb, 2 * gb)), None).expect("over budget");
        assert_eq!((text, command), ("GPU Memory Full", "edit.preferences.performance"));
        // Another reason is the cause, not the fit.
        let other = CpuReason::Unsupported("Advanced Blending on `Layer 1`".into());
        let (text, tip, command) = label(Some((0, 3 * gb, 2 * gb)), Some(other)).expect("another reason");
        assert_eq!((text, command), ("CPU Rendering", "help.systemInfo"));
        assert!(tip.contains("Advanced Blending"), "{tip}");
        // The budget raised: the fit clears the over-budget reason too.
        let over = CpuReason::OverBudget { needed: 3 * gb, budget: 2 * gb };
        assert_eq!(label(Some((0, 3 * gb, 2 * gb)), Some(over.clone())).map(|l| l.0), Some("GPU Memory Full"));
        assert!(label(Some((0, 3 * gb, 4 * gb)), None).is_none());
    }

    #[test]
    fn the_profile_name_describes_each_profile_and_leaves_the_document_alone() {
        let mut rgb = doc();
        rgb.icc_profile = Some(photocraft_engine::color_cmds::working_profile(rgb.mode).to_bytes());
        let before = rgb.icc_profile.clone();
        assert_eq!(profile_name(&rgb), "sRGB IEC61966-2.1");
        assert_eq!(rgb.icc_profile, before);

        let mut gray = doc();
        gray.mode = photocraft_doc::ColorMode::Grayscale;
        gray.icc_profile = Some(photocraft_engine::color_cmds::working_profile(gray.mode).to_bytes());
        assert_eq!(profile_name(&gray), "Dot Gain 20%-compatible (gamma 1.732, PhotoCraft)");

        let mut custom = (*photocraft_engine::color_cmds::working_profile(photocraft_doc::ColorMode::Cmyk)).clone();
        custom.description = "PhotoCraft Studio CMYK".into();
        let mut custom_doc = doc();
        custom_doc.mode = photocraft_doc::ColorMode::Cmyk;
        custom_doc.icc_profile = Some(custom.with_encoded_bytes().to_bytes());
        assert_eq!(profile_name(&custom_doc), "PhotoCraft Studio CMYK");

        let mut malformed = doc();
        malformed.icc_profile = Some(std::sync::Arc::new(vec![1, 2, 3]));
        assert_eq!(profile_name(&malformed), "Invalid RGB profile");

        let mut unnamed_profile = (*photocraft_engine::color_cmds::working_profile(photocraft_doc::ColorMode::Cmyk)).clone();
        unnamed_profile.description.clear();
        let mut unnamed = doc();
        unnamed.mode = photocraft_doc::ColorMode::Cmyk;
        unnamed.icc_profile = Some(unnamed_profile.with_encoded_bytes().to_bytes());
        assert_eq!(profile_name(&unnamed), "Unnamed CMYK profile");

        let mut mismatched = doc();
        mismatched.mode = photocraft_doc::ColorMode::Grayscale;
        mismatched.icc_profile = Some(photocraft_engine::color_cmds::working_profile(photocraft_doc::ColorMode::Rgb).to_bytes());
        assert_eq!(profile_name(&mismatched), "Invalid Gray profile");
    }

    #[test]
    fn document_sizes_match_photoshop_rounding() {
        // 2400 x 1500 x 3 bytes = 10.3M flattened, as Photoshop shows.
        let (flat, layered) = document_sizes(&doc());
        assert_eq!((fmt_bytes(flat), fmt_bytes(layered)), ("10.3M".to_owned(), "13.7M".to_owned()));
        assert_eq!(fmt_bytes(512 * 1024), "512.0K");
    }

    #[test]
    fn the_home_screen_leaves_when_the_document_count_changes_and_not_when_a_tab_closes_back() {
        let mut c = ChromeState::default();
        assert!(c.shows_home(0, true));
        assert!(!c.shows_home(2, true));
        c.home = Some(2);
        assert!(c.shows_home(2, true));
        assert!(!c.shows_home(3, true));
        assert!(c.home.is_none(), "leaving Home by opening a document dismisses it");
        c.home = Some(1);
        assert!(!c.shows_home(2, true));
        assert!(!c.shows_home(1, true), "closing back to the stored count must not re-open Home");
        assert!(c.home.is_none());
        // Auto-show off: no Home by itself, the Home button still opens it, and opening a document leaves it.
        let mut off = ChromeState::default();
        assert!(!off.shows_home(0, false));
        off.home = Some(0);
        assert!(off.shows_home(0, false));
        assert!(!off.shows_home(1, false));
    }

    #[test]
    fn the_status_item_is_saved_by_its_key() {
        let mut chrome = ChromeState::default();
        assert_eq!(status_item(&chrome), StatusItem::DocumentDimensions);
        for item in StatusItem::ALL {
            set_status_item(&mut chrome, item);
            assert_eq!(status_item(&chrome), item);
        }
        chrome.status_info = "not a key".into();
        assert_eq!(status_item(&chrome), StatusItem::DocumentDimensions);
    }

    #[test]
    fn marquee_styles_constrain_the_drag() {
        assert_eq!(marquee_end("normal", 1.0, 1.0, false, [10.0, 10.0], [40.0, 20.0]), [40.0, 20.0]);
        assert_eq!(marquee_end("normal", 1.0, 1.0, true, [10.0, 10.0], [40.0, 20.0]), [40.0, 40.0]);
        assert_eq!(marquee_end("fixedSize", 64.0, 32.0, false, [10.0, 10.0], [5.0, 50.0]), [-54.0, 42.0]);
        assert_eq!(marquee_end("fixedRatio", 2.0, 1.0, false, [0.0, 0.0], [10.0, 30.0]), [60.0, 30.0]);
    }

    #[test]
    fn crop_ratios_parse() {
        assert_eq!(crop_ratio("16:9", 1.0, 1.0), Some((16.0, 9.0)));
        assert_eq!(crop_ratio("original", 2400.0, 1500.0), Some((2400.0, 1500.0)));
        assert_eq!(crop_ratio("", 1.0, 1.0), None);
        let (w, h) = crop_ratio("1:1", 0.0, 0.0).unwrap();
        assert_eq!(marquee_end("fixedRatio", w, h, false, [0.0, 0.0], [50.0, 20.0]), [50.0, 50.0]);
    }
}
