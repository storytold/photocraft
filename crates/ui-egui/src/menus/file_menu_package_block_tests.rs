//! File menu: Package… shares the Place Embedded… / Place Linked… block, with no separator
//! between Place Linked… and Package… (#14084).

use super::*;

/// Top-level File menu ids around Package…, from the separator before Place Embedded… to the
/// separator after Package….
fn place_block() -> Vec<String> {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let file: Vec<String> = menu_items(&app).into_iter().filter(|i| i.path.len() == 1 && i.path[0] == "File").map(|i| i.id).collect();
    let embedded = file.iter().position(|id| id == "file.placeEmbedded").expect("Place Embedded… in File");
    file[embedded - 1..=embedded + 3].to_vec()
}

#[test]
fn package_sits_in_the_place_block_without_a_separator() {
    assert_eq!(place_block(), ["---", "file.placeEmbedded", "file.placeLinked", "file.package", "---"]);
}
