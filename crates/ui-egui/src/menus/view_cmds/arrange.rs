use craft::behavior::canvas::ZoomLadder;
use craft::ui::docking::{ArrangeCommands, ArrangeSpace, Arranged, arrange as run_arrange};
use photocraft_doc::DocId;

use super::{PhotocraftApp, Value, json};

/// Window › Arrange ids (`window.arrange.<name>`) and the framework command each runs: the one table that says which
/// ids exist.
const COMMANDS: [(&str, ArrangeCommands); 19] = [
    ("tileAllVertically", ArrangeCommands::TileAllVertically),
    ("tileAllHorizontally", ArrangeCommands::TileAllHorizontally),
    ("twoUpVertical", ArrangeCommands::TwoUpVertical),
    ("twoUpHorizontal", ArrangeCommands::TwoUpHorizontal),
    ("threeUpVertical", ArrangeCommands::ThreeUpVertical),
    ("threeUpHorizontal", ArrangeCommands::ThreeUpHorizontal),
    ("threeUpStacked", ArrangeCommands::ThreeUpStacked),
    ("fourUp", ArrangeCommands::FourUp),
    ("sixUp", ArrangeCommands::SixUp),
    ("consolidateAllToTabs", ArrangeCommands::ConsolidateAllToTabs),
    ("cascade", ArrangeCommands::Cascade),
    ("tile", ArrangeCommands::Tile),
    ("floatInWindow", ArrangeCommands::FloatInWindow),
    ("floatAllInWindows", ArrangeCommands::FloatAllInWindows),
    ("newWindowForDocument", ArrangeCommands::NewWindowForDocument),
    ("matchZoom", ArrangeCommands::MatchZoom),
    ("matchLocation", ArrangeCommands::MatchLocation),
    ("matchRotation", ArrangeCommands::MatchRotation),
    ("matchAll", ArrangeCommands::MatchAll),
];

/// The n-up layouts: the entries that show a check mark while the document area is arranged that way.
const N_UP: [&str; 7] = ["twoUpVertical", "twoUpHorizontal", "threeUpVertical", "threeUpHorizontal", "threeUpStacked", "fourUp", "sixUp"];

/// The command of the id's last part (`"fourUp"`), if there is one.
pub(super) fn command(k: &str) -> Option<ArrangeCommands> {
    COMMANDS.iter().find(|(name, _)| *name == k).map(|(_, command)| *command)
}

/// The check mark of an n-up entry: whether the document area shows that layout. `None` for the other entries.
pub(super) fn layout_checked(app: &PhotocraftApp, k: &str) -> Option<bool> {
    let arrangement = command(k)?.arrangement();
    N_UP.contains(&k).then(|| app.ui.document_windows.arrangement == Some(arrangement))
}

/// A finite `[x, y]` screen position within a sane range (window systems reject wild values).
fn screen_position(v: &Value) -> Option<[f32; 2]> {
    const LIMIT: f64 = 1.0e6;
    let a = v.as_array().filter(|a| a.len() == 2)?;
    let n = |i: usize| a.get(i).and_then(Value::as_f64).filter(|x| x.is_finite() && x.abs() <= LIMIT).map(|x| x as f32);
    Some([n(0)?, n(1)?])
}

/// The area and sizes the framework's arrangements work in. New windows take half the document area; a window that
/// follows its document's size is resized by `DocumentWindow` afterwards.
fn space(app: &PhotocraftApp) -> ArrangeSpace<DocId> {
    let area = app.ctx().content_rect();
    let step = craft::ui::style::StyleCx::with_root(app.ctx(), |cx| cx.tokens.size.window.cascade);
    ArrangeSpace::new(area, area.size() * 0.5, step, app.session.active().map(|st| st.doc.id))
}

/// The Window › Arrange items: the framework's layouts and window commands over `app.ui.document_windows`, and the
/// Match commands over the document views.
pub(super) fn arrange(app: &mut PhotocraftApp, k: &str, p: &Value) -> Result<Value, String> {
    let command = command(k).ok_or_else(|| format!("unknown arrangement {k}"))?;
    let given = |key: &str| p.get(key).is_some_and(|v| !v.is_null());
    if matches!(command, ArrangeCommands::FloatInWindow) && (given("document") || given("position")) {
        return float_in_window(app, p);
    }
    let space = space(app);
    let arranged = run_arrange(&mut app.ui.document_windows, command, &space);
    crate::canvas::document_area::apply_arranged(app, arranged);
    if matches!(command, ArrangeCommands::NewWindowForDocument) {
        keep_own_view(app);
    }
    match arranged {
        Arranged::Views { zoom, location, rotation, .. } => match_views(app, zoom, location, rotation),
        _ => Ok(json!({"arrange": k})),
    }
}

