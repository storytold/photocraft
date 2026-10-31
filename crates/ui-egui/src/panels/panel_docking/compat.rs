//! Compatibility for the fork's view commands and saved layouts, using the current Workspace.
use super::{
    Action, CANVAS, Column, Floating, Layout, Node, PANELS, PhotocraftApp, Placement, Side, SplitAxis, SplitSize, Target, Value, Workspace, commit, ensure,
    floats, group_members, in_dock_column, json, normalize, permissions, registered, reveal_group, tree_groups, valid_workspace, workspace_of,
};
use crate::state::PanelPresentation;
use craft_ui::docking::Group as DockGroup;
use serde::Deserialize;

/// Replace the redundant launcher with real, independently movable panel owners once.
pub(crate) fn initialize(app: &mut PhotocraftApp) -> Result<(), String> {
    if app.ui.panel_presentation.migrated || app.ui.docking_kept.is_some() || !app.ui.panels.rail || app.session.prefs().workspace_locked {
        return Ok(());
    }
    // Existing modern workspaces already express their author's arrangement.
    if app.ui.docking.is_some() {
        return Ok(());
    }
    let mut workspace = workspace_of(app)?;
    let selected: Vec<_> = workspace.layout.root.iter().flat_map(tree_groups).filter_map(|g| g.panels.get(g.active).cloned()).collect();
    let entries = ["properties", "navigator", "color", "history", "brushes"];
    let mut previous = None;
    for panel in entries {
        ensure(app, &mut workspace, panel)?;
        let group = PANELS.iter().find(|p| p.0 == panel).map(|p| p.2);
        for &(id, _, _) in PANELS.iter().filter(|p| Some(p.2) == group && p.0 != panel) {
            workspace.set_visible(id.to_owned(), false, None, permissions).map_err(|e| e.to_string())?;
        }
        let to = match previous {
            Some(anchor) => Target::At { anchor, placement: Placement::Group { below: true, height: None } },
            None if in_dock_column(&workspace.layout, "layers") => Target::At { anchor: "layers".into(), placement: Placement::Column { after: true } },
            None => Target::NewDock { side: Side::End },
        };
        apply(&mut workspace, Action::MovePanel { panel: panel.into(), to })?;
        previous = Some(panel.to_owned());
    }
    apply(&mut workspace, Action::SetIconic { panel: "properties".into(), iconic: true })?;
    for panel in selected.into_iter().filter(|id| !entries.contains(&id.as_str())) {
        if workspace.layout.contains(&panel) {
            apply(&mut workspace, Action::Activate { panel })?;
        }
    }
    if !valid_workspace(&workspace) {
        return Err("invalid migrated panel launcher".into());
    }
    let returns = app.ui.docking_returns.clone();
    commit(app, workspace, returns);
    app.ui.panel_presentation.migrated = true;
    Ok(())
}

pub(super) fn columns(layout: &Layout<String>) -> Vec<(Side, &Column<String>)> {
    let mut pending: Vec<_> = layout.root.iter().collect();
    let mut out = Vec::new();
    while let Some(node) = pending.pop() {
        match node {
            Node::Dock { side, columns } => out.extend(columns.iter().map(|c| (*side, c))),
            Node::Split { first, second, .. } => pending.extend([second.as_ref(), first.as_ref()]),
            _ => {}
        }
    }
    out
}
pub(super) fn independent_window(window: &Floating<String>) -> bool {
    let mut pending = vec![&window.root];
    while let Some(node) = pending.pop() {
        if let Node::Split { axis, first, second, .. } = node {
            if *axis == SplitAxis::Horizontal {
                return true;
            }
            pending.extend([first.as_ref(), second.as_ref()]);
        }
    }
    false
}

