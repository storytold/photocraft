//! File › Export › Save for Web (Legacy), File › Export › Export Preferences, the Quick Export
//! pipeline that honours them, and File › Generate › Image Assets (Photoshop's Generator).
//!
//! Save for Web optimises the flattened (optionally resized, sRGB-converted) image as GIF,
//! PNG-8, PNG-24, JPEG or WBMP, per slice, into a folder with an optional HTML table; without an
//! output it only reports the optimised size (the dialog's 2-Up/4-Up annotations). Colour
//! reduction and dithering reuse `algo::palette_map`; GIF/WBMP/progressive JPEG writers live in
//! `codecs::web`.
//!
//! Image Assets implements the naming grammar of Adobe's public Generator documentation: a
//! layer or group named `foo.png`, `200% foo@2x.png, 48x48 icons/foo.png8`, `photo.jpg80%`,
//! `?x100 thumb.gif` is exported into `<document>-assets/` whenever the document is saved while
//! the feature is on; a `default` layer (`default 50% low/ + 200% @2x`) adds variants.

use photocraft_algo::palette_map::{AlphaMode, Method, Palette, Settings};
use photocraft_algo::quantize::{self, Dither, Forced, PaletteKind};
use photocraft_doc::slices::{self, ResolvedSlice, SliceKind};
use photocraft_doc::{DocId, Document, LayerContent, LayerId};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::file_cmds::{is_windows_device_name, join, native_doc, portable_device_name, stem, write_file};
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}
fn other(e: impl std::fmt::Display) -> EngineError {
    EngineError::Other(e.to_string())
}

// ---------- settings ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebFormat {
    Gif,
    Png8,
    Png24,
    Jpeg,
    Wbmp,
}

impl WebFormat {
    pub fn from_id(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "gif" => WebFormat::Gif,
            "png8" | "png-8" => WebFormat::Png8,
            "png24" | "png-24" | "png" | "png32" => WebFormat::Png24,
            "jpeg" | "jpg" => WebFormat::Jpeg,
            "wbmp" => WebFormat::Wbmp,
            _ => return None,
        })
    }
    pub fn ext(self) -> &'static str {
        match self {
            WebFormat::Gif => "gif",
            WebFormat::Png8 | WebFormat::Png24 => "png",
            WebFormat::Jpeg => "jpg",
            WebFormat::Wbmp => "wbmp",
        }
    }
    pub fn indexed(self) -> bool {
        matches!(self, WebFormat::Gif | WebFormat::Png8)
    }
}

/// Which metadata Save for Web embeds (JPEG only, like Photoshop's legacy exporter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebMetadata {
    None,
    Copyright,
    CopyrightAndContact,
    /// All the XMP, none of the photo's EXIF (camera, date taken, GPS).
    AllExceptCameraInfo,
    /// All the XMP and the photo's EXIF.
    All,
}

/// One optimisation preset (the right-hand side of the Save for Web dialog).
#[derive(Clone, Debug, PartialEq)]
pub struct WebSettings {
    pub format: WebFormat,
    pub palette: PaletteKind,
    pub colors: usize,
    /// Fixed opaque source palette in index order.
    pub color_table: Option<Vec<[u8; 3]>>,
    /// Replacements applied after quantization, preserving pixel indices.
    pub color_table_edits: std::collections::BTreeMap<usize, [u8; 3]>,
    /// An explicit, embedded sRGB palette, independent of the installed catalog.
    pub palette_snapshot: Option<Palette>,
    pub palette_id: Option<String>,
    /// Shared renderer settings. When absent, the basic dither fields below apply.
    pub mapping: Option<Settings>,
    /// Deliberately reduce an oversized fixed palette to the indexed-format capacity.
    pub reduce_palette: bool,
    pub dither: Dither,
    /// 0..=1.
    pub dither_amount: f32,
    pub transparency: bool,
    /// Edge / background colour (0..=1 RGB); None = no matte (hard alpha threshold).
    pub matte: Option<[f32; 3]>,
    pub interlaced: bool,
    /// Web Snap: palette entries within this distance (0..=1 of the channel range) snap to the
    /// web-safe cube.
    pub web_snap: f32,
    /// JPEG quality 0..=100.
    pub quality: u8,
    pub progressive: bool,
    pub optimized: bool,
    pub embed_icc: bool,
    pub metadata: WebMetadata,
    pub convert_to_srgb: bool,
}

impl Default for WebSettings {
    fn default() -> Self {
        WebSettings {
            format: WebFormat::Png24,
            palette: PaletteKind::Selective,
            colors: 256,
            color_table: None,
            color_table_edits: Default::default(),
            palette_snapshot: None,
            palette_id: None,
            mapping: None,
            reduce_palette: false,
            dither: Dither::Diffusion,
            dither_amount: 0.88,
            transparency: true,
            matte: Some([1.0, 1.0, 1.0]),
            interlaced: false,
            web_snap: 0.0,
            quality: 60,
            progressive: false,
            optimized: true,
            embed_icc: false,
            metadata: WebMetadata::Copyright,
            convert_to_srgb: true,
        }
    }
}

fn hex3(s: &str) -> Option<[f32; 3]> {
    let s = s.trim_start_matches('#');
    let b = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok().map(|v| f32::from(v) / 255.0);
    (s.len() == 6).then_some(())?;
    Some([b(0)?, b(2)?, b(4)?])
}

