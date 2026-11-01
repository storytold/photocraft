//! Procreate brushes (`.brush`, `.brushset`) → brush presets. Layout observed from files
//! Procreate writes (no Procreate code used):
//!
//! - A `.brush` is a ZIP holding `Brush.archive`, the brush's `Shape.png`, its own `Grain.png`
//!   when it has one, and a `QuickLook` preview.
//! - A `.brushset` is a ZIP with one folder per brush, named by a UUID and laid out like a
//!   `.brush`, and an XML `brushset.plist` with the set's name and its brushes in order. A
//!   folder's `Reset/` copy (the brush as first made) is not read.
//! - `Brush.archive` is a binary property list written by `NSKeyedArchiver`: a `SilicaBrush`
//!   whose fields, mostly 0..1 numbers, follow Brush Studio's settings.
//! - Shape and grain images are grayscale: white paints, black doesn't (`shapeInverted` and
//!   `textureInverted` swap them).
//!
//! The shape becomes a sampled tip on its whole square, so spacing means the same in both
//! apps: on Procreate's own stroke previews the gaps match the stored spacing within 7 %.
//! Grain becomes a texture on every stamp, fixed to the canvas as Procreate's Texturized grain
//! is. Procreate stores sizes relative to its largest brush, not in pixels ([`SIZE_SCALE`]),
//! and Stroke Path › Jitter is taken as a scatter of that many tip radii; neither has been
//! measured against Procreate. Settings without a PhotoCraft equivalent are listed in
//! [`ProcreateImport::warnings`].

mod bplist;
#[cfg(test)]
mod tests;

use std::collections::{BTreeSet, HashMap};

use photocraft_codecs::{ChannelLayout, DecodeOptions, Image, Limits};
use photocraft_format::zip::{ZipReader, ZipWriter};
use photocraft_paint::{BrushPreset, BrushSettings, Control, GrayTile, MAX_BRUSH_SIZE, MAX_SCATTER, Pattern, ResponseCurve, TipShape};
use photocraft_raster::Interrupt;

use crate::abr_map::{MAX_PATTERN_EDGE, MAX_TIP_EDGE, fit};
use bplist::{Obj, Plist};

/// Pixels for Procreate's largest brush size (1.0). Procreate stores no pixel size; this keeps
/// imported brushes in a usable range, and the Size slider takes them from there.
pub const SIZE_SCALE: f32 = 1000.0;
const MAX_ARCHIVE: usize = 8 << 20;
const MAX_SET_PLIST: usize = 1 << 20;
const MAX_IMAGE_FILE: usize = 64 << 20;
const IMAGE_LIMITS: Limits = Limits { max_width: 8192, max_height: 8192, max_pixels: 1 << 24, max_alloc: 128 << 20 };
/// Tip and texture bytes one import keeps (16-bit samples); later brushes are left out.
const MAX_KEPT_BYTES: usize = if cfg!(target_pointer_width = "64") { 1 << 30 } else { 256 << 20 };
/// Unreadable brushes reported one by one; the rest are counted.
const MAX_NOTED: usize = 10;

/// The presets read from one file plus everything that could not be mapped.
#[derive(Clone, Debug, Default)]
pub struct ProcreateImport {
    pub presets: Vec<BrushPreset>,
    pub warnings: Vec<String>,
    /// The set's own name (`brushset.plist`), if it has one.
    pub set_name: Option<String>,
}

/// Read a `.brush` or `.brushset` into presets in group `group` (usually the file name).
pub fn read_procreate(bytes: &[u8], group: &str) -> Result<ProcreateImport, String> {
    read_procreate_with(bytes, group, &Interrupt::NONE)
}