/// Gives the window New Window for Document just opened its own copy of its document's view, so the two scroll and
/// zoom apart.
fn keep_own_view(app: &mut PhotocraftApp) {
    let Some((group, _)) = app.ui.document_windows.floating.last() else { return };
    let Some(view) = group.documents.first().and_then(|doc| crate::canvas::tabs::document_index(app, *doc)).and_then(|doc| app.ui.views.get(doc)).cloned()
    else {
        return;
    };
    app.ui.window_views.insert(group.id, view);
}

/// Float in Window for a given document (default the active one) and screen position (`[x, y]` in points, default the
/// next cascade slot): dragging a tab off the tab strip uses both. A document alone in a floating window keeps it.
fn float_in_window(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let doc = match p.get("document") {
        None | Some(Value::Null) => app.session.active().ok_or("no document")?.doc.id,

        Some(v) => serde_json::from_value::<DocId>(v.clone())
            .ok()
            .filter(|id| app.session.documents().iter().any(|st| st.doc.id == *id))
            .ok_or("document must be the id of an open document")?,
    };
    let position = match p.get("position") {
        None | Some(Value::Null) => None,
        Some(v) => Some(screen_position(v).ok_or("position must be [x, y] in points")?),
    };
    let space = space(app);
    // A document alone in a floating window keeps the window where it is.
    let kept = app.ui.document_windows.floating.iter().find(|(group, _)| group.documents == [doc]).map(|(_, rect)| rect.min);
    let min = match (position, kept) {
        (Some([x, y]), _) => egui::pos2(x, y),
        (None, Some(min)) => min,
        (None, None) => craft::behavior::placement::cascade_rects(space.area, space.size, app.ui.document_windows.floating.len() + 1, space.step)
            .pop()
            .map_or(space.area.min, |r| r.min),
    };
    let window = app.ui.document_windows.float(&doc, egui::Rect::from_min_size(min, space.size)).ok_or("the document is in no window")?;
    if let Some(i) = app.session.documents().iter().position(|st| st.doc.id == doc) {
        app.session.set_active(i);
    }
    Ok(json!({"window": window}))
}

/// Match Zoom, Location and Rotation: every view takes the active view's values, each zoom within its own
/// document's range.
fn match_views(app: &mut PhotocraftApp, zoom: bool, loc: bool, rot: bool) -> Result<Value, String> {
    let i = app.session.active_index().ok_or("no document")?;
    let src = app.ui.views.get(i).cloned().ok_or("no document")?;
    let animated_zoom = app.session.prefs().general.animated_zoom;
    // A view not drawn yet has no `doc_size`, so the size comes from the document.
    let sizes: Vec<[u32; 2]> = app.session.documents().iter().map(|d| [d.doc.size.width, d.doc.size.height]).collect();
    // A default-mode crop turns the active view's camera; that turn is the crop's, not the view's own rotation
    // (which picking the Crop tool made 0), so it isn't copied (#9308).
    let crop = crate::crop_ui::turns_view(app);
    let rotation = if crop { 0.0 } else { src.rotation };
    let take = |d: usize, v: &mut crate::state::View| {
        if zoom {
            let [w, h] = sizes.get(d).copied().unwrap_or(v.doc_size);
            v.request_zoom_animation(animated_zoom);
            v.zoom = ZoomLadder::clamp(f64::from(src.zoom), craft::behavior::kurbo::Size::new(f64::from(w), f64::from(h))) as f32;
        }
        if loc {
            v.center = src.center;
        }
        if rot && !(crop && d == i) {
            v.rotation = rotation;
        }
        v.clear_pending();
    };
    for (d, v) in app.ui.views.iter_mut().enumerate() {
        take(d, v);
    }
    // The windows with a view of their own (New Window for Document) match too, each within its document's range.
    let owners: Vec<(u64, usize)> = app
        .ui
        .document_windows
        .floating
        .iter()
        .filter_map(|(group, _)| Some((group.id, crate::canvas::tabs::document_index(app, group.shown.or_else(|| group.documents.first().copied())?)?)))
        .collect();
    for (window, d) in owners {
        if let Some(v) = app.ui.window_views.get_mut(&window) {
            take(d, v);
        }
    }
    // Match Zoom and Match Location copy no rotation: they report the camera's as it is.
    Ok(json!({"zoom": src.zoom, "center": src.center, "rotation": if rot { rotation } else { src.rotation }}))
}

/// Whether `doc` shows in a floating window.
pub(crate) fn document_is_floating(app: &PhotocraftApp, doc: DocId) -> bool {
    app.ui.document_windows.floating.iter().any(|(group, _)| group.documents.contains(&doc))
}