impl WebSettings {
    /// Settings from command parameters (missing keys keep the defaults; Photoshop's preset
    /// names such as "GIF 128 Dithered" or "JPEG High" are accepted as `"preset"`).
    ///
    /// Without a preset there is no matte, as in Photoshop's scripted Save for Web
    /// (`ExportOptionsSaveForWeb` left at its defaults): a GIF or PNG-8 keeps the colour of its
    /// partly transparent pixels instead of lightening them towards white (#4115). Formats
    /// without transparency still go over white.
    pub fn from_params(p: &Value, cmd: &str) -> Result<Self> {
        let mut st = WebSettings { matte: None, ..WebSettings::default() };
        if let Some(preset) = p.get("preset").and_then(Value::as_str) {
            st = preset_settings(preset).ok_or_else(|| bad(cmd, format!("unknown preset `{preset}`")))?;
        }
        let s = |k: &str| p.get(k).and_then(Value::as_str);
        if let Some(f) = s("format") {
            st.format = WebFormat::from_id(f).ok_or_else(|| bad(cmd, format!("unknown format `{f}` (gif|png8|png24|jpeg|wbmp)")))?;
        }
        if let Some(k) = s("palette") {
            st.palette = match k {
                "restrictive" | "web" => PaletteKind::Web,
                k => PaletteKind::from_id(k).ok_or_else(|| {
                    bad(cmd, format!("unknown palette `{k}` (blackWhite|perceptual|selective|adaptive|restrictive|exact|systemMac|systemWindows|uniform)"))
                })?,
            };
        }
        if let Some(n) = crate::commands::int(p, "colors").filter(|v| *v > 0).map(|v| v as u64) {
            st.colors = (n as usize).clamp(2, 256);
        }
        if let Some(d) = s("dither") {
            st.dither = match d {
                "none" | "noDither" => Dither::None,
                "diffusion" => Dither::Diffusion,
                "pattern" => Dither::Pattern,
                "noise" => Dither::Noise,
                d if Method::from_id(d).is_some() => Dither::Diffusion,
                d => return Err(bad(cmd, format!("unknown dither `{d}`"))),
            };
        }
        if let Some(a) = p.get("ditherAmount").and_then(Value::as_f64) {
            st.dither_amount = (a as f32 / 100.0).clamp(0.0, 1.0);
        }
        if p.get("mapping").is_some() || s("dither").is_some_and(|d| !matches!(d, "none" | "noDither" | "diffusion" | "pattern" | "noise")) {
            st.mapping = Some(crate::palette_cmds::mapping_settings(p, "ditherAmount").map_err(|e| match e {
                EngineError::BadParams { msg, .. } => bad(cmd, msg),
                other => other,
            })?);
        }
        st.palette_snapshot = crate::palette_cmds::resolve_palette(p).map_err(|e| match e {
            EngineError::BadParams { msg, .. } => bad(cmd, msg),
            other => other,
        })?;
        if let Some(id) = p.get("paletteId") {
            st.palette_id = Some(id.as_str().ok_or_else(|| bad(cmd, "paletteId must be a string"))?.to_string());
        }
        if let Some(reduce) = p.get("reducePalette") {
            st.reduce_palette = reduce.as_bool().ok_or_else(|| bad(cmd, "reducePalette must be a boolean"))?;
        }
        if let Some(b) = p.get("transparency").and_then(Value::as_bool) {
            st.transparency = b;
        }
        match s("matte") {
            Some("none") => st.matte = None,
            Some(c) => st.matte = Some(hex3(c).ok_or_else(|| bad(cmd, "matte is \"none\" or \"#rrggbb\""))?),
            None => {}
        }
        for (k, slot) in [
            ("interlaced", &mut st.interlaced),
            ("progressive", &mut st.progressive),
            ("optimized", &mut st.optimized),
            ("embedIcc", &mut st.embed_icc),
            ("convertToSrgb", &mut st.convert_to_srgb),
        ] {
            if let Some(b) = p.get(k).and_then(Value::as_bool) {
                *slot = b;
            }
        }
        if let Some(w) = p.get("webSnap").and_then(Value::as_f64) {
            st.web_snap = (w as f32 / 100.0).clamp(0.0, 1.0);
        }
        if let Some(q) = p.get("quality").and_then(Value::as_f64) {
            st.quality = q.clamp(0.0, 100.0).round() as u8;
        }
        if let Some(m) = s("metadata") {
            st.metadata = match m {
                "none" => WebMetadata::None,
                "copyright" => WebMetadata::Copyright,
                "copyrightAndContact" | "contact" => WebMetadata::CopyrightAndContact,
                "allExceptCameraInfo" => WebMetadata::AllExceptCameraInfo,
                "all" => WebMetadata::All,
                m => return Err(bad(cmd, format!("unknown metadata `{m}` (none|copyright|copyrightAndContact|allExceptCameraInfo|all)"))),
            };
        }
        if let Some(table) = p.get("colorTable") {
            let table = table.as_array().filter(|a| (1..=256).contains(&a.len())).ok_or_else(|| bad(cmd, "colorTable must contain 1–256 #rrggbb colours"))?;
            st.color_table = Some(
                table
                    .iter()
                    .map(|v| v.as_str().and_then(hex3).map(|c| c.map(to8)).ok_or_else(|| bad(cmd, "colorTable entries must be #rrggbb colours")))
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        if let Some(edits) = p.get("colorTableEdits") {
            let edits = edits
                .as_object()
                .filter(|m| m.len() <= 256)
                .ok_or_else(|| bad(cmd, "colorTableEdits must be an object of palette indices to #rrggbb colours"))?;
            let table = st.color_table.as_ref().ok_or_else(|| bad(cmd, "colorTableEdits requires colorTable"))?;
            for (index, value) in edits {
                let index = index.parse::<usize>().ok().filter(|i| *i < table.len()).ok_or_else(|| bad(cmd, "colorTableEdits index is outside colorTable"))?;
                let color = value.as_str().and_then(hex3).map(|c| c.map(to8)).ok_or_else(|| bad(cmd, "colorTableEdits values must be #rrggbb colours"))?;
                st.color_table_edits.insert(index, color);
            }
        }
        Ok(st)
    }
}

impl WebSettings {
    /// The settings as command parameters (the inverse of [`WebSettings::from_params`]).
    pub fn to_params(&self) -> Value {
        let format = match self.format {
            WebFormat::Gif => "gif",
            WebFormat::Png8 => "png8",
            WebFormat::Png24 => "png24",
            WebFormat::Jpeg => "jpeg",
            WebFormat::Wbmp => "wbmp",
        };
        let palette = match self.palette {
            PaletteKind::BlackWhite => "blackWhite",
            PaletteKind::Exact => "exact",
            PaletteKind::SystemMac => "systemMac",
            PaletteKind::SystemWindows => "systemWindows",
            PaletteKind::Web => "restrictive",
            PaletteKind::Uniform => "uniform",
            PaletteKind::Perceptual => "perceptual",
            PaletteKind::Selective => "selective",
            PaletteKind::Adaptive => "adaptive",
        };
        let dither = match self.dither {
            Dither::None => "none",
            Dither::Diffusion => "diffusion",
            Dither::Pattern => "pattern",
            Dither::Noise => "noise",
        };
        let metadata = match self.metadata {
            WebMetadata::None => "none",
            WebMetadata::Copyright => "copyright",
            WebMetadata::CopyrightAndContact => "copyrightAndContact",
            WebMetadata::AllExceptCameraInfo => "allExceptCameraInfo",
            WebMetadata::All => "all",
        };
        let matte = self.matte.map_or("none".to_string(), |c| format!("#{:02x}{:02x}{:02x}", to8(c[0]), to8(c[1]), to8(c[2])));
        let mut p = json!({"format": format, "palette": palette, "colors": self.colors, "dither": dither, "ditherAmount": (self.dither_amount * 100.0).round(), "transparency": self.transparency, "matte": matte, "interlaced": self.interlaced, "webSnap": (self.web_snap * 100.0).round(), "quality": self.quality, "progressive": self.progressive, "optimized": self.optimized, "embedIcc": self.embed_icc, "metadata": metadata, "convertToSrgb": self.convert_to_srgb});
        if let Some(table) = &self.color_table {
            p["colorTable"] = json!(table.iter().map(|c| rgb_hex(*c)).collect::<Vec<_>>());
        }
        if !self.color_table_edits.is_empty() {
            p["colorTableEdits"] =
                json!(self.color_table_edits.iter().map(|(i, c)| (i.to_string(), rgb_hex(*c))).collect::<std::collections::BTreeMap<_, _>>());
        }
        if let Some(palette) = &self.palette_snapshot {
            p["paletteSnapshot"] = json!(palette);
        }
        if let Some(id) = &self.palette_id {
            p["paletteId"] = json!(id);
        }
        if let Some(mapping) = &self.mapping {
            p["mapping"] = json!(mapping);
        }
        if self.reduce_palette {
            p["reducePalette"] = json!(true);
        }
        p
    }

    fn mapping_settings(&self) -> Settings {
        self.mapping.clone().unwrap_or_else(|| Settings {
            method: match self.dither {
                Dither::None => Method::None,
                Dither::Diffusion => Method::FloydSteinberg,
                Dither::Pattern => Method::Bayer,
                Dither::Noise => Method::WhiteNoise,
            },
            amount: self.dither_amount,
            ..Settings::default()
        })
    }
}

/// A palette colour in command/control-channel notation.
pub fn rgb_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Save for Web's named presets.
pub const PRESETS: [&str; 12] = [
    "GIF 128 Dithered",
    "GIF 128 No Dither",
    "GIF 32 Dithered",
    "GIF 32 No Dither",
    "GIF 64 Dithered",
    "GIF 64 No Dither",
    "GIF Restrictive",
    "JPEG High",
    "JPEG Low",
    "JPEG Medium",
    "PNG-24",
    "PNG-8 128 Dithered",
];

pub fn preset_settings(name: &str) -> Option<WebSettings> {
    let d = WebSettings::default();
    let gif = |colors: usize, dither: bool| WebSettings {
        format: WebFormat::Gif,
        colors,
        dither: if dither { Dither::Diffusion } else { Dither::None },
        ..WebSettings::default()
    };
    Some(match name {
        "GIF 128 Dithered" => gif(128, true),
        "GIF 128 No Dither" => gif(128, false),
        "GIF 32 Dithered" => gif(32, true),
        "GIF 32 No Dither" => gif(32, false),
        "GIF 64 Dithered" => gif(64, true),
        "GIF 64 No Dither" => gif(64, false),
        "GIF Restrictive" => WebSettings { palette: PaletteKind::Web, ..gif(256, true) },
        "JPEG High" => WebSettings { format: WebFormat::Jpeg, quality: 60, ..d },
        "JPEG Low" => WebSettings { format: WebFormat::Jpeg, quality: 10, ..d },
        "JPEG Medium" => WebSettings { format: WebFormat::Jpeg, quality: 30, ..d },
        "JPEG Maximum" => WebSettings { format: WebFormat::Jpeg, quality: 100, ..d },
        "JPEG Very High" => WebSettings { format: WebFormat::Jpeg, quality: 80, ..d },
        "PNG-24" => WebSettings { format: WebFormat::Png24, ..d },
        "PNG-8 128 Dithered" => WebSettings { format: WebFormat::Png8, colors: 128, ..d },
        _ => return None,
    })
}

// ---------- the image ----------

/// The document as Save for Web sees it: RGB (converted to sRGB unless told otherwise), resized
/// by `"width"`/`"height"`/`"percent"`, flattened. Returns the document and the scale factors.
pub fn web_document(doc: &Document, p: &Value, st: &WebSettings) -> Result<(Document, f64, f64)> {
    let mut tmp = Session::new();
    tmp.add_document(doc.clone(), None);
    let mode = doc.mode;
    if mode != photocraft_color::ColorMode::Rgb {
        tmp.execute("image.mode.rgb", json!({}))?;
    }
    if st.convert_to_srgb && doc.icc_profile.is_some() {
        // Documents without a profile are treated as sRGB already.
        tmp.execute("edit.convertToProfile", json!({"profile": "srgb", "intent": "perceptual"}))?;
    }
    let (w0, h0) = (f64::from(doc.size.width), f64::from(doc.size.height));
    let pct = p.get("percent").and_then(Value::as_f64).filter(|v| *v > 0.0);
    let w = p.get("width").and_then(Value::as_f64).filter(|v| *v > 0.0);
    let h = p.get("height").and_then(Value::as_f64).filter(|v| *v > 0.0);
    let (nw, nh) = match (pct, w, h) {
        (Some(pc), _, _) => (w0 * pc / 100.0, h0 * pc / 100.0),
        (_, Some(w), Some(h)) => (w, h),
        (_, Some(w), None) => (w, h0 * w / w0),
        (_, None, Some(h)) => (w0 * h / h0, h),
        _ => (w0, h0),
    };
    let (nw, nh) = (nw.round().max(1.0), nh.round().max(1.0));
    if (nw - w0).abs() >= 1.0 || (nh - h0).abs() >= 1.0 {
        let resample = p.get("resample").and_then(Value::as_str).unwrap_or("bicubic");
        tmp.execute("image.imageSize", json!({"width": nw, "height": nh, "resample": resample}))?;
    }
    let out = (*tmp.active().ok_or(EngineError::NoDocument)?.doc).clone();
    Ok((out, nw / w0, nh / h0))
}

/// An optimised image.
#[derive(Clone, Debug)]
pub struct Optimized {
    pub bytes: Vec<u8>,
    pub ext: &'static str,
    pub width: u32,
    pub height: u32,
    /// Colours in the palette (indexed formats).
    pub colors: Option<usize>,
    /// Opaque source palette before replacements (empty for non-indexed formats).
    pub color_table: Vec<[u8; 3]>,
    pub transparent_index: Option<usize>,
    /// Indexed pixels for previews; empty for non-indexed formats or when preview is disabled.
    pub indices: Vec<u8>,
    /// What the optimised file looks like, straight RGBA8 (for the dialog's previews).
    pub preview: Vec<u8>,
}

fn to8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// XMP to embed for the metadata choice.
pub fn web_xmp(doc: &Document, m: WebMetadata) -> Option<String> {
    let xmp = doc.metadata.xmp.as_deref()?;
    match m {
        WebMetadata::None => None,
        WebMetadata::All | WebMetadata::AllExceptCameraInfo => Some(xmp.to_string()),
        WebMetadata::Copyright | WebMetadata::CopyrightAndContact => {
            let info = crate::file_cmds::read_file_info(Some(xmp));
            let mut keep = serde_json::Map::new();
            let mut keys = vec!["copyright", "copyrightStatus", "copyrightUrl"];
            if m == WebMetadata::CopyrightAndContact {
                keys.extend(["author", "authorTitle"]);
            }
            for k in keys {
                if let Some(v) = info.get(k).filter(|v| v.as_str().is_some_and(|s| !s.is_empty()) || v.is_array()) {
                    keep.insert(k.into(), v.clone());
                }
            }
            (!keep.is_empty()).then(|| crate::file_cmds::write_file_info(None, &Value::Object(keep)))
        }
    }
}

/// The photo's EXIF to embed for the metadata choice: only "All" carries it (#7436).
pub fn web_exif(doc: &Document, m: WebMetadata) -> Option<&[u8]> {
    doc.metadata.exif.as_deref().map(Vec::as_slice).filter(|_| m == WebMetadata::All)
}

/// The response's warnings: EXIF that a JPEG couldn't hold (one segment, about 64 KB).
fn web_warnings(st: &WebSettings, exif: Option<&[u8]>, dpi: f32) -> Vec<String> {
    exif.filter(|x| st.format == WebFormat::Jpeg && !photocraft_codecs::web::jpeg_exif_fits(x, Some(dpi)))
        .map(|x| photocraft_codecs::FidelityWarning::MetadataTooLarge { what: "EXIF", bytes: x.len() }.to_string())
        .into_iter()
        .collect()
}

/// Optimises `rect` of `px` (straight RGBA, `bw` wide) with `st`. `preview` also returns what
/// the file looks like (the dialog's panes); exports skip it.
#[allow(clippy::too_many_arguments)]
pub fn optimize(
    px: &[[f32; 4]],
    bw: usize,
    rect: Rect,
    st: &WebSettings,
    icc: Option<&[u8]>,
    exif: Option<&[u8]>,
    xmp: Option<&str>,
    dpi: f32,
    preview: bool,
) -> Result<Optimized> {
    if bw == 0 || rect.x0 < 0 || rect.y0 < 0 || rect.is_empty() || rect.x1 as usize > bw || (rect.y1 as usize).checked_mul(bw).is_none_or(|end| end > px.len())
    {
        return Err(other("slice is outside the pixel buffer"));
    }
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    let row = |y: i32| {
        let o = y as usize * bw;
        px.get(o + rect.x0 as usize..o + rect.x1 as usize).unwrap_or(&[])
    };
    let region = || (rect.y0..rect.y1).flat_map(row);
    let matte = st.matte.unwrap_or([1.0, 1.0, 1.0]);
    let over = |p: &[f32; 4]| -> [f32; 4] {
        [p[0] * p[3] + matte[0] * (1.0 - p[3]), p[1] * p[3] + matte[1] * (1.0 - p[3]), p[2] * p[3] + matte[2] * (1.0 - p[3]), 1.0]
    };
    let (ww, hh) = (w as u32, h as u32);
    let e = |e: photocraft_codecs::CodecError| other(e);
    let mut look: Vec<u8> = Vec::new();
    let mut color_table = Vec::new();
    let mut transparent_index = None;
    let mut indices = Vec::new();
    let (bytes, colors) = match st.format {
        WebFormat::Png24 => {
            let alpha = st.transparency && region().any(|p| p[3] < 1.0);
            let k = if alpha { 4 } else { 3 };
            let mut data = Vec::with_capacity(w * h * k);
            for p in region() {
                let q = if alpha { *p } else { over(p) };
                data.extend_from_slice(&[to8(q[0]), to8(q[1]), to8(q[2]), to8(q[3])][..k]);
            }
            if preview {
                look = if alpha { data.clone() } else { data.as_chunks::<3>().0.iter().flat_map(|c| [c[0], c[1], c[2], 255]).collect() };
            }
            let layout = if alpha { photocraft_codecs::ChannelLayout::Rgba } else { photocraft_codecs::ChannelLayout::Rgb };
            let meta = photocraft_codecs::Metadata { dpi: Some((dpi, dpi)), xmp: xmp.map(str::to_string), ..Default::default() };
            let img = photocraft_codecs::Image::from_u8(ww, hh, layout, data).map_err(e)?.with_icc(icc.map(<[u8]>::to_vec)).with_meta(meta);
            let opts = photocraft_codecs::EncodeOptions {
                png_interlaced: st.interlaced,
                embed_icc: icc.is_some(),
                embed_metadata: xmp.is_some(),
                ..Default::default()
            };
            (photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &opts).map_err(e)?, None)
        }
        WebFormat::Jpeg => {
            let mut rgb = Vec::with_capacity(w * h * 3);
            for p in region() {
                let q = over(p);
                rgb.extend_from_slice(&[to8(q[0]), to8(q[1]), to8(q[2])]);
            }
            // Save for Web's 0–100 quality → encoder 1–100.
            let q = st.quality.max(1);
            let bytes =
                photocraft_codecs::web::encode_jpeg_rgb8(ww, hh, &rgb, q, st.progressive, st.optimized, icc.filter(|_| st.embed_icc), Some(dpi), exif, xmp)
                    .map_err(e)?;
            if preview {
                // The preview shows the compression artefacts.
                look = match photocraft_codecs::decode(&bytes) {
                    Ok(img) => img.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8).data().to_vec(),
                    Err(_) => rgb.as_chunks::<3>().0.iter().flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
                };
            }
            (bytes, None)
        }
        WebFormat::Wbmp => {
            let mut work: Vec<[f32; 4]> = region().map(over).collect();
            let profile = indexed_profile(icc)?;
            convert_working_rgb(&mut work, profile.as_ref())?;
            for p in &mut work {
                let gray = 0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2];
                *p = [gray, gray, gray, 1.0];
            }
            let palette = Palette { name: "Black and white".into(), colors: vec![[0.0; 3], [1.0; 3]] };
            let mut mapping = st.mapping_settings();
            mapping.alpha = AlphaMode::Binary;
            let mapped = photocraft_algo::palette_map::render(&mut work, w, &palette, &mapping, [rect.x0, rect.y0], None).map_err(other)?;
            let white: Vec<bool> = mapped.iter().map(|index| *index == 1).collect();
            if preview {
                look = white.iter().flat_map(|b| if *b { [255, 255, 255, 255] } else { [0, 0, 0, 255] }).collect();
            }
            (photocraft_codecs::web::encode_wbmp(ww, hh, &white).map_err(e)?, Some(2))
        }
        WebFormat::Gif | WebFormat::Png8 => {
            let mut mapping = st.mapping_settings();
            mapping.validate().map_err(other)?;
            // Indexed formats have binary alpha. Preserve remains useful for RGB filters,
            // but here it means the established hard threshold, unless alpha dithering is chosen.
            if mapping.alpha == AlphaMode::Preserve {
                mapping.alpha = AlphaMode::Binary;
            }
            let transparent = st.transparency && region().any(|p| p[3] < if mapping.alpha == AlphaMode::Dither { 1.0 } else { 0.5 });
            let mut work: Vec<[f32; 4]> = region()
                .map(|p| {
                    if !st.transparency {
                        over(p)
                    } else if st.matte.is_some() {
                        let mut q = over(p);
                        q[3] = p[3];
                        q
                    } else {
                        *p
                    }
                })
                .collect();
            let capacity = 256 - usize::from(transparent);
            let fixed = st.color_table.is_some() || st.palette_snapshot.is_some();
            // BlackWhite is a fixed two-opaque-entry generation preset. Its count is independent
            // of the indexed format's total-count field so transparency can add a third slot.
            let generated_black_white = !fixed && st.palette == PaletteKind::BlackWhite;
            let room = if generated_black_white { 2 } else { st.colors.saturating_sub(usize::from(transparent)).clamp(1, capacity) };
            let profile = indexed_profile(icc)?;
            let mut pal = if let Some(table) = &st.color_table {
                if table.is_empty() {
                    return Err(other("colorTable must contain at least one opaque entry"));
                }
                if table.len() > capacity {
                    if !st.reduce_palette {
                        return Err(other(format!(
                            "colorTable has {} entries; this image allows {capacity}. Enable Reduce Palette explicitly or choose a smaller palette",
                            table.len()
                        )));
                    }
                    if !st.color_table_edits.is_empty() {
                        return Err(other("reducing colorTable changes its indices; clear colorTableEdits before reducing the palette"));
                    }
                    let palette = Palette { name: "Color Table".into(), colors: table.iter().map(|color| color.map(|v| f32::from(v) / 255.0)).collect() };
                    crate::palette_cmds::reduce_palette(&palette, capacity)?.colors.iter().map(|color| color.map(to8)).collect()
                } else {
                    table.clone()
                }
            } else if let Some(snapshot) = &st.palette_snapshot {
                snapshot.validate().map_err(other)?;
                let mut palette = if snapshot.colors.len() > capacity {
                    if !st.reduce_palette {
                        return Err(other(format!(
                            "palette has {} entries; this image allows {capacity}. Enable Reduce Palette explicitly or choose a smaller palette",
                            snapshot.colors.len()
                        )));
                    }
                    crate::palette_cmds::reduce_palette(snapshot, capacity)?
                } else {
                    snapshot.clone()
                };
                // Library snapshots are sRGB. If conversion is disabled, encode palette
                // entries in the export document's profile, just like its other pixel data.
                if let Some(profile) = &profile {
                    convert_palette(&mut palette, photocraft_cms::Builtin::Srgb.profile(), profile)?;
                }
                palette.colors.iter().map(|color| color.map(to8)).collect()
            } else {
                let generated = quantize::build_palette(&work, st.palette, room, Forced::None).map_err(other)?;
                if !generated_black_white && generated.len() > room {
                    let palette = Palette { name: "Generated".into(), colors: generated.iter().map(|c| c.map(|v| f32::from(v) / 255.0)).collect() };
                    crate::palette_cmds::reduce_palette(&palette, room)?.colors.iter().map(|c| c.map(to8)).collect()
                } else {
                    generated
                }
            };
            if st.web_snap > 0.0 && !fixed {
                let tol = (st.web_snap * 255.0) as u8;
                for c in &mut pal {
                    let snap = c.map(|x| ((f32::from(x) / 51.0).round() * 51.0) as u8);
                    if c.iter().zip(snap).all(|(a, b)| a.abs_diff(b) <= tol) {
                        *c = snap;
                    }
                }
            }
            let first = pal.first().copied().ok_or_else(|| other("empty colour table"))?;
            color_table = pal.clone();
            let t = transparent.then(|| {
                pal.push(first);
                pal.len() - 1
            });
            transparent_index = t;
            let mut palette = Palette { name: "Export".into(), colors: pal.iter().map(|c| c.map(|v| f32::from(v) / 255.0)).collect() };
            if let Some(profile) = &profile {
                convert_palette(&mut palette, profile, photocraft_cms::Builtin::Srgb.profile())?;
            }
            convert_working_rgb(&mut work, profile.as_ref())?;
            let mapped = photocraft_algo::palette_map::render(&mut work, w, &palette, &mapping, [rect.x0, rect.y0], t).map_err(other)?;
            let idx = mapped.into_iter().map(|index| u8::try_from(index).map_err(other)).collect::<Result<Vec<_>>>()?;
            for (index, color) in &st.color_table_edits {
                let slot = pal.get_mut(*index).filter(|_| Some(*index) != t).ok_or_else(|| other("colour edit is outside the opaque palette"))?;
                *slot = *color;
            }
            if preview {
                look = idx
                    .iter()
                    .flat_map(|k| {
                        let c = pal.get(*k as usize).copied().unwrap_or(first);
                        [c[0], c[1], c[2], if Some(*k as usize) == t { 0 } else { 255 }]
                    })
                    .collect();
            }
            let tt = t.map(|v| v as u8);
            let bytes = if st.format == WebFormat::Gif {
                photocraft_codecs::web::encode_gif_indexed(ww, hh, &idx, &pal, tt, st.interlaced).map_err(e)?
            } else {
                photocraft_codecs::encode_png_indexed(ww, hh, &idx, &pal, tt, None, None).map_err(e)?
            };
            if preview {
                indices = idx;
            }
            (bytes, Some(pal.len()))
        }
    };
    Ok(Optimized { bytes, ext: st.format.ext(), width: ww, height: hh, colors, color_table, transparent_index, indices, preview: look })
}