/// [`read_procreate`] that checks `ctl` before each brush (and reports progress): a cancelled
/// import fails with "cancelled".
pub fn read_procreate_with(bytes: &[u8], group: &str, ctl: &Interrupt) -> Result<ProcreateImport, String> {
    let zip = ZipReader::new(bytes).map_err(|e| format!("not a Procreate brush file (.brush, .brushset): {e}"))?;
    let mut out = ProcreateImport::default();
    // A `.brush` keeps its files at the top; a `.brushset` has a folder per brush.
    let dirs = if zip.find("Brush.archive").is_some() { vec![String::new()] } else { brush_dirs(&zip, &mut out) };
    if dirs.is_empty() {
        return Err("the file holds no Procreate brushes".into());
    }
    let mut m = Mapper { zip: &zip, unmapped: BTreeSet::new(), kept: 0 };
    let mut unreadable = 0usize;
    for (i, dir) in dirs.iter().enumerate() {
        ctl.check().map_err(|e| e.to_string())?;
        ctl.progress(i as f32 / dirs.len() as f32);
        match m.brush(dir, i, &mut out.warnings) {
            Ok(Some(brush)) => out.presets.push(BrushPreset {
                name: brush.0,
                brush: brush.1,
                builtin: false,
                group: group.to_string(),
                folder: Vec::new(),
                captures: Default::default(),
            }),
            Ok(None) => {
                let n = out.presets.len();
                out.warnings.push(format!("the set's tips pass the {} MiB memory budget: {n} of {} brushes imported", MAX_KEPT_BYTES >> 20, dirs.len()));
                break;
            }
            Err(e) => {
                unreadable += 1;
                if unreadable <= MAX_NOTED {
                    out.warnings.push(format!("brush {}: {e}; skipped", i + 1));
                }
            }
        }
    }
    if unreadable > MAX_NOTED {
        out.warnings.push(format!("{} more brushes could not be read; skipped", unreadable - MAX_NOTED));
    }
    if out.presets.is_empty() {
        return Err(out.warnings.first().cloned().unwrap_or_else(|| "no brush in the file could be read".into()));
    }
    if !m.unmapped.is_empty() {
        out.warnings
            .push(format!("Procreate settings without a PhotoCraft equivalent were left out: {}", m.unmapped.into_iter().collect::<Vec<_>>().join(", ")));
    }
    ctl.progress(1.0);
    Ok(out)
}

/// The brush folders of a `.brushset` (`"<uuid>/"`), in the set's order.
fn brush_dirs(zip: &ZipReader, out: &mut ProcreateImport) -> Vec<String> {
    let has = |dir: &str| zip.find(&format!("{dir}/Brush.archive")).is_some();
    let listed = zip.find("brushset.plist").map(|e| zip.read(e, MAX_SET_PLIST).map_err(|e| e.to_string()).and_then(|x| set_plist(&x)));
    let mut seen = std::collections::HashSet::new();
    let mut dirs = Vec::new();
    match listed {
        Some(Ok((name, brushes))) => {
            out.set_name = name;
            let mut missing = Vec::new();
            for id in brushes {
                if id.is_empty() || id.contains('/') || !seen.insert(id.clone()) {
                    continue;
                }
                if has(&id) {
                    dirs.push(format!("{id}/"));
                } else {
                    missing.push(id);
                }
            }
            match &missing[..] {
                [] => {}
                [id] => out.warnings.push(format!("brush {id} is listed but not in the file; skipped")),
                ids => out.warnings.push(format!("{} listed brushes are not in the file; skipped", ids.len())),
            }
        }
        other => {
            if let Some(Err(e)) = other {
                out.warnings.push(format!("brushset.plist could not be read ({e}); brushes are in archive order"));
            }
            for e in &zip.entries {
                if let Some(dir) = e.name.strip_suffix("/Brush.archive").filter(|d| !d.is_empty() && !d.contains('/'))
                    && seen.insert(dir.to_string())
                {
                    dirs.push(format!("{dir}/"));
                }
            }
        }
    }
    dirs
}

/// `brushset.plist`: the set's name and its brush folders in order.
fn set_plist(bytes: &[u8]) -> Result<(Option<String>, Vec<String>), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "not UTF-8")?;
    // Procreate's DOCTYPE declares nothing; declared entities could expand far past the file's size.
    if text.contains("<!ENTITY") {
        return Err("entity declarations are not allowed".into());
    }
    let opts = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(text, opts).map_err(|e| e.to_string())?;
    let dict = doc.root_element().children().find(|n| n.has_tag_name("dict")).ok_or("no dict")?;
    let (mut name, mut brushes) = (None, Vec::new());
    let mut items = dict.children().filter(roxmltree::Node::is_element);
    while let Some(key) = items.next() {
        let Some(value) = items.next() else { break };
        match key.text() {
            Some("name") => name = value.text().map(str::to_string),
            Some("brushes") => brushes = value.children().filter(|n| n.has_tag_name("string")).filter_map(|n| n.text().map(str::to_string)).collect(),
            _ => {}
        }
    }
    Ok((name, brushes))
}

