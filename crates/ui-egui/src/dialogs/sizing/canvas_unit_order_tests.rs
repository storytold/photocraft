//! #14584: Canvas Size lists Percent after Picas in its unit menus; Image Size keeps it second.

use super::*;

fn labels(key: &str, canvas: bool) -> Vec<&'static str> {
    unit_options(key, canvas).into_iter().map(|(_, l)| l).collect()
}

#[test]
fn canvas_size_lists_percent_after_picas() {
    assert_eq!(labels("width", true), ["Pixels", "Inches", "Centimeters", "Millimeters", "Points", "Picas", "Percent", "Columns"]);
    assert_eq!(labels("height", true), ["Pixels", "Inches", "Centimeters", "Millimeters", "Points", "Picas", "Percent"]);
}

#[test]
fn image_size_keeps_percent_second() {
    assert_eq!(labels("width", false), ["Pixels", "Percent", "Inches", "Centimeters", "Millimeters", "Points", "Picas", "Columns"]);
    assert_eq!(labels("height", false), ["Pixels", "Percent", "Inches", "Centimeters", "Millimeters", "Points", "Picas"]);
}

#[test]
fn reordering_keeps_the_unit_ids() {
    let ids = |canvas| {
        let mut ids: Vec<String> = unit_options("width", canvas).into_iter().map(|(k, _)| k).collect();
        ids.sort();
        ids
    };
    assert_eq!(ids(true), ids(false));
    let canvas = unit_options("width", true);
    assert!(canvas.contains(&("percent".to_string(), "Percent")));
}