/// Save for Web has already converted its source to RGB, but may retain a non-sRGB profile.
fn indexed_profile(icc: Option<&[u8]>) -> Result<Option<photocraft_cms::Profile>> {
    let Some(bytes) = icc else { return Ok(None) };
    let profile = photocraft_cms::Profile::parse(bytes).map_err(other)?;
    if profile.color_space != photocraft_cms::ColorSpace::Rgb {
        return Err(other("indexed export requires an RGB source profile"));
    }
    Ok(Some(profile))
}

fn convert_working_rgb(pixels: &mut [[f32; 4]], source: Option<&photocraft_cms::Profile>) -> Result<()> {
    let Some(source) = source else { return Ok(()) };
    let target = photocraft_cms::Builtin::Srgb.profile();
    if source.content_hash() != target.content_hash() {
        let transform = photocraft_cms::transform::cached(source, target, Default::default()).map_err(other)?;
        transform.apply(pixels.as_flattened_mut(), 4);
    }
    Ok(())
}

fn convert_palette(palette: &mut Palette, source: &photocraft_cms::Profile, target: &photocraft_cms::Profile) -> Result<()> {
    if source.content_hash() != target.content_hash() {
        let transform = photocraft_cms::transform::cached(source, target, Default::default()).map_err(other)?;
        transform.apply(palette.colors.as_flattened_mut(), 3);
    }
    Ok(())
}

