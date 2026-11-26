//! Select the authoritative configuration record before applying native text policy.

use craft_xcf::records::text::TextRecord;
use craft_xcf::records::{self, Budget, Property, text};
use photocraft_raster::Interrupt;

use super::{Result, invalid, map_error};

mod native;
pub(super) use native::{Mapping, NativeBudget, NativeColors, NativeOptions, map_layer};

pub(super) enum TextSource<'a> {
    Missing,
    ModifiedPixels { configuration: Option<(usize, &'a [u8])> },
    Incomplete(records::StopReason),
    Configuration { record: TextRecord<'a>, keep_layer_name: bool },
}

impl<'a> TextSource<'a> {
    pub(super) fn configuration_bytes(&self) -> Option<(usize, &'a [u8])> {
        match self {
            Self::Configuration { record, .. } => Some((record.span.start, record.raw)),
            Self::ModifiedPixels { configuration } => *configuration,
            _ => None,
        }
    }
}

pub(super) fn read_layer<'a>(properties: &records::Scan<Property<'a>>, budget: &mut Budget, ctl: &Interrupt<'_>) -> Result<TextSource<'a>> {
    ctl.check().map_err(|_| crate::IoError::Cancelled)?;
    if let Some(diagnostic) = properties.stopped {
        return Ok(TextSource::Incomplete(diagnostic.reason));
    }
    let keep_going = || !ctl.cancelled();
    select(&properties.records, budget, &keep_going)
}

fn select<'a>(properties: &[Property<'a>], budget: &mut Budget, keep_going: &dyn Fn() -> bool) -> Result<TextSource<'a>> {
    if !keep_going() {
        return Err(crate::IoError::Cancelled);
    }
    let mut modern = None;
    let mut legacy = None;
    let mut flags = 0;
    // Modified saved pixels are authoritative. Such layers can retain the raw
    // configuration as evidence without interpreting its text fields.
    for property in properties.iter().filter(|property| property.kind == 26) {
        if !keep_going() {
            return Err(crate::IoError::Cancelled);
        }
        let Ok(bytes) = <[u8; 4]>::try_from(property.payload) else { return Ok(TextSource::Incomplete(records::StopReason::Malformed)) };
        flags = u32::from_be_bytes(bytes);
    }
    let keep_layer_name = flags & 1 != 0;
    let modified = flags & 2 != 0;
    for property in properties {
        if !keep_going() {
            return Err(crate::IoError::Cancelled);
        }
        if property.kind == 21 {
            let offset = property.offset.checked_add(8).ok_or_else(|| invalid("XCF parasite offset overflow"))?;
            let scan = records::parasites(property.payload, offset, records::Limits::default(), budget, keep_going).map_err(map_error)?;
            // Incomplete scanning can hide a later authoritative duplicate.
            if let Some(diagnostic) = scan.stopped {
                return Ok(if modified { TextSource::ModifiedPixels { configuration: None } } else { TextSource::Incomplete(diagnostic.reason) });
            }
            for record in scan.records {
                match record.name {
                    b"gimp-text-layer" => modern = Some(record),
                    b"plug_in_gdyntext/data" => legacy = Some(record),
                    _ => {}
                }
            }
        }
    }
    let selected = modern.map(|record| (record, false)).or_else(|| legacy.map(|record| (record, true)));
    let Some((record, is_legacy)) = selected else {
        return Ok(if modified { TextSource::ModifiedPixels { configuration: None } } else { TextSource::Missing });
    };
    let header = record.raw.len().saturating_sub(record.payload.len());
    let offset = record.offset.checked_add(header).ok_or_else(|| invalid("XCF text source offset overflow"))?;
    if modified {
        return Ok(TextSource::ModifiedPixels { configuration: Some((offset, record.payload)) });
    }
    let limits = text::Limits::default();
    let mut scan = if is_legacy {
        text::gdyntext(record.payload, offset, limits, budget, keep_going)
    } else {
        text::gimp(record.payload, offset, limits, budget, keep_going)
    }
    .map_err(map_error)?;
    if let Some(diagnostic) = scan.stopped {
        return Ok(TextSource::Incomplete(diagnostic.reason));
    }
    Ok(scan.records.pop().map_or(TextSource::Incomplete(records::StopReason::Malformed), |record| TextSource::Configuration { record, keep_layer_name }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parasites(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for (name, payload) in entries {
            bytes.extend_from_slice(&(name.len() as u32 + 1).to_be_bytes());
            bytes.extend_from_slice(name.as_bytes());
            bytes.push(0);
            bytes.extend_from_slice(&0u32.to_be_bytes());
            bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            bytes.extend_from_slice(payload);
        }
        bytes
    }

    fn property(payload: &[u8]) -> Property<'_> {
        Property { offset: 0, kind: 21, declared_length: payload.len() as u32, payload, raw: &[] }
    }

    #[test]
    fn a_modern_text_configuration_takes_priority_and_the_last_record_wins() {
        let bytes =
            parasites(&[("gimp-text-layer", br#"(text "old")"#), ("gimp-text-layer", br#"(text "current")"#), ("plug_in_gdyntext/data", b"legacy payload")]);
        let source = select(&[property(&bytes)], &mut Budget::default(), &|| true).unwrap();
        let TextSource::Configuration { record, .. } = source else { panic!("editable text configuration") };
        assert_eq!(record.text().unwrap(), Some("current"));
    }

    #[test]
    fn a_malformed_preferred_record_or_modified_pixels_keeps_the_saved_raster() {
        let bytes = parasites(&[("gimp-text-layer", br#"(text "old")"#), ("gimp-text-layer", b"(text")]);
        assert!(matches!(select(&[property(&bytes)], &mut Budget::default(), &|| true).unwrap(), TextSource::Incomplete(_)));
        let flags = Property { offset: 0, kind: 26, declared_length: 4, payload: &2u32.to_be_bytes(), raw: &[] };
        let source = select(&[property(&bytes), flags], &mut Budget::default(), &|| true).unwrap();
        assert!(matches!(source, TextSource::ModifiedPixels { .. }));
        assert_eq!(source.configuration_bytes().map(|(_, bytes)| bytes), Some(b"(text".as_slice()));
    }

    #[test]
    fn a_shared_incomplete_property_scan_is_not_reinterpreted_as_complete_text() {
        let bytes = parasites(&[("gimp-text-layer", br#"(text "prefix")"#)]);
        let scan = records::Scan {
            records: vec![property(&bytes)],
            stopped: Some(records::Diagnostic { offset: 100, ordinal: 1, reason: records::StopReason::WorkLimit }),
            end_offset: 100,
        };
        assert!(matches!(read_layer(&scan, &mut Budget::default(), &Interrupt::NONE).unwrap(), TextSource::Incomplete(records::StopReason::WorkLimit)));
        let cancelled = || true;
        assert!(matches!(read_layer(&scan, &mut Budget::default(), &Interrupt::cancel_only(&cancelled)), Err(crate::IoError::Cancelled)));
    }
}
