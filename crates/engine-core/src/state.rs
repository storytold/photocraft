//! Session state the engine crates share: plain data held in [`Session`] fields, read by core's
//! pipeline or by more than one crate. The commands that use each type live in the module its
//! section names, which re-exports it.

use std::sync::{Arc, Mutex};

use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::vector::Path;
use photocraft_doc::{AlphaChannel, ColorIndicates, DocId, Document, LayerId, Pattern, SmartContentsId};
use photocraft_geom::Rect;
use photocraft_paint::StrokePoint;
use photocraft_paint::mixer::MixerState;
use photocraft_raster::Surface;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{EngineError, Result, Session};

// ---------------------------------------------------------------------------------------------
// File › Automate › Batch (`file_cmds`)

/// Input and successfully opened document identities of one Override Open scratch session.
pub(crate) struct BatchOpen {
    pub path: String,
    pub sources: Vec<photocraft_doc::DocId>,
    pub raw_ppi: f64,
    /// A recorded Open read a source that must not be replaced (Affinity, Krita, XCF, PDN).
    pub read_only: bool,
}

// ---------------------------------------------------------------------------------------------
// Clipboard (`edit_cmds`)

/// Pixels on the session clipboard, with where they came from (for Paste in Place).
#[derive(Clone, Debug, craft_ui_macros::PlainData)]
pub struct Clip {
    /// Original SVG snapshot, when available. The raster mirror belongs to the same source.
    pub svg: Option<std::sync::Arc<Vec<u8>>>,
    /// Native colour indices for a multi-component clip, in channel order. Singletons are grayscale.
    pub components: Option<Vec<usize>>,
    pub surface: Surface,
    pub bounds: Rect,
    /// The copied layer itself, when Copy took a whole layer that Paste should recreate rather
    /// than rasterize (see [`whole_layer`](crate::edit_cmds::whole_layer)): a scratch document in the source's colour holding it.
    /// `surface` still holds its pixels, for Paste Into, masks, channels and other apps.
    pub layers: Option<Box<Document>>,
    /// The source document's resolution (ppi): File › New from the clipboard takes it, as
    /// Photoshop's Clipboard preset does. None for an image from another app.
    pub resolution: Option<f32>,
    /// The document the pixels were copied from: Paste keeps them in place only there, and
    /// centres them in any other (#8945). None for an image from another app.
    pub source: Option<DocId>,
    /// The artboard the pixels were copied from (its id and rect at the time), when they came
    /// from one: Paste into another board of the same document keeps their place on the board
    /// ([`board_shift`](crate::edit_cmds::board_shift), #8788). None outside boards, across several, or for a copied board.
    pub board: Option<(LayerId, Rect)>,
}

// ---------------------------------------------------------------------------------------------
// Smart objects (`smart_cmds`)

/// An open Edit Contents document and the shared contents it updates when saved or closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SmartLink {
    pub child: DocId,
    pub parent: DocId,
    pub layer: LayerId,
    pub contents_id: SmartContentsId,
}

// ---------------------------------------------------------------------------------------------
// Quick Mask (`channel_cmds`)

/// Session-wide Quick Mask Options (double-click the Quick Mask button in Photoshop).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickMaskOptions {
    pub color: Color,
    pub opacity: f32,
    pub indicates: ColorIndicates,
}

impl Default for QuickMaskOptions {
    fn default() -> Self {
        Self { color: AlphaChannel::DEFAULT_COLOR, opacity: 0.5, indicates: ColorIndicates::MaskedAreas }
    }
}

// ---------------------------------------------------------------------------------------------
// Edit menu (`edit_menu_cmds`)

/// What Edit › Fade would fade: the result of the last filter, adjustment or paint command.
#[derive(Clone, Debug, PartialEq)]
pub struct FadeSource {
    pub doc: DocId,
    /// Document revision right after the command, or after a later step that only showed or hid
    /// other layers (#13805); any other change disables Fade.
    pub revision: u64,
    /// History id of the state before the command, which Fade blends back towards. Steps that only
    /// showed or hid other layers can sit between that state and the current one.
    pub before: Option<u64>,
    pub target: FadeTarget,
    pub label: String,
}