/// A file-name-safe slice name.
fn slice_file(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_') { c } else { '_' }).collect();
    let s = if s.is_empty() { "slice".into() } else { s };
    portable_generated_stem(s)
}

/// Generated filenames should remain usable on Windows even when a document or slice is named
/// after a reserved device. Caller-supplied path components are still rejected below.
fn portable_generated_stem(stem: String) -> String {
    portable_device_name(stem)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Imported slice URLs are document data. Only ordinary web/contact links and relative links
/// become anchors; an unsupported scheme leaves the image unlinked.
fn safe_slice_url(url: &str) -> bool {
    if url.is_empty() || url.trim() != url || url.chars().any(|c| c.is_control() || c == '\\') {
        return false;
    }
    let delimiter = url.find(['/', '?', '#']).unwrap_or(url.len());
    let Some(colon) = url.find(':').filter(|pos| *pos < delimiter) else {
        return true;
    };
    let scheme = url.get(..colon).unwrap_or_default().to_ascii_lowercase();
    let rest = url.get(colon + 1..).unwrap_or_default();
    match scheme.as_str() {
        "http" | "https" | "ftp" | "ftps" => rest.strip_prefix("//").is_some_and(|host| !host.is_empty() && !host.starts_with('/')),
        "mailto" | "tel" => !rest.is_empty(),
        _ => false,
    }
}

/// Keep the image folder relative to the selected export directory on every platform.
fn web_images_folder(images: &str, cmd: &str) -> Result<String> {
    if images.is_empty() {
        return Ok(String::new());
    }
    let trimmed = images.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return Err(bad(cmd, "imagesFolder must be a relative folder name"));
    }
    let parts = asset_path_components(trimmed).map_err(|e| bad(cmd, format!("imagesFolder: {e}")))?;
    Ok(parts.join("/"))
}

fn web_output_path(dir: &str, relative: &str) -> Result<String> {
    let root = if dir.is_empty() { "." } else { dir };
    asset_output_path(root, relative).map(|(actual, _)| actual)
}

fn web_derived_path(dir: &str, relative: &str, native_paths: bool) -> Result<String> {
    if native_paths {
        web_output_path(dir, relative)
    } else {
        // Browser downloads use logical names, without a host filesystem to inspect.
        asset_path_components(relative)?;
        Ok(join(dir, relative))
    }
}

fn scale_rect(r: Rect, sx: f64, sy: f64) -> Rect {
    Rect::new(
        (f64::from(r.x0) * sx).round() as i32,
        (f64::from(r.y0) * sy).round() as i32,
        (f64::from(r.x1) * sx).round() as i32,
        (f64::from(r.y1) * sy).round() as i32,
    )
}

/// The HTML table for `cells` (rect, td body) over a `w`×`h` image: one column per distinct x
/// edge, one row per y edge, colspan/rowspan per slice, and a spacer row fixing column widths.
fn html_table(title: &str, w: u32, h: u32, cells: &[(Rect, String, String)], spacer: Option<&str>) -> String {
    let mut xs: Vec<i32> = cells.iter().flat_map(|(r, _, _)| [r.x0, r.x1]).collect();
    let mut ys: Vec<i32> = cells.iter().flat_map(|(r, _, _)| [r.y0, r.y1]).collect();
    xs.sort_unstable();
    xs.dedup();
    ys.sort_unstable();
    ys.dedup();
    let col = |x: i32| xs.iter().position(|v| *v == x).unwrap_or(0);
    let row = |y: i32| ys.iter().position(|v| *v == y).unwrap_or(0);
    let (nc, nr) = (xs.len().saturating_sub(1), ys.len().saturating_sub(1));
    let mut taken = vec![false; nc * nr];
    let mut by_row: Vec<Vec<(usize, String)>> = vec![Vec::new(); nr];
    for (r, attrs, body) in cells {
        let (c0, c1, r0, r1) = (col(r.x0), col(r.x1), row(r.y0), row(r.y1));
        if (r0..r1).any(|j| (c0..c1).any(|i| taken[j * nc + i])) {
            continue; // overlapped (the caller cuts slices into visible pieces, so this can't happen)
        }
        for j in r0..r1 {
            for i in c0..c1 {
                taken[j * nc + i] = true;
            }
        }
        let mut td = String::from("\t\t<td");
        if c1 - c0 > 1 {
            td.push_str(&format!(" colspan=\"{}\"", c1 - c0));
        }
        if r1 - r0 > 1 {
            td.push_str(&format!(" rowspan=\"{}\"", r1 - r0));
        }
        td.push_str(attrs);
        td.push('>');
        td.push_str(body);
        td.push_str("</td>\n");
        by_row[r0].push((c0, td));
    }
    let mut s = format!(
        "<html>\n<head>\n<title>{t}</title>\n<meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\">\n</head>\n<body bgcolor=\"#FFFFFF\" leftmargin=\"0\" topmargin=\"0\" marginwidth=\"0\" marginheight=\"0\">\n<!-- Save for Web Slices ({t}) -->\n<table id=\"Table_01\" width=\"{w}\" height=\"{h}\" border=\"0\" cellpadding=\"0\" cellspacing=\"0\">\n",
        t = html_escape(title)
    );
    for (j, mut tds) in by_row.into_iter().enumerate() {
        tds.sort_by_key(|(c, _)| *c);
        s.push_str("\t<tr>\n");
        for (_, td) in tds {
            s.push_str(&td);
        }
        if let Some(sp) = spacer {
            s.push_str(&format!("\t\t<td>\n\t\t\t<img src=\"{}\" width=\"1\" height=\"{}\" alt=\"\"></td>\n", html_escape(sp), ys[j + 1] - ys[j]));
        }
        s.push_str("\t</tr>\n");
    }
    if let Some(sp) = spacer {
        s.push_str("\t<tr>\n");
        for i in 0..nc {
            s.push_str(&format!("\t\t<td>\n\t\t\t<img src=\"{}\" width=\"{}\" height=\"1\" alt=\"\"></td>\n", html_escape(sp), xs[i + 1] - xs[i]));
        }
        s.push_str("\t\t<td></td>\n\t</tr>\n");
    }
    s.push_str("</table>\n<!-- End Save for Web Slices -->\n</body>\n</html>\n");
    s
}

