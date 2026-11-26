//! View extras: rulers (⌘R), grid (⌘'), guides (⌘;) on the framework's `Rulers`, `CanvasGrid` and `Guides`. Guides
//! are dragged out of the rulers and moved with the Move tool (dropped off the document or on a ruler they go), and
//! every change runs as an undoable engine command (`view.newGuide` / `view.moveGuide` / `view.deleteGuide`).

use craft::behavior::kurbo::{Point, Rect as DocRect, Vec2};
use craft::behavior::{Axis, EditEvent};
use craft::theme::Rgba;
use craft::ui::canvas::{Guide, GuideDrag, GuideKey, GuideLine, Rulers};
use craft::ui::canvas_controls::{CanvasGrid, GridLine, Guides, GuidesOutput};
use craft::ui::contract::{Bind, Show as _};
use craft::ui::runtime::env::UnitKind;
use craft::ui::style::DataColor;
use egui::Color32;
use photocraft_engine::prefs::{GuidesGridAndSlices, LineStyle, Unit};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::document_display::CanvasHost;
use crate::canvas::tools::Family;
use crate::guide_select;
use crate::prefs::unit_kind;
use crate::state::Tool;

/// Every length unit, in the Preferences › Units & Rulers order.
const UNITS: [Unit; 7] = [Unit::Pixels, Unit::Inches, Unit::Centimeters, Unit::Millimeters, Unit::Points, Unit::Picas, Unit::Percent];

/// [`UNITS`] as the rulers' own units menu lists them.
static UNIT_KINDS: [UnitKind; 7] =
    [UnitKind::Pixels, UnitKind::Inches, UnitKind::Centimetres, UnitKind::Millimetres, UnitKind::Points, UnitKind::Picas, UnitKind::Percent];

/// Grid spacing in document pixels. Percentages use each axis's own extent.
pub(crate) fn grid_major_px(g: &GuidesGridAndSlices, dpi: f64, size: [f64; 2], ppi: f64) -> [f64; 2] {
    size.map(|extent| g.major_px(dpi, extent, ppi))
}

/// A `#rrggbb` preference colour.
fn pref_rgb(s: &str, fallback: [u8; 3]) -> [u8; 3] {
    photocraft_engine::prefs::parse_hex(s).map_or(fallback, |c| [c[0], c[1], c[2]])
}

/// The framework's line for a Guides, Grid & Slices line style.
fn guide_line(style: LineStyle) -> GuideLine {
    match style {
        LineStyle::Lines => GuideLine::Solid,
        LineStyle::Dashed => GuideLine::Dashed,
        LineStyle::Dots => GuideLine::Dots,
    }
}

fn orientation(vertical: bool) -> &'static str {
    if vertical { "vertical" } else { "horizontal" }
}

/// A vertical guide runs along the y axis, at an x position.
fn is_vertical(along: Axis) -> bool {
    along == Axis::Y
}

fn along(vertical: bool) -> Axis {
    if vertical { Axis::Y } else { Axis::X }
}

/// A guide position as the engine stores it.
fn round(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// The guides `rulers::show` hands to `Guides`: `list` (the document's, or a dialog's preview) keyed with
/// [`guide_select::key`], drawn in the preference style, in each guide's own colour or `color`.
fn guides_of(list: &photocraft_doc::Guides, color: [u8; 3], line: GuideLine, locked: bool) -> Vec<Guide> {
    let mut out = Vec::with_capacity(list.vertical.len() + list.horizontal.len());
    for (vertical, positions) in [(true, &list.vertical), (false, &list.horizontal)] {
        for (index, &position) in positions.iter().enumerate() {
            let [r, g, b] = list.color(vertical, position).unwrap_or(color);
            let mut guide = Guide::new(guide_select::key(vertical, index), along(vertical), position);
            guide.color = Some(Rgba::rgb(r, g, b));
            guide.line = line;
            guide.locked = locked;
            out.push(guide);
        }
    }
    out
}

/// Whether `tool` takes guides with the platform command key held: the selection tools and every tool on which ⌘ is
/// the Move tool (`tools::takes_command_stand_in`), not while a lasso outline is open, Free Transform runs or type
/// is being edited. A tool's own drag holds the pointer, so the guides get no press during one.
fn moves_with_command(app: &PhotocraftApp, tool: Tool) -> bool {
    let takes = crate::canvas::tools::takes_command_stand_in(tool) || crate::canvas::tools::family(tool) == Family::Selection;
    let idle = !app.canvas_tools.selection.outline_open() && app.ui.transform.is_none() && app.ui.text_edit.is_none();
    takes && idle
}

/// The rulers of document `host.idx`'s canvas, which the canvas shows along its top and left edges while View >
/// Rulers is on: the view lies inside them. `zero` holds the zero point they edit; the document's own is set here.
/// Only the active document's canvas takes the pointer. [`run_rulers`] runs what they reported.
pub(crate) fn strips<'a>(app: &PhotocraftApp, host: CanvasHost, zero: &'a mut Point) -> Option<Rulers<'a>> {
    let doc = app.session.documents().get(host.idx)?.doc.clone();
    *zero = Point::new(doc.ruler_origin[0], doc.ruler_origin[1]);
    let primary = app.session.active_index() == Some(host.idx);
    app.ui.extras.rulers.then(|| Rulers::new("Rulers", Bind::bound(zero)).interactive(primary).units(&UNIT_KINDS[..]))
}