/// A keyed archive's root object (`NSKeyedArchiver`): its fields, read on demand.
struct Archive<'a> {
    plist: Plist<'a>,
    /// `$objects`, the archive's object table, as property-list object indices.
    objects: Vec<usize>,
    fields: HashMap<String, usize>,
}

impl<'a> Archive<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self, String> {
        let plist = Plist::parse(bytes)?;
        let not_archive = || "not a keyed archive".to_string();
        let Obj::Dict(top) = plist.object(plist.top())? else { return Err(not_archive()) };
        let get = |entries: &[(usize, usize)], key: &str| plist.lookup(entries, key).and_then(|i| plist.object(i).ok());
        if get(&top, "$archiver") != Some(Obj::Str("NSKeyedArchiver".into())) {
            return Err(not_archive());
        }
        let (Some(Obj::Array(objects)), Some(Obj::Dict(roots))) = (get(&top, "$objects"), get(&top, "$top")) else { return Err(not_archive()) };
        let Some(Obj::Uid(root)) = get(&roots, "root") else { return Err(not_archive()) };
        let root = usize::try_from(root).ok().and_then(|r| objects.get(r)).copied().ok_or_else(not_archive)?;
        let Obj::Dict(entries) = plist.object(root)? else { return Err(not_archive()) };
        let mut fields = HashMap::with_capacity(entries.len());
        for (k, v) in entries {
            if let Ok(Obj::Str(name)) = plist.object(k) {
                fields.entry(name).or_insert(v);
            }
        }
        Ok(Archive { plist, objects, fields })
    }

    /// An archive reference (UID) followed once; anything else as it is.
    fn deref(&self, o: Obj<'a>) -> Option<Obj<'a>> {
        match o {
            Obj::Uid(u) => self.plist.object(*self.objects.get(usize::try_from(u).ok()?)?).ok(),
            o => Some(o),
        }
    }

    fn get(&self, key: &str) -> Option<Obj<'a>> {
        self.deref(self.plist.object(*self.fields.get(key)?).ok()?)
    }

    fn num(&self, key: &str) -> Option<f32> {
        match self.get(key)? {
            Obj::Real(v) => Some(v),
            Obj::Int(v) => Some(v as f64),
            _ => None,
        }
        .filter(|v| v.is_finite())
        .map(|v| v as f32)
    }

    fn flag(&self, key: &str) -> bool {
        match self.get(key) {
            Some(Obj::Bool(b)) => b,
            Some(Obj::Int(v)) => v != 0,
            _ => false,
        }
    }

    fn text(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            Obj::Str(s) if s != "$null" => Some(s),
            Obj::Dict(d) => match self.plist.object(self.plist.lookup(&d, "NS.string")?).ok()? {
                Obj::Str(s) => Some(s),
                _ => None,
            },
            _ => None,
        }
    }

    /// A Pencil pressure curve: its `points`, each a `"{x, y}"` string.
    fn curve(&self, key: &str) -> Option<ResponseCurve> {
        let Obj::Dict(c) = self.get(key)? else { return None };
        let items = match self.deref(self.plist.object(self.plist.lookup(&c, "points")?).ok()?)? {
            Obj::Array(items) => items,
            Obj::Dict(array) => match self.plist.object(self.plist.lookup(&array, "NS.objects")?).ok()? {
                Obj::Array(items) => items,
                _ => return None,
            },
            _ => return None,
        };
        let points: Vec<[f32; 2]> = items
            .iter()
            .take(ResponseCurve::MAX_POINTS * 4)
            .filter_map(|&i| match self.deref(self.plist.object(i).ok()?)? {
                Obj::Str(s) => point(&s),
                _ => None,
            })
            .collect();
        (points.len() >= 2).then(|| ResponseCurve::new(&points))
    }
}