/// File › Export › Save for Web (Legacy).
fn save_for_web(s: &mut Session, p: &Value) -> Result<Value> {
    save_for_web_impl(s, p, &mut write_file, true)
}

/// Save for Web through a platform writer (native files or browser downloads).
/// The writer receives every image, spacer and HTML file. A write error stops the export;
/// only a complete export remembers its settings and fires the export event. A custom writer
/// that targets the local filesystem must enforce its own path confinement.
pub fn save_for_web_with_writer(s: &mut Session, p: &Value, write: &mut dyn FnMut(&str, &[u8]) -> Result<()>) -> Result<Value> {
    save_for_web_impl(s, p, write, false)
}

/// Settings to seed the Save for Web dialog from on the next launch: `file_menu` is session
/// state, so a complete export also writes the preferences' remembered dialog settings (like
/// Fill, Liquify and the filter dialogs). The destination is per export, never remembered.
fn remember_web_settings(s: &mut Session, cmd: &str, settings: &Value) {
    let mut m = settings.clone();
    if let Some(o) = m.as_object_mut() {
        for k in ["path", "dir", "trustedSliceHtml"] {
            o.remove(k);
        }
    }
    s.prefs.edit(|p| {
        p.dialogs.insert(cmd.to_string(), m);
    });
}

fn save_for_web_impl(s: &mut Session, p: &Value, write: &mut dyn FnMut(&str, &[u8]) -> Result<()>, native_paths: bool) -> Result<Value> {
    let cmd = "file.export.saveForWebLegacy";
    if let Some(authorize) = s.authorize {
        authorize(cmd, p)?;
    }
    // Direct platform calls must export the moved selection as it appears on the canvas,
    // just as Session::execute does before dispatching the native command.
    crate::float_cmds::before_command(s, cmd)?;
    let st = WebSettings::from_params(p, cmd)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let doc = d.doc.clone();
    let (wdoc, sx, sy) = web_document(&doc, p, &st)?;
    let buf = photocraft_compose::flatten(&wdoc);
    let bw = wdoc.size.width as usize;
    let icc = wdoc.icc_profile.as_deref().map(|v| v.as_slice());
    let xmp = web_xmp(&doc, st.metadata);
    let exif = web_exif(&doc, st.metadata);
    let dpi = wdoc.resolution_dpi;
    let warnings = web_warnings(&st, exif, dpi);
    let all = slices::resolve(&doc);
    let pick = p.get("slices").and_then(Value::as_str).unwrap_or("all");
    let numbers: Option<Vec<u64>> = p.get("numbers").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect());
    let chosen: Vec<&ResolvedSlice> = all
        .iter()
        .filter(|r| match (&numbers, pick) {
            (Some(n), _) => n.contains(&(r.number as u64)),
            (None, "user") => r.origin != photocraft_doc::SliceOrigin::Auto,
            _ => true,
        })
        .collect();
    let single = p.get("path").and_then(Value::as_str).filter(|v| !v.is_empty());
    let dir = p.get("dir").and_then(Value::as_str).filter(|v| !v.is_empty());
    let mut last_web = p.clone();
    // A trusted-markup decision applies to this export only, never to another document opened
    // with the dialog's remembered settings.
    if let Some(last) = last_web.as_object_mut() {
        last.remove("trustedSliceHtml");
    }
    // Estimate only (the dialog's annotations): the whole image, or each chosen slice.
    if single.is_none() && dir.is_none() {
        let o = optimize(&buf.px, bw, wdoc.bounds(), &st, icc, exif, xmp.as_deref(), dpi, false)?;
        return Ok(
            json!({"format": o.ext, "bytes": o.bytes.len(), "width": o.width, "height": o.height, "colors": o.colors, "colorTable": o.color_table.iter().map(|c| rgb_hex(*c)).collect::<Vec<_>>(), "transparentIndex": o.transparent_index}),
        );
    }
    if let Some(path) = single.filter(|_| dir.is_none() && (all.len() == 1 || p.get("slices").is_none())) {
        let o = optimize(&buf.px, bw, wdoc.bounds(), &st, icc, exif, xmp.as_deref(), dpi, false)?;
        write(path, &o.bytes)?;
        remember_web_settings(s, cmd, &last_web);
        s.file_menu.last_web = Some(last_web);
        crate::automate_cmds::fire_event(s, "export");
        return Ok(json!({"files": [path], "bytes": o.bytes.len(), "width": o.width, "height": o.height, "colors": o.colors, "warnings": warnings}));
    }
    let dir =
        dir.map(str::to_string).or_else(|| single.and_then(|f| std::path::Path::new(f).parent().map(|d| d.to_string_lossy().into_owned()))).unwrap_or_default();
    let html = p.get("html").and_then(Value::as_bool).unwrap_or(false);
    let images = web_images_folder(p.get("imagesFolder").and_then(Value::as_str).unwrap_or("images"), cmd)?;
    let trusted_slice_html = p.get("trustedSliceHtml").and_then(Value::as_bool).unwrap_or(false);
    let base = portable_generated_stem(single.map(stem).unwrap_or_else(|| slices::base_name(&doc)));
    let mut files = Vec::new();
    let mut cells: Vec<(Rect, String, String)> = Vec::new();
    let mut total = 0usize;
    // Output names are unique (case-insensitively, for macOS and Windows file systems): two
    // slices named alike, or names that sanitize alike, must not overwrite each other. The HTML
    // page's spacer image is reserved.
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    if html {
        used.insert("spacer.gif".into());
    }
    let mut unique = |stem: &str, ext: &str| {
        let mut name = format!("{stem}.{ext}");
        let mut k = 2u32;
        while !used.insert(name.to_lowercase()) {
            name = format!("{stem}_{k}.{ext}");
            k = k.saturating_add(1);
        }
        name
    };
    // Slices never overlap here: one covered by later slices comes as its visible pieces (see
    // `slices::resolve`), each its own image and cell, as Photoshop's subslices. A no-image cell's
    // text goes into its first piece only.
    let mut texted = std::collections::HashSet::new();
    for r in &chosen {
        let piece = scale_rect(r.rect, sx, sy).intersect(&wdoc.bounds());
        if piece.is_empty() {
            continue;
        }
        let stored = r.id.and_then(|id| doc.slices.get(id));
        if r.kind == SliceKind::NoImage {
            let text = stored
                .filter(|s| texted.insert(s.id))
                .map(|s| if trusted_slice_html && s.cell_text_is_html { s.cell_text.clone() } else { html_escape(&s.cell_text) })
                .unwrap_or_default();
            let bg = stored.and_then(|s| s.background).map(|c| format!(" bgcolor=\"#{:02X}{:02X}{:02X}\"", c[1], c[2], c[3])).unwrap_or_default();
            cells.push((piece, bg, text));
            continue;
        }
        let o = optimize(&buf.px, bw, piece, &st, icc, exif, xmp.as_deref(), dpi, false)?;
        let name = unique(&slice_file(&r.name), o.ext);
        let rel = if images.is_empty() { name.clone() } else { format!("{images}/{name}") };
        let out = join(&dir, &rel);
        write(&web_derived_path(&dir, &rel, native_paths)?, &o.bytes)?;
        total += o.bytes.len();
        files.push(out);
        let alt = stored.map_or(String::new(), |s| html_escape(&s.alt));
        let img = format!("\n\t\t\t<img src=\"{}\" width=\"{}\" height=\"{}\" alt=\"{alt}\">", html_escape(&rel), piece.width(), piece.height());
        let body = match stored.filter(|s| safe_slice_url(&s.url)) {
            Some(s) => {
                let target = if s.target.is_empty() { String::new() } else { format!(" target=\"{}\"", html_escape(&s.target)) };
                format!("\n\t\t\t<a href=\"{}\"{target}>{img}</a>", html_escape(&s.url))
            }
            None => img,
        };
        cells.push((piece, String::new(), body));
    }
    let mut html_path = None;
    if html {
        let grid = cells.len() > 1;
        let spacer = grid.then(|| if images.is_empty() { "spacer.gif".to_string() } else { format!("{images}/spacer.gif") });
        if let Some(sp) = &spacer {
            let gif = photocraft_codecs::web::encode_gif_indexed(1, 1, &[0], &[[0, 0, 0]], Some(0), false).map_err(other)?;
            write(&web_derived_path(&dir, sp, native_paths)?, &gif)?;
        }
        let page = html_table(&base, wdoc.size.width, wdoc.size.height, &cells, spacer.as_deref());
        let hp = join(&dir, &format!("{base}.html"));
        write(&web_derived_path(&dir, &format!("{base}.html"), native_paths)?, page.as_bytes())?;
        html_path = Some(hp);
    }
    remember_web_settings(s, cmd, &last_web);
    s.file_menu.last_web = Some(last_web);
    crate::automate_cmds::fire_event(s, "export");
    Ok(
        json!({"files": files, "html": html_path, "bytes": total, "slices": chosen.len(), "width": wdoc.size.width, "height": wdoc.size.height, "warnings": warnings}),
    )
}