/// The view extras of document `host.idx`'s canvas, in the canvas body after the selection overlay and before the
/// tools: the grid and the guides, with every edit run as its engine command. Only the active document's
/// canvas takes the pointer; another tile or window shows them as they are. Returns the guides' output, which
/// `canvas_tool_menu::run` reads for the guide menu.
pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui, host: CanvasHost, ruler: Option<GuideDrag>) -> Option<GuidesOutput> {
    let doc = app.session.documents().get(host.idx).map(|state| state.doc.clone())?;
    let primary = app.session.active_index() == Some(host.idx);
    let content = ui.max_rect();
    let prefs = app.session.prefs();
    let g = prefs.guides_grid_and_slices.clone();
    let ppi = prefs.units_and_rulers.point_size.per_inch();
    let (w, h) = (f64::from(doc.size.width), f64::from(doc.size.height));
    let zero = Point::new(doc.ruler_origin[0], doc.ruler_origin[1]);
    let page = DocRect::new(0.0, 0.0, w, h);

    if app.ui.extras.grid && app.ui.view.extras {
        let major = grid_major_px(&g, f64::from(doc.resolution_dpi.max(1.0)), [w, h], ppi);
        let [r, gr, b] = pref_rgb(&g.grid_color, [140, 140, 140]);
        CanvasGrid::new("Grid")
            .spacing(Vec2::new(major[0], major[1]))
            .subdivisions(g.subdivisions.max(1))
            .origin(zero)
            .bounds(page)
            .color(DataColor::new(Color32::from_rgb(r, gr, b)))
            .line(GridLine::from(guide_line(g.grid_style)))
            .show_at(ui, content);
    }

    // New Guide's edit preview, else Guide Layout's, else the document's guides. Guides show while a preview is open
    // even with View › Show › Guides off; a preview only draws.
    let edit_preview = crate::new_guide_ui::edit_preview(app, &doc).cloned();
    let layout_preview = crate::guide_layout_ui::preview(app, &doc);
    let previewing = edit_preview.is_some() || layout_preview.is_some();
    let preference = pref_rgb(&g.guide_color, [74, 255, 255]);
    let (list, color) = match (&edit_preview, &layout_preview) {
        (Some(list), _) => (list.clone(), preference),
        (None, Some((list, c))) => (list.clone(), [c.r(), c.g(), c.b()]),
        (None, None) => (doc.guides.clone(), preference),
    };
    let visible = app.ui.extras.guides && app.ui.view.shows(app.ui.view.show.canvas_guides);
    // A guide dragged out of a ruler is placed with the guides hidden, and placing it shows them.
    if !visible && !previewing && ruler.is_none() {
        return None;
    }
    let shown = guides_of(&list, color, guide_line(g.guide_style), app.ui.extras.lock_guides);
    // The selected guide, found by its position in the list shown; none while New Guide edits a guide.
    let selected = edit_preview.is_none().then(|| guide_select::selected(app)).flatten().and_then(|(vertical, index)| {
        let positions = if vertical { &doc.guides.vertical } else { &doc.guides.horizontal };
        let position = *positions.get(index)?;
        shown.iter().find(|guide| is_vertical(guide.along) == vertical && guide.position == position).map(|guide| guide.key)
    });
    let takes = primary && !previewing && guide_select::interactive(app);
    let tool = app.ui.tool;
    let mut bound = shown.clone();
    let mut guides = Guides::new("Guides", host.canvas_id(), Bind::bound(&mut bound))
        .interactive(takes && tool == Tool::Move)
        // PLAN: the sheet makes the selection tools `interactive`; they take guides only with ⌘ held, as
        // `command_guide_at` had it, so a plain marquee drag over a guide still draws a marquee.
        .moves_with_command(takes && tool != Tool::Move && moves_with_command(app, tool))
        .bounds(page);
    if let Some(key) = &selected {
        guides = guides.selected_guides(std::slice::from_ref(key));
    }
    let output = guides.show_at(ui, content);
    // A preview's guides are not the document's: nothing they report is an edit.
    if primary && !previewing {
        let ctx = ui.ctx().clone();
        run_guides(app, &ctx, &shown, &output);
    }
    Some(output)
}

