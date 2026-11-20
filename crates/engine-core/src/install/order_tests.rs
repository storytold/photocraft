use super::TABLES;

const HINT: &str = "\nadd the id to crates/engine-core/src/install/command_ids.txt, or run this test with PHOTOCRAFT_WRITE_COMMAND_IDS=1 and review the diff";

/// Menus, the Command Palette and the automation docs list commands in registry order, so the
/// order is part of the behaviour. `command_ids.txt` pins it; regenerate the file from main when
/// rebasing, adding each new command's id after its predecessor. With `PHOTOCRAFT_WRITE_COMMAND_IDS`
/// set, the test writes the file from the tables instead; review the diff before committing it.
#[test]
fn registered_command_ids_keep_their_order() {
    let ids: Vec<&str> = TABLES.iter().flat_map(|(_, specs)| specs()).map(|c| c.id).collect();
    if std::env::var_os("PHOTOCRAFT_WRITE_COMMAND_IDS").is_some() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/install/command_ids.txt");
        std::fs::write(path, ids.iter().map(|id| format!("{id}\n")).collect::<String>()).expect("write command_ids.txt");
        return;
    }
    let expected: Vec<&str> = include_str!("command_ids.txt").lines().collect();
    if let Some(i) = ids.iter().zip(&expected).position(|(a, b)| a != b) {
        panic!("command {i} is {} where {} was registered{}{HINT}", ids[i], expected[i], table_of(i));
    }
    assert_eq!(ids.len(), expected.len(), "the number of registered commands changed{}{HINT}", table_of(ids.len().min(expected.len())));
}

/// Names the table that registers command `index`, for the failure message.
fn table_of(index: usize) -> String {
    let mut first = 0;
    for (name, specs) in TABLES {
        let len = specs().len();
        if index < first + len {
            return format!(" (table `{name}`, commands {first}..{})", first + len);
        }
        first += len;
    }
    String::new()
}
