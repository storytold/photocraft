//! The canvas Shift+right-click menu of the painting tools, as in the reference application: Edit
//! Brush… (the Brush Preset picker a plain right-click opens) and the tool's own Mode list from
//! its options bar (blend modes, the Eraser's Brush / Pencil / Block, the Dodge and Burn Range,
//! the Sponge's mode). Every row sets exactly what the tool's options-bar control sets, through
//! the same command (`tools.setBrush` for a brush-based tool's mode) or tool option.

use craft::keymap::Mods;
use craft::ui::contract::Entry; // PLAN: assumed path of the framework `Entry`
use photocraft_doc::BlendMode;
use serde_json::{Value, json};

use crate::canvas_tool_menu::MenuKind;
use crate::{PhotocraftApp, state::Tool};

/// One row of the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    /// Opens the Brush Preset picker at the click point.
    EditBrush,
    Separator,
    /// The tool's blend mode.
    Blend(BlendMode),
    /// Painting-only: paint behind the layer's pixels (Brush, Pencil and Pattern Stamp).
    Behind,
    /// Painting-only: paint transparency (Brush, Pencil, Paint Bucket).
    Clear,
    /// The Eraser's Mode: `brush`, `pencil` or `block`.
    Eraser(&'static str),
    /// The Dodge and Burn Range: `shadows`, `midtones` or `highlights`.
    Range(&'static str),
    /// The Sponge's mode: `desaturate` or `saturate`.
    Sponge(&'static str),
}

impl Item {
    /// The English label (translated when drawn); `None` for a separator.
    pub(crate) fn label(self) -> Option<&'static str> {
        Some(match self {
            Item::EditBrush => "Edit Brush…",
            Item::Separator => return None,
            Item::Blend(m) => m.label(),
            Item::Behind => "Behind",
            Item::Clear => "Clear",
            Item::Eraser(m) => match m {
                "pencil" => "Pencil",
                "block" => "Block",
                _ => "Brush",
            },
            Item::Range(r) => match r {
                "shadows" => "Shadows",
                "highlights" => "Highlights",
                _ => "Midtones",
            },
            Item::Sponge(m) => match m {
                "saturate" => "Saturate",
                _ => "Desaturate",
            },
        })
    }
}

/// The Brush's and Pencil's Mode list in the options bar's order, Behind and Clear after Dissolve:
/// `(mode, behind)`, `(None, false)` being Clear.
pub(crate) fn choices() -> Vec<((Option<BlendMode>, bool), &'static str)> {
    let mut choices: Vec<_> = BlendMode::LAYER_MODES.iter().map(|m| ((Some(*m), false), m.label())).collect();
    let at = choices.iter().position(|o| o.0 == (Some(BlendMode::Dissolve), false)).map_or(choices.len(), |i| i + 1);
    choices.insert(at, ((None, true), "Behind"));
    choices.insert(at + 1, ((None, false), "Clear"));
    choices
}

/// The Pattern Stamp's Mode list in the options bar's order: [`choices`] without Clear, which the
/// reference application's list doesn't have (#12879).
pub(crate) fn pattern_stamp_choices() -> Vec<((Option<BlendMode>, bool), &'static str)> {
    choices().into_iter().filter(|c| c.0 != (None, false)).collect()
}

/// The blend modes in their menu groups (a separator before Darken, Lighten, Overlay, Difference
/// and Hue), with Behind and/or Clear after Dissolve.
fn blend_items(behind: bool, clear: bool) -> Vec<Item> {
    let mut out = Vec::new();
    for m in BlendMode::LAYER_MODES {
        if crate::panels::blend_shortcut(m).is_some_and(craft::ui::patterns::blend_keys::BlendShortcut::starts_group) {
            out.push(Item::Separator);
        }
        out.push(Item::Blend(m));
        if m == BlendMode::Dissolve {
            if behind {
                out.push(Item::Behind);
            }
            if clear {
                out.push(Item::Clear);
            }
        }
    }
    out
}

/// `tool`'s menu, top to bottom. Empty for a tool without one (its Shift+right-click keeps its
/// other route). Only what the tool's options bar offers is listed: the reference application's
/// Behind for the Clone Stamp, History Brush, Gradient and Paint Bucket, the Blur, Sharpen and Smudge
/// modes, the Spot Healing modes and the Healing Brush's source and Replace have no setting here.
pub(crate) fn items(tool: Tool) -> Vec<Item> {
    let with_brush = |rest: Vec<Item>| std::iter::once(Item::EditBrush).chain(std::iter::once(Item::Separator)).chain(rest).collect::<Vec<_>>();
    match tool {
        Tool::Brush | Tool::Pencil => with_brush(blend_items(true, true)),
        Tool::PatternStamp => with_brush(blend_items(true, false)),
        Tool::CloneStamp | Tool::HistoryBrush => with_brush(blend_items(false, false)),
        Tool::MixerBrush | Tool::ColorReplacement | Tool::BackgroundEraser | Tool::Blur | Tool::Sharpen | Tool::Smudge => vec![Item::EditBrush],
        Tool::Eraser => ["brush", "pencil", "block"].into_iter().map(Item::Eraser).collect(),
        Tool::Dodge | Tool::Burn => with_brush(["shadows", "midtones", "highlights"].into_iter().map(Item::Range).collect()),
        Tool::Sponge => with_brush(["desaturate", "saturate"].into_iter().map(Item::Sponge).collect()),
        Tool::Healing => crate::blend_keys::HEALING_MODES.into_iter().map(Item::Blend).collect(),
        Tool::Gradient => blend_items(false, false),
        Tool::PaintBucket => blend_items(false, true),
        _ => Vec::new(),
    }
}

/// The rows' labels, separators left out (what `ui.inspect` reports as `choices`).
pub(crate) fn labels(tool: Tool) -> Vec<&'static str> {
    items(tool).into_iter().filter_map(Item::label).collect()
}

