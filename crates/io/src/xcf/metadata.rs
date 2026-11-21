//! Optional source metadata translated into ordinary native data and one-time view hints.
//! Wire fields follow the public XCF property layout; source-only settings stay inert.

use photocraft_color::Color;
use photocraft_doc::{AlphaChannel, ColorIndicates, Document};

use super::{Prop, Result, be_f32, be_i32, be_u32, conversion};
use crate::initial_view::{MAX_INITIAL_SAMPLERS, NativeInitialView};

#[cfg(test)]
mod cancellation_tests;
#[cfg(test)]
mod guide_storage_tests;
#[cfg(test)]
mod resolution_tests;
#[cfg(test)]
mod text_storage_tests;
#[cfg(test)]
mod unused_colormap_tests;

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
pub(super) fn effective_resolution(props: &[Prop<'_>]) -> Option<(f32, f32)> {
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
        if let Some(((r, g), b)) = rgb.filter(|&((r, g), b)| [r, g, b].iter().all(|c| (0.0..=1.0).contains(c))) {
            channel.color = Color::rgb(r, g, b);
            handled.push(38);
        }
    }
    if let Some(v) = word(props, 6).filter(|v| *v <= 255) {
        channel.opacity = v as f32 / 255.0;
        handled.push(6);
    }
    if let Some(v) =
        props.iter().rev().find(|p| p.kind == 33).and_then(|p| p.data.as_array::<4>().map(|word| f32::from_be_bytes(*word))).filter(|v| (0.0..=1.0).contains(v))
    {
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
fn metadata_parasites<'a>(
    props: &[Prop<'a>],
    work: &mut super::wire::ParasiteWork,
    ctl: &photocraft_raster::Interrupt<'_>,
) -> Result<([MetadataParasite<'a>; 4], bool, bool)> {
    let mut selected =
        ["gimp-comment", "exif-data", "gimp-image-metadata", "gimp-metadata"].map(|name| MetadataParasite { name, ordinal: usize::MAX, data: None });
    let mut ordinal = 0usize;
    let mut limited = false;
    let mut malformed = false;
    for property in props.iter().filter(|property| property.kind == 21) {
        let scan = super::wire::scan_parasites_with_work(property.data, work, ctl, |record| {
            for slot in &mut selected {
                if record.name == slot.name.as_bytes() {
                    if slot.data.is_none() {
                        slot.ordinal = ordinal;
                    }
                    slot.data = Some(record.payload);
                }
            }
            ordinal = ordinal.saturating_add(1);
            Ok(true)
        })?;
        malformed |= scan.malformed;
        if scan.limited {
            limited = true;
            break;
        }
    }
    // Admit the final raw EXIF packet before optional XML synthesis shares its budget.
    selected.sort_by_key(|slot| (slot.name != "exif-data", slot.ordinal));
    if limited {
        for slot in &mut selected {
            slot.data = None;
        }
    }
    Ok((selected, limited, malformed))
}

#[test]
fn metadata_parasite_selection_preserves_first_order_and_final_empty_values() {
    let mut records = super::tests::W { out: Vec::new(), wide: false };
    for (name, value) in [("gimp-comment", "old"), ("gimp-metadata", "xml"), ("gimp-comment", "")] {
        records.string(name);
        records.u32(0);
        records.u32(value.len() as u32);
        records.out.extend_from_slice(value.as_bytes());
    }
    records.out.extend_from_slice(&[0; 2]); // Malformed tail.
    let props = [Prop { kind: 21, offset: 0, data: &records.out }];
    let (selected, limited, malformed) = metadata_parasites(&props, &mut Default::default(), &photocraft_raster::Interrupt::NONE).unwrap();
    assert!(malformed && !limited);
    let winners: Vec<_> = selected.iter().filter_map(|slot| slot.data.map(|data| (slot.name, data))).collect();
    assert_eq!(winners, [("gimp-comment", b"".as_slice()), ("gimp-metadata", b"xml".as_slice())]);
}

#[allow(clippy::too_many_arguments)] // Guide conversion shares the image's storage budget and report.
fn image_guides(
    doc: &mut Document,
    property: &Prop<'_>,
    version: u32,
    outcomes: &mut conversion::Outcomes<'_>,
    owner: usize,
    budget: &mut usize,
    ctl: &photocraft_raster::Interrupt<'_>,
) -> Result<()> {
    use photocraft_doc::import_conversion::ConversionReason as Reason;
    const SLOT: usize = std::mem::size_of::<f64>();
    let (records, tail) = property.data.as_chunks::<5>();
    let bounds = [doc.size.height, doc.size.width];
    let mut targets = [&mut doc.guides.horizontal, &mut doc.guides.vertical];
    let spare = targets.each_ref().map(|target| target.capacity() - target.len());
    let slots = *budget / SLOT;
    let mut additions = [0usize; 2];
    let mut minimum = 0;
    let mut prefix = 0;
    let mut invalid_data = !tail.is_empty();
    for record in records {
        ctl.check().map_err(|_| super::IoError::Cancelled)?;
        if let Some((axis, _)) = guide_record(record, bounds, version) {
            let axis = usize::from(axis);
            if additions[axis] >= spare[axis] {
                if minimum == slots {
                    break;
                }
                minimum += 1;
            }
            additions[axis] += 1;
        } else {
            invalid_data = true;
        }
        prefix += 1;
    }

    let mut limited = prefix != records.len();
    // Preserve the admitted prefix under tight budgets. Spare growth from earlier
    // records is already charged. Spend any extra only after covering both axes.
    let mut extra = slots - minimum;
    for ((target, count), free) in targets.iter_mut().zip(&mut additions).zip(spare) {
        let needed = count.saturating_sub(free);
        if needed == 0 {
            continue;
        }
        let requested = needed.max(target.capacity().min(needed.saturating_add(extra)));
        let Some(actual) = reserve_guides(target, requested, needed.saturating_add(extra)) else {
            *count = free;
            limited = true;
            continue;
        };
        extra -= actual.saturating_sub(needed);
        *budget -= actual * SLOT;
    }

    for record in records.iter().take(prefix) {
        ctl.check().map_err(|_| super::IoError::Cancelled)?;
        if let Some((axis, position)) = guide_record(record, bounds, version) {
            let axis = usize::from(axis);
            if additions[axis] != 0 {
                targets[axis].push(f64::from(position));
                additions[axis] -= 1;
            }
        }
    }
    if limited || invalid_data {
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

fn guide_record(bytes: &[u8; 5], [height, width]: [u32; 2], version: u32) -> Option<(bool, i32)> {
    let &[a, b, c, d, orientation] = bytes;
    let position = i32::from_be_bytes([a, b, c, d]);
    let (vertical, limit) = match orientation {
        1 => (false, height),
        2 => (true, width),
        _ => return None,
    };
    (version >= 15 || (position >= 0 && i64::from(position) <= i64::from(limit))).then_some((vertical, position))
}

/// Stage one allocation per axis, then check actual capacity before retaining it.
fn reserve_guides(target: &mut Vec<f64>, additional: usize, allowed: usize) -> Option<usize> {
    let previous = target.capacity();
    let mut next = Vec::new();
    next.try_reserve_exact(previous.checked_add(additional)?).ok()?;
    let added = next.capacity().checked_sub(previous)?;
    if added > allowed {
        return None;
    }
    next.append(target);
    *target = next;
    Some(added)
}

/// Map native document metadata without changing native DPI, unit, guide or sampler semantics.
#[allow(clippy::too_many_arguments)] // One reader-owned set of bounded outputs, without a new persistent model.
#[cfg(test)]
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
    image_with_work(doc, props, version, view, outcomes, owner, budget, warnings, &mut super::wire::ParasiteWork::default(), ctl)
}

#[allow(clippy::too_many_arguments)] // One reader-owned set of bounded outputs, without a new persistent model.
pub(super) fn image_with_work<'a>(
    doc: &mut Document,
    props: &[Prop<'a>],
    version: u32,
    view: &mut NativeInitialView,
    outcomes: &mut conversion::Outcomes<'a>,
    owner: usize,
    budget: &mut usize,
    warnings: &mut crate::ora::Warnings,
    parasite_work: &mut super::wire::ParasiteWork,
    ctl: &photocraft_raster::Interrupt<'_>,
) -> Result<()> {
    use photocraft_doc::import_conversion::ConversionReason as Reason;
    ctl.check().map_err(|_| super::IoError::Cancelled)?;
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
        ctl.check().map_err(|_| super::IoError::Cancelled)?;
        match p.kind {
            18 => image_guides(doc, p, version, outcomes, owner, budget, ctl)?,
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
            _ => {}
        }
    }
    let (selected, limited, malformed) = metadata_parasites(props, parasite_work, ctl)?;
    if limited {
        metadata_incomplete = true;
        retention_limited = true;
        outcomes.notice(
            owner,
            "xcf.parasite_work",
            Reason::RetentionLimit,
            (
                "Image metadata parasite selection exceeded the shared scan-work allowance".into(),
                "Incomplete final-value selection omitted; usable image pixels retained".into(),
            ),
            Some("reached the shared parasite scan-work allowance; usable image content was preserved"),
            warnings,
        )?;
    }
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
                if super::metadata_packets::has_exif_directory(raw) {
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
                let Ok(xml) = std::str::from_utf8(data.strip_suffix(&[0]).unwrap_or(data)) else { continue };
                let tree = match parse_xml(xml, ctl) {
                    Ok(tree) => tree,
                    Err(crate::IoError::Cancelled) => return Err(crate::IoError::Cancelled),
                    Err(error) => {
                        outcomes.notice(
                            owner,
                            "xcf.metadata_xml",
                            if matches!(error, crate::IoError::Unsupported(_)) { Reason::RetentionLimit } else { Reason::InvalidOptionalData },
                            (error.to_string(), "Optional XML metadata skipped; saved image pixels retained".into()),
                            Some("skipped optional XML metadata; saved image pixels were preserved"),
                            warnings,
                        )?;
                        continue;
                    }
                };
                if tree.root_element().has_tag_name("metadata") {
                    for node in tree.root_element().children().filter(|n| n.has_tag_name("tag")) {
                        ctl.check().map_err(|_| super::IoError::Cancelled)?;
                        if node.attribute("encoding").is_some() {
                            metadata_incomplete = true;
                            continue;
                        }
                        if let (Some(key), Some(value)) = (node.attribute("name"), plain_tag_text(node))
                            && matches!(key.split('.').next(), Some("Exif" | "Xmp" | "Iptc"))
                        {
                            let omitted = !push_text(doc, key, &value, &mut text_storage, budget);
                            metadata_incomplete |= omitted;
                            retention_limited |= omitted;
                        }
                    }
                    use super::metadata_packets::PacketStatus;
                    let (xmp, exif) = super::metadata_packets::translate(doc, &tree, budget);
                    let omitted = [xmp, exif].contains(&PacketStatus::OverBudget);
                    metadata_incomplete |= omitted;
                    retention_limited |= omitted;
                    outcomes.detail(owner, "xcf.metadata", Reason::NativePolicy,
                        "Serialized source EXIF/XMP/IPTC tag values".into(),
                        format!("Plain tags retained as native image text. Native XMP: {}. Native EXIF: {}. Existing raw packets take priority. Unknown, binary or ambiguous source structures remain inert; full packet equivalence is not claimed", xmp.label(), exif.label()))?;
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
            ctl.check().map_err(|_| super::IoError::Cancelled)?;
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

/// Admit XML parser work before allocating a DOM for optional metadata.
/// Borrow a single segment and bound copies before joining split text nodes.
pub(super) fn plain_tag_text<'a>(tag: roxmltree::Node<'a, '_>) -> Option<std::borrow::Cow<'a, str>> {
    use std::borrow::Cow;

    if tag.attribute("encoding").is_some() || tag.children().any(|child| child.is_element()) {
        return None;
    }
    let mut text = tag.children().filter(|child| child.is_text()).filter_map(|child| child.text());
    let first = text.next()?;
    let Some(second) = text.next() else { return Some(Cow::Borrowed(first)) };
    let total = text.clone().try_fold(first.len().checked_add(second.len())?, |total, next| total.checked_add(next.len()))?;
    // The XML preflight limits the whole packet to this size, including tags.
    if total > super::MAX_METADATA_BYTES {
        return None;
    }
    let mut joined = String::new();
    joined.try_reserve_exact(total).ok()?;
    joined.push_str(first);
    joined.push_str(second);
    for segment in text {
        joined.push_str(segment);
    }
    Some(Cow::Owned(joined))
}

fn parse_xml<'a>(xml: &'a str, ctl: &photocraft_raster::Interrupt<'_>) -> Result<roxmltree::Document<'a>> {
    super::xml_work::admit(xml, "source metadata", super::MAX_METADATA_BYTES, super::MAX_DEPTH, ctl)?;
    let tree =
        roxmltree::Document::parse_with_options(xml, roxmltree::ParsingOptions { nodes_limit: 4096, ..Default::default() }).map_err(|error| match error {
            roxmltree::Error::NodesLimitReached => crate::IoError::Unsupported("source metadata exceeds the bounded XML node allowance".into()),
            _ => super::invalid(format!("invalid source metadata: {error}")),
        })?;
    ctl.check().map_err(|_| crate::IoError::Cancelled)?;
    Ok(tree)
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
