//! A generative layer (Edit › Generative Fill): one pixel layer that keeps every variation the
//! model returned, showing one of them, as Photoshop's generative layer does.

use photocraft_raster::Surface;

/// What a generative layer keeps besides the pixels it shows: the prompt it was made with and
/// every variation, each in the document's mode and depth, over the same area. The layer's own
/// pixels are a copy of `variations[selected]` (copy-on-write, so the copy costs nothing until
/// one of them is painted on).
#[derive(Clone, Debug, craft_ui_macros::PlainData)]
pub struct Generative {
    /// The prompt as typed; empty when the fill continued the picture through the selection.
    pub prompt: String,
    /// The variations in the order they arrived, Generate More's after the first fill's.
    pub variations: Vec<Surface>,
    /// The variation the layer shows.
    pub selected: usize,
}

impl Generative {
    /// A generative layer's data showing its first variation.
    pub fn new(prompt: impl Into<String>, variations: Vec<Surface>) -> Self {
        Generative { prompt: prompt.into(), variations, selected: 0 }
    }

    /// The variation the layer shows, when there is one.
    pub fn shown(&self) -> Option<&Surface> {
        self.variations.get(self.selected)
    }
}

// Surfaces aren't `PartialEq` (and comparing pixels would be pointless here): compare by shape,
// as `VideoData` does, which is enough for change detection.
impl PartialEq for Generative {
    fn eq(&self, o: &Self) -> bool {
        self.prompt == o.prompt
            && self.selected == o.selected
            && self.variations.len() == o.variations.len()
            && self.variations.iter().zip(&o.variations).all(|(a, b)| a.generation() == b.generation())
    }
}