// ---------- Export Preferences / Quick Export ----------

/// File › Export › Export Preferences…: sets the given `export.*` preferences through `prefs.set`
/// (validated like any other preference) and returns the whole section.
fn export_preferences(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.export.exportPreferences";
    let mut values = serde_json::Map::new();
    for (k, key) in [
        ("quickExportFormat", "export.quickExportFormat"),
        ("quickExportLocation", "export.quickExportLocation"),
        ("jpegQuality", "export.jpegQuality"),
        ("webpLossless", "export.webpLossless"),
        ("webpQuality", "export.webpQuality"),
        ("metadata", "export.metadata"),
        ("convertToSrgb", "export.convertToSrgb"),
    ] {
        if let Some(v) = p.get(k) {
            values.insert(key.into(), v.clone());
        }
    }
    if !values.is_empty() {
        s.execute("prefs.set", json!({"values": values})).map_err(|e| bad(cmd, e.to_string()))?;
    }
    Ok(json!({"section": "export", "values": s.prefs().get("export").unwrap_or(Value::Null)}))
}

/// The Quick Export format's extension (Export Preferences › Format).
fn quick_export_ext(s: &Session) -> String {
    serde_json::to_value(s.prefs().export.quick_export_format).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "png".into())
}

/// Where Quick Export writes the active document with no `path` given: `<name>.<format>` beside
/// its file, when Export Preferences › Location is Same Folder and the document has a file.
pub fn quick_export_target(s: &Session) -> Option<String> {
    let same = s.prefs().export.quick_export_location == crate::prefs::ExportLocation::SameFolder;
    let dp = s.active()?.path.as_deref().filter(|_| same)?;
    let dir = std::path::Path::new(dp).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    Some(join(&dir, &format!("{}.{}", stem(dp), quick_export_ext(s))))
}

/// Whether [`quick_export_target`] is the active document's own file (a document opened from
/// `photo.png` and exported as PNG), which Quick Export must not replace without asking (#6599).
pub fn quick_export_target_is_own_file(s: &Session) -> bool {
    let own = s.active().and_then(|d| d.path.as_deref());
    quick_export_target(s).zip(own).is_some_and(|(target, own)| crate::file_cmds::same_file(&target, own))
}

/// File › Export › Quick Export as <format>: the format, quality, metadata, colour space and
/// location from Export Preferences. `path` overrides the location ("ask" needs one).
fn quick_export(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.export.quickExport";
    let prefs = s.prefs().export.clone();
    let fmt = quick_export_ext(s);
    let path = match p.get("path").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        Some(v) => v.to_string(),
        None => {
            let Some(target) = quick_export_target(s) else {
                return Err(bad(cmd, "missing `path` (Export Preferences › Location is \"ask\", or the document was never saved)"));
            };
            // The document's own file is never replaced by an export the user didn't name (#6599).
            if quick_export_target_is_own_file(s) {
                return Err(bad(cmd, format!("the export would replace the document's own file {target}; give a `path`")));
            }
            target
        }
    };
    // The name typed in the save dialog wins over Export Preferences' format: Quick Export as PNG
    // renamed to `photo.jpeg` must write a JPEG, not PNG bytes upload checks refuse (#8519).
    let fmt = match crate::file_cmds::extension(&path).as_deref() {
        None => fmt,
        Some("jpg" | "jpeg") => "jpg".into(),
        Some(e @ ("png" | "gif" | "webp" | "exr")) => e.into(),
        Some(e) => return Err(bad(cmd, format!("Quick Export writes PNG, JPEG, GIF, WebP or EXR, not .{e}; use Export As"))),
    };
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let doc = d.doc.clone();
    if fmt == "exr" {
        // EXR keeps floating-point scene values. The ordinary exporter converts to linear
        // light; the web/sRGB pipeline would instead clamp HDR and negative values.
        let opts = photocraft_io::ExportOptions { xmp: photocraft_io::XmpEmbed::None, ..Default::default() };
        let out = photocraft_io::export(&doc, "exr", &opts).map_err(other)?;
        write_file(&path, &out.bytes)?;
        crate::automate_cmds::fire_event(s, "export");
        return Ok(json!({"path": path, "format": fmt, "bytes": out.bytes.len(), "warnings": out.warnings}));
    }
    let metadata = serde_json::to_value(prefs.metadata).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    let mut o = serde_json::Map::new();
    o.insert(
        "metadata".into(),
        json!(if metadata == "all" {
            "all"
        } else if metadata == "none" {
            "none"
        } else {
            "copyright"
        }),
    );
    o.insert("convertToSrgb".into(), json!(prefs.convert_to_srgb));
    o.insert("transparency".into(), json!(true));
    o.insert("matte".into(), json!("#ffffff"));
    match fmt.as_str() {
        "jpg" => {
            o.insert("format".into(), json!("jpeg"));
            o.insert("quality".into(), json!(prefs.jpeg_quality.min(100)));
        }
        "gif" => {
            o.insert("format".into(), json!("gif"));
        }
        "webp" => {
            // WebP uses the regular exporter. Its optional quality is in the Photoshop-style
            // 0–12 save scale: invert that mapping so the codec receives the chosen 1–100.
            // Keeping quality absent preserves the historical lossless Quick Export behavior.
            let (wdoc, _, _) = web_document(&doc, &json!({}), &WebSettings { convert_to_srgb: prefs.convert_to_srgb, ..Default::default() })?;
            let quality = if prefs.webp_lossless { None } else { Some((f64::from(prefs.webp_quality.clamp(1, 100)) - 1.0) * 12.0 / 99.0) };
            let warnings = crate::file_cmds::save_doc(&wdoc, &path, quality)?;
            crate::automate_cmds::fire_event(s, "export");
            return Ok(json!({"path": path, "format": fmt, "warnings": warnings}));
        }
        _ => {
            o.insert("format".into(), json!("png24"));
        }
    }
    let q = Value::Object(o);
    let st = WebSettings::from_params(&q, cmd)?;
    let (wdoc, _, _) = web_document(&doc, &json!({}), &st)?;
    let buf = photocraft_compose::flatten(&wdoc);
    let xmp = web_xmp(&doc, st.metadata);
    let exif = web_exif(&doc, st.metadata);
    let opt = optimize(
        &buf.px,
        wdoc.size.width as usize,
        wdoc.bounds(),
        &st,
        wdoc.icc_profile.as_deref().map(|v| v.as_slice()),
        exif,
        xmp.as_deref(),
        wdoc.resolution_dpi,
        false,
    )?;
    write_file(&path, &opt.bytes)?;
    crate::automate_cmds::fire_event(s, "export");
    Ok(json!({"path": path, "format": fmt, "bytes": opt.bytes.len(), "warnings": web_warnings(&st, exif, wdoc.resolution_dpi)}))
}

// ---------- Generate › Image Assets ----------

/// One asset requested by a layer name.
#[derive(Clone, Debug, PartialEq)]
pub struct AssetSpec {
    /// Relative output path (folders allowed), extension included.
    pub file: String,
    /// png8 | png24 | png32 | jpg | gif | webp
    pub format: String,
    /// JPEG quality 1..=100.
    pub quality: Option<u8>,
    /// Uniform scale (1.0 = 100 %).
    pub scale: Option<f64>,
    /// Target width / height in px (`?` = keep aspect).
    pub width: Option<f64>,
    pub height: Option<f64>,
}

/// A `default` layer variant: size plus a folder (`low/`) or name suffix (`@2x`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AssetDefault {
    pub scale: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub folder: String,
    pub suffix: String,
}

fn unit_px(tok: &str, dpi: f64) -> Option<Option<f64>> {
    if tok == "?" {
        return Some(None);
    }
    let (num, k) = if let Some(n) = tok.strip_suffix("px") {
        (n, 1.0)
    } else if let Some(n) = tok.strip_suffix("in") {
        (n, dpi)
    } else if let Some(n) = tok.strip_suffix("cm") {
        (n, dpi / 2.54)
    } else if let Some(n) = tok.strip_suffix("mm") {
        (n, dpi / 25.4)
    } else {
        (tok, 1.0)
    };
    num.trim().parse::<f64>().ok().filter(|v| *v > 0.0).map(|v| Some(v * k))
}

