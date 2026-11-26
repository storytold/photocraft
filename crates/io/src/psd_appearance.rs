//! Private PSD layer tag for the policy of an imported shape or type cache.
//! The existing PSD pixel channels carry the appearance.

pub(crate) const PSD_KEY: [u8; 4] = *b"pcAP";

/// Fixed text names are independent of whether the saved type cache is still authoritative.
pub(crate) const FIXED_TEXT_NAME_KEY: [u8; 4] = *b"pcTN";
pub(crate) const FIXED_TEXT_NAME_V1: [u8; 4] = [0, 0, 0, 1];

pub(crate) fn known_tag(key: &[u8; 4], bytes: &[u8]) -> bool {
    (*key == PSD_KEY && Appearance::parse(bytes).is_some()) || (*key == FIXED_TEXT_NAME_KEY && bytes == FIXED_TEXT_NAME_V1)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Appearance {
    Shape = 1,
    Text = 2,
}

impl Appearance {
    pub(crate) fn parse(bytes: &[u8]) -> Option<Self> {
        match bytes {
            [0, 0, 0, 1, 0, 0, 0, 1] => Some(Self::Shape),
            [0, 0, 0, 1, 0, 0, 0, 2] => Some(Self::Text),
            _ => None,
        }
    }

    pub(crate) fn encode(self) -> Vec<u8> {
        vec![0, 0, 0, 1, 0, 0, 0, self as u8]
    }
}

#[cfg(test)]
mod tests;
