//! Optional source metadata translated into ordinary native data and one-time view hints.
//! Wire fields follow the public XCF property layout; source-only settings stay inert.

use photocraft_color::Color;
use photocraft_doc::{AlphaChannel, ColorIndicates, Document};

use super::{Prop, Result, be_f32, be_i32, be_u32, conversion};
use crate::initial_view::{MAX_INITIAL_SAMPLERS, NativeInitialView};

#[cfg(test)]
mod resolution_review_tests;
#[cfg(test)]
mod text_storage_review_tests;

/// Decode the last matching metadata property as an unsigned word; malformed payloads yield no value.
fn word(props: &[Prop<'_>], kind: u32) -> Option<u32> {
    props.iter().rev().find(|p| p.kind == kind).and_then(|p| be_u32(p.data))
}

/// Validate both source axes together; a malformed optional pair cannot change native DPI.
fn resolution_pair(data: &[u8]) -> Option<(f32, f32)> {
    be_f32(data).zip(data.get(4..).and_then(be_f32)).filter(|(x, y)| [*x, *y].iter().all(|axis| axis.is_finite() && *axis > 0.0 && *axis < 1.0e6))
}

/// Source properties apply in order: the last valid pair wins, while invalid pairs leave the earlier DPI intact.
/// Text decoding and document metadata must share this final resolution and validation policy.
pub(super) fn image_resolution(props: &[Prop<'_>], fallback: f32) -> f32 {
    effective_resolution(props).map(|(x, _)| x).unwrap_or(fallback)
}

/// Resolve the same final valid pair for native DPI and its physical-size warning.
fn effective_resolution(props: &[Prop<'_>]) -> Option<(f32, f32)> {
    props.iter().filter(|property| property.kind == 19).filter_map(|property| resolution_pair(property.data)).next_back()
}

/// PROP_UNIT controls source dialog display only; documented physical units do not change native DPI.
pub(super) fn unit_reason(data: &[u8]) -> photocraft_doc::import_conversion::ConversionReason {
    use photocraft_doc::import_conversion::ConversionReason as Reason;
    if data.len() != 4 {
        Reason::InvalidOptionalData
    } else if be_u32(data).is_some_and(|unit| (1..=4).contains(&unit)) {
        Reason::NativePolicy
    } else {
        Reason::NativeLimitation
    }
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

/// Recognize a source active-item marker only when its declared payload is empty.
pub(super) fn active(props: &[Prop<'_>], kind: u32) -> bool {
    props.iter().any(|p| p.kind == kind && p.data.is_empty())
}

/// Interpret the last decodable metadata word as a nonzero flag without inventing an absent value.
pub(super) fn flag(props: &[Prop<'_>], kind: u32) -> bool {
    word(props, kind).is_some_and(|v| v != 0)
}

struct MetadataParasite<'a> {
    name: &'static str,
    ordinal: usize,
    data: Option<&'a [u8]>,
}

/// Resolve only supported names without an owned name index. Keep first-seen processing
/// order and final duplicate values, even after arbitrarily many unused bounded records.
fn metadata_parasites<'a>(data: &'a [u8], ctl: &photocraft_raster::Interrupt<'_>) -> Result<([MetadataParasite<'a>; 4], bool)> {
    let mut selected =
        ["gimp-comment", "exif-data", "gimp-image-metadata", "gimp-metadata"].map(|name| MetadataParasite { name, ordinal: usize::MAX, data: None });
    let mut ordinal = 0usize;
    let malformed = super::wire::scan_parasites(data, ctl, |name, data| {
        for slot in &mut selected {
            if name == slot.name.as_bytes() {
                if slot.data.is_none() {
                    slot.ordinal = ordinal;
                }
                slot.data = Some(data);
            }
        }
        ordinal = ordinal.saturating_add(1);
    })?;
    selected.sort_by_key(|slot| slot.ordinal);
    Ok((selected, malformed))
}

fn image_guides(
    doc: &mut Document,
    property: &Prop<'_>,
    version: u32,
    outcomes: &mut conversion::Outcomes<'_>,
    owner: usize,
    budget: &mut usize,
) -> Result<()> {
    use photocraft_doc::import_conversion::ConversionReason as Reason;
    let (guides, tail) = property.data.as_chunks::<5>();
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
    Ok(())
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
    warnings: &mut crate::ora::Warnings,
    ctl: &photocraft_raster::Interrupt<'_>,
) -> Result<()> {
    use photocraft_doc::import_conversion::ConversionReason as Reason;
    doc.resolution_dpi = image_resolution(props, doc.resolution_dpi);
    // Only the effective pair describes the admitted document's physical size.
    // This warning must survive optional report truncation and invalid trailing pairs.
    if let Some((x, y)) = effective_resolution(props)
        && let Some(warning) = crate::unequal_resolution_warning(f64::from(x), f64::from(y))
    {
        warnings.note(&warning);
    }
    let mut text_storage = TextStorage::default();
    let mut metadata_incomplete = false;
    let mut retention_limited = false;
    for p in props {
        match p.kind {
            18 => image_guides(doc, p, version, outcomes, owner, budget)?,
            19 => {
                if let Some((x, y)) = resolution_pair(p.data) {
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
            22 if unit_reason(p.data) == Reason::NativePolicy => {}
            22 | 24 => {
                outcomes.detail(
                    owner,
                    "xcf.unit",
                    if p.kind == 22 { unit_reason(p.data) } else { Reason::NativeLimitation },
                    "Source image display unit or custom unit definition".into(),
                    "Native document has no per-document unit; native ruler and DPI semantics retained".into(),
                )?;
            }
            21 => {
                let (selected, malformed) = metadata_parasites(p.data, ctl)?;
                if malformed {
                    outcomes.notice(
                        owner,
                        "xcf.parasite_framing",
                        Reason::InvalidOptionalData,
                        (
                            "Truncated or malformed optional parasite record".into(),
                            "Valid preceding metadata imported; remaining source configuration retained inert within report limits".into(),
                        ),
                        Some("has malformed optional metadata; valid preceding records were preserved"),
                        warnings,
                    )?;
                }
                for MetadataParasite { name, data, .. } in selected {
                    let Some(data) = data else { continue };
                    ctl.check().map_err(|_| super::IoError::Cancelled)?;
                    match name {
                        "gimp-comment" => {
                            if let Ok(text) = std::str::from_utf8(data.strip_suffix(&[0]).unwrap_or(data))
                                && !text.is_empty()
                            {
                                let omitted = !push_text(doc, "Comment", text, &mut text_storage, budget);
                                metadata_incomplete |= omitted;
                                retention_limited |= omitted;
                            }
                        }
                        "exif-data" => {
                            let raw = data.strip_prefix(b"Exif\0\0").unwrap_or(data);
                            // Native EXIF is a TIFF payload. Other legacy container layouts stay inert.
                            if raw.starts_with(b"II\x2a\0") || raw.starts_with(b"MM\0\x2a") {
                                if charge(budget, raw.len()) {
                                    doc.metadata.exif = Some(std::sync::Arc::new(raw.to_vec()));
                                } else {
                                    metadata_incomplete = true;
                                    retention_limited = true;
                                }
                            }
                        }
                        "gimp-image-metadata" | "gimp-metadata" => {
                            // Refuse oversized optional XML before its parser allocates a node table.
                            if data.len() > *budget {
                                metadata_incomplete = true;
                                retention_limited = true;
                                continue;
                            }
                            if let Ok(xml) = std::str::from_utf8(data.strip_suffix(&[0]).unwrap_or(data))
                                && let Ok(tree) = parse_xml(xml)
                                && tree.root_element().has_tag_name("metadata")
                            {
                                for node in tree.root_element().children().filter(|n| n.has_tag_name("tag")) {
                                    ctl.check().map_err(|_| super::IoError::Cancelled)?;
                                    if node.attribute("encoding").is_some() {
                                        metadata_incomplete = true;
                                        continue;
                                    }
                                    if let (Some(key), Some(value)) = (node.attribute("name"), node.text())
                                        && matches!(key.split('.').next(), Some("Exif" | "Xmp" | "Iptc"))
                                    {
                                        let omitted = !push_text(doc, key, value, &mut text_storage, budget);
                                        metadata_incomplete |= omitted;
                                        retention_limited |= omitted;
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
        outcomes.notice(
            owner,
            "xcf.metadata_partial",
            if retention_limited { Reason::RetentionLimit } else { Reason::MissingMapping },
            (
                "Source binary/oversized metadata tags or shared native metadata allowance exceeded".into(),
                "Representable plain tags imported within native allowance; remaining source metadata retained inert subject to report limits".into(),
            ),
            Some("has optional metadata that could not be retained within native metadata limits; usable image content was preserved"),
            warnings,
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

/// Parse bounded optional metadata XML with the parser defaults that reject DTD processing.
fn parse_xml(xml: &str) -> std::result::Result<roxmltree::Document<'_>, roxmltree::Error> {
    roxmltree::Document::parse_with_options(xml, roxmltree::ParsingOptions { nodes_limit: 4096, ..Default::default() })
}

/// Debit optional metadata bytes only when the complete requested amount fits.
fn charge(budget: &mut usize, amount: usize) -> bool {
    let Some(remaining) = budget.checked_sub(amount) else {
        return false;
    };
    *budget = remaining;
    true
}

/// Slots already admitted during this image's metadata traversal. Capacity is charged,
/// not just length, so short duplicate entries and spare vector slots cannot evade the cap.
#[derive(Default)]
struct TextStorage {
    charged_capacity: usize,
}

/// Copy into fallibly allocated staging storage without mutating the admitted document.
fn staged_text(value: &str) -> Option<String> {
    let mut text = String::new();
    text.try_reserve_exact(value.len()).ok()?;
    text.push_str(value);
    Some(text)
}

/// Admit all newly retained string and pair-vector storage before committing an entry.
/// Duplicates remain separate: XMP bags, sequences and language alternatives need them.
fn push_text(doc: &mut Document, key: &str, value: &str, storage: &mut TextStorage, budget: &mut usize) -> bool {
    if key.len() > super::MAX_STRING || value.len() > super::MAX_STRING {
        return false;
    }
    let entries = &mut doc.metadata.text;
    let Some(length) = entries.len().checked_add(1) else { return false };
    // Geometric growth keeps repeated short entries linear in total copying work.
    // If the next complete buffer cannot fit, omit the optional entry rather
    // than retrying exact +1 growth and copying the whole prefix on every tag.
    let capacity = if length > entries.capacity() {
        if entries.capacity() == 0 {
            1
        } else {
            let Some(grown) = entries.capacity().checked_mul(2) else { return false };
            grown.max(length)
        }
    } else {
        entries.capacity()
    };
    let slot_bytes = std::mem::size_of::<(String, String)>();
    let added_slots = capacity.saturating_sub(storage.charged_capacity);
    let Some(minimum) = added_slots.checked_mul(slot_bytes).and_then(|bytes| bytes.checked_add(key.len())).and_then(|bytes| bytes.checked_add(value.len()))
    else {
        return false;
    };
    // Refuse before allocating even temporary copies. A previously allocated but
    // uncharged vector still spends its complete retained capacity on first use.
    if minimum > *budget {
        return false;
    }
    let (Some(key), Some(value)) = (staged_text(key), staged_text(value)) else { return false };
    let mut replacement = if length > entries.capacity() {
        let mut next = Vec::new();
        if next.try_reserve_exact(capacity).is_err() {
            return false;
        }
        Some(next)
    } else {
        None
    };
    let capacity = replacement.as_ref().map_or(entries.capacity(), Vec::capacity);
    // An allocator may grant extra capacity. Account for its actual retained
    // buffers before swapping anything; refusal drops all staging and needs no refund.
    let Some(cost) = capacity
        .saturating_sub(storage.charged_capacity)
        .checked_mul(slot_bytes)
        .and_then(|bytes| bytes.checked_add(key.capacity()))
        .and_then(|bytes| bytes.checked_add(value.capacity()))
    else {
        return false;
    };
    let Some(remaining) = budget.checked_sub(cost) else { return false };
    if let Some(next) = &mut replacement {
        next.append(entries);
        next.push((key, value));
    } else {
        entries.push((key, value));
    }
    if let Some(next) = replacement {
        *entries = next;
    }
    storage.charged_capacity = capacity;
    *budget = remaining;
    true
}
