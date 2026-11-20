//! What the Layers panel rows read from the document that only PhotoCraft knows: the adjustments it keeps as imported
//! but cannot render ([`unsupported_key`]).

use photocraft_doc::{Adjustment, Layer, LayerContent};

/// The PSD key of the adjustment `l` holds when PhotoCraft keeps it as imported but cannot
/// render it.
pub fn unsupported_key(l: &Layer) -> Option<&str> {
    match &l.content {
        LayerContent::Adjustment(Adjustment::Unsupported { psd_key, .. }) => Some(psd_key),
        _ => None,
    }
}
