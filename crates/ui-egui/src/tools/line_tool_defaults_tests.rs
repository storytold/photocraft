//! #13241: the preferences file keeps the Line values it saved, and a key it lacks takes the
//! default.

use serde_json::json;

use crate::state::ToolOptions;

#[test]
fn line_options_in_a_saved_file_keep_their_values_and_missing_keys_take_the_defaults() {
    let saved = json!({"line_weight": 3.0, "arrow_end": true, "arrow_width": 52.0, "arrow_length": 50.0, "arrow_concavity": 25.0});
    let loaded: ToolOptions = serde_json::from_value(saved).unwrap();
    let expected = ToolOptions { line_weight: 3.0, arrow_end: true, arrow_width: 52.0, arrow_length: 50.0, arrow_concavity: 25.0, ..ToolOptions::default() };
    assert_eq!(serde_json::to_value(&loaded).unwrap(), serde_json::to_value(&expected).unwrap());
}