/// `200%` or `WxH` (units px/in/cm/mm, `?` wildcard) at the start of `item`; returns the size and
/// the rest.
fn parse_size(item: &str, dpi: f64) -> (Option<f64>, Option<f64>, Option<f64>, &str) {
    let item = item.trim();
    let (first, rest) = item.split_once(char::is_whitespace).map_or((item, ""), |(a, b)| (a, b.trim()));
    if let Some(pc) = first.strip_suffix('%').and_then(|n| n.parse::<f64>().ok()).filter(|v| *v > 0.0) {
        return (Some(pc / 100.0), None, None, rest);
    }
    // "100x50", "2in x 1in", "?x100"
    if let Some((w, h)) = first.split_once(['x', 'X']).filter(|(w, h)| !w.is_empty() && !h.is_empty())
        && let (Some(w), Some(h)) = (unit_px(w, dpi), unit_px(h, dpi))
    {
        return (None, w, h, rest);
    }
    // Spaced form "2in x 1in name.png": three tokens.
    let toks: Vec<&str> = item.split_whitespace().collect();
    if toks.len() >= 4
        && toks[1].eq_ignore_ascii_case("x")
        && let (Some(w), Some(h)) = (unit_px(toks[0], dpi), unit_px(toks[2], dpi))
    {
        // Skip exactly the three size tokens; searching for the name's first word could match inside the size.
        let mut rest = item;
        for _ in 0..3 {
            rest = rest.trim_start().trim_start_matches(|c: char| !c.is_whitespace());
        }
        return (None, w, h, rest.trim_start());
    }
    (None, None, None, item)
}

/// Parses a layer name into its asset specs (empty when it names no asset).
pub fn parse_asset_name(name: &str, dpi: f64) -> Vec<AssetSpec> {
    let mut out = Vec::new();
    for item in name.split([',', '+']) {
        let (scale, width, height, rest) = parse_size(item, dpi);
        let file = rest.trim();
        let Some((stem_part, ext)) = file.rsplit_once('.') else { continue };
        if stem_part.is_empty() {
            continue;
        }
        let ext_l = ext.to_ascii_lowercase();
        let (format, quality, out_ext) = if let Some(q) = ext_l.strip_prefix("jpeg").or_else(|| ext_l.strip_prefix("jpg")) {
            let q = q.trim();
            let quality = if q.is_empty() {
                None
            } else if let Some(pc) = q.strip_suffix('%') {
                pc.parse::<u32>().ok().map(|v| v.clamp(1, 100) as u8)
            } else {
                // 1–10 scale.
                q.parse::<u32>().ok().map(|v| (v.clamp(1, 10) * 10) as u8)
            };
            if !q.is_empty() && quality.is_none() {
                continue;
            }
            ("jpg".to_string(), quality, if ext_l.starts_with("jpeg") { "jpeg" } else { "jpg" })
        } else {
            match ext_l.as_str() {
                "png" | "png32" => ("png32".to_string(), None, "png"),
                "png24" => ("png24".to_string(), None, "png"),
                "png8" => ("png8".to_string(), None, "png"),
                "gif" => ("gif".to_string(), None, "gif"),
                "webp" => ("webp".to_string(), None, "webp"),
                _ => continue,
            }
        };
        out.push(AssetSpec { file: format!("{stem_part}.{out_ext}"), format, quality, scale, width, height });
    }
    out
}

/// Parses a `default …` layer name into its variants.
pub fn parse_defaults(name: &str, dpi: f64) -> Option<Vec<AssetDefault>> {
    let rest = name.trim().strip_prefix("default")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let mut v = Vec::new();
    for item in rest.split([',', '+']) {
        let (scale, width, height, tail) = parse_size(item, dpi);
        let tail = tail.trim();
        let (folder, suffix) = match tail.rfind('/') {
            Some(i) => (tail[..=i].to_string(), tail[i + 1..].to_string()),
            None => (String::new(), tail.to_string()),
        };
        if scale.is_none() && width.is_none() && height.is_none() && folder.is_empty() && suffix.is_empty() {
            continue;
        }
        v.push(AssetDefault { scale, width, height, folder, suffix });
    }
    Some(v)
}

fn asset_settings(spec: &AssetSpec) -> WebSettings {
    let base = WebSettings { transparency: true, matte: None, metadata: WebMetadata::None, ..WebSettings::default() };
    match spec.format.as_str() {
        "jpg" => WebSettings { format: WebFormat::Jpeg, quality: spec.quality.unwrap_or(90), matte: Some([1.0, 1.0, 1.0]), ..base },
        "png8" => WebSettings { format: WebFormat::Png8, dither: Dither::None, ..base },
        "png24" => WebSettings { format: WebFormat::Png24, transparency: false, matte: Some([1.0, 1.0, 1.0]), ..base },
        "gif" => WebSettings { format: WebFormat::Gif, dither: Dither::None, ..base },
        _ => WebSettings { format: WebFormat::Png24, ..base },
    }
}

/// Layer names and `default` folders are document data, so reject unsafe output components.
fn asset_path_components(relative: &str) -> Result<Vec<&str>> {
    // Split both separator styles even on Unix, where a backslash would otherwise be a legal
    // filename. This also catches Windows drive, rooted, and UNC paths on every platform.
    let mut parts = Vec::new();
    for part in relative.split(['/', '\\']) {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with('.')
            || part.ends_with(' ')
            || part.contains(':')
            || part.contains('\0')
            || is_windows_device_name(part)
        {
            return Err(other(format!("asset path {relative:?} has an unsafe component {part:?}")));
        }
        parts.push(part);
    }
    Ok(parts)
}

/// Resolve a generated asset below the selected output directory. The atomic writer follows an
/// existing file symlink, so reject it and directory symlinks before writing.
fn asset_output_path(dir: &str, relative: &str) -> Result<(String, String)> {
    if dir.is_empty() {
        return Err(other("image assets need an output directory"));
    }
    let mut parts = asset_path_components(relative)?;
    // Keep the caller's spelling in the result (for example /var on macOS), while writes use
    // the canonical directory so a symlink in the selected root cannot move the destination.
    let mut reported = std::path::PathBuf::from(dir);
    for part in &parts {
        reported.push(part);
    }
    let file = parts.pop().ok_or_else(|| other(format!("asset path {relative:?} is empty")))?;
    let chosen_root: std::path::PathBuf = std::path::Path::new(dir).components().collect();
    std::fs::create_dir_all(&chosen_root).map_err(|e| other(format!("asset output directory {dir:?}: {e}")))?;
    // The caller chose this root, so a symlink to an existing directory is intentional. All
    // document-derived components below it are checked against its canonical target.
    let root = std::fs::canonicalize(&chosen_root).map_err(|e| other(format!("asset output directory {dir:?}: {e}")))?;
    let mut parent = root.clone();
    for part in parts {
        parent.push(part);
        match std::fs::symlink_metadata(&parent) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(other(format!("asset path {relative:?} crosses a symlink at {}", parent.display())));
            }
            Ok(meta) if !meta.is_dir() => {
                return Err(other(format!("asset path {relative:?} has a non-directory at {}", parent.display())));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&parent).map_err(|e| other(format!("asset folder {}: {e}", parent.display())))?;
            }
            Err(e) => return Err(other(format!("asset folder {}: {e}", parent.display()))),
        }
        let actual = std::fs::canonicalize(&parent).map_err(|e| other(format!("asset folder {}: {e}", parent.display())))?;
        if !actual.starts_with(&root) {
            return Err(other(format!("asset path {relative:?} escapes output directory {dir:?}")));
        }
        parent = actual;
    }
    let parent = std::fs::canonicalize(&parent).map_err(|e| other(format!("asset folder {}: {e}", parent.display())))?;
    if !parent.starts_with(&root) {
        return Err(other(format!("asset path {relative:?} escapes output directory {dir:?}")));
    }
    let output = parent.join(file);
    match std::fs::symlink_metadata(&output) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(other(format!("asset path {relative:?} points to a symlink")));
        }
        Ok(meta) if !meta.is_file() => {
            return Err(other(format!("asset path {relative:?} is not a file")));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(other(format!("asset path {relative:?}: {e}"))),
    }
    Ok((output.to_string_lossy().into_owned(), reported.to_string_lossy().into_owned()))
}

