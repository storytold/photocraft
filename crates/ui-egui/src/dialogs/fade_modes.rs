//! Edit › Fade's Mode list (#13806): the layer blend modes in the Layers panel's order and groups, with the modes the
//! document's colour mode and depth can't fade with greyed (listed, never chosen). The framework's blend row draws the
//! groups and skips a greyed mode; a remembered mode that is greyed opens as the default, Normal.

use craft::ui::patterns::form_dialogs::Param;
use craft::ui::runtime::env::Label;

use crate::PhotocraftApp;

pub(crate) const COMMAND: &str = "edit.fade";

/// The Mode row of the Fade dialog, labelled `label`: the blend list with the active document's greyed modes disabled.
pub(crate) fn param(app: &PhotocraftApp, label: Label) -> Param {
    super::blend_options::document_mode_param(app, label)
}

#[cfg(test)]
#[path = "fade_mode_list_tests.rs"]
mod tests;