/// What the faded command changed, and so what Fade blends back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FadeTarget {
    /// A layer's pixels.
    Pixels(LayerId),
    /// A layer's layer mask.
    Mask(LayerId),
    /// An alpha or spot channel (index into [`Document::channels`]).
    Channel(usize),
    /// The Quick Mask.
    QuickMask,
}

/// A custom shape (Edit › Define Custom Shape), usable by the Custom Shape tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomShape {
    pub name: String,
    pub path: Path,
}

/// Remembered Find and Replace Text match (or collapsed cursor after a replacement) and the
/// search query it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindCursor {
    pub layer: LayerId,
    pub start: usize,
    pub end: usize,
    pub find: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub forward: bool,
    pub all_layers: bool,
}

/// Edit-menu state kept by the session.
#[derive(Clone, Debug, Default)]
pub struct EditState {
    pub fade: Option<FadeSource>,
    /// Custom shape library (the vector tools read it).
    pub custom_shapes: Vec<CustomShape>,
    /// Find and Replace Text position and the query it belongs to.
    pub find_cursor: Option<FindCursor>,
    /// First layer visited in the current Find Next round (so wrapping stops before it).
    pub find_start: Option<LayerId>,
}

// ---------------------------------------------------------------------------------------------
// Image › Analysis (`analysis_cmds`)

const COMMON: &[&str] = &["label", "dateTime", "document", "source", "scale", "scaleUnits", "scaleFactor"];
const SELECTION_ONLY: &[&str] =
    &["count", "area", "perimeter", "circularity", "height", "width", "grayMin", "grayMax", "grayMean", "grayMedian", "integratedDensity", "histogram"];
const RULER_ONLY: &[&str] = &["length", "angle"];
const COUNT_ONLY: &[&str] = &["count"];

/// Data points a measurement source can record.
pub fn available(source: &str) -> Vec<&'static str> {
    let extra = match source {
        "selection" => SELECTION_ONLY,
        "ruler" => RULER_ONLY,
        _ => COUNT_ONLY,
    };
    COMMON.iter().chain(extra).copied().collect()
}

/// Image › Analysis › Select Data Points: what Record Measurements writes per source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataPoints {
    pub selection: Vec<String>,
    pub ruler: Vec<String>,
    pub count: Vec<String>,
}

impl Default for DataPoints {
    fn default() -> Self {
        let all = |s| available(s).into_iter().map(String::from).collect();
        Self { selection: all("selection"), ruler: all("ruler"), count: all("count") }
    }
}

impl DataPoints {
    pub(crate) fn of(&self, source: &str) -> &[String] {
        match source {
            "selection" => &self.selection,
            "ruler" => &self.ruler,
            _ => &self.count,
        }
    }
    pub(crate) fn of_mut(&mut self, source: &str) -> &mut Vec<String> {
        match source {
            "selection" => &mut self.selection,
            "ruler" => &mut self.ruler,
            _ => &mut self.count,
        }
    }
}

/// One Measurement Log row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogRow {
    pub id: u64,
    /// Data point values by key (see [`COLUMNS`](crate::analysis_cmds::COLUMNS)).
    pub values: Map<String, Value>,
}

/// Session-wide analysis state (`Session::analysis`).
#[derive(Clone, Debug, Default)]
pub struct AnalysisState {
    pub log: Vec<LogRow>,
    pub(crate) next_row: u64,
    /// Counter for "Measurement N" labels.
    pub(crate) next_measurement: u64,
    pub data_points: DataPoints,
}

// ---------------------------------------------------------------------------------------------
// File menu (`automate_cmds`)

/// Session state of the File-menu commands (not saved, not history).
#[derive(Clone, Debug, Default)]
pub struct FileMenuState {
    /// View › Lock Slices.
    pub slices_locked: bool,
    /// Documents with File › Generate › Image Assets on.
    pub image_assets: craft_tracked::Tracked<Vec<DocId>>,
    /// File › Print settings used last (Print One Copy repeats them).
    pub last_print: Option<Value>,
    /// Save for Web settings used last.
    pub last_web: Option<Value>,
    /// Results of the most recent script event (for inspection), newest last, capped.
    pub event_log: Vec<Value>,
    /// Set while event scripts run, so their commands don't fire events recursively.
    pub firing: bool,
}

