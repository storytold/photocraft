//! #14360: Image › Adjustments has a separator between Exposure… and Vibrance…, as in the
//! reference application.

use crate::menu_catalog::CATALOG;

/// The submenu's command ids, top to bottom ("---" for a separator).
fn ids() -> Vec<&'static str> {
    CATALOG.iter().filter(|e| e.0 == ["Image", "Adjustments"]).map(|e| e.3).collect()
}

#[test]
fn a_separator_follows_exposure_and_precedes_vibrance() {
    let ids = ids();
    let exposure = ids.iter().position(|id| *id == "image.adjustments.exposure").unwrap();
    assert_eq!(ids[exposure + 1..exposure + 3], ["---", "image.adjustments.vibrance"]);
}

#[test]
fn the_first_group_keeps_its_entries_and_order() {
    assert_eq!(ids()[..4], ["image.adjustments.brightnessContrast", "image.adjustments.levels", "image.adjustments.curves", "image.adjustments.exposure",]);
}