/// `"{0.5, 0.25}"` (a point written as a string) as `[x, y]`.
fn point(s: &str) -> Option<[f32; 2]> {
    let (x, y) = s.trim().strip_prefix('{')?.strip_suffix('}')?.split_once(',')?;
    Some([x.trim().parse().ok()?, y.trim().parse().ok()?])
}

fn ordered(a: f32, b: f32) -> (f32, f32) {
    (a.min(b), a.max(b))
}

/// Brush Studio settings onto [`BrushSettings`]; ones PhotoCraft can't reproduce go to
/// `unmapped` by their Brush Studio name.
fn settings(a: &Archive, unmapped: &mut BTreeSet<&'static str>) -> BrushSettings {
    let num = |k: &str| a.num(k).unwrap_or(0.0);
    let on = |k: &str| num(k).abs() > 1e-3;
    let mut b = BrushSettings { pressure_size: false, ..Default::default() };
    // The size and opacity limits bound the sidebar sliders.
    let (lo, hi) = ordered(a.num("minSize").unwrap_or(0.0), a.num("maxSize").unwrap_or(1.0));
    if let Some(size) = a.num("paintSize") {
        b.size = (size.clamp(lo, hi) * SIZE_SCALE).clamp(1.0, MAX_BRUSH_SIZE);
    }
    let (lo, hi) = ordered(a.num("minOpacity").unwrap_or(0.0), a.num("maxOpacity").unwrap_or(1.0));
    b.opacity = a.num("paintOpacity").unwrap_or(1.0).clamp(lo, hi).clamp(0.0, 1.0);
    // Stroke Path.
    b.spacing = a.num("plotSpacing").unwrap_or(b.spacing).clamp(0.01, 10.0);
    b.smoothing.amount = num("plotSmoothing").clamp(0.0, 1.0);
    let count = 1 + (num("shapeCount").clamp(0.0, 1.0) * 15.0).round() as u32;
    let count_jitter = num("shapeCountJitter").clamp(0.0, 1.0);
    if on("plotJitter") || count > 1 || count_jitter > 0.0 {
        let sc = &mut b.scattering;
        sc.enabled = true;
        sc.scatter.jitter = num("plotJitter").clamp(0.0, MAX_SCATTER);
        sc.both_axes = true;
        sc.count = count;
        sc.count_jitter.jitter = count_jitter;
    }
    // Shape: Rotation runs from -1 (against the stroke) through 0 (fixed) to 1 (with it).
    b.roundness = a.num("shapeRoundness").unwrap_or(1.0).clamp(0.01, 1.0);
    let sd = &mut b.shape_dynamics;
    let rotation = num("shapeRotation");
    if rotation >= 0.5 {
        sd.angle.control = Control::Direction;
    } else if rotation <= -0.5 {
        unmapped.insert("Rotation against the stroke");
    }
    if a.flag("shapeAzimuth") {
        if sd.angle.control == Control::Off {
            sd.angle.control = Control::PenTilt;
        } else {
            unmapped.insert("Azimuth with Rotation");
        }
    }
    // Scatter turns each stamp at random; its slider stores up to 2.
    sd.angle.jitter = (num("shapeScatter") / 2.0).clamp(0.0, 1.0);
    sd.flip_x_jitter = a.flag("shapeFlipXJitter");
    sd.flip_y_jitter = a.flag("shapeFlipYJitter");
    if a.flag("shapeRandomise") {
        unmapped.insert("Randomized");
    }
    // Apple Pencil pressure and per-stamp jitter.
    let pressure = num("dynamicsPressureSize").clamp(0.0, 1.0);
    if pressure > 0.0 {
        sd.size.control = Control::PenPressure;
        sd.size.minimum = 1.0 - pressure;
        sd.size.curve = a.curve("dynamicsPressureSizeCurve").unwrap_or(sd.size.curve);
    }
    sd.size.jitter = num("dynamicsJitterSize").clamp(0.0, 1.0);
    sd.enabled = sd.angle.is_active() || sd.size.is_active() || sd.flip_x_jitter || sd.flip_y_jitter;
    let t = &mut b.transfer;
    let pressure = num("dynamicsPressureOpacity").clamp(0.0, 1.0);
    if pressure > 0.0 {
        t.opacity.control = Control::PenPressure;
        t.opacity.minimum = 1.0 - pressure;
        t.opacity.curve = a.curve("dynamicsPressureOpacityCurve").unwrap_or(t.opacity.curve);
    }
    t.opacity.jitter = num("dynamicsJitterOpacity").clamp(0.0, 1.0);
    t.enabled = t.opacity.is_active();
    // Color Dynamics: per stamp, or per stroke when only that is set.
    let jitter = |prefix: &str| ["Hue", "Saturation", "Lightness", "Darkness"].map(|c| num(&format!("{prefix}{c}")).clamp(0.0, 1.0));
    let (stamp, stroke) = (jitter("dynamicsJitter"), jitter("dynamicsJitterStroke"));
    let any = |v: &[f32; 4]| v.iter().any(|x| *x > 1e-3);
    let (picked, per_tip) = if any(&stamp) { (stamp, true) } else { (stroke, false) };
    if any(&picked) {
        let c = &mut b.color_dynamics;
        c.enabled = true;
        c.per_tip = per_tip;
        c.hue_jitter = picked[0];
        c.saturation_jitter = picked[1];
        c.brightness_jitter = picked[2].max(picked[3]);
    }
    if any(&stamp) && any(&stroke) {
        unmapped.insert("Stroke Color Jitter");
    }
    // What PhotoCraft's brush engine has no counterpart for.
    for (keys, name) in [
        (&["jitterSecondary", "jitterStrokeSecondary"][..], "Secondary Color jitter"),
        (&["dynamicsSpeedSize", "dynamicsSpeedOpacity"], "Speed dynamics"),
        (&["dynamicsTiltSize", "dynamicsTiltOpacity"], "Tilt dynamics"),
        (&["dynamicsPressureHue", "dynamicsPressureSaturation", "dynamicsPressureBrightness"], "Color Pressure"),
        (&["dynamicsFalloff"], "Fall Off"),
        (&["dynamicsMix"], "Wet Mix"),
        (&["blendMode"], "Blend Mode"),
    ] {
        if keys.iter().any(|k| on(k)) {
            unmapped.insert(name);
        }
    }
    let taper = |p: &str| (on(&format!("{p}StartLength")) || on(&format!("{p}EndLength"))) && (on(&format!("{p}Size")) || on(&format!("{p}Opacity")));
    if taper("pencilTaper") {
        unmapped.insert("Pressure Taper");
    }
    if taper("taper") {
        unmapped.insert("Touch Taper");
    }
    b
}

