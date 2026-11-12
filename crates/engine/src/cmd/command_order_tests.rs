use super::command_specs;

#[test]
fn registered_command_ids_keep_their_order() {
    let ids: Vec<&str> = command_specs().iter().map(|c| c.id).collect();
    let expected: Vec<&str> = include_str!("command_ids.txt").lines().collect();
    if let Some(i) = ids.iter().zip(&expected).position(|(a, b)| a != b) {
        panic!("command {i} is {} where {} was registered", ids[i], expected[i]);
    }
    assert_eq!(ids.len(), expected.len(), "the number of registered commands changed");
}