// ---------------------------------------------------------------------------------------------
// Colour channel restriction (`color_channels`)

/// A bounded, cheap-to-copy set of native colour components (never transparency).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorChannels(u64);

/// The colour channels the running command may change, on one layer.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Restriction {
    pub colors: ColorChannels,
    pub layer: photocraft_doc::LayerId,
}

impl ColorChannels {
    pub fn contains(self, index: usize) -> bool {
        index < 64 && self.0 & (1u64.checked_shl(index as u32).unwrap_or(0)) != 0
    }
    pub fn indices(self) -> impl Iterator<Item = usize> {
        (0..64).filter(move |i| self.contains(*i))
    }
    pub fn len(self) -> usize {
        self.0.count_ones() as usize
    }
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
    pub fn from_indices(indices: impl IntoIterator<Item = usize>, count: usize) -> Result<Self> {
        let mut bits = 0;
        for i in indices {
            if i >= count || i >= 64 {
                return Err(EngineError::Other("colour channel index is out of range".into()));
            }
            bits |= 1u64.checked_shl(i as u32).unwrap_or(0);
        }
        if bits == 0 {
            return Err(EngineError::Other("select at least one colour channel".into()));
        }
        Ok(Self(bits))
    }
    pub fn params(self) -> Value {
        json!({"colors": self.indices().collect::<Vec<_>>()})
    }
}

// ---------------------------------------------------------------------------------------------
// Live strokes (`brush_cmds`)

/// A finished live stroke offered to its own commit ([`Session::offer_live_stroke`],
/// [`Session::offer_live_retouch`]): when the committing command matches it (same command,
/// document revision, params, seed and points), it takes these pixels instead of rendering the
/// stroke again.
pub struct LiveCommit {
    /// Mixer load before and after the stroke; transient, never serialized into the journal.
    pub(crate) mixer: Option<(MixerState, MixerState)>,
    pub(crate) cmd: String,
    pub(crate) base: (photocraft_doc::DocId, u64),
    pub(crate) params: Value,
    /// The Brush's and Pencil's seed; the retouching tools have none.
    pub(crate) seed: Option<u64>,
    pub(crate) points: Vec<StrokePoint>,
    pub(crate) surface: Surface,
    pub(crate) bounds: Rect,
}

impl LiveCommit {
    /// The params that decide the pixels, without the points and seed (compared separately).
    fn settings(p: &Value) -> Value {
        let mut p = p.clone();
        if let Some(o) = p.as_object_mut() {
            o.remove("points");
            o.remove("seed");
        }
        p
    }

    pub(crate) fn matches(&self, s: &Session, cmd: &str, p: &Value, pts: &[StrokePoint]) -> bool {
        let base = s.active().map(|st| (st.doc.id, st.revision));
        self.cmd == cmd
            && base == Some(self.base)
            && p.get("seed").and_then(Value::as_u64) == self.seed
            && self.points == pts
            && Self::settings(p) == Self::settings(&self.params)
    }
}

// ---------------------------------------------------------------------------------------------
// Pattern library (`pattern_cmds`)

/// The app-level pattern library (Window › Patterns).
#[derive(Clone, Debug)]
pub struct PatternLibrary {
    pub items: Vec<Pattern>,
    /// Bumped when a pattern's pixels are replaced under the same id (an imported `.pat` whose
    /// id is already in the library), which its id, name and size don't show.
    pub pixels_rev: u64,
    /// Saved patterns held as a small preview until used, by id: their full size. The preset
    /// store reads the pixels ([`full`](crate::pattern_cmds::full)); everything that paints goes through
    /// [`resolve`](crate::pattern_cmds::resolve).
    pub stored: std::collections::HashMap<String, (u32, u32)>,
}

impl Default for PatternLibrary {
    fn default() -> Self {
        PatternLibrary { items: builtin(), pixels_rev: 0, stored: Default::default() }
    }
}