pub(super) fn members(layout: &Layout<String>, panel: &str) -> Vec<String> {
    if let Some(window) = layout.floating.iter().find(|w| w.contains(&panel.to_owned())) {
        if independent_window(window) { group_members(layout, panel) } else { window.panels().cloned().collect() }
    } else if let Some((_, col)) = columns(layout).into_iter().find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|p| p == panel))) {
        col.groups.iter().flat_map(|g| g.panels.clone()).collect()
    } else {
        group_members(layout, panel)
    }
}
pub(super) fn compact(layout: &Layout<String>, settings: &PanelPresentation, panel: &str) -> bool {
    columns(layout)
        .into_iter()
        .find(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|p| p == panel)))
        .map_or_else(|| members(layout, panel).iter().all(|id| settings.compact.contains(id)), |(_, c)| c.iconic)
}

/// One-time migration of the previous default, preserving all nondefault personal sizes.
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
fn finite(params: &Value, key: &str, fallback: f32) -> Result<f32, String> {
    match params.get(key) {
        None => Ok(fallback),
        Some(v) => v.as_f64().filter(|v| v.is_finite() && v.abs() <= 1_000_000.0).map(|v| v as f32).ok_or_else(|| format!("{key} must be a finite number")),
    }
}
fn apply(workspace: &mut Workspace, action: Action<String>) -> Result<(), String> {
    workspace.apply(action, permissions).map(|_| ()).map_err(|e| e.to_string())
}
pub(crate) fn command(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let operation = params.get("operation").and_then(Value::as_str).unwrap_or("");
    if id == "window.panel.group" && operation == "drop" {
        return Some(super::presentation::drop_command(app, params));
    }
    let handled = id == "window.panel.edge"
        || (id == "window.panel.group"
            && matches!(
                operation,
                "collapse" | "expand" | "peek" | "strip-width" | "edge-size" | "flyout-size" | "tab" | "native" | "close-row" | "close-window"
            ));
    if !handled {
        return None;
    }
    Some(change(app, id, params))
}
fn change(app: &mut PhotocraftApp, id: &str, params: &Value) -> Result<Value, String> {
    let operation = params.get("operation").and_then(Value::as_str).unwrap_or("");
    if app.session.prefs().workspace_locked && operation != "peek" {
        return Err("workspace is locked".into());
    }
    let panel = params.get("panel").and_then(Value::as_str).and_then(normalize).ok_or("panel must name a registered panel")?.to_owned();
    let mut workspace = workspace_of(app)?;
    ensure(app, &mut workspace, &panel)?;
    let mut settings = app.ui.panel_presentation.clone();
    settings.migrated = true;
    let ids = members(&workspace.layout, &panel);
    let was_compact = compact(&workspace.layout, &settings, &panel);
    let mut flyout = app.ui.docking_flyout.clone().filter(|_| operation == "flyout-size");
    let mut root_closed = None;
    let mut changed_ids = ids.clone();
    if id == "window.panel.edge" {
        (changed_ids, root_closed) = attach_edge(&mut workspace, &mut settings, panel.clone(), &ids, was_compact, params)?;
    } else {
        match operation {
            "collapse" | "expand" => {
                if !floats(&workspace.layout, &panel) && !in_dock_column(&workspace.layout, &panel) {
                    root_closed = Some(operation == "collapse");
                }
                set_collapsed(&mut workspace, &mut settings, &panel, &ids, operation == "collapse", params)?;
            }
            "peek" => {
                apply(&mut workspace, Action::Activate { panel: panel.clone() })?;
                flyout = Some(panel.clone());
            }
            "strip-width" => {
                let width = finite(params, "width", super::COMPACT_COLUMN_WIDTH)?;
                if !(super::COMPACT_COLUMN_WIDTH..=240.0).contains(&width) {
                    return Err("strip width must be 42..240".into());
                }
                if in_dock_column(&workspace.layout, &panel) {
                    apply(&mut workspace, Action::ResizeIconic { panel: panel.clone(), width: Some(width) })?;
                }
                if !in_dock_column(&workspace.layout, &panel) {
                    for id in &ids {
                        settings.widths.insert(id.clone(), width);
                    }
                }
            }
            "edge-size" => resize_column(&mut workspace, &mut settings, &panel, &ids, was_compact, params)?,
            "flyout-size" => resize_flyout(&mut workspace, &mut settings, &panel, params)?,
            "tab" => {
                let anchor = params.get("anchor").and_then(Value::as_str).and_then(normalize).ok_or("tab requires an anchor")?;
                let closed = super::collapsed_members(app, &workspace.layout).contains(anchor);
                apply(&mut workspace, Action::Move { panel: panel.clone(), anchor: anchor.into(), placement: Placement::Tab { before: None } })?;
                changed_ids = vec![panel.clone()];
                if !floats(&workspace.layout, &panel) && !in_dock_column(&workspace.layout, &panel) {
                    root_closed = Some(closed);
                }
            }
            "close-window" => {
                apply(&mut workspace, Action::CloseWindow { panel: panel.clone() })?;
            }
            "close-row" => {
                apply(&mut workspace, Action::CloseGroup { panel: panel.clone() })?;
            }
            "native" => native_window(&mut workspace, &panel, params)?,
            _ => return Err("unsupported presentation operation".into()),
        }
    }
    if !valid_workspace(&workspace) {
        return Err("invalid panel workspace".into());
    }
    settings.compact.sort();
    settings.compact.dedup();
    let returns = app.ui.docking_returns.clone();
    app.ui.panel_presentation = settings;
    commit(app, workspace, returns);
    app.ui.docking_collapsed.retain(|id| !changed_ids.contains(id));
    if let Some(closed) = root_closed {
        let members = app.ui.docking.as_ref().map(|w| group_members(&w.layout, &panel)).unwrap_or_default();
        app.ui.docking_collapsed.retain(|id| !members.contains(id));
        if closed {
            app.ui.docking_collapsed.extend(members);
        } else {
            reveal_group(app, &panel);
        }
    }
    if operation == "expand" {
        for id in &ids {
            reveal_group(app, id);
        }
    }
    if let Some(panel) = flyout {
        app.ui.docking_flyout = Some(panel.clone());
        app.ui.docking_reveal = Some(panel);
    } else if app.ui.docking_flyout.as_ref().is_some_and(|p| ids.contains(p)) {
        app.ui.docking_flyout = None;
    }
    Ok(json!({"layout":app.ui.docking.as_ref().map(|w| &w.layout),"presentation":app.ui.panel_presentation}))
}