/// `f(luminance, alpha)` for each pixel (Rec. 601 luminance), in one pass.
fn luma_alpha(img: &Image, f: impl Fn(f32, f32) -> f32) -> Result<Vec<f32>, String> {
    let (layout, n) = (img.layout(), img.pixel_count());
    let ch = layout.channels();
    let s = |p: usize, c: usize| img.sample_normalized(p * ch + c);
    let luma = |p: usize| 0.299 * s(p, 0) + 0.587 * s(p, 1) + 0.114 * s(p, 2);
    Ok(match layout {
        ChannelLayout::Gray => (0..n).map(|p| f(s(p, 0), 1.0)).collect(),
        ChannelLayout::GrayA => (0..n).map(|p| f(s(p, 0), s(p, 1))).collect(),
        ChannelLayout::Rgb => (0..n).map(|p| f(luma(p), 1.0)).collect(),
        ChannelLayout::Rgba => (0..n).map(|p| f(luma(p), s(p, 3))).collect(),
        _ => return Err("the image is not grayscale or RGB".into()),
    })
}

fn tile(w: u32, h: u32, v: &[f32]) -> GrayTile {
    GrayTile { width: w, height: h, data: v.iter().map(|x| (x.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16).collect() }
}

struct Mapper<'a> {
    zip: &'a ZipReader<'a>,
    unmapped: BTreeSet<&'static str>,
    /// Tip and texture bytes kept so far.
    kept: usize,
}

