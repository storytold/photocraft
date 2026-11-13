//! PhotoCraft's private smart-source interpretation hint. Photoshop ignores this block.

use photocraft_doc::HistoricalXcfSampleOrder;

pub(crate) const PSD_KEY: [u8; 4] = *b"pcXO";

/// Exact fixed-size versioned framing; unknown versions and values stay inert raw data.
pub(crate) fn parse(bytes: &[u8]) -> Option<HistoricalXcfSampleOrder> {
    match bytes {
        [0, 0, 0, 1, 0, 0, 0, 1] => Some(HistoricalXcfSampleOrder::LittleEndian),
        [0, 0, 0, 1, 0, 0, 0, 2] => Some(HistoricalXcfSampleOrder::BigEndian),
        _ => None,
    }
}

pub(crate) fn encode(order: HistoricalXcfSampleOrder) -> Vec<u8> {
    vec![
        0,
        0,
        0,
        1,
        0,
        0,
        0,
        match order {
            HistoricalXcfSampleOrder::LittleEndian => 1,
            HistoricalXcfSampleOrder::BigEndian => 2,
        },
    ]
}

pub(crate) fn decode(
    name: &str,
    bytes: &[u8],
    order: Option<HistoricalXcfSampleOrder>,
    ctl: &photocraft_raster::Interrupt,
) -> Result<photocraft_doc::Document, crate::IoError> {
    crate::import_with_options_and_interrupt(name, bytes, &crate::ImportOptions { historical_xcf_sample_order: order, ..Default::default() }, ctl)
        .map(|result| result.document)
}

pub(crate) fn geometry(
    name: &str,
    bytes: &[u8],
    order: Option<HistoricalXcfSampleOrder>,
    ctl: &photocraft_raster::Interrupt,
) -> Result<((f64, f64), f64), crate::IoError> {
    ctl.check().map_err(|_| crate::IoError::Cancelled)?;
    if order.is_none() || crate::is_psd(bytes) {
        let result = crate::linked::source_geometry(name, bytes, ctl).map_err(crate::IoError::Unsupported);
        // The legacy geometry API returns strings; retain cancellation as a typed outcome.
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        return result;
    }
    let doc = decode(name, bytes, order, ctl)?;
    Ok(((f64::from(doc.size.width), f64::from(doc.size.height)), f64::from(doc.resolution_dpi)))
}
