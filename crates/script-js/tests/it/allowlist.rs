//! The commands a script may send, and their parameters, are ones the engine takes: each is a
//! command it has, and each parameter a key its documented schema accepts (the engine refuses
//! an undocumented key), so no allowed request fails for being unknown.

use photocraft_script_js::commands::COMMANDS;

#[test]
fn every_allowed_command_and_parameter_is_one_the_engine_takes() {
    let mut missing = Vec::new();
    for (id, params) in COMMANDS {
        let Some(spec) = photocraft_engine::commands::find(id) else {
            missing.push(format!("{id}: no such command"));
            continue;
        };
        let Some(schema) = photocraft_engine::param_schema::of(spec) else {
            // Without a schema nothing says the engine takes these parameters.
            if !params.is_empty() {
                missing.push(format!("{id}: no parameter schema to check its parameters against"));
            }
            continue;
        };
        if schema.open() || photocraft_engine::registry::is_incomplete(id) {
            continue;
        }
        for (key, _, _) in *params {
            if !schema.has_key(key) {
                missing.push(format!("{id}: `{key}` (documented: {})", schema.keys().join(", ")));
            }
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}