impl Mapper<'_> {
    fn image(&self, dir: &str, file: &str) -> Result<Option<Image>, String> {
        let Some(e) = self.zip.find(&format!("{dir}{file}")) else { return Ok(None) };
        let bytes = self.zip.read(e, MAX_IMAGE_FILE).map_err(|e| e.to_string())?;
        photocraft_codecs::decode_with(&bytes, &DecodeOptions { limits: IMAGE_LIMITS, keep_orientation: false }).map(Some).map_err(|e| format!("{file}: {e}"))
    }

    /// Keep a `w × h` 16-bit plane if the budget allows.
    fn keep(&mut self, w: u32, h: u32) -> bool {
        let bytes = (w as usize).saturating_mul(h as usize).saturating_mul(2);
        self.kept = self.kept.saturating_add(bytes);
        self.kept <= MAX_KEPT_BYTES
    }

    /// The brush in folder `dir` as a name and settings; `None` once the memory budget is spent.
    fn brush(&mut self, dir: &str, index: usize, notes: &mut Vec<String>) -> Result<Option<(String, BrushSettings)>, String> {
        let bytes = self.zip.read_by_name(&format!("{dir}Brush.archive"), MAX_ARCHIVE).map_err(|e| e.to_string())?;
        let a = Archive::parse(&bytes)?;
        let name = a.text("name").map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Brush {}", index + 1));
        let mut b = settings(&a, &mut self.unmapped);
        match self.image(dir, "Shape.png")? {
            Some(img) => {
                let invert = a.flag("shapeInverted");
                let v = luma_alpha(&img, |l, al| (if invert { 1.0 - l } else { l }) * al)?;
                let (iw, ih) = (img.width(), img.height());
                drop(img);
                let (w, h, v) = fit(iw, ih, v, MAX_TIP_EDGE);
                if !self.keep(w, h) {
                    return Ok(None);
                }
                b.tip = TipShape::Sampled(tile(w, h, &v));
            }
            None => match a.text("bundledShapePath") {
                Some(path) => notes.push(format!("\"{name}\" uses Procreate's built-in shape \"{path}\"; a round tip stands in")),
                None => notes.push(format!("\"{name}\" has no shape image; a round tip stands in")),
            },
        }
        match self.image(dir, "Grain.png") {
            Ok(Some(img)) => {
                // Transparent grain lets paint through: white, or black when the grain is inverted.
                let back = if a.flag("textureInverted") { 0.0 } else { 1.0 };
                let v = luma_alpha(&img, |l, al| l * al + back * (1.0 - al))?;
                let (iw, ih) = (img.width(), img.height());
                drop(img);
                let (w, h, v) = fit(iw, ih, v, MAX_PATTERN_EDGE);
                if !self.keep(w, h) {
                    return Ok(None);
                }
                let t = &mut b.texture;
                t.enabled = true;
                // The shape holds the grain: every stamp is textured, and stamps build up.
                t.each_tip = true;
                t.pattern = Pattern::Tile(tile(w, h, &v));
                t.invert = a.flag("textureInverted");
                // A grain downsampled to fit keeps its size on the canvas.
                t.scale = (a.num("textureScale").unwrap_or(1.0) * iw as f32 / w as f32).clamp(0.01, 10.0);
                t.depth = a.num("grainDepth").unwrap_or(1.0).clamp(0.0, 1.0);
                // Brightness is in levels of 255 (a Photoshop brush's -30 comes in as -30/255),
                // ours in units of 150 levels.
                t.brightness = (a.num("textureBrightness").unwrap_or(0.0) * 255.0 / 150.0).clamp(-1.0, 1.0);
                t.contrast = a.num("textureContrast").unwrap_or(0.0).clamp(-1.0, 1.0);
                if a.num("grainBlendMode").is_some_and(|m| m != 1.0) {
                    self.unmapped.insert("Grain Blend Mode");
                }
            }
            // Procreate's blank grain is no grain.
            Ok(None) => {
                if let Some(path) = a.text("bundledGrainPath").filter(|p| !p.contains("Blank")) {
                    notes.push(format!("\"{name}\" uses Procreate's built-in grain \"{path}\"; left out"));
                }
            }
            Err(e) => notes.push(format!("\"{name}\": its grain could not be read ({e}); left out")),
        }
        Ok(Some((name, b)))
    }
}

