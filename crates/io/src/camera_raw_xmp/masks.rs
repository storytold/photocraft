//! Camera Raw masks in settings files.
//!
//! Masks Photoshop reads from an `.xmp` are written in its local-correction layout (observed in
//! Lightroom / Camera Raw sidecars, and checked by opening such files in Photoshop 2026):
//! `crs:GradientBasedCorrections` (linear gradients: `ZeroX/ZeroY/FullX/FullY`),
//! `crs:CircularGradientBasedCorrections` (radial: the ellipse's `Top/Left/Bottom/Right`,
//! `Angle`, `Feather`, `Flipped` = effect inside) and `crs:PaintBasedCorrections` (brush:
//! `Radius` as a fraction of the long side, `Flow`, `CenterWeight` = 1 − feather, `Dabs` as
//! `d x y`), each correction with its local sliders (`LocalExposure2012` in quarter stops of
//! the ±4 EV range, the others in hundredths). A mask that layout cannot hold (colour or
//! luminance ranges, AI kinds, subtract / intersect, inverted parts) is written only in
//! PhotoCraft's own attribute, `pcraft:CameraRawMasks` (the masks as JSON), which also keeps the
//! exact PhotoCraft settings of every mask; when present it wins on load.

use photocraft_algo::camera_raw::masks::{self, Component, Dab, Local, Mask, Op, Shape};

use super::{CRS_NS, RDF_NS, escape};

/// PhotoCraft's namespace in settings files.
pub const PCRAFT_NS: &str = "http://ns.photocraft.app/camera-raw/1.0/";

/// Local slider names: `crs:` key, field, and how many slider units one XMP unit is.
type LocalField = fn(&mut Local) -> &mut f32;
const LOCALS: &[(&str, LocalField, f32)] = &[
    ("LocalTemperature", |l| &mut l.temperature, 100.0),
    ("LocalTint", |l| &mut l.tint, 100.0),
    ("LocalExposure2012", |l| &mut l.exposure, 4.0),
    ("LocalContrast2012", |l| &mut l.contrast, 100.0),
    ("LocalHighlights2012", |l| &mut l.highlights, 100.0),
    ("LocalShadows2012", |l| &mut l.shadows, 100.0),
    ("LocalWhites2012", |l| &mut l.whites, 100.0),
    ("LocalBlacks2012", |l| &mut l.blacks, 100.0),
    ("LocalTexture", |l| &mut l.texture, 100.0),
    ("LocalClarity2012", |l| &mut l.clarity, 100.0),
    ("LocalDehaze", |l| &mut l.dehaze, 100.0),
    ("LocalHue", |l| &mut l.hue, 100.0),
    ("LocalSaturation", |l| &mut l.saturation, 100.0),
    ("LocalSharpness", |l| &mut l.sharpness, 100.0),
    ("LocalLuminanceNoise", |l| &mut l.noise, 100.0),
    ("LocalMoire", |l| &mut l.moire, 100.0),
    ("LocalDefringe", |l| &mut l.defringe, 100.0),
    ("LocalToningHue", |l| &mut l.color_hue, 1.0),
    ("LocalToningSaturation", |l| &mut l.color_sat, 100.0),
];

const KINDS: [(&str, &str); 3] =
    [("GradientBasedCorrections", "Mask/Gradient"), ("CircularGradientBasedCorrections", "Mask/CircularGradient"), ("PaintBasedCorrections", "Mask/Paint")];

/// Whether the structured `crs:` element `name` holds corrections this module reads.
pub fn is_corrections(name: &str) -> bool {
    KINDS.iter().any(|(k, _)| *k == name)
}

fn num(s: Option<&str>) -> Option<f32> {
    let s = s?.trim();
    let v: f32 = s.strip_prefix('+').unwrap_or(s).parse().ok()?;
    v.is_finite().then_some(v)
}

fn crs<'a>(n: roxmltree::Node<'a, '_>, key: &str) -> Option<&'a str> {
    n.attribute((CRS_NS, key)).or_else(|| n.children().find(|c| c.is_element() && c.tag_name().namespace() == Some(CRS_NS) && c.tag_name().name() == key).and_then(|c| c.text()))
}

/// The `rdf:li` items of the `rdf:Seq` under `n`, each as the node carrying its properties
/// (the `rdf:li` itself or its `rdf:Description`).
fn items<'a, 'i>(n: roxmltree::Node<'a, 'i>) -> Vec<roxmltree::Node<'a, 'i>> {
    let Some(seq) = n.children().find(|c| c.is_element() && c.tag_name().namespace() == Some(RDF_NS) && c.tag_name().name() == "Seq") else { return Vec::new() };
    seq.children()
        .filter(|c| c.is_element() && c.tag_name().name() == "li")
        .map(|li| li.children().find(|c| c.is_element() && c.tag_name().namespace() == Some(RDF_NS) && c.tag_name().name() == "Description").unwrap_or(li))
        .collect()
}

