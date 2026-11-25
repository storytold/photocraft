use super::crate_tables;

const HINT: &str =
    "\nadd the id to crates/engine/src/install/command_ids.txt after its predecessor, or run this test with PHOTOCRAFT_WRITE_COMMAND_IDS=1 and review the diff";

/// Menus, the Command Palette and the automation docs list commands in registry order, which
/// `command_ids.txt` sets. Every installed command has its line there, and every line names an
/// installed command. With `PHOTOCRAFT_WRITE_COMMAND_IDS` set, the test writes the file from the
/// registry instead (an unlisted command last); review the diff before committing it.
#[test]
fn every_command_has_its_place_in_the_registry_order() {
    let ids: Vec<&str> = crate::command_specs().iter().map(|c| c.id).collect();
    if std::env::var_os("PHOTOCRAFT_WRITE_COMMAND_IDS").is_some() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/install/command_ids.txt");
        std::fs::write(path, ids.iter().map(|id| format!("{id}\n")).collect::<String>()).expect("write command_ids.txt");
        return;
    }
    let listed: Vec<&str> = super::ORDER.lines().collect();
    for id in &ids {
        assert!(listed.contains(id), "command {id} has no place in the order{}{HINT}", table_of(id));
    }
    for id in &listed {
        assert!(ids.contains(id), "command_ids.txt lists {id}, which no engine crate registers{HINT}");
    }
    assert_eq!(ids, listed, "each command once, in the file's order{HINT}");
}

/// Names the table that registers command `id`, for the failure message.
fn table_of(id: &str) -> String {
    let table = crate_tables().into_iter().flatten().find(|(_, specs)| specs().iter().any(|c| c.id == id));
    table.map_or_else(String::new, |(name, _)| format!(" (table `{name}`)"))
}