/// Is a row available? Behind and Clear need transparency: greyed on the Background layer.
pub(crate) fn enabled(app: &PhotocraftApp, item: Item) -> bool {
    match item {
        Item::Separator => false,
        Item::Behind | Item::Clear => !app
            .session
            .active()
            .is_some_and(|st| st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| crate::doc_props_ui::is_background(&st.doc, l))),
        _ => true,
    }
}

/// The checked row: `tool`'s current mode, as its options bar shows it.
pub(crate) fn current(app: &PhotocraftApp, tool: Tool) -> Option<Item> {
    let o = &app.ui.tool_options;
    let brush = &app.session.tools.brush;
    let painting = |clear: bool| {
        if clear {
            Item::Clear
        } else if brush.behind {
            Item::Behind
        } else {
            Item::Blend(brush.mode)
        }
    };
    let item = match tool {
        Tool::Brush => painting(o.brush_clear),
        Tool::Pencil => painting(o.pencil_clear),
        Tool::PatternStamp => painting(false),
        Tool::CloneStamp | Tool::HistoryBrush | Tool::Healing => Item::Blend(crate::blend_keys::brush_tool_mode(tool, brush.mode)),
        Tool::Gradient => Item::Blend(o.gradient_blend_mode),
        Tool::PaintBucket if o.bucket_clear => Item::Clear,
        Tool::PaintBucket => Item::Blend(o.bucket_blend_mode),
        _ => return items(tool).into_iter().find(|i| ui_value(app, *i).is_some_and(|v| v)),
    };
    Some(item)
}

/// Whether a tool-option row (Eraser, Range, Sponge) is the current value; `None` for other rows.
fn ui_value(app: &PhotocraftApp, item: Item) -> Option<bool> {
    let o = &app.ui.tool_options;
    Some(match item {
        Item::Eraser(m) => o.eraser_mode == m,
        Item::Range(r) => o.tone_range == r,
        Item::Sponge(m) => o.sponge_mode == m,
        _ => return None,
    })
}

/// Does the active tool have a menu for this gesture (Shift, without Alt, Ctrl or Command)?
pub(crate) fn is_gesture(tool: Tool, mods: Mods) -> bool {
    mods.shift && !mods.alt && !mods.ctrl && !mods.cmd && !items(tool).is_empty()
}

/// Does a right-click with `mods` at document point `doc` open the menu? The Shift gesture for
/// every tool that has one, and a plain right-click with the Eraser in Block mode, where the Brush
/// Preset picker's Size and Hardness mean nothing (#11937), or with the Paint Bucket (#12530), as
/// in the reference application.
/// A plain right-click opens it only where it would otherwise open the picker or nothing: with the
/// Erase preference it still erases, and on the pasteboard its colour menu opens. `doc` is `None`
/// where the press has no document point (the pasteboard check is then the caller's).
pub(crate) fn opens_on(app: &PhotocraftApp, tool: Tool, mods: Mods, doc: Option<[f64; 2]>) -> bool {
    let plain = mods.is_none();
    let block = tool == Tool::Eraser && app.ui.tool_options.eraser_mode == "block";
    let pasteboard = doc.is_some_and(|d| crate::canvas_tool_menu::pasteboard_at(app, tool, d));
    // An open Free Transform box owns the plain right-click: its transform menu opens instead.
    let bucket = tool == Tool::PaintBucket && app.ui.transform.is_none();
    is_gesture(tool, mods) || (plain && (block || bucket) && !crate::paint_mouse::right_erases(app, tool) && !pasteboard)
}

