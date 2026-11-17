//! Edit › Fade's Mode list (#13806): the layer blend modes in the Layers panel's order and groups, with the modes the
//! document's colour mode and depth can't fade with greyed (listed, never chosen). The framework's blend row draws the
//! groups and skips a greyed mode; a remembered mode that is greyed opens as the default, Normal.

use craft::ui::patterns::form_dialogs::Param;
use craft::ui::runtime::env::Label;
use photocraft_color::{BlendMode, ColorMode, SampleType};

use crate::PhotocraftApp;
use crate::panels::layers_tree::blend_key;

pub(crate) const COMMAND: &str = "edit.fade";

/// The modes the reference application greys in its Fade dialog for a document of `mode` at `depth`. Combinations it
/// wasn't measured in (Grayscale 16/32, Lab 16, CMYK 16, Multichannel, Duotone…) grey none.
pub(crate) fn greyed(mode: ColorMode, depth: SampleType) -> &'static [BlendMode] {
    use BlendMode::*;
    match (mode, depth) {
        (ColorMode::Grayscale, SampleType::U8) => &[DarkerColor, LighterColor, Hue, Saturation, Color, Luminosity],
        (ColorMode::Lab, SampleType::U8) => &[Darken, ColorBurn, Lighten, ColorDodge, Difference, Exclusion, Subtract, Divide],
        (ColorMode::Rgb, SampleType::F32) => {
            &[ColorBurn, LinearBurn, Screen, ColorDodge, Overlay, SoftLight, HardLight, VividLight, LinearLight, PinLight, HardMix, Exclusion]
        }
        _ => &[],
    }
}

/// The Mode row of the Fade dialog, labelled `label`: the blend list with the active document's greyed modes disabled.
pub(crate) fn param(app: &PhotocraftApp, label: Label) -> Param {
    let greyed = app.session.active().map_or(&[][..], |st| greyed(st.doc.mode, st.doc.depth));
    super::blend_options::blend_param("mode", label, false, BlendMode::Normal).disabled_options(greyed.iter().map(|m| blend_key(*m)))
}

#[cfg(test)]
#[path = "fade_mode_list_tests.rs"]
mod tests;