fn child<'a, 'i>(n: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    n.children().find(|c| c.is_element() && c.tag_name().namespace() == Some(CRS_NS) && c.tag_name().name() == name)
}

/// The masks in one `crs:…BasedCorrections` element (one mask per correction).
pub fn read(n: roxmltree::Node<'_, '_>, out: &mut Vec<Mask>) {
    for corr in items(n).into_iter().take(masks::MAX_MASKS) {
        if crs(corr, "CorrectionActive").is_some_and(|v| v.eq_ignore_ascii_case("false")) {
            continue;
        }
        let mut adjust = Local::default();
        for (key, field, scale) in LOCALS {
            if let Some(v) = num(crs(corr, key)) {
                *field(&mut adjust) = if *scale == 1.0 { v.rem_euclid(360.0) } else { (v * scale).clamp(-100.0, 100.0) };
            }
        }
        let amount = num(crs(corr, "CorrectionAmount")).map_or(100.0, |a| (a * 100.0).clamp(0.0, 200.0));
        let mut components = Vec::new();
        let mut brush: Vec<Dab> = Vec::new();
        for m in child(corr, "CorrectionMasks").map(items).unwrap_or_default().into_iter().take(masks::MAX_COMPONENTS) {
            let what = crs(m, "What").unwrap_or("");
            let value = num(crs(m, "MaskValue")).unwrap_or(1.0);
            let shape = match what {
                "Mask/Gradient" => {
                    let p = |k| num(crs(m, k)).unwrap_or(0.0);
                    Some(Shape::Linear { zero: [p("ZeroX"), p("ZeroY")], full: [p("FullX"), p("FullY")] })
                }
                "Mask/CircularGradient" => {
                    let p = |k, d| num(crs(m, k)).unwrap_or(d);
                    let (t, l, b, r) = (p("Top", 0.25), p("Left", 0.25), p("Bottom", 0.75), p("Right", 0.75));
                    let inside = crs(m, "Flipped").is_some_and(|v| v.eq_ignore_ascii_case("true"));
                    let shape = Shape::Radial {
                        center: [(l + r) / 2.0, (t + b) / 2.0],
                        radius: [((r - l) / 2.0).abs(), ((b - t) / 2.0).abs()],
                        angle: p("Angle", 0.0),
                        feather: p("Feather", 50.0).clamp(0.0, 100.0),
                    };
                    // Camera Raw's radial gradient acts outside the ellipse unless flipped.
                    components.push(Component { op: Op::Add, invert: !inside, shape });
                    None
                }
                "Mask/Paint" => {
                    let size = num(crs(m, "Radius")).unwrap_or(0.05).clamp(0.0, masks::MAX_DAB_SIZE);
                    let flow = num(crs(m, "Flow")).unwrap_or(1.0).clamp(0.0, 1.0) * 100.0;
                    let feather = (1.0 - num(crs(m, "CenterWeight")).unwrap_or(0.0).clamp(0.0, 1.0)) * 100.0;
                    for d in child(m, "Dabs").map(items).unwrap_or_default() {
                        if brush.len() >= masks::MAX_DABS {
                            break;
                        }
                        let text = d.text().unwrap_or("");
                        let mut parts = text.split_whitespace();
                        if parts.next() != Some("d") {
                            continue;
                        }
                        let (Some(x), Some(y)) = (num(parts.next()), num(parts.next())) else { continue };
                        brush.push(Dab { x, y, size, feather, flow, density: value.clamp(0.0, 1.0) * 100.0, erase: value <= 0.0, auto: false });
                    }
                    None
                }
                _ => None,
            };
            if let Some(shape) = shape {
                components.push(Component { op: Op::Add, invert: false, shape });
            }
        }
        if !brush.is_empty() {
            components.push(Component { op: Op::Add, invert: false, shape: Shape::Brush { dabs: brush } });
        }
        if !components.is_empty() && out.len() < masks::MAX_MASKS {
            out.push(Mask { name: format!("Mask {}", out.len() + 1), amount, components, adjust, ..Mask::default() });
        }
    }
}

/// Reads `pcraft:CameraRawMasks`, if valid.
pub fn read_own(json: &str) -> Option<Vec<Mask>> {
    let v: Vec<Mask> = serde_json::from_str(json).ok()?;
    masks::validate(&v).ok()?;
    Some(v)
}

fn locals(l: &Local) -> String {
    let mut l = l.clone();
    LOCALS
        .iter()
        .filter_map(|(key, field, scale)| {
            let v = *field(&mut l);
            (v != 0.0 && v.is_finite()).then(|| format!(" crs:{key}=\"{:.4}\"", v / scale))
        })
        .collect()
}

