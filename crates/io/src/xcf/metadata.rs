//! Optional source metadata translated into ordinary native data and one-time view hints.
//! Wire fields follow the public XCF property layout; source-only settings stay inert.

use photocraft_color::Color;
use photocraft_doc::{AlphaChannel, ColorIndicates, Document};

use super::{Prop, Result, be_f32, be_i32, be_u32, conversion, parasites};
use crate::initial_view::{MAX_INITIAL_SAMPLERS, NativeInitialView};

fn word(props: &[Prop<'_>], kind: u32) -> Option<u32> {
    props.iter().rev().find(|p| p.kind == kind).and_then(|p| be_u32(p.data))
}

/// Channel colours are explicitly stored as nonlinear RGB, independently of image precision.
/// This is the same RGB colour value used by native channel overlays, not document samples.
pub(super) fn channel_overlay(channel: &mut AlphaChannel, props: &[Prop<'_>]) -> Vec<u32> {
    let mut handled = Vec::new();
    if let Some(p) = props.iter().rev().find(|p| p.kind == 16)
        && let Some(&[r, g, b]) = p.data.as_array::<3>()
    {
        channel.color = Color::rgb(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0);
        handled.push(16);
    }
    // A valid float property takes precedence over the compatibility byte property.
    if let Some(p) = props.iter().rev().find(|p| p.kind == 38)
        && p.data.len() == 12
    {
        let rgb = be_f32(p.data).zip(p.data.get(4..).and_then(be_f32)).zip(p.data.get(8..).and_then(be_f32));
        if let Some(((r, g), b)) = rgb.filter(|&((r, g), b)| [r, g, b].iter().all(|c| c.is_finite())) {
            channel.color = Color::rgb(r, g, b);
            handled.push(38);
        }
    }
    if let Some(v) = word(props, 6).filter(|v| *v <= 255) {
        channel.opacity = v as f32 / 255.0;
        handled.push(6);
    }
    if let Some(v) = props.iter().rev().find(|p| p.kind == 33).and_then(|p| be_f32(p.data)).filter(|v| v.is_finite() && (0.0..=1.0).contains(v)) {
        channel.opacity = v;
        handled.push(33);
    }
    if let Some(v) = word(props, 14).filter(|v| *v <= 1) {
        channel.indicates = if v == 0 { ColorIndicates::SelectedAreas } else { ColorIndicates::MaskedAreas };
        handled.push(14);
    }
    handled
}

pub(super) fn active(props: &[Prop<'_>], kind: u32) -> bool {
    props.iter().any(|p| p.kind == kind && p.data.is_empty())
}

pub(super) fn flag(props: &[Prop<'_>], kind: u32) -> bool {
    word(props, kind).is_some_and(|v| v != 0)
}

/// Map native document metadata without changing native DPI, unit, guide or sampler semantics.
#[allow(clippy::too_many_arguments)] // One reader-owned set of bounded outputs, without a new persistent model.
pub(super) fn image<'a>(
    doc: &mut Document,
    props: &[Prop<'a>],
    version: u32,
    view: &mut NativeInitialView,
    outcomes: &mut conversion::Outcomes<'a>,
    owner: usize,
    budget: &mut usize,
) -> Result<()> {
    use photocraft_doc::import_conversion::ConversionReason as Reason;
    let mut metadata_incomplete = false;
    for p in props {
        match p.kind {
            18 => {
                let (guides, tail) = p.data.as_chunks::<5>();
                let mut skipped = !tail.is_empty();
                let mut limited = false;
                for &[a, b, c, d, orientation] in guides {
                    let position = i32::from_be_bytes([a, b, c, d]);
                    let limit = match orientation {
                        1 => doc.size.height,
                        2 => doc.size.width,
                        _ => {
                            skipped = true;
                            continue;
                        }
                    };
                    if version < 15 && (position < 0 || i64::from(position) > i64::from(limit)) {
                        skipped = true;
                        continue;
                    }
                    if !charge(budget, 8) {
                        skipped = true;
                        limited = true;
                        break;
                    }
                    let target = if orientation == 1 { &mut doc.guides.horizontal } else { &mut doc.guides.vertical };
                    target.push(f64::from(position));
                }
                if skipped {
                    outcomes.detail(
                        owner,
                        "xcf.guides",
                        if limited { Reason::RetentionLimit } else { Reason::InvalidOptionalData },
                        "Unknown/truncated guides, legacy off-canvas positions or metadata allowance exceeded".into(),
                        "Documented valid native guide positions imported within allowance; raw source property retained within report limits".into(),
                    )?;
                }
            }
            19 => {
                if let Some((x, y)) = be_f32(p.data).zip(p.data.get(4..).and_then(be_f32))
                    && x.is_finite()
                    && x > 0.0
                    && x < 1.0e6
                    && y.is_finite()
                    && y > 0.0
                    && y < 1.0e6
                {
                    doc.resolution_dpi = x;
                    if x != y {
                        outcomes.detail(
                            owner,
                            "xcf.resolution",
                            Reason::NativeLimitation,
                            format!("Horizontal {x} DPI; vertical {y} DPI"),
                            "Native document uses horizontal DPI for both axes; source axis values retained".into(),
                        )?;
                    }
                } else {
                    outcomes.detail(
                        owner,
                        "xcf.resolution",
                        Reason::InvalidOptionalData,
                        "Invalid or truncated source resolution".into(),
                        "Existing native DPI retained; source property remains inert".into(),
                    )?;
                }
            }
            22 | 24 => {
                outcomes.detail(
                    owner,
                    "xcf.unit",
                    Reason::NativeLimitation,
                    "Source image display unit or custom unit definition".into(),
                    "Native document has no per-document unit; native ruler and DPI semantics retained".into(),
                )?;
            }
            21 => {
                for (name, data) in parasites(p.data) {
                    match name.as_str() {
                        "gimp-comment" => {
                            if let Ok(text) = std::str::from_utf8(data.strip_suffix(&[0]).unwrap_or(&data))
                                && !text.is_empty()
                            {
                                metadata_incomplete |= !push_text(doc, "Comment", text, budget);
                            }
                        }
                        "exif-data" => {
                            let raw = data.strip_prefix(b"Exif\0\0").unwrap_or(&data);
                            // Native EXIF is a TIFF payload. Other legacy container layouts stay inert.
                            if (raw.starts_with(b"II\x2a\0") || raw.starts_with(b"MM\0\x2a")) && charge(budget, raw.len()) {
                                doc.metadata.exif = Some(std::sync::Arc::new(raw.to_vec()));
                            }
                        }
                        "gimp-image-metadata" | "gimp-metadata" => {
                            if let Ok(xml) = std::str::from_utf8(data.strip_suffix(&[0]).unwrap_or(&data))
                                && let Ok(tree) = parse_xml(xml)
                                && tree.root_element().has_tag_name("metadata")
                            {
                                for node in tree.root_element().children().filter(|n| n.has_tag_name("tag")) {
                                    if node.attribute("encoding").is_some() {
                                        metadata_incomplete = true;
                                        continue;
                                    }
                                    if let (Some(key), Some(value)) = (node.attribute("name"), node.text())
                                        && matches!(key.split('.').next(), Some("Exif" | "Xmp" | "Iptc"))
                                    {
                                        metadata_incomplete |= !push_text(doc, key, value, budget);
                                    }
                                }
                                let packets = super::metadata_packets::translate(doc, &tree, budget);
                                outcomes.detail(owner, "xcf.metadata", Reason::NativePolicy,
                                    "Serialized source EXIF/XMP/IPTC tag values".into(),
                                    format!("Plain tags retained as native image text. Known unambiguous schema fields translated into native XMP packet: {}; native EXIF packet: {}. Existing raw packets take priority. Unknown, binary or ambiguous source structures remain inert; full packet equivalence is not claimed", packets.0, packets.1))?;
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    if metadata_incomplete {
        outcomes.detail(
            owner,
            "xcf.metadata_partial",
            Reason::MissingMapping,
            "Source binary/oversized metadata tags or shared native metadata allowance exceeded".into(),
            "Representable plain tags imported within native allowance; remaining source metadata retained inert subject to report limits".into(),
        )?;
    }
    // New records take precedence over compatibility old records regardless of property order.
    let modern = props.iter().rev().find(|p| p.kind == 39);
    let samples = modern.or_else(|| props.iter().rev().find(|p| p.kind == 27));
    if let Some(p) = samples {
        let stride = if p.kind == 39 { 20 } else { 8 };
        for record in p.data.chunks_exact(stride) {
            if let Some((x, y)) = be_i32(record).zip(record.get(4..).and_then(be_i32))
                && x >= 0
                && y >= 0
                && (x as u32) < doc.size.width
                && (y as u32) < doc.size.height
                && view.samplers.len() < MAX_INITIAL_SAMPLERS
            {
                view.samplers.push([f64::from(x), f64::from(y)]);
            }
        }
        outcomes.detail(
            owner,
            "xcf.sample_points",
            Reason::NativeLimitation,
            "Source sample positions, pick modes and padding; original records retained inert".into(),
            format!(
                "{} valid positions handed to session-only native samplers (capacity 10); native sampling modes/history/save rules apply",
                view.samplers.len()
            ),
        )?;
    }
    Ok(())
}

fn parse_xml(xml: &str) -> std::result::Result<roxmltree::Document<'_>, roxmltree::Error> {
    roxmltree::Document::parse_with_options(xml, roxmltree::ParsingOptions { nodes_limit: 4096, ..Default::default() })
}

fn charge(budget: &mut usize, amount: usize) -> bool {
    let Some(remaining) = budget.checked_sub(amount) else {
        return false;
    };
    *budget = remaining;
    true
}

fn push_text(doc: &mut Document, key: &str, value: &str, budget: &mut usize) -> bool {
    if key.len() > super::MAX_STRING || value.len() > super::MAX_STRING || !charge(budget, key.len().saturating_add(value.len())) {
        return false;
    }
    // Duplicate tag values are meaningful (XMP bags, sequences and language alternatives).
    doc.metadata.text.push((key.to_owned(), value.to_owned()));
    true
}
