//! Order-preserving projection and traversal for nested asset-browser groups.
//!
//! This module owns hierarchy shape and traversal only. Consumers retain collapse state,
//! group presentation, item rendering, and domain actions.

/// Maximum group depth rendered by the shared browser. This matches the preset import/storage
/// limit while keeping recursive projection and traversal bounded for malformed external data.
pub(crate) const MAX_HIERARCHY_DEPTH: usize = 32;

/// An item or nested group in the projected hierarchy.
#[derive(Debug)]
pub enum HierarchyNode<'a, T> {
    /// An item that belongs directly at this level of the hierarchy.
    Preset(&'a T),
    /// A nested group, kept in the position of its first descendant.
    Group(HierarchyGroup<'a, T>),
}

/// A group node in the shared, order-preserving hierarchy projection.
#[derive(Debug)]
pub struct HierarchyGroup<'a, T> {
    /// Full path from the item's root group to this nested group.
    pub path: Vec<String>,
    /// Items and child groups in their original library order.
    pub children: Vec<HierarchyNode<'a, T>>,
    /// Number of items in this group subtree, including nested groups.
    pub count: usize,
}

/// Project ordered items and their group paths into a hierarchy, truncating paths at `max_depth`.
/// Paths deeper than `max_depth` are truncated before grouping, so the view cannot recurse beyond
/// the consumer's supported nesting limit. Item references remain borrowed from the source library.
pub(crate) fn hierarchy_tree<'a, T>(items: Vec<(&'a [String], &'a T)>, max_depth: usize) -> Vec<HierarchyNode<'a, T>> {
    let bounded = items.into_iter().map(|(path, item)| (&path[..path.len().min(max_depth)], item)).collect();
    hierarchy_level(bounded, &[])
}