/// Attach without mixing edge placement branches with the other view commands.
fn attach_edge(
    workspace: &mut Workspace,
    settings: &mut PanelPresentation,
    panel: String,
    ids: &[String],
    was_compact: bool,
    params: &Value,
) -> Result<(Vec<String>, Option<bool>), String> {
    let mut root_closed = None;
    let edge = params.get("edge").and_then(Value::as_str).ok_or("edge must name a workspace side")?;
    let individual = match params.get("individual") {
        None => false,
        Some(v) => v.as_bool().ok_or("individual must be a boolean")?,
    };
    let collapsed = match params.get("collapsed") {
        None => was_compact,
        Some(v) => v.as_bool().ok_or("collapsed must be a boolean")?,
    };
    if matches!(edge, "top" | "bottom") {
        if let Some(size) = workspace
            .layout
            .floating
            .iter()
            .find(|w| w.contains(&panel) && !independent_window(w))
            .map(|w| [w.rect[2], w.rect[3]])
            .filter(|s| s[0] >= 220.0 && s[1] >= 120.0)
        {
            for id in ids {
                settings.floating_sizes.insert(id.clone(), size);
            }
        }
        root_closed = Some(collapsed);
    }
    let to = if matches!(edge, "top" | "bottom") {
        Target::At { anchor: CANVAS.into(), placement: Placement::Edge(if edge == "top" { super::Zone::Top } else { super::Zone::Bottom }) }
    } else if edge == "floating" {
        Target::Float {
            rect: Some([finite(params, "x", 120.0)?, finite(params, "y", 140.0)?, finite(params, "width", 320.0)?, finite(params, "height", 400.0)?]),
        }
    } else {
        let side = match edge {
            "left" => Side::Start,
            "rightInner" | "rightOuter" | "rightRail" => Side::End,
            _ => return Err("unknown workspace edge".into()),
        };
        if let Some(anchor) = params.get("stack").and_then(Value::as_str).and_then(normalize) {
            Target::At {
                anchor: anchor.into(),
                placement: Placement::Group { below: params.get("position").and_then(Value::as_str) != Some("top"), height: None },
            }
        } else if let Some(before) = params.get("before").and_then(Value::as_str).and_then(normalize) {
            Target::At { anchor: before.into(), placement: Placement::Column { after: false } }
        } else {
            let cols: Vec<_> = columns(&workspace.layout).into_iter().filter(|(s, _)| *s == side).collect();
            let target = if edge == "rightOuter" { cols.first() } else { cols.last() };
            let moved = if individual { std::slice::from_ref(&panel) } else { ids };
            match target.and_then(|(_, c)| c.groups.iter().flat_map(|g| &g.panels).find(|id| !moved.contains(id))) {
                Some(anchor) => Target::At { anchor: anchor.clone(), placement: Placement::Column { after: edge != "rightOuter" } },
                _ => Target::NewDock { side },
            }
        }
    };
    let action = if individual {
        Action::MovePanel { panel: panel.clone(), to }
    } else if workspace.layout.floating.iter().any(|w| w.contains(&panel)) {
        Action::MoveWindow { panel: panel.clone(), to }
    } else if in_dock_column(&workspace.layout, &panel) {
        Action::MoveColumn { panel: panel.clone(), to }
    } else {
        Action::MoveGroup { panel: panel.clone(), to }
    };
    let same_edge = matches!(&action,Action::MoveColumn {to:Target::NewDock {side},..} | Action::MovePanel {to:Target::NewDock {side},..} | Action::MoveGroup {to:Target::NewDock {side},..} if columns(&workspace.layout).into_iter().any(|(s,c)|s==*side && c.groups.iter().any(|g|g.panels.contains(&panel))));
    if !same_edge {
        apply(workspace, action)?;
    }
    let moved = if individual { vec![panel.clone()] } else { ids.to_vec() };
    settings.compact.retain(|id| !moved.contains(id));
    if in_dock_column(&workspace.layout, &panel) {
        apply(workspace, Action::SetIconic { panel: panel.clone(), iconic: collapsed })?;
    } else if collapsed && floats(&workspace.layout, &panel) {
        settings.compact.extend(moved.clone());
    }
    Ok((moved, root_closed))
}