/// The engine command a row runs, if any: a brush-based tool's mode goes through
/// `tools.setBrush`, as its options-bar dropdown does. The Brush, Pencil and Pattern Stamp also
/// set or clear Behind (Clear keeps the underlying mode, as the options bar does).
pub(crate) fn engine_call(app: &PhotocraftApp, tool: Tool, item: Item) -> Option<(&'static str, Value)> {
    let brush = match (tool, item) {
        (Tool::Brush | Tool::Pencil | Tool::PatternStamp, Item::Blend(m)) => json!({"mode": m, "behind": false}),
        (Tool::Brush | Tool::Pencil | Tool::PatternStamp, Item::Behind) => json!({"mode": BlendMode::Normal, "behind": true}),
        (Tool::Brush | Tool::Pencil, Item::Clear) => json!({"mode": app.session.tools.brush.mode, "behind": false}),
        (Tool::CloneStamp | Tool::HistoryBrush | Tool::Healing, Item::Blend(m)) => json!({"mode": m}),
        _ => return None,
    };
    Some(("tools.setBrush", json!({ "brush": brush })))
}

/// The row of the open menu whose English label is `label`, if it can be chosen now.
pub(crate) fn find(app: &PhotocraftApp, label: &str) -> Result<Item, String> {
    if !app.ui.canvas_tool_menu.as_ref().is_some_and(|m| matches!(m.kind, MenuKind::BrushMode { .. }) && m.tool == app.ui.tool) {
        return Err("no Shift+right-click menu is open".into());
    }
    let item = items(app.ui.tool).into_iter().find(|i| i.label() == Some(label)).ok_or_else(|| format!("the menu has no row `{label}`"))?;
    if !enabled(app, item) {
        return Err(format!("`{label}` is unavailable"));
    }
    Ok(item)
}

/// Chooses `item` for the active tool and closes the menu.
pub(crate) fn choose(app: &mut PhotocraftApp, item: Item) {
    let tool = app.ui.tool;
    let pos = app.ui.canvas_tool_menu.take().filter(|m| matches!(m.kind, MenuKind::BrushMode { .. })).map(|m| m.pos);
    if !enabled(app, item) {
        return;
    }
    crate::paint_mouse::sync_tool_brush(app);
    if let Some((id, params)) = engine_call(app, tool, item) {
        match app.run(id, params) {
            Ok(_) => match tool {
                Tool::Brush => app.ui.tool_options.brush_clear = item == Item::Clear,
                Tool::Pencil => app.ui.tool_options.pencil_clear = item == Item::Clear,
                _ => {}
            },
            Err(e) => app.ui.status = e,
        }
        return;
    }
    match item {
        Item::EditBrush => {
            if let Some(at) = pos {
                crate::brush_picker::open(app, at);
            }
        }
        Item::Eraser(m) => app.ui.tool_options.eraser_mode = m.into(),
        Item::Range(r) => app.ui.tool_options.tone_range = r.into(),
        Item::Sponge(m) => app.ui.tool_options.sponge_mode = m.into(),
        Item::Blend(m) if tool == Tool::Gradient => crate::blend_keys::set_gradient_mode(app, m),
        Item::Blend(m) if tool == Tool::PaintBucket => {
            app.ui.tool_options.bucket_clear = false;
            app.ui.tool_options.bucket_blend_mode = m;
        }
        Item::Clear if tool == Tool::PaintBucket => app.ui.tool_options.bucket_clear = true,
        _ => {}
    }
}

/// The row drawn checked. The Paint Bucket's plain right-click menu checks none, as in the
/// reference application; its Shift+right-click menu checks the current mode. `plain` is whether
/// the menu was opened by a plain right-click (`MenuKind::BrushMode { plain }`).
pub(crate) fn checked(app: &PhotocraftApp, tool: Tool, plain: bool) -> Option<Item> {
    if plain && tool == Tool::PaintBucket { None } else { current(app, tool) }
}

/// `tool`'s menu rows for the framework menu: the English label is the row id (what
/// `ui.context.choose` sends), the checked row is [`checked`], and Behind and Clear are greyed
/// on the Background. The tool's own brush is synced when the menu opens.
pub(crate) fn entries(app: &PhotocraftApp, tool: Tool, plain: bool) -> Vec<Entry<String>> {
    let current = checked(app, tool, plain);
    items(tool)
        .into_iter()
        .map(|item| match item.label() {
            Some(label) => Entry::new(label.to_string(), tl!(label).to_string()).enabled(enabled(app, item)).checked(Some(item) == current),
            None => Entry::separator(),
        })
        .collect()
}

#[cfg(test)]
#[path = "eraser_block_right_click_tests.rs"]
mod eraser_block_right_click_tests;

#[cfg(test)]
#[path = "pattern_stamp_behind_tests.rs"]
mod pattern_stamp_behind_tests;

#[cfg(test)]
#[path = "paint_bucket_right_click_menu_tests.rs"]
mod paint_bucket_right_click_menu_tests;