/// Shared ordered traversal for nested hierarchy views. Consumers provide their item-run renderer,
/// group presentation/interaction, and collapse policy; traversal owns recursive sequencing.
/// Adjacent presets are batched into one `draw_items` call and flushed before each group header;
/// children are traversed only when `is_open` returns true and `max_depth` has not been reached.
#[cfg(test)]
type HierarchyOpen<'a, 'b, T> = dyn FnMut(&HierarchyGroup<'a, T>) -> bool + 'b;
#[cfg(test)]
type HierarchyDrawGroup<'a, 'b, T> = dyn FnMut(&mut egui::Ui, &HierarchyGroup<'a, T>, bool, usize) + 'b;
#[cfg(test)]
type HierarchyDrawItems<'a, 'b, T> = dyn FnMut(&mut egui::Ui, &[&'a T], usize) + 'b;

#[cfg(test)]
pub(crate) fn draw_hierarchy<'a, 'b, T>(
    ui: &mut egui::Ui,
    nodes: &[HierarchyNode<'a, T>],
    depth: usize,
    max_depth: usize,
    is_open: &mut HierarchyOpen<'a, 'b, T>,
    draw_group: &mut HierarchyDrawGroup<'a, 'b, T>,
    draw_items: &mut HierarchyDrawItems<'a, 'b, T>,
) {
    let mut run: Vec<&'a T> = Vec::new();
    let flush = |ui: &mut egui::Ui, run: &mut Vec<&'a T>, draw_items: &mut dyn FnMut(&mut egui::Ui, &[&'a T], usize)| {
        if !run.is_empty() {
            draw_items(ui, run, depth);
            run.clear();
        }
    };
    for node in nodes {
        match node {
            HierarchyNode::Preset(item) => run.push(*item),
            HierarchyNode::Group(group) => {
                flush(ui, &mut run, draw_items);
                let open = is_open(group);
                draw_group(ui, group, open, depth);
                if open && depth < max_depth {
                    draw_hierarchy(ui, &group.children, depth + 1, max_depth, is_open, draw_group, draw_items);
                }
            }
        }
    }
    flush(ui, &mut run, draw_items);
}

type HierarchyEntry<'a, T> = (&'a [String], &'a T);

/// Build one level of the tree while retaining the first-seen order of items and groups.
fn hierarchy_level<'a, T>(items: Vec<HierarchyEntry<'a, T>>, prefix: &[String]) -> Vec<HierarchyNode<'a, T>> {
    enum Slot {
        Item(usize),
        Group(usize),
    }
    let mut slots = Vec::new();
    let mut groups: Vec<(&String, Vec<HierarchyEntry<'a, T>>)> = Vec::new();
    for (i, (path, item)) in items.iter().enumerate() {
        match path.split_first() {
            None => slots.push(Slot::Item(i)),
            Some((name, rest)) => match groups.iter_mut().position(|(n, _)| *n == name) {
                Some(k) => {
                    if let Some((_, v)) = groups.get_mut(k) {
                        v.push((rest, *item));
                    }
                }
                None => {
                    slots.push(Slot::Group(groups.len()));
                    groups.push((name, vec![(rest, *item)]));
                }
            },
        }
    }
    type NamedGroupEntries<'a, T> = (&'a String, Vec<HierarchyEntry<'a, T>>);
    let mut groups: Vec<Option<NamedGroupEntries<'a, T>>> = groups.into_iter().map(Some).collect();
    slots
        .into_iter()
        .filter_map(|slot| match slot {
            Slot::Item(i) => items.get(i).map(|(_, item)| HierarchyNode::Preset(*item)),
            Slot::Group(i) => {
                let (name, sub) = groups.get_mut(i)?.take()?;
                let path: Vec<String> = prefix.iter().cloned().chain(std::iter::once(name.clone())).collect();
                let count = sub.len();
                let children = hierarchy_level(sub, &path);
                Some(HierarchyNode::Group(HierarchyGroup { path, children, count }))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{HierarchyGroup, MAX_HIERARCHY_DEPTH, draw_hierarchy, hierarchy_tree};
    use std::collections::BTreeSet;

    fn always_open<T>(_: &HierarchyGroup<'_, T>) -> bool {
        true
    }

    #[test]
    fn child_group_starts_a_new_card_run_between_parent_card_runs() {
        let items = [
            (Vec::<String>::new(), "A"),
            (Vec::new(), "B"),
            (vec!["Child".to_string()], "Child 1"),
            (vec!["Child".to_string()], "Child 2"),
            (Vec::new(), "C"),
            (Vec::new(), "D"),
        ];
        let tree = hierarchy_tree(items.iter().map(|(path, name)| (path.as_slice(), name)).collect(), MAX_HIERARCHY_DEPTH);
        let ctx = egui::Context::default();
        let sequence = std::cell::RefCell::new(Vec::new());
        let mut is_open = always_open;
        let mut draw_group = |_: &mut egui::Ui, group: &HierarchyGroup<'_, &str>, _: bool, _| {
            sequence.borrow_mut().push(format!("group:{}", group.path.join("/")));
        };
        let mut draw_items = |_: &mut egui::Ui, run: &[&&str], _| {
            sequence.borrow_mut().push(format!("cards:{}", run.iter().map(|name| **name).collect::<Vec<_>>().join(",")));
        };

        let mut output = ctx.run_ui(Default::default(), |ui| {
            draw_hierarchy(ui, &tree, 0, MAX_HIERARCHY_DEPTH, &mut is_open, &mut draw_group, &mut draw_items);
        });
        output.textures_delta.clear();

        assert_eq!(*sequence.borrow(), ["cards:A,B", "group:Child", "cards:Child 1,Child 2", "cards:C,D"]);
    }

    #[test]
    fn projection_preserves_interleaved_cards_and_nested_groups_in_first_seen_order() {
        let items = [
            (Vec::<String>::new(), "root-before"),
            (vec!["A".to_string()], "a-before"),
            (vec!["A".to_string(), "B".to_string()], "b-before"),
            (vec!["A".to_string(), "B".to_string(), "C".to_string()], "c-leaf"),
            (vec!["A".to_string(), "B".to_string()], "b-after"),
            (vec!["A".to_string()], "a-after"),
            (vec!["D".to_string()], "d-leaf"),
            (Vec::new(), "root-after"),
        ];
        let tree = hierarchy_tree(items.iter().map(|(path, name)| (path.as_slice(), name)).collect(), MAX_HIERARCHY_DEPTH);
        let ctx = egui::Context::default();
        let sequence = std::cell::RefCell::new(Vec::new());
        let mut is_open = always_open::<&str>;
        let mut draw_group = |_: &mut egui::Ui, group: &HierarchyGroup<'_, &str>, _: bool, depth| {
            sequence.borrow_mut().push(format!("group:{}:{depth}", group.path.join("/")));
        };
        let mut draw_items = |_: &mut egui::Ui, run: &[&&str], depth| {
            sequence.borrow_mut().push(format!("cards:{depth}:{}", run.iter().map(|name| **name).collect::<Vec<_>>().join(",")));
        };

        let mut output = ctx.run_ui(Default::default(), |ui| {
            draw_hierarchy(ui, &tree, 0, MAX_HIERARCHY_DEPTH, &mut is_open, &mut draw_group, &mut draw_items);
        });
        output.textures_delta.clear();

        assert_eq!(
            *sequence.borrow(),
            [
                "cards:0:root-before",
                "group:A:0",
                "cards:1:a-before",
                "group:A/B:1",
                "cards:2:b-before",
                "group:A/B/C:2",
                "cards:3:c-leaf",
                "cards:2:b-after",
                "cards:1:a-after",
                "group:D:0",
                "cards:1:d-leaf",
                "cards:0:root-after",
            ]
        );
    }

    #[test]
    fn every_nested_collapse_combination_only_traverses_open_ancestor_paths() {
        let items = [
            (Vec::<String>::new(), "root-before"),
            (vec!["A".to_string()], "a-before"),
            (vec!["A".to_string(), "B".to_string()], "b-before"),
            (vec!["A".to_string(), "B".to_string(), "C".to_string()], "c-leaf"),
            (vec!["A".to_string(), "B".to_string()], "b-after"),
            (vec!["A".to_string()], "a-after"),
            (vec!["D".to_string()], "d-leaf"),
            (Vec::new(), "root-after"),
        ];
        let tree = hierarchy_tree(items.iter().map(|(path, name)| (path.as_slice(), name)).collect(), MAX_HIERARCHY_DEPTH);
        let paths = ["A", "A/B", "A/B/C", "D"];

        for mask in 0..(1usize << paths.len()) {
            let open: BTreeSet<String> = paths.iter().enumerate().filter(|(index, _)| mask & (1 << index) != 0).map(|(_, path)| (*path).to_owned()).collect();
            let groups_seen = std::cell::RefCell::new(Vec::new());
            let items_seen = std::cell::RefCell::new(Vec::new());
            let mut is_open = |group: &HierarchyGroup<'_, &str>| -> bool { open.contains(&group.path.join("/")) };
            let mut draw_group = |_: &mut egui::Ui, group: &HierarchyGroup<'_, &str>, group_open: bool, depth| {
                groups_seen.borrow_mut().push((group.path.join("/"), group_open, depth));
            };
            let mut draw_items = |_: &mut egui::Ui, run: &[&&str], depth| {
                items_seen.borrow_mut().extend(run.iter().map(|name| (depth, (**name).to_owned())));
            };
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(Default::default(), |ui| {
                draw_hierarchy(ui, &tree, 0, MAX_HIERARCHY_DEPTH, &mut is_open, &mut draw_group, &mut draw_items);
            });
            output.textures_delta.clear();

            let expected_groups = [("A", 0usize), ("A/B", 1usize), ("A/B/C", 2usize), ("D", 0usize)]
                .into_iter()
                .filter(|(path, _)| match *path {
                    "A" | "D" => true,
                    "A/B" => open.contains("A"),
                    "A/B/C" => open.contains("A") && open.contains("A/B"),
                    _ => false,
                })
                .map(|(path, depth)| (path.to_owned(), open.contains(path), depth))
                .collect::<Vec<_>>();
            assert_eq!(*groups_seen.borrow(), expected_groups, "collapse mask {mask:04b}");

            let mut expected_items = vec![(0usize, "root-before".to_owned())];
            if open.contains("A") {
                expected_items.push((1usize, "a-before".to_owned()));
                if open.contains("A/B") {
                    expected_items.push((2usize, "b-before".to_owned()));
                    if open.contains("A/B/C") {
                        expected_items.push((3usize, "c-leaf".to_owned()));
                    }
                    expected_items.push((2usize, "b-after".to_owned()));
                }
                expected_items.push((1usize, "a-after".to_owned()));
            }
            if open.contains("D") {
                expected_items.push((1usize, "d-leaf".to_owned()));
            }
            expected_items.push((0usize, "root-after".to_owned()));
            assert_eq!(*items_seen.borrow(), expected_items, "collapse mask {mask:04b}");
        }
    }
}