fn set_collapsed(workspace: &mut Workspace, settings: &mut PanelPresentation, panel: &str, ids: &[String], iconic: bool, params: &Value) -> Result<(), String> {
    let gap = finite(params, "gap", 1.0)?;
    if !(0.0..=4.0).contains(&gap) {
        return Err("invalid panel gap".into());
    }
    if iconic && let Some(window) = workspace.layout.floating.iter().find(|w| w.contains(panel) && !independent_window(w)) {
        let size = [window.rect[2], window.rect[3]];
        if normal_size(size) {
            for id in window.panels() {
                settings.floating_sizes.entry(id.clone()).or_insert(size);
            }
        }
    }
    if iconic
        && params.get("normalSize").is_none()
        && params.get("normalSizes").is_none()
        && let Some(window) = workspace.layout.floating.iter().find(|w| w.contains(panel) && !independent_window(w))
    {
        let [x, y, w, h] = window.rect;
        remember_normal_frame(&window.root, egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h)), settings, gap, 0);
    }
    if let Some(raw) = params.get("windowRect") {
        let rect: [f32; 4] = serde_json::from_value(raw.clone()).map_err(|e| format!("invalid window rect: {e}"))?;
        apply(workspace, Action::MoveFloating { panel: panel.into(), rect })?;
    }
    if iconic && let Some(raw) = params.get("normalSize") {
        let size: [f32; 2] = serde_json::from_value(raw.clone()).map_err(|e| format!("invalid normal size: {e}"))?;
        if !size.iter().all(|v| v.is_finite()) || !(220.0..=100_000.0).contains(&size[0]) || !(120.0..=100_000.0).contains(&size[1]) {
            return Err("invalid normal size".into());
        }
        for id in ids {
            settings.floating_sizes.insert(id.clone(), size);
        }
    }
    if iconic && let Some(raw) = params.get("normalSizes") {
        let sizes = raw.as_object().ok_or("normalSizes must be an object")?;
        if sizes.len() > ids.len() {
            return Err("normalSizes names panels outside the group".into());
        }
        for (id, raw) in sizes {
            let size: [f32; 2] = serde_json::from_value(raw.clone()).map_err(|e| format!("invalid normal size: {e}"))?;
            if !ids.contains(id) || !normal_size(size) {
                return Err("invalid normal size or panel".into());
            }
            settings.stacked_sizes.insert(id.clone(), size);
        }
    }

    if !iconic
        && let Some(window) = workspace.layout.floating.iter_mut().find(|w| w.contains(panel))
        && !independent_window(window)
        && window.panels().all(|p| settings.compact.contains(p))
        && let restore_stacked = !matches!(window.root, Node::Tabs { .. })
        && let Some([width, height]) = normal_frame(&mut window.root, settings, gap, restore_stacked, 0)
    {
        window.rect[2] = width;
        window.rect[3] = height;
    }

    if !floats(&workspace.layout, panel) && !in_dock_column(&workspace.layout, panel) {
        settings.compact.retain(|id| !ids.contains(id));
        return Ok(());
    }
    if in_dock_column(&workspace.layout, panel) {
        apply(workspace, Action::SetIconic { panel: panel.into(), iconic })?;
    } else {
        if !iconic
            && (params.get("x").is_some() || params.get("y").is_some())
            && let Some(window) = workspace.layout.floating.iter().find(|w| w.contains(panel))
        {
            let mut rect = window.rect;
            rect[0] = finite(params, "x", rect[0])?;
            rect[1] = finite(params, "y", rect[1])?;
            apply(workspace, Action::MoveFloating { panel: panel.into(), rect })?;
        }
        if iconic && !workspace.layout.floating.iter().any(|w| w.contains(panel)) {
            let [width, height] = settings.floating_sizes.get(panel).copied().unwrap_or([320.0, 400.0]);
            let rect = [finite(params, "x", 120.0)?, finite(params, "y", 140.0)?, width, height];
            apply(workspace, Action::MoveGroup { panel: panel.into(), to: Target::Float { rect: Some(rect) } })?;
        }
        if iconic && workspace.layout.floating.iter().any(|w| w.native.is_some() && w.contains(panel)) {
            apply(workspace, Action::SetNative { panel: panel.into(), rect: None })?;
        }
        settings.compact.retain(|id| !ids.contains(id));
        if iconic {
            settings.compact.extend_from_slice(ids);
        }
    }
    Ok(())
}