/// The layout a mask can be written in, if Photoshop's: its kind, if every component is that
/// kind, added and not inverted (radial ones may be inverted: that is Camera Raw's default).
fn classic(m: &Mask) -> Option<usize> {
    if m.invert || !m.visible || m.components.is_empty() || m.components.iter().any(|c| c.op != Op::Add) {
        return None;
    }
    let kind = |c: &Component| match c.shape {
        Shape::Linear { .. } if !c.invert => Some(0),
        Shape::Radial { .. } => Some(1),
        Shape::Brush { .. } if !c.invert => Some(2),
        _ => None,
    };
    let first = kind(&m.components[0])?;
    m.components.iter().all(|c| kind(c) == Some(first)).then_some(first)
}

/// Photoshop-readable corrections for the masks that fit its layout, as child elements of
/// the settings' `rdf:Description`.
pub fn write(all: &[Mask]) -> String {
    let mut by_kind: [Vec<String>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for m in all {
        let Some(k) = classic(m) else { continue };
        let mut parts = String::new();
        for c in &m.components {
            match &c.shape {
                Shape::Linear { zero, full } => parts.push_str(&format!(
                    "<rdf:li><rdf:Description crs:What=\"Mask/Gradient\" crs:MaskValue=\"1\" crs:ZeroX=\"{:.6}\" crs:ZeroY=\"{:.6}\" crs:FullX=\"{:.6}\" crs:FullY=\"{:.6}\"/></rdf:li>",
                    zero[0], zero[1], full[0], full[1]
                )),
                Shape::Radial { center, radius, angle, feather } => parts.push_str(&format!(
                    "<rdf:li><rdf:Description crs:What=\"Mask/CircularGradient\" crs:MaskValue=\"1\" crs:Top=\"{:.6}\" crs:Left=\"{:.6}\" crs:Bottom=\"{:.6}\" crs:Right=\"{:.6}\" crs:Angle=\"{:.4}\" crs:Midpoint=\"50\" crs:Roundness=\"0\" crs:Feather=\"{:.0}\" crs:Flipped=\"{}\" crs:Version=\"2\"/></rdf:li>",
                    center[1] - radius[1], center[0] - radius[0], center[1] + radius[1], center[0] + radius[0], angle, feather, !c.invert
                )),
                Shape::Brush { dabs } => {
                    // One Mask/Paint per run of dabs with the same brush.
                    let mut i = 0;
                    while i < dabs.len() {
                        let d0 = dabs[i];
                        let same = |d: &Dab| d.size == d0.size && d.feather == d0.feather && d.flow == d0.flow && d.density == d0.density && d.erase == d0.erase;
                        let mut li = String::new();
                        while i < dabs.len() && same(&dabs[i]) {
                            li.push_str(&format!("<rdf:li>d {:.6} {:.6}</rdf:li>", dabs[i].x, dabs[i].y));
                            i += 1;
                        }
                        let value = if d0.erase { 0.0 } else { d0.density / 100.0 };
                        parts.push_str(&format!(
                            "<rdf:li><rdf:Description crs:What=\"Mask/Paint\" crs:MaskValue=\"{value:.4}\" crs:Radius=\"{:.6}\" crs:Flow=\"{:.4}\" crs:CenterWeight=\"{:.4}\"><crs:Dabs><rdf:Seq>{li}</rdf:Seq></crs:Dabs></rdf:Description></rdf:li>",
                            d0.size, d0.flow / 100.0, 1.0 - d0.feather / 100.0
                        ));
                    }
                }
                _ => {}
            }
        }
        by_kind[k].push(format!(
            "<rdf:li><rdf:Description crs:What=\"Correction\" crs:CorrectionAmount=\"{:.4}\" crs:CorrectionActive=\"true\"{}><crs:CorrectionMasks><rdf:Seq>{parts}</rdf:Seq></crs:CorrectionMasks></rdf:Description></rdf:li>",
            m.amount / 100.0,
            locals(&m.adjust)
        ));
    }
    let mut out = String::new();
    for (k, list) in by_kind.iter().enumerate() {
        if !list.is_empty() {
            let name = KINDS[k].0;
            out.push_str(&format!("   <crs:{name}><rdf:Seq>{}</rdf:Seq></crs:{name}>\n", list.concat()));
        }
    }
    out
}

/// `pcraft:CameraRawMasks="…"` with every mask, or nothing when there are none.
pub fn own_attribute(all: &[Mask]) -> String {
    if all.is_empty() {
        return String::new();
    }
    let json = serde_json::to_string(all).unwrap_or_default();
    format!("   pcraft:CameraRawMasks=\"{}\"\n", escape(&json))
}

#[cfg(test)]
mod tests;