impl PatternLibrary {
    /// Add `pat`, or replace the pattern with its id (marking the pixels changed).
    pub fn upsert(&mut self, pat: Pattern) {
        self.stored.remove(&pat.id);
        match self.items.iter_mut().find(|q| q.id == pat.id) {
            Some(q) => {
                *q = pat;
                self.pixels_rev += 1;
            }
            None => self.items.push(pat),
        }
    }
}

fn hash01(x: u32, y: u32, seed: u32) -> f32 {
    let mut h = x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65535.0
}

fn procedural(name: &str, w: u32, h: u32, f: impl Fn(u32, u32) -> [f32; 3]) -> Pattern {
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, false);
    let mut s = Surface::new(fmt);
    let mut px = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(&f(x, y));
        }
    }
    // Quantise through the 8-bit surface so ids are stable.
    s.write_region(Rect::new(0, 0, w as i32, h as i32), &px);
    Pattern::new(name, s, w, h)
}

/// Built-in patterns: generated procedurally here (our own, CC0), so the library is never empty.
pub fn builtin() -> Vec<Pattern> {
    let g = |v: f32| [v, v, v];
    vec![
        procedural("Checkerboard", 16, 16, |x, y| g(if (x / 8 + y / 8) % 2 == 0 { 0.8 } else { 1.0 })),
        procedural("Diagonal Lines", 8, 8, |x, y| g(if (x + y) % 8 < 2 { 0.2 } else { 1.0 })),
        procedural("Dots", 12, 12, |x, y| {
            let (dx, dy) = (x as f32 + 0.5 - 6.0, y as f32 + 0.5 - 6.0);
            g(((dx * dx + dy * dy).sqrt() - 3.0).clamp(0.0, 1.0))
        }),
        procedural("Grid", 16, 16, |x, y| g(if x == 0 || y == 0 { 0.55 } else { 1.0 })),
        procedural("Bricks", 32, 16, |x, y| {
            let row = y / 8;
            let xo = (x + if row % 2 == 1 { 8 } else { 0 }) % 16;
            let mortar = y % 8 == 7 || xo == 15;
            let n = hash01(x, y, 7) * 0.08;
            if mortar { [0.78 + n, 0.76 + n, 0.72 + n] } else { [0.62 + n, 0.27 + n * 0.5, 0.2] }
        }),
        procedural("Weave", 16, 16, |x, y| {
            let over = ((x / 4) + (y / 4)) % 2 == 0;
            let t = if over { (y % 4) as f32 / 3.0 } else { (x % 4) as f32 / 3.0 };
            let v = 0.55 + 0.35 * (1.0 - (2.0 * t - 1.0).abs());
            [v, v * 0.92, v * 0.8]
        }),
        procedural("Paper Noise", 64, 64, |x, y| {
            // Two octaves of smoothed value noise (wraps at 64 px).
            let oct = |cell: u32, seed: u32| {
                let (cx, cy) = (x / cell, y / cell);
                let (fx, fy) = ((x % cell) as f32 / cell as f32, (y % cell) as f32 / cell as f32);
                let n = 64 / cell;
                let v = |i: u32, j: u32| hash01(i % n, j % n, seed);
                let s = |t: f32| t * t * (3.0 - 2.0 * t);
                let (sx, sy) = (s(fx), s(fy));
                let a = v(cx, cy) + (v(cx + 1, cy) - v(cx, cy)) * sx;
                let b = v(cx, cy + 1) + (v(cx + 1, cy + 1) - v(cx, cy + 1)) * sx;
                a + (b - a) * sy
            };
            g(0.78 + 0.14 * oct(8, 1) + 0.08 * oct(2, 2))
        }),
    ]
}

// ---------------------------------------------------------------------------------------------
// Linked smart object files (`link_watch`)

/// A file's size and modification time (nanoseconds since 1970).
pub type Stamp = (u64, u128);

/// Stamps of a document's linked files taken on a worker thread: (document id, [(stored path,
/// the file it resolved to, layer ids, stamp)]).
pub type LinkScan = Arc<Mutex<Option<(u64, Vec<(String, Option<std::path::PathBuf>, Vec<u64>, Option<Stamp>)>)>>>;