fn normal_size(size: [f32; 2]) -> bool {
    size.iter().all(|v| v.is_finite()) && (220.0..=100_000.0).contains(&size[0]) && (120.0..=100_000.0).contains(&size[1])
}

fn remember_normal_frame(node: &Node<String>, rect: egui::Rect, settings: &mut PanelPresentation, gap: f32, depth: usize) {
    if depth > 64 {
        return;
    }
    match node {
        Node::Tabs { panels, .. } if !panels.iter().all(|p| settings.compact.contains(p)) && normal_size([rect.width(), rect.height()]) => {
            for panel in panels {
                settings.stacked_sizes.insert(panel.clone(), [rect.width(), rect.height()]);
            }
        }
        Node::Split { axis, size, first, second } => {
            if let Some(split) = craft_ui::layout::split_rect(rect, *axis, *size, gap, 0.0) {
                remember_normal_frame(first, split.first, settings, gap, depth + 1);
                remember_normal_frame(second, split.second, settings, gap, depth + 1);
            }
        }
        _ => {}
    }
}

/// Fit normal group sizes in the existing split tree; compact frame heights are not editor sizes.
fn normal_frame(node: &mut Node<String>, settings: &PanelPresentation, gap: f32, restore_stacked: bool, depth: usize) -> Option<[f32; 2]> {
    if depth > 64 {
        return None;
    }
    match node {
        Node::Tabs { panels, .. } if !panels.is_empty() => Some(panels.iter().fold([220.0_f32, 120.0_f32], |size, panel| {
            let next = settings
                .stacked_sizes
                .get(panel)
                .filter(|_| restore_stacked)
                .or_else(|| settings.floating_sizes.get(panel))
                .copied()
                .filter(|s| normal_size(*s))
                .unwrap_or([320.0, 400.0]);
            [size[0].max(next[0]), size[1].max(next[1])]
        })),
        Node::Split { axis, size, first, second } => {
            let a = normal_frame(first, settings, gap, restore_stacked, depth + 1)?;
            let b = normal_frame(second, settings, gap, restore_stacked, depth + 1)?;
            let horizontal = *axis == super::SplitAxis::Horizontal;
            let extent_a = if horizontal { a[0] } else { a[1] };
            let extent_b = if horizontal { b[0] } else { b[1] };
            let total = extent_a + extent_b;
            *size = super::SplitSize::Ratio(if total > 0.0 { extent_a / total } else { 0.5 });
            Some(if horizontal { [(a[0] + b[0] + gap).min(1_000_000.0), a[1].max(b[1])] } else { [a[0].max(b[0]), (a[1] + b[1] + gap).min(1_000_000.0)] })
        }
        _ => None,
    }
}

