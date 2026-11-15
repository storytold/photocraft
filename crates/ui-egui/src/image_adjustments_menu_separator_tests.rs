//! #14360: Image › Adjustments has a separator between Exposure… and Vibrance…, as in the
//! reference application.

use crate::menu_catalog::CATALOG;

/// The submenu's command ids, top to bottom ("---" for a separator).
fn ids() -> Vec<&'static str> {
    CATALOG.iter().filter(|e| e.0 == ["Image", "Adjustments"]).map(|e| e.3).collect()
}

/// The submenu's labels, top to bottom: the menus draw a separator for the label "---".
fn labels() -> Vec<&'static str> {
    CATALOG.iter().filter(|e| e.0 == ["Image", "Adjustments"]).map(|e| e.1).collect()
}

#[test]
fn a_separator_follows_exposure_and_precedes_vibrance() {
    let ids = ids();
    let exposure = ids.iter().position(|id| *id == "image.adjustments.exposure").unwrap();
    assert_eq!(ids[exposure + 1..exposure + 3], ["---", "image.adjustments.vibrance"]);
    assert_eq!(labels()[exposure + 1], "---", "the row is drawn as a separator");
}

#[test]
fn the_first_group_keeps_its_entries_and_order() {
    assert_eq!(ids()[..4], ["image.adjustments.brightnessContrast", "image.adjustments.levels", "image.adjustments.curves", "image.adjustments.exposure",]);
}
