use super::{SPECS, command_specs};

/// Menus, the Command Palette and the automation docs list commands in registry order, so the
/// order is part of the behaviour. `command_ids.txt` pins it; regenerate the file from main when
/// rebasing, adding each new command's id after its predecessor.
#[test]
fn registered_command_ids_keep_their_order() {
    let ids: Vec<&str> = command_specs().iter().map(|c| c.id).collect();
    let expected: Vec<&str> = include_str!("command_ids.txt").lines().collect();
    if let Some(i) = ids.iter().zip(&expected).position(|(a, b)| a != b) {
        panic!("command {i} is {} where {} was registered{}", ids[i], expected[i], fragment_of(i));
    }
    assert_eq!(ids.len(), expected.len(), "the number of registered commands changed{}", fragment_of(ids.len().min(expected.len())));
}

/// Names the fragment that registers command `index`, for the failure message.
fn fragment_of(index: usize) -> String {
    let mut first = 0;
    for (name, specs) in SPECS {
        let len = specs().len();
        if index < first + len {
            return format!(" (fragment `{name}`, commands {first}..{})", first + len);
        }
        first += len;
    }
    String::new()
}
