//! Shared name-collision policy for preset libraries shown in picker browsers.
//!
//! Preset commands and picker actions still address most entries by name, so names must be
//! unique across a library even when the underlying file format permits duplicates. Keep the
//! first occurrence and suffix later collisions with `_1`, `_2`, …, case-insensitively.

use super::{Group, Named};

/// Return `base`, or the first available `base_1`, `base_2`, … name.
pub fn unique_name_from_names(names: &[String], base: &str) -> String {
    let used = |candidate: &str| names.iter().any(|name| name.eq_ignore_ascii_case(candidate));
    if !used(base) {
        return base.to_string();
    }
    (1..).map(|index| format!("{base}_{index}")).find(|candidate| !used(candidate)).unwrap_or_else(|| base.to_string())
}

/// Resolve a new item name against every group in a preset library.
pub fn unique_name<T: Named>(groups: &[Group<T>], base: &str) -> String {
    let names = groups.iter().flat_map(|group| group.items.iter().map(|item| item.name().to_string())).collect::<Vec<_>>();
    unique_name_from_names(&names, base)
}

/// Return a group name that is unique across the library, case-insensitively.
pub fn unique_group_name<T>(groups: &[Group<T>], base: &str) -> String {
    let names = groups.iter().map(|group| group.name.clone()).collect::<Vec<_>>();
    unique_group_name_from_names(&names, base)
}

/// Resolve a group name using the shared case-insensitive policy (`Name 1`, `Name 2`, …).
pub fn unique_group_name_from_names(names: &[String], base: &str) -> String {
    let used = |candidate: &str| names.iter().any(|name| name.eq_ignore_ascii_case(candidate));
    if !used(base) {
        return base.to_string();
    }
    (1..).map(|index| format!("{base} {index}")).find(|candidate| !used(candidate)).unwrap_or_else(|| base.to_string())
}

/// Repair duplicate item names across groups. Original names are reserved before generating
/// suffixes, so a collision repair never steals the name of a later, otherwise-unique item.
pub fn make_group_item_names_unique<T: Named>(groups: &mut [Group<T>]) -> bool {
    let reserved = groups.iter().flat_map(|group| group.items.iter().map(|item| item.name().to_string())).collect::<Vec<_>>();
    let mut taken = Vec::<String>::new();
    let mut changed = false;
    for group in groups {
        for item in &mut group.items {
            let name = item.name().to_string();
            if taken.iter().any(|existing| existing.eq_ignore_ascii_case(&name)) {
                let unique = next_available(&taken, &reserved, &name);
                item.set_name(unique);
                changed = true;
            }
            taken.push(item.name().to_string());
        }
    }
    changed
}

/// Repair duplicate names in a flat preset library (Patterns and Brushes).
pub fn make_flat_item_names_unique<T: Named>(items: &mut [T]) -> bool {
    let reserved = items.iter().map(|item| item.name().to_string()).collect::<Vec<_>>();
    let mut taken = Vec::<String>::new();
    let mut changed = false;
    for item in items {
        let name = item.name().to_string();
        if taken.iter().any(|existing| existing.eq_ignore_ascii_case(&name)) {
            item.set_name(next_available(&taken, &reserved, &name));
            changed = true;
        }
        taken.push(item.name().to_string());
    }
    changed
}

/// Repair duplicate group names, preserving the first occurrence and original unique names.
pub fn make_group_names_unique<T>(groups: &mut [Group<T>]) -> bool {
    let reserved = groups.iter().map(|group| group.name.clone()).collect::<Vec<_>>();
    let mut taken = Vec::<String>::new();
    let mut changed = false;
    for group in groups {
        if taken.iter().any(|existing| existing.eq_ignore_ascii_case(&group.name)) {
            let base = group.name.clone();
            let unique = (1..)
                .map(|index| format!("{base} {index}"))
                .find(|candidate| {
                    !taken.iter().any(|name| name.eq_ignore_ascii_case(candidate)) && !reserved.iter().any(|name| name.eq_ignore_ascii_case(candidate))
                })
                .unwrap_or(base);
            group.name = unique;
            changed = true;
        }
        taken.push(group.name.clone());
    }
    changed
}

/// Append defaults without changing existing names or leaving collisions in the library.
pub fn append_groups_unique<T: Named>(groups: &mut Vec<Group<T>>, defaults: Vec<Group<T>>) {
    groups.extend(defaults);
    make_group_item_names_unique(groups);
    make_group_names_unique(groups);
}

fn next_available(taken: &[String], reserved: &[String], base: &str) -> String {
    (1..)
        .map(|index| format!("{base}_{index}"))
        .find(|candidate| !taken.iter().any(|name| name.eq_ignore_ascii_case(candidate)) && !reserved.iter().any(|name| name.eq_ignore_ascii_case(candidate)))
        .unwrap_or_else(|| base.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct Item(String);

    impl Named for Item {
        fn name(&self) -> &str {
            &self.0
        }
        fn set_name(&mut self, name: String) {
            self.0 = name;
        }
    }

    #[test]
    fn repairs_case_insensitive_duplicates_without_stealing_reserved_names() {
        let mut groups = vec![
            Group::new("First", vec![Item("Paper".into())]),
            Group::new("Second", vec![Item("paper".into()), Item("PAPER".into()), Item("Paper_1".into())]),
        ];
        assert!(make_group_item_names_unique(&mut groups));
        assert_eq!(
            groups.iter().flat_map(|group| group.items.iter().map(|item| item.0.as_str())).collect::<Vec<_>>(),
            ["Paper", "paper_2", "PAPER_3", "Paper_1"]
        );
        assert!(!make_group_item_names_unique(&mut groups));
    }

    #[test]
    fn group_names_are_unique_case_insensitively() {
        let mut groups = vec![Group::<String>::new("Textures", vec![]), Group::new("textures", vec![]), Group::new("Textures 1", vec![])];
        assert!(make_group_names_unique(&mut groups));
        assert_eq!(groups.iter().map(|group| group.name.as_str()).collect::<Vec<_>>(), ["Textures", "textures 2", "Textures 1"]);
    }
}
