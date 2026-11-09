//! Compatibility for the protected document-tab surface in previously saved workspaces.
use super::{CANVAS, Layout, Node, SplitAxis, SplitSize};

pub(super) const ID: &str = "__photocraft_document_tabs__";

pub(super) fn ensure(layout: &mut Layout<String>) -> Result<(), String> {
    if layout.contains(&ID.to_owned()) {
        return Ok(());
    }
    fn insert(node: &mut Node<String>, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }
        match node {
            Node::Tabs { panels, .. } if panels.iter().any(|p| p == CANVAS) => {
                let canvas = node.clone();
                *node = Node::Split {
                    axis: SplitAxis::Vertical,
                    size: SplitSize::Ratio(0.5),
                    first: Box::new(Node::Tabs { panels: vec![ID.to_owned()], active: 0 }),
                    second: Box::new(canvas),
                };
                true
            }
            Node::Split { first, second, .. } => insert(first, depth + 1) || insert(second, depth + 1),
            _ => false,
        }
    }
    if layout.root.as_mut().is_some_and(|node| insert(node, 0)) { Ok(()) } else { Err("document area is missing".into()) }
}

pub(super) fn valid(layout: &Layout<String>) -> bool {
    if !layout.contains(&ID.to_owned()) {
        return true;
    }
    if layout.floating.iter().any(|w| w.contains(ID)) {
        return false;
    }
    fn shape(node: &Node<String>, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }
        match node {
            Node::Tabs { panels, active } if panels.iter().any(|p| p == ID) => panels.len() == 1 && *active == 0,
            Node::Split { first, second, .. } => shape(first, depth + 1) && shape(second, depth + 1),
            Node::Dock { columns, .. } => columns.iter().flat_map(|c| &c.groups).flat_map(|g| &g.panels).all(|p| p != ID),
            Node::Stack { entries } => entries.iter().all(|e| e.panel != ID),
            _ => true,
        }
    }
    layout.root.as_ref().is_some_and(|node| shape(node, 0))
}
