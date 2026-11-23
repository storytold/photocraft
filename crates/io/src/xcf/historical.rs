//! Sample interpretation and inert native audit evidence, never a rendering switch.
use craft_xcf::samples::{HistoricalOrder, SampleOrder, SampleOrderRequest};
use photocraft_doc::import_conversion::{ConversionDisposition, ConversionEntry, ConversionReason, SourceItemRef};

#[cfg(test)]
mod tests;

/// Return one bounded entry for the final XCF report builder to merge with its
/// other outcomes before attachment. Modern/byte-wide sources need no entry.
pub(crate) fn evidence(request: Option<SampleOrderRequest>, policy: HistoricalOrder) -> Option<ConversionEntry> {
    let request = request?;
    let (order, explicit) = match policy {
        HistoricalOrder::Default => (SampleOrder::BigEndian, false),
        HistoricalOrder::Explicit(order) => (order, true),
        HistoricalOrder::RequireExplicit => return None,
    };
    let order = match order {
        SampleOrder::BigEndian => "big-endian",
        SampleOrder::LittleEndian => "little-endian",
    };
    Some(ConversionEntry {
        feature_code: "xcf.historical_sample_order".into(),
        source: SourceItemRef { kind: "image".into(), ..Default::default() },
        targets: Vec::new(),
        disposition: if explicit { ConversionDisposition::NativeEquivalent } else { ConversionDisposition::NativeApproximation },
        reason: ConversionReason::NativePolicy,
        source_description: format!(
            "XCF version {} with {}-bit samples does not record sample byte order",
            request.version,
            request.precision.storage.bytes * 8
        ),
        native_description: if explicit {
            format!("Imported {order} samples using the caller's explicit source-specific choice")
        } else {
            format!("Imported {order} samples using the compatible default; source byte order was not inferred")
        },
        attachments: Vec::new(),
    })
}