/// Exports every asset named by `doc`'s layers into `dir`. Returns (files, errors).
///
/// Each file is written once per run (#6614): when two layers (or two default variants) name the
/// same file, the one higher in the Layers panel keeps it and the other is reported in `errors`.
/// Names are compared ignoring case, as `Logo.png` and `logo.png` are one file on Windows and
/// macOS.
///
/// A layer is trimmed to its visible pixels; an artboard keeps its whole board, transparent edges
/// included, as File › Export › Artboards to Files writes it (Photoshop's Generator exports an
/// artboard asset at the board's bounds).
pub fn generate_assets(doc: &Document, dir: &str) -> (Vec<String>, Vec<Value>) {
    let dpi = f64::from(doc.resolution_dpi.max(1.0));
    let mut defaults: Option<Vec<AssetDefault>> = None;
    let mut by_layer: Vec<Vec<(LayerId, AssetSpec)>> = Vec::new();
    for (_, _, l) in doc.walk() {
        if let Some(d) = parse_defaults(&l.name, dpi) {
            defaults = Some(d);
            continue;
        }
        if matches!(l.content, LayerContent::Adjustment(_)) {
            continue;
        }
        by_layer.push(parse_asset_name(&l.name, dpi).into_iter().map(|spec| (l.id, spec)).collect());
    }
    // Top layer first, as the Layers panel lists them (`walk` goes bottom to top).
    let jobs: Vec<(LayerId, AssetSpec)> = by_layer.into_iter().rev().flatten().collect();
    let variants: Vec<AssetDefault> = match defaults {
        Some(d) if !d.is_empty() => d,
        _ => vec![AssetDefault::default()],
    };
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let mut written = std::collections::HashSet::new();
    for (id, spec) in &jobs {
        for v in &variants {
            let r = (|| -> Result<String> {
                let (name_stem, ext) = spec.file.rsplit_once('.').unwrap_or((spec.file.as_str(), "png"));
                let rel = format!("{}{}{}.{ext}", v.folder, name_stem, v.suffix);
                let key = asset_path_components(&rel)?.join("/").to_lowercase();
                if written.contains(&key) {
                    return Err(other(format!("{rel}: another layer already writes this file; rename one of the layers")));
                }
                let mut one = match crate::artboard_cmds::artboard_document(doc, *id)? {
                    // Like Photoshop's Generator, a board with nothing on it is refused rather
                    // than written as a blank file.
                    Some(board) if !has_pixels(&board) => return Err(other("the layer is empty")),
                    Some(board) => board,
                    None => crate::layer_menu_cmds::layer_document(doc, *id)?,
                };
                one.metadata = Default::default();
                let (w0, h0) = (f64::from(one.size.width), f64::from(one.size.height));
                // The layer's own size spec wins; default variants scale on top of it.
                let mut k = spec.scale.unwrap_or(1.0);
                let (mut tw, mut th) = (w0 * k, h0 * k);
                match (spec.width, spec.height) {
                    (Some(w), Some(h)) => (tw, th) = (w, h),
                    (Some(w), None) => (tw, th) = (w, h0 * w / w0),
                    (None, Some(h)) => (tw, th) = (w0 * h / h0, h),
                    _ => {}
                }
                k = v.scale.unwrap_or(1.0);
                tw *= k;
                th *= k;
                match (v.width, v.height) {
                    (Some(w), Some(h)) => (tw, th) = (w, h),
                    (Some(w), None) => (tw, th) = (w, th * w / tw),
                    (None, Some(h)) => (tw, th) = (tw * h / th, h),
                    _ => {}
                }
                let st = asset_settings(spec);
                let p = json!({"width": tw.round().max(1.0), "height": th.round().max(1.0)});
                let (wdoc, _, _) = web_document(&one, &p, &WebSettings { convert_to_srgb: true, ..st.clone() })?;
                if spec.format == "webp" {
                    let (out, reported) = asset_output_path(dir, &rel)?;
                    crate::file_cmds::save_doc(&wdoc, &out, None)?;
                    written.insert(key);
                    return Ok(reported);
                }
                let buf = photocraft_compose::flatten(&wdoc);
                let o = optimize(&buf.px, wdoc.size.width as usize, wdoc.bounds(), &st, None, None, None, wdoc.resolution_dpi, false)?;
                let (out, reported) = asset_output_path(dir, &rel)?;
                write_file(&out, &o.bytes)?;
                written.insert(key);
                Ok(reported)
            })();
            match r {
                Ok(f) => files.push(f),
                Err(e) => errors.push(json!({"asset": spec.file, "error": e.to_string()})),
            }
        }
    }
    (files, errors)
}

/// `<document folder>/<document name>-assets`.
/// Whether any pixel of `doc` is not fully transparent.
fn has_pixels(doc: &Document) -> bool {
    photocraft_compose::render(doc, doc.bounds()).px.iter().any(|q| q[3] > 0.0)
}

pub fn assets_dir(doc_path: &str) -> String {
    let p = std::path::Path::new(doc_path);
    let dir = p.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    join(&dir, &format!("{}-assets", stem(doc_path)))
}

/// File › Generate › Image Assets: toggles generation for the active document (or sets it with
/// `on`) and, when it is on and the output folder is known, writes the assets now.
fn image_assets(s: &mut Session, p: &Value) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let id: DocId = d.doc.id;
    let was = s.file_menu.image_assets.contains(&id);
    let on = p.get("on").and_then(Value::as_bool).unwrap_or(!was);
    if on {
        if !was {
            s.file_menu.image_assets.push(id);
        }
    } else {
        s.file_menu.image_assets.retain(|x| *x != id);
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let dir = p.get("dir").and_then(Value::as_str).map(str::to_string).or_else(|| d.path.as_deref().map(assets_dir));
    if !on {
        return Ok(json!({"enabled": false}));
    }
    match dir {
        Some(dir) if !cfg!(target_arch = "wasm32") => {
            let (files, errors) = generate_assets(&d.doc, &dir);
            Ok(json!({"enabled": true, "dir": dir, "files": files, "errors": errors}))
        }
        _ => Ok(json!({"enabled": true, "dir": null, "files": [], "note": "assets are written when the document is saved"})),
    }
}

/// After a save: regenerate the document's image assets when Generate › Image Assets is on.
pub fn on_saved(s: &Session, index: usize) -> Option<Value> {
    let d = s.documents().get(index)?;
    if !s.file_menu.image_assets.contains(&d.doc.id) || !s.prefs().plug_ins.generator_enabled || cfg!(target_arch = "wasm32") {
        return None;
    }
    let dir = assets_dir(d.path.as_deref()?);
    let (files, errors) = generate_assets(&d.doc, &dir);
    Some(json!({"dir": dir, "files": files, "errors": errors}))
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $sc:expr, $params:expr, $enabled:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: $sc, params: $params, enabled: $enabled, journal: true, run: $run }
        };
    }
    fn has_doc(s: &Session) -> std::result::Result<(), String> {
        s.active().map(|_| ()).ok_or_else(|| "no document open".into())
    }

    /// Generate ▸ Image Assets needs Preferences ▸ Plug-ins ▸ Generator enabled, as in
    /// Photoshop (where "Enable Generator" is unchecked until turned on).
    fn generator_enabled(s: &Session) -> std::result::Result<(), String> {
        has_doc(s)?;
        if !s.prefs().plug_ins.generator_enabled {
            return Err("Enable Generator in Preferences ▸ Plug-ins".into());
        }
        Ok(())
    }
    vec![
        spec!(
            "file.export.saveForWebLegacy",
            "Save for Web (Legacy)…",
            &["File", "Export"],
            Some("Cmd+Alt+Shift+S"),
            r##"{"preset":"GIF 128 Dithered|JPEG High|PNG-24|…"?,"format":"gif|png8|png24|jpeg|wbmp"="png24","palette":"blackWhite|perceptual|selective|adaptive|restrictive|exact|systemMac|systemWindows|uniform"="selective","paletteId":text,"paletteSnapshot":json,"mapping":json,"reducePalette":bool=false,"colors":2..256=256,"colorTable":["#rrggbb",…]? (fixed opaque source palette, 1–256 entries; 255 with transparency),"colorTableEdits":{"0":"#rrggbb",…}? (replace entries after indexing; requires colorTable),"dither":"none|diffusion|pattern|noise|bayer|clusteredDot|floydSteinberg|sierraLite|atkinson|jarvis|stucki|sierra|burkes|blueNoise|whiteNoise|riemersma"="diffusion","ditherAmount":0..100=88,"transparency":bool=true,"matte":"#rrggbb|none"="none" (a preset's own: white; formats without transparency go over white),"interlaced":bool=false,"webSnap":0..100=0,"quality":0..100=60,"progressive":bool=false,"optimized":bool=true,"embedIcc":bool=false,"metadata":"none|copyright|copyrightAndContact|allExceptCameraInfo|all"="copyright" (EXIF only with all),"convertToSrgb":bool=true,"width"|"height"|"percent"? (image size),"resample":"bicubic|bilinear|nearest"?,"path":file? (whole image),"dir":folder? (one file per slice in images/),"imagesFolder":relative-folder?="images","html":bool=false,"trustedSliceHtml":bool=false (only for trusted documents; emit raw no-image cell HTML),"slices":"all|user"="all","numbers":[n]?} → mapping uses shared Palette Map settings (method,amount,matrixSize,scale,seed,serpentine,history,decay,distance,alpha,patterns); paletteSnapshot overrides paletteId, preserves order/duplicates; oversized snapshots require explicit reducePalette; no path/dir: {bytes,width,height,colors,colorTable,transparentIndex} estimate; else {files,html,bytes,warnings}"##,
            has_doc,
            save_for_web
        ),
        spec!(
            "file.export.exportPreferences",
            "Export Preferences…",
            &["File", "Export"],
            None,
            r##"{"quickExportFormat":"png|jpg|gif|webp|exr"?,"quickExportLocation":"ask|sameFolder"?,"jpegQuality":1..100?,"webpLossless":bool?,"webpQuality":1..100?,"metadata":"none|copyright|all"?,"convertToSrgb":bool?} → {values}"##,
            |_| Ok(()),
            export_preferences
        ),
        spec!(
            "file.export.quickExport",
            "Quick Export",
            &[],
            None,
            r##"{"path":str? (required unless Export Preferences › Location is "sameFolder" and the document is saved)} → {path, format, bytes, warnings}"##,
            native_doc,
            quick_export
        ),
        spec!(
            "file.generate.imageAssets",
            "Image Assets",
            &["File", "Generate"],
            None,
            r##"{"on":bool? (default: toggle),"dir":folder? (default <document>-assets next to the file)} → {enabled, files, errors}; layers named like "foo.png", "200% foo@2x.png", "48x48 icons/a.png8", "photo.jpg80%" are exported, now and after each save"##,
            generator_enabled,
            image_assets
        ),
    ]
}

#[cfg(test)]
#[path = "web_cmds/tests.rs"]
mod tests;
