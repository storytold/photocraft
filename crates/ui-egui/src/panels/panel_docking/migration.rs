//! Restore legacy layout/column data into the authoritative shared Workspace.
use super::{
    Action, CANVAS, Column, Floating, Layout, Node, PANELS, PhotocraftApp, Placement, Side, SplitAxis, SplitSize, Value, Workspace, normalize, permissions,
    registered, tree_groups, valid_workspace,
};
use crate::state::PanelPresentation;
use craft_ui::docking::Group as DockGroup;
use serde::Deserialize;
pub(crate) fn upgrade_metrics(app: &mut PhotocraftApp) {
    if app.ui.panel_presentation.metrics_revision != 0 || app.ui.docking_kept.is_some() {
        return;
    }
    for width in app.ui.panel_presentation.widths.values_mut().filter(|w| **w == 44.0) {
        *width = super::COMPACT_COLUMN_WIDTH;
    }
    if let Some(workspace) = &mut app.ui.docking {
        let mut pending: Vec<_> = workspace.layout.root.iter_mut().collect();
        while let Some(node) = pending.pop() {
            match node {
                Node::Split { first, second, .. } => pending.extend([first.as_mut(), second.as_mut()]),
                Node::Dock { columns, .. } => {
                    for column in columns {
                        if column.iconic_width == Some(44.0) {
                            column.iconic_width = None;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    app.ui.panel_presentation.metrics_revision = 1;
}
fn normal_size(size: [f32; 2]) -> bool {
    size.iter().all(|v| v.is_finite()) && (220.0..=100_000.0).contains(&size[0]) && (120.0..=100_000.0).contains(&size[1])
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct OldRow {
    panels: Vec<String>,
    active: usize,
    weight: f32,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct OldColumn {
    members: Vec<String>,
    rows: Vec<OldRow>,
    native: Option<[f32; 4]>,
    attachment: Option<OldAttachment>,
    height: Option<f32>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct OldAttachment {
    side: String,
    order: usize,
    width: f32,
    expanded: bool,
}
fn field<T: serde::de::DeserializeOwned>(value: &Value, key: &str) -> Option<T> {
    serde_json::from_value(value.get(key)?.clone()).ok()
}
/// A changed or damaged optional view field cannot reset every other panel preference.
fn read_presentation(raw: &Value) -> PanelPresentation {
    if let Ok(mut presentation) = serde_json::from_value::<PanelPresentation>(raw.clone()) {
        if raw.get("metrics_revision").is_none() {
            presentation.metrics_revision = 0;
        }
        return presentation;
    }
    PanelPresentation {
        compact: field(raw, "compact").unwrap_or_default(),
        widths: field(raw, "widths").unwrap_or_default(),
        heights: field(raw, "heights").unwrap_or_default(),
        migrated: field(raw, "migrated").unwrap_or(false),
        flyouts: field(raw, "flyouts").unwrap_or_default(),
        floating_sizes: field(raw, "floating_sizes").unwrap_or_default(),
        stacked_sizes: field(raw, "stacked_sizes").unwrap_or_default(),
        tools_size: field(raw, "tools_size").unwrap_or_default(),
        metrics_revision: field(raw, "metrics_revision").unwrap_or(if raw.get("metrics_revision").is_none() { 0 } else { 1 }),
    }
}

pub(crate) fn presentation(saved: &Value) -> PanelPresentation {
    let mut p: PanelPresentation = saved.get("panelPresentation").map(read_presentation).unwrap_or_default();
    if saved.get("dockingColumns").is_some() {
        p.migrated = true;
        p.compact = saved.get("dockingCollapsed").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
        p.widths = saved.get("dockingStripWidths").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
        let columns: Vec<OldColumn> = saved.get("dockingColumns").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
        for c in columns.into_iter().take(64) {
            if let Some(height) = c.height {
                for id in c.members {
                    p.heights.insert(id, height);
                }
            }
        }
    }
    p.compact.retain(|id| normalize(id).is_some());
    p.compact.sort();
    p.compact.dedup();
    p.widths.retain(|id, w| normalize(id).is_some() && w.is_finite() && (super::COMPACT_COLUMN_WIDTH..=240.0).contains(w));
    p.flyouts.retain(|id, size| {
        normalize(id).is_some() && size.iter().all(|v| v.is_finite()) && (220.0..=100_000.0).contains(&size[0]) && (120.0..=100_000.0).contains(&size[1])
    });
    p.floating_sizes.retain(|id, size| {
        normalize(id).is_some() && size.iter().all(|v| v.is_finite()) && (220.0..=100_000.0).contains(&size[0]) && (120.0..=100_000.0).contains(&size[1])
    });
    p.stacked_sizes.retain(|id, size| normalize(id).is_some() && normal_size(*size));
    p.tools_size = p
        .tools_size
        .filter(|s| s.iter().all(|v| v.is_finite()) && (super::COMPACT_COLUMN_WIDTH..=240.0).contains(&s[0]) && (160.0..=100_000.0).contains(&s[1]));
    p.heights.retain(|id, h| normalize(id).is_some() && h.is_finite() && (100.0..=100_000.0).contains(h));
    p
}
fn row_tree(mut rows: Vec<(Node<String>, f32)>) -> Option<Node<String>> {
    let (first, weight) = rows.first().cloned()?;
    rows.remove(0);
    if rows.is_empty() {
        return Some(first);
    }
    let sum: f32 = rows.iter().map(|(_, w)| *w).sum();
    Some(Node::Split {
        axis: SplitAxis::Vertical,
        size: SplitSize::Ratio(weight / (weight + sum).max(f32::EPSILON)),
        first: Box::new(first),
        second: Box::new(row_tree(rows)?),
    })
}
fn vertical_rows(root: &Node<String>) -> bool {
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        match node {
            Node::Tabs { .. } | Node::Stack { .. } => {}
            Node::Split { axis: SplitAxis::Vertical, first, second, .. } => pending.extend([first.as_ref(), second.as_ref()]),
            _ => return false,
        }
    }
    true
}
/// Migrate the fork's bare Layout plus column metadata instead of discarding it.
pub(crate) fn migrate(saved: &Value) -> Option<Workspace> {
    let raw = saved.get("docking")?;
    if !["dockingColumns", "dockingEdges", "dockingStripWidths"].iter().any(|key| saved.get(key).is_some()) {
        return None;
    }
    if raw.get("layout").is_some() {
        return None;
    }
    let mut layout: Layout<String> = serde_json::from_value(raw.clone()).ok()?;
    layout.validate().ok()?;
    if layout.panels().iter().any(|id| !registered(id)) {
        return None;
    }
    let meta: Vec<OldColumn> = saved.get("dockingColumns").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
    if meta.len() > 64 {
        return None;
    }
    let widths = presentation(saved).widths;
    let mut attached = Vec::new();
    for col in meta {
        if col.members.len() > PANELS.len() || col.rows.len() > PANELS.len() {
            return None;
        }
        let visible: Vec<_> = col.members.iter().filter(|id| layout.floating.iter().any(|w| w.contains(*id))).cloned().collect();
        if visible.is_empty() {
            continue;
        }
        let r = layout.floating.iter().find(|w| visible.iter().any(|id| w.contains(id))).map(|w| w.rect)?;
        let mut groups = Vec::new();
        let mut used = Vec::new();
        let mut trees = Vec::new();
        for row in col.rows {
            let panels: Vec<_> = row.panels.into_iter().filter(|id| visible.contains(id) && !used.contains(id)).collect();
            if panels.is_empty() {
                continue;
            }
            used.extend(panels.clone());
            let active = row.active.min(panels.len().saturating_sub(1));
            let weight = if row.weight.is_finite() && row.weight > 0.0 { row.weight.min(1_000_000.0) } else { 1.0 };
            groups.push(DockGroup::new(panels.clone(), active));
            trees.push((Node::Tabs { panels, active }, weight));
        }
        for id in &visible {
            if !used.contains(id) {
                groups.push(DockGroup::new(vec![id.clone()], 0));
                trees.push((Node::Tabs { panels: vec![id.clone()], active: 0 }, 1.0));
            }
        }
        for id in &visible {
            layout.apply(Action::Close { panel: id.clone() }).ok()?;
        }
        if let Some(at) = col.attachment {
            if !["left", "rightInner", "rightOuter", "rightRail"].contains(&at.side.as_str()) {
                return None;
            }
            let mut c = Column::new(groups, at.width.clamp(220.0, 520.0));
            c.iconic = !at.expanded;
            c.iconic_width = visible.first().and_then(|id| widths.get(id)).copied();
            let height = col.height.filter(|h| h.is_finite() && (100.0..=100_000.0).contains(h)).unwrap_or(600.0);
            let sum: f32 = trees.iter().map(|(_, w)| *w).sum();
            for (group, (_, weight)) in c.groups.iter_mut().zip(&trees) {
                group.height = Some(height * weight / sum.max(f32::EPSILON));
            }
            attached.push((at.side, at.order, c));
        } else {
            let mut w = Floating::new(row_tree(trees)?, r);
            w.native = col.native.filter(|r| r.iter().all(|v| v.is_finite()) && r[2] >= 44.0 && r[3] >= 50.0);
            layout.floating.push(w);
        }
    }
    // The common pre-workspace root is the native end dock's vertical groups.
    let main = layout.root.take();
    // A horizontal legacy tree is a deliberate split, not one vertical column.
    // Preserve it verbatim instead of flattening it into unrelated tab groups.
    let preserve_main = main.as_ref().is_some_and(|node| !vertical_rows(node) || layout_node_has_canvas(node));
    let native: Vec<_> = main.iter().filter(|_| !preserve_main).flat_map(tree_groups).map(|g| DockGroup::new(g.panels, g.active)).collect();
    let mut end = Vec::new();
    let mut start = Vec::new();
    attached.sort_by_key(|a| std::cmp::Reverse(a.1));
    for (side, _, col) in &attached {
        if side == "rightOuter" {
            end.push(col.clone());
        }
    }
    if !native.is_empty() {
        end.push(Column::new(native, 290.0));
    }
    for (side, _, col) in &attached {
        if matches!(side.as_str(), "rightInner" | "rightRail") {
            end.push(col.clone());
        }
    }
    for (side, _, col) in attached.into_iter().rev() {
        if side == "left" {
            start.push(col);
        }
    }
    let mut root = Node::Tabs { panels: vec![CANVAS.into()], active: 0 };
    if let Some(main) = main.filter(|_| preserve_main) {
        root = if layout_node_has_canvas(&main) {
            main
        } else {
            Node::Split { axis: SplitAxis::Horizontal, size: SplitSize::FixedSecond(580.0), first: Box::new(root), second: Box::new(main) }
        };
    }
    if !end.is_empty() {
        root = Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::FixedSecond(290.0),
            first: Box::new(root),
            second: Box::new(Node::Dock { side: Side::End, columns: end }),
        };
    }
    if !start.is_empty() {
        root = Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::FixedFirst(44.0),
            first: Box::new(Node::Dock { side: Side::Start, columns: start }),
            second: Box::new(root),
        };
    }
    layout.root = Some(root);
    let mut workspace = Workspace::new(layout);
    migrate_hidden(&mut workspace, saved);
    valid_workspace(&workspace).then_some(workspace)
}
fn layout_node_has_canvas(node: &Node<String>) -> bool {
    tree_groups(node).iter().any(|group| group.panels.iter().any(|id| id == CANVAS))
}
fn migrate_hidden(workspace: &mut Workspace, saved: &Value) {
    // Retain hidden old panels at a valid nearby anchor; opening will not lose them.
    if let Some(hidden) = saved.get("dockingHidden").and_then(Value::as_object) {
        for (id, raw) in hidden.iter().take(PANELS.len()) {
            if registered(id) && !workspace.layout.contains(id) {
                if let Ok(at) = serde_json::from_value::<super::Location<String>>(raw.clone()) {
                    let mut candidate = workspace.clone();
                    candidate.hidden.push((id.clone(), at));
                    if valid_workspace(&candidate) {
                        *workspace = candidate;
                        continue;
                    }
                }
                let mut all = workspace.clone();
                let a = default_place_placeholder(id);
                if all.apply(a, permissions).is_ok() && all.apply(Action::Close { panel: id.clone() }, permissions).is_ok() {
                    workspace.hidden = all.hidden;
                }
            }
        }
    }
}

fn default_place_placeholder(id: &str) -> Action<String> {
    Action::OpenAt { panel: id.into(), anchor: CANVAS.into(), placement: Placement::Split(craft_ui::docking::Zone::Right) }
}

#[cfg(test)]
mod tests;
