//! Tests that run vector commands, moved from the engine-core `param_schema` tests: 1 test(s).

use photocraft_engine::param_schema::Params;

fn specs() -> &'static [photocraft_engine::commands::CommandSpec] {
    photocraft_engine::install().unwrap();
    photocraft_engine::commands::command_specs()
}

/// The registry docs written in another notation (unquoted keys): not checked for unknown keys
/// until rewritten in the grammar. The list may only shrink.
const UNPARSED: &[&str] = &[
    "edit.presets.migratePresets",
    "file.export.dataSetsAsFiles",
    "file.import.variableDataSets",
    "image.applyDataSet",
    "image.trap",
    "image.variables.dataSets",
    "image.variables.define",
    "timeline.create",
    "timeline.setFrame",
    "timeline.setProps",
];

#[test]
fn every_registry_doc_parses_and_renders_back_byte_for_byte() {
    let mut unparsed = Vec::new();
    for c in specs() {
        match Params::parse(c.params) {
            Some(p) => assert_eq!(p.render(), c.params, "{}: the schema must reproduce the doc", c.id),
            None => unparsed.push(c.id),
        }
    }
    unparsed.sort_unstable();
    unparsed.dedup();
    assert_eq!(unparsed, UNPARSED);
}