fn native_window(workspace: &mut Workspace, panel: &str, params: &Value) -> Result<(), String> {
    if cfg!(target_arch = "wasm32") {
        return Err("separate panel windows require the desktop app".into());
    }
    let on = params.get("enabled").and_then(Value::as_bool).ok_or("enabled must be a boolean")?;
    if !workspace.layout.floating.iter().any(|w| w.contains(panel)) {
        let to = Target::Float { rect: None };
        let action = if in_dock_column(&workspace.layout, panel) {
            Action::MoveColumn { panel: panel.to_owned(), to }
        } else {
            Action::MoveGroup { panel: panel.to_owned(), to }
        };
        apply(workspace, action)?;
    }
    let r = workspace.layout.floating.iter().find(|w| w.contains(panel)).map(|w| w.native.unwrap_or(w.rect)).ok_or("panel must be floating")?;
    let rect =
        if on { Some([finite(params, "x", r[0])?, finite(params, "y", r[1])?, finite(params, "width", r[2])?, finite(params, "height", r[3])?]) } else { None };
    apply(workspace, Action::SetNative { panel: panel.to_owned(), rect })?;
    Ok(())
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
    if let Ok(presentation) = serde_json::from_value(raw.clone()) {
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

/// Transfer the authoritative width only when an owner crosses between attached and floating.
pub(super) fn transfer_widths(previous: Option<&Layout<String>>, next: &mut Layout<String>, settings: &mut PanelPresentation) {
    let Some(previous) = previous else { return };
    for window in &previous.floating {
        if (220.0..=100_000.0).contains(&window.rect[2]) && window.rect[3] >= 120.0 {
            for id in window.panels().filter(|id| next.contains(id) && !next.floating.iter().any(|w| w.contains(id.as_str()))) {
                settings.floating_sizes.insert(id.clone(), [window.rect[2], window.rect[3]]);
            }
        }
    }
    let previously_attached: Vec<_> = columns(previous).into_iter().flat_map(|(_, c)| c.groups.iter().flat_map(|g| &g.panels)).collect();
    // Attached geometry is authoritative. Copy it only on an actual transition to float.
    for (_, column) in columns(previous).into_iter().filter(|(_, c)| c.iconic) {
        let width = column.iconic_width.unwrap_or(super::COMPACT_COLUMN_WIDTH).clamp(super::COMPACT_COLUMN_WIDTH, 240.0);
        for id in column.groups.iter().flat_map(|g| &g.panels).filter(|id| next.floating.iter().any(|w| w.contains(id.as_str()))) {
            settings.widths.insert(id.clone(), width);
        }
    }
    let mut pending: Vec<_> = next.root.iter_mut().collect();
    while let Some(node) = pending.pop() {
        match node {
            Node::Split { first, second, .. } => pending.extend([first.as_mut(), second.as_mut()]),
            Node::Dock { columns, .. } => {
                for column in columns.iter_mut().filter(|c| c.iconic) {
                    if column.groups.iter().flat_map(|g| &g.panels).any(|id| previously_attached.contains(&id)) {
                        continue;
                    }
                    if let Some(id) = column.groups.iter().flat_map(|g| &g.panels).find(|id| previous.floating.iter().any(|w| w.contains(id.as_str()))) {
                        column.iconic_width =
                            Some(settings.widths.get(id).copied().unwrap_or(super::COMPACT_COLUMN_WIDTH).clamp(super::COMPACT_COLUMN_WIDTH, 240.0));
                    }
                }
            }
            _ => {}
        }
    }
}

/// Keep appearance metadata synchronized when modern Workspace commands move a column.
pub(super) fn sync_presentation(layout: &Layout<String>, settings: &mut PanelPresentation) {
    for (_, column) in columns(layout) {
        for id in column.groups.iter().flat_map(|g| &g.panels) {
            settings.compact.retain(|p| p != id);
            if column.iconic {
                settings.compact.push(id.clone());
            }
        }
    }
    settings.compact.sort();
    settings.compact.dedup();
}

fn resize_column(
    workspace: &mut Workspace,
    settings: &mut PanelPresentation,
    panel: &str,
    ids: &[String],
    was_compact: bool,
    params: &Value,
) -> Result<(), String> {
    if params.get("width").is_some() {
        let width = finite(params, "width", 320.0)?;
        let (lo, hi) = if was_compact { (super::COMPACT_COLUMN_WIDTH, 240.0) } else { (220.0, 520.0) };
        if !(lo..=hi).contains(&width) {
            return Err("column width is out of range".into());
        }
        if in_dock_column(&workspace.layout, panel) {
            apply(
                workspace,
                if was_compact {
                    Action::ResizeIconic { panel: panel.to_owned(), width: Some(width) }
                } else {
                    Action::ResizeColumn { panel: panel.to_owned(), width }
                },
            )?;
        } else if let Some(w) = workspace.layout.floating.iter().find(|w| w.contains(panel)) {
            if was_compact {
                for id in ids {
                    settings.widths.insert(id.clone(), width);
                }
            } else {
                let mut r = w.rect;
                r[2] = width;
                apply(workspace, Action::MoveFloating { panel: panel.to_owned(), rect: r })?;
            }
        }
    }
    if let Some(height) = params.get("height") {
        if height.is_null() {
            for id in ids {
                settings.heights.remove(id);
            }
        } else {
            let h = finite(params, "height", 400.0)?;
            if !(100.0..=100_000.0).contains(&h) {
                return Err("height must be 100..100000".into());
            }
            if let Some(w) = workspace.layout.floating.iter().find(|w| w.contains(panel)) {
                let mut rect = w.rect;
                rect[3] = h;
                apply(workspace, Action::MoveFloating { panel: panel.into(), rect })?;
            } else if let Some(previous) = settings
                .heights
                .get(panel)
                .copied()
                .or(params.get("previousHeight").and_then(Value::as_f64).filter(|v| v.is_finite() && *v >= 100.0 && *v <= 100_000.0).map(|v| v as f32))
                .filter(|h| *h > 0.0)
            {
                let rows: Vec<_> = columns(&workspace.layout)
                    .into_iter()
                    .filter(|(_, c)| c.groups.iter().any(|g| g.panels.iter().any(|id| id == panel)))
                    .flat_map(|(_, c)| c.groups.iter())
                    .filter_map(|g| Some((g.panels.first()?.clone(), g.height? * h / previous)))
                    .collect();
                for (panel, height) in rows {
                    apply(workspace, Action::ResizeGroup { panel, height: Some(height) })?;
                }
            }
            for id in ids {
                settings.heights.insert(id.clone(), h);
            }
        }
    }
    if let Some(weights) = params.get("heights") {
        let weights = weights.as_object().ok_or("heights must be an object")?;
        for (id, v) in weights {
            if !ids.contains(id) || !v.as_f64().is_some_and(|v| v.is_finite() && v > 0.0 && v <= 1_000_000.0) {
                return Err("invalid row height weight".into());
            }
        }
        let height = settings.heights.get(panel).copied().unwrap_or(600.0);
        if let Some(window) = workspace.layout.floating.iter_mut().find(|w| w.contains(panel)) {
            resize_floating_rows(&mut window.root, weights, height, 0)?;
            return Ok(());
        }
        let sum: f64 = weights.values().filter_map(Value::as_f64).sum();
        for (id, v) in weights {
            apply(
                workspace,
                Action::ResizeGroup { panel: id.clone(), height: Some((v.as_f64().unwrap_or(1.0) / sum.max(f64::EPSILON) * f64::from(height)) as f32) },
            )?;
        }
    }

    Ok(())
}

fn resize_floating_rows(node: &mut Node<String>, weights: &serde_json::Map<String, Value>, height: f32, depth: usize) -> Result<f32, String> {
    if depth > 64 {
        return Err("panel tree is too deep".into());
    }
    match node {
        Node::Tabs { panels, .. } => {
            let sum: f64 = panels.iter().filter_map(|id| weights.get(id)?.as_f64()).sum();
            Ok(if sum > 0.0 { sum as f32 } else { 1.0 })
        }
        Node::Split { axis: SplitAxis::Vertical, first, second, size } => {
            let a = resize_floating_rows(first, weights, height, depth + 1)?;
            let b = resize_floating_rows(second, weights, height, depth + 1)?;
            *size = SplitSize::Ratio(a / (a + b).max(f32::EPSILON));
            Ok(a + b)
        }
        Node::Stack { entries } => {
            let sum: f64 = entries.iter().map(|e| weights.get(&e.panel).and_then(Value::as_f64).unwrap_or(1.0)).sum();
            for entry in entries {
                entry.height = Some((weights.get(&entry.panel).and_then(Value::as_f64).unwrap_or(1.0) / sum.max(f64::EPSILON)) as f32 * height);
            }
            Ok(sum as f32)
        }
        _ => Err("row heights require a vertical panel stack".into()),
    }
}

fn resize_flyout(workspace: &mut Workspace, settings: &mut PanelPresentation, panel: &str, params: &Value) -> Result<(), String> {
    let defaults = settings.flyouts.get(panel).copied().unwrap_or([320.0, 440.0]);
    let width = finite(params, "width", defaults[0])?;
    let height = finite(params, "height", defaults[1])?;
    if !(220.0..=100_000.0).contains(&width) || !(120.0..=100_000.0).contains(&height) {
        return Err("flyout size is out of range".into());
    }
    if in_dock_column(&workspace.layout, panel) {
        apply(workspace, Action::ResizeColumn { panel: panel.into(), width: width.min(520.0) })?;
    } else if let Some(window) = workspace.layout.floating.iter().find(|w| w.contains(panel) && !independent_window(w)) {
        let mut rect = window.rect;
        rect[2] = width;
        rect[3] = height;
        apply(workspace, Action::MoveFloating { panel: panel.into(), rect })?;
    }
    for id in group_members(&workspace.layout, panel) {
        settings.floating_sizes.insert(id, [width, height]);
    }
    settings.flyouts.insert(panel.into(), [width, height]);
    Ok(())
}