/// The rulers' zero point and unit pick, as engine commands.
pub(crate) fn run_rulers(app: &mut PhotocraftApp, rulers: &craft::ui::canvas::RulersOutput) {
    let result = match rulers.zero.events.ended() {
        // A double click on the corner and the menu's Reset Zero Point set the origin back.
        Some(EditEvent::Set { after, .. }) if *after == Point::ZERO => app.run("view.resetRulerOrigin", json!({})).map(drop),
        Some(EditEvent::Commit { after, .. } | EditEvent::Set { after, .. }) => {
            app.run("view.setRulerOrigin", json!({"x": round(after.x), "y": round(after.y)})).map(drop)
        }
        _ => Ok(()),
    };
    if let Err(e) = result {
        app.ui.status = e;
    }
    if let Some(unit) = rulers.unit_picked.and_then(|kind| UNITS.into_iter().find(|&u| unit_kind(u) == kind)) {
        let _ = app.run("prefs.set", json!({"path": "unitsAndRulers.rulers", "value": unit.name()}));
    }
}

/// The guides' edit, selection and clicks as engine commands and the app's guide selection. `shown` is the list the
/// guides were given: only its keys decode to a document guide, and a committed guide with another key is the one a
/// ruler drag created. The guides report an edit, a click or a selection only in the pass that took the input, so a
/// second pass of a frame runs nothing again. A click with no drag commits nothing: it only selects (or, with ⌥, turns).
fn run_guides(app: &mut PhotocraftApp, ctx: &egui::Context, shown: &[Guide], output: &GuidesOutput) {
    let passed = |key: GuideKey| shown.iter().find(|guide| guide.key == key);
    // Where the selection lands after the commands ran: (vertical, position). A guide the commands placed is at its
    // rounded position; one they left alone keeps the document's own.
    let select_at = |after: &[Guide], key: GuideKey| {
        let guide = after.iter().find(|guide| guide.key == key)?;
        let unchanged = passed(key).is_some_and(|before| before.position == guide.position && before.along == guide.along);
        Some((is_vertical(guide.along), if unchanged { guide.position } else { round(guide.position) }))
    };
    let committed = output.edit.events.committed();
    if let Some(after) = committed {
        let mut status = Ok(());
        // A drag moves one guide and a delete removes one, so a commit carries one change; deletions run from the
        // highest index so the ones before keep their numbers.
        for guide in after {
            let Some(before) = passed(guide.key) else { continue };
            if before.position != guide.position || before.along != guide.along {
                let (vertical, index) = guide_select::unkey(guide.key);
                let params = json!({
                    "orientation": orientation(vertical),
                    "index": index,
                    "position": round(guide.position),
                    "toOrientation": orientation(is_vertical(guide.along)),
                });
                status = status.and(app.run("view.moveGuide", params).map(drop));
            }
        }
        let mut gone: Vec<(bool, usize)> = shown.iter().filter(|g| !after.iter().any(|a| a.key == g.key)).map(|g| guide_select::unkey(g.key)).collect();
        gone.sort_by_key(|a| std::cmp::Reverse(a.1));
        for (vertical, index) in gone {
            status = status.and(app.run("view.deleteGuide", guide_select::delete_params(vertical, index)).map(drop));
        }
        for guide in after.iter().filter(|guide| passed(guide.key).is_none()) {
            let params = json!({"orientation": orientation(is_vertical(guide.along)), "position": round(guide.position)});
            status = status.and(app.run("view.newGuide", params).map(drop));
            // A guide placed out of a ruler shows the guides.
            app.ui.extras.guides = true;
        }
        if let Err(e) = status {
            app.ui.status = e;
        }
    }
    match output.select {
        Some(Some(key)) => {
            if let Some((vertical, position)) = select_at(committed.map_or(shown, |after| after.as_slice()), key) {
                select_position(app, vertical, position);
            }
        }
        Some(None) => guide_select::clear(app),
        None => {}
    }
    // ⌥-click turns the guide through the point clicked, as one "Edit Guide" step; the turned guide stays selected.
    if let Some((key, _, at)) = output.clicked.filter(|(key, mods, _)| mods.alt && passed(*key).is_some()) {
        let (vertical, index) = guide_select::unkey(key);
        let position = round(if vertical { at.y } else { at.x });
        let params = json!({"orientation": orientation(vertical), "index": index, "newOrientation": orientation(!vertical), "position": position});
        match app.run("view.editGuide", params) {
            Ok(_) => select_position(app, !vertical, position),
            Err(e) => app.ui.status = e,
        }
    }
    if output.double_clicked.is_some_and(|key| passed(key).is_some())
        && let Err(e) = crate::menus::invoke(app, ctx, "edit.preferences.guidesGridAndSlices", json!({}))
    {
        app.ui.status = e;
    }
}

/// Selects the active document's guide of an orientation at `position`, if there is one.
fn select_position(app: &mut PhotocraftApp, vertical: bool, position: f64) {
    let index = app.session.active().and_then(|st| {
        let list = if vertical { &st.doc.guides.vertical } else { &st.doc.guides.horizontal };
        list.iter().position(|p| *p == position)
    });
    if let Some(index) = index {
        guide_select::select(app, vertical, index);
    }
}

#[cfg(test)]
mod tests;
