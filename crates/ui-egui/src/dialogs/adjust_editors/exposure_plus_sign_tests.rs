//! #14639: the Exposure dialog shows values at one and above with a plus sign (`+1`,
//! `+1.23`, `+9.99`), as the reference application does. Only these three rows opt in.

use crate::widgets::{format_field_text, parse_num};

/// The Exposure, Offset and Gamma rows prefix values at one and above with `+`; anything
/// below one (and zero and negatives) reads as today.
#[test]
fn exposure_fields_show_plus_at_one_and_above() {
    // Exposure (two decimals): the dialog opens at 0, Gamma at `+1`.
    assert_eq!(format_field_text(1.0, Some(2), false, false, false, true), "+1");
    assert_eq!(format_field_text(1.23456, Some(2), false, false, false, true), "+1.23");
    assert_eq!(format_field_text(20.0, Some(2), false, false, false, true), "+20");
    assert_eq!(format_field_text(0.12, Some(2), false, false, false, true), "0.12");
    assert_eq!(format_field_text(-1.0, Some(2), false, false, false, true), "-1");
    assert_eq!(format_field_text(0.0, Some(2), false, false, false, true), "0");
    // Offset (four decimals) stays plain below one.
    assert_eq!(format_field_text(0.5, Some(4), false, false, false, true), "0.5");
    // Gamma Correction (small range: two decimals picked by the range) defaults to `+1`.
    assert_eq!(format_field_text(1.0, None, true, false, false, true), "+1");
    assert_eq!(format_field_text(9.99, None, true, false, false, true), "+9.99");
}

/// Without the opt-in the same values read as today, so every other dialog is unchanged.
#[test]
fn other_rows_keep_todays_display() {
    assert_eq!(format_field_text(1.0, Some(2), false, false, false, false), "1");
    assert_eq!(format_field_text(20.0, Some(2), false, false, false, false), "20");
    assert_eq!(format_field_text(9.99, None, true, false, false, false), "9.99");
}

/// Typing with or without `+` reads the same number, so stored values are unchanged.
#[test]
fn typed_plus_sign_parses_like_a_plain_number() {
    assert_eq!(parse_num("+1.23"), Some(1.23));
    assert_eq!(parse_num("+20"), Some(20.0));
    assert_eq!(parse_num("1.23"), Some(1.23));
}
