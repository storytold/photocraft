//! PhotoCraft's private embedded-source hint; unknown versions stay inert PSD blocks.
use photocraft_doc::HistoricalXcfSampleOrder as Order;

pub(crate) const PSD_KEY: [u8; 4] = *b"pcXO";

pub(crate) fn parse(bytes: &[u8]) -> Option<Order> {
    match bytes {
        [0, 0, 0, 1, 0, 0, 0, 1] => Some(Order::LittleEndian),
        [0, 0, 0, 1, 0, 0, 0, 2] => Some(Order::BigEndian),
        _ => None,
    }
}

pub(crate) fn encode(order: Order) -> Vec<u8> {
    vec![
        0,
        0,
        0,
        1,
        0,
        0,
        0,
        match order {
            Order::LittleEndian => 1,
            Order::BigEndian => 2,
        },
    ]
}

pub(crate) fn decode(name: &str, bytes: &[u8], order: Order, ctl: &photocraft_raster::Interrupt) -> Result<photocraft_doc::Document, crate::IoError> {
    let order = match order {
        Order::BigEndian => crate::HistoricalXcfSampleOrder::BigEndian,
        Order::LittleEndian => crate::HistoricalXcfSampleOrder::LittleEndian,
    };
    let options = crate::ImportOptions { historical_xcf_order: crate::HistoricalXcfOrder::Explicit(order), ..Default::default() };
    crate::import_with_options_and_interrupt(name, bytes, &options, ctl).map(|result| result.document)
}