/// A field for [`write_archive`].
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Num(f64),
    Flag(bool),
    Text(String),
    /// Pencil pressure curve points.
    Curve(Vec<[f64; 2]>),
}

/// A `Brush.archive` with `fields`, in the shape Procreate writes. Exposed for tests.
#[doc(hidden)]
pub fn write_archive(fields: &[(&str, Field)]) -> Vec<u8> {
    use bplist::Value as V;
    let class = |name: &str| V::Dict(vec![("$classname".into(), V::Str(name.into())), ("$classes".into(), V::Array(vec![V::Str(name.into())]))]);
    // `$objects`: 0 is `$null`, 1 the brush, 2 its class; the brush refers to the rest by UID.
    let mut objects = vec![V::Str("$null".into()), V::Dict(Vec::new()), class("SilicaBrush")];
    let add = |objects: &mut Vec<V>, v: V| {
        objects.push(v);
        V::Uid(objects.len() as u64 - 1)
    };
    let mut brush = vec![("$class".to_string(), V::Uid(2))];
    for (k, f) in fields {
        let v = match f {
            Field::Num(n) => V::Real(*n),
            Field::Flag(b) => V::Bool(*b),
            Field::Text(s) => add(&mut objects, V::Str(s.clone())),
            Field::Curve(pts) => {
                let strings: Vec<V> = pts.iter().map(|p| add(&mut objects, V::Str(format!("{{{:.6}, {:.6}}}", p[0], p[1])))).collect();
                let array_class = add(&mut objects, class("NSArray"));
                let array = add(&mut objects, V::Dict(vec![("$class".into(), array_class), ("NS.objects".into(), V::Array(strings))]));
                let curve_class = add(&mut objects, class("ValkyrieMagnitudinalCurve"));
                add(&mut objects, V::Dict(vec![("$class".into(), curve_class), ("points".into(), array)]))
            }
        };
        brush.push(((*k).to_string(), v));
    }
    if let Some(slot) = objects.get_mut(1) {
        *slot = V::Dict(brush);
    }
    bplist::write(&V::Dict(vec![
        ("$version".into(), V::Int(100_000)),
        ("$archiver".into(), V::Str("NSKeyedArchiver".into())),
        ("$top".into(), V::Dict(vec![("root".into(), V::Uid(1))])),
        ("$objects".into(), V::Array(objects)),
    ]))
}

/// One brush of a [`write_brushset`] set: `Brush.archive` bytes, then optional `Shape.png` and
/// `Grain.png` bytes. Exposed for tests.
#[doc(hidden)]
pub type BrushFiles = (Vec<u8>, Option<Vec<u8>>, Option<Vec<u8>>);

/// A `.brushset` named `name` holding `brushes`, each in a folder named after its position.
/// Exposed for tests.
#[doc(hidden)]
pub fn write_brushset(name: &str, brushes: &[BrushFiles]) -> Vec<u8> {
    let ids: Vec<String> = (0..brushes.len()).map(|i| format!("00000000-0000-0000-0000-{i:012}")).collect();
    let list: String = ids.iter().map(|id| format!("\t\t<string>{id}</string>\n")).collect();
    let name = name.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>brushes</key>\n\t<array>\n{list}\t</array>\n\t<key>name</key>\n\t<string>{name}</string>\n</dict>\n</plist>\n"
    );
    let mut z = ZipWriter::new();
    let mut add = |path: &str, data: &[u8]| {
        let _ = z.add(path, data);
    };
    add("brushset.plist", plist.as_bytes());
    for (id, (archive, shape, grain)) in ids.iter().zip(brushes) {
        add(&format!("{id}/Brush.archive"), archive);
        if let Some(s) = shape {
            add(&format!("{id}/Shape.png"), s);
        }
        if let Some(g) = grain {
            add(&format!("{id}/Grain.png"), g);
        }
    }
    z.finish().unwrap_or_default()
}
