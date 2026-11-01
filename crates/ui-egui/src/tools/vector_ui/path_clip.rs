//! Edit › Copy / Cut / Paste for path components (Photoshop's Path Selection clipboard):
//! while the Path Selection tool has components selected, the Edit menu's Copy and Cut take
//! those components onto the engine's path clipboard and Paste lays them down centred on the
//! view — appended to the path selected in the Paths panel, as in Photoshop, or as a new
//! saved path "Pasted Path" when nothing is selected there. The Edit commands and shortcuts
//! (⌘C/X/V) keep their ids: this intercepts them in `PhotocraftApp::run` before the engine
//! dispatch, so menus, the canvas context menu, agents and MCP all agree.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::path_components;
use crate::tools::vector_ui::paths_panel::{PathEntry, PathRow, path_rows};

/// The live component selection as copy/cut address it: the engine's target params for the
/// selected path plus every subpath of each selected component (a `PathOp::Join` subpath
/// belongs to its component, so it is included). `None` with no components selected, another
/// tool active, or a selection gone stale.
pub(crate) fn live_selection(app: &PhotocraftApp) -> Option<Value> {
    let (r, path, heads) = path_components::selected(app)?;
    let components = path.components();
    let subpaths: Vec<usize> =
        heads.iter().flat_map(|h| components.iter().find(|c| c.start == *h).map(|c| c.clone().collect::<Vec<_>>()).unwrap_or_default()).collect();
    if subpaths.is_empty() {
        return None;
    }
    let mut params = r.params(app);
    if let Some(o) = params.as_object_mut() {
        o.insert("subpaths".into(), json!(subpaths));
    }
    Some(params)
}

/// The Paths panel's explicit selection — where Edit › Paste puts the copied components, as
/// in Photoshop: an existing Work Path / layer-path / saved-path row, as the engine's path
/// commands address it. `None` when nothing (or a stale row) is selected, so the paste
/// becomes a new saved "Pasted Path" instead.
fn panel_target(app: &PhotocraftApp) -> Option<Value> {
    let st = app.session.active()?;
    let sel = app.ui.selected_path.as_deref()?;
    let rows = path_rows(&st.doc, st.active_layer);
    let key = |r: &PathEntry| match r.kind {
        PathRow::Work => "work".to_string(),
        PathRow::Layer => "layer".to_string(),
        PathRow::Saved => r.name.clone(),
    };
    let row = rows.iter().find(|r| key(r) == sel)?;
    Some(match row.kind {
        PathRow::Work => json!({"name": "work"}),
        // The layer row addresses the active layer's shape path (or its vector mask).
        PathRow::Layer => json!({"name": "layer"}),
        PathRow::Saved => json!({"name": row.name}),
    })
}

/// The document coordinates of the active view's centre: where Photoshop centres a pasted path.
fn view_center(app: &PhotocraftApp) -> Option<[f64; 2]> {
    let xf = crate::canvas::ViewXform::active(app)?;
    Some([f64::from(xf.center[0]), f64::from(xf.center[1])])
}

/// Edit › Copy/Cut/Paste for path components; `None` = not this context, dispatch as usual.
pub(crate) fn intercept(app: &mut PhotocraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    // A newer OS clipboard image supersedes a path clip, like any newer clip: refresh the import
    // first — its freshness guard leaves an unchanged OS image (and known-fresh internal
    // content) alone, and learns the baseline it needs to order the two clips. Routing then
    // dispatches whatever is still the freshest clip.
    if id == "edit.paste" && app.session.path_clipboard.is_some() && app.services.clipboard_get_image.is_some() {
        let _ = app.import_os_clipboard();
    }
    // The intercept only takes the plain menu/shortcut form: explicit params address the
    // command elsewhere (a mask, a channel, a layer), and a live component selection must not
    // redirect a caller's request (review on #10482).
    let plain = params.as_object().map_or(params.is_null(), |o| o.is_empty());
    match id {
        "edit.copy" | "edit.cut" if plain => {
            let params = live_selection(app)?;
            let cmd = if id == "edit.copy" { "path.copy" } else { "path.cut" };
            let r = app.run(cmd, params);
            if r.is_ok() && cmd == "path.cut" {
                // The indices addressed the path as it was; the cut changed it.
                crate::direct_select::cancel_stale(app);
            }
            Some(r)
        }
        // A path clip pastes into the active document only; with none open the OS clipboard
        // (a new document from its image) is the only route, as usual.
        "edit.paste" if app.session.path_clipboard.is_some() && app.session.active().is_some() && params.get("target").is_none() => {
            // An explicit centre wins; otherwise Photoshop centres the paste on the view.
            let mut p = if params.get("center").is_some() { params.clone() } else { view_center(app).map_or_else(|| json!({}), |c| json!({"center": c})) };
            // Photoshop pastes into the path selected in the Paths panel; only with nothing
            // selected does the paste become a new saved "Pasted Path" (selected afterwards).
            let target = panel_target(app);
            if let (Some(o), Some(t)) = (p.as_object_mut(), target.as_ref().and_then(Value::as_object)) {
                o.extend(t.clone());
            }
            let r = app.run("path.paste", p);
            if r.is_ok() && target.is_none() {
                // Photoshop selects the pasted path in the Paths panel.
                app.ui.selected_path = Some("Pasted Path".into());
            }
            Some(r)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! Edit › Copy/Cut/Paste for path components: the engine clipboard (`path_clip_cmds`) plus
    //! this intercept's routing — selected-component copy, cut with one undo step, Paste into
    //! the panel-selected path, else a new saved "Pasted Path".

    use photocraft_doc::{LayerContent, LayerId};
    use serde_json::json;

    use crate::PhotocraftApp;
    use crate::state::Tool;
    use crate::vector_ui::{pen_down, pen_up};

    /// A fresh app with a 200×200 document, drawing shapes with the Pen.
    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.ui.tool = Tool::Pen;
        app.ui.tool_options.vector_mode = "shape".into();
        app
    }

    /// One Pen click at `(x, y)`.
    fn click(app: &mut PhotocraftApp, x: f64, y: f64) {
        pen_down(app, x, y, false);
        pen_up(app);
    }

    /// A closed square with the Pen, from `(x, y)` with side `side`.
    fn square(app: &mut PhotocraftApp, x: f64, y: f64, side: f64) {
        for (dx, dy) in [(0.0, 0.0), (side, 0.0), (side, side), (0.0, side), (0.5, 0.5)] {
            click(app, x + dx, y + dy);
        }
    }

    /// The active layer as its `(id, shape layer)`; panics on any other layer.
    fn shape(app: &PhotocraftApp) -> (LayerId, photocraft_doc::ShapeLayer) {
        let st = app.session.active().unwrap();
        let id = st.active_layer.unwrap();
        match &st.doc.layer(id).unwrap().content {
            LayerContent::Shape(sh) => (id, sh.clone()),
            _ => panic!("not a shape layer"),
        }
    }

    /// The saved path named `name`, cloned.
    fn saved(app: &PhotocraftApp, name: &str) -> Option<photocraft_doc::Path> {
        app.session.active().unwrap().doc.paths.iter().find(|p| p.name == name).map(|p| p.path.clone())
    }

    /// Two squares on one shape layer, the inner one selected with the black arrow.
    fn two_squares_inner_selected() -> (PhotocraftApp, LayerId) {
        let mut app = app();
        square(&mut app, 20.0, 20.0, 120.0);
        app.ui.tool_options.pen_path_op = "combine".into();
        square(&mut app, 60.0, 60.0, 40.0);
        app.ui.tool = Tool::PathSelection;
        crate::path_components::press(&mut app, [80.0, 80.0], false);
        let (id, _) = shape(&app);
        (app, id)
    }

    #[test]
    fn copy_pastes_the_selected_component_as_pasted_path() {
        let (mut app, id) = two_squares_inner_selected();
        // Edit › Copy routes to the path clipboard (the pixel clipboard is replaced)…
        app.run("edit.copy", json!({})).unwrap();
        let clip = app.session.path_clipboard.as_ref().expect("the component is on the path clipboard");
        assert_eq!(clip.subpaths.len(), 1);
        assert_eq!(clip.subpaths[0].knots[0].anchor, photocraft_geom::Point::new(60.0, 60.0));
        assert!(app.session.clipboard.is_none(), "a path clip replaces the pixel clip");
        assert_eq!(shape(&app).1.path.subpaths.len(), 2, "copy leaves the shape alone");
        // …and Edit › Paste lays it down as "Pasted Path": the component's centre (80, 80)
        // lands on the paste centre (100, 100). The drawn shape's "layer" row is deselected
        // first (a user clicks the panel's empty area): with no Paths panel row selected the
        // paste becomes a new saved path.
        app.ui.selected_path = None;
        app.run("edit.paste", json!({"center": [100.0, 100.0]})).unwrap();
        let pasted = saved(&app, "Pasted Path").expect("the pasted path");
        assert_eq!(pasted.subpaths.len(), 1);
        assert_eq!(pasted.subpaths[0].knots[0].anchor, photocraft_geom::Point::new(80.0, 80.0));
        assert_eq!(app.ui.selected_path.as_deref(), Some("Pasted Path"), "Photoshop selects the pasted path");
        assert_eq!(shape(&app).0, id);
        assert_eq!(shape(&app).1.path.subpaths.len(), 2, "the shape is untouched");
    }

    #[test]
    fn paste_appends_into_the_panel_selected_path() {
        let (mut app, _) = two_squares_inner_selected();
        // Copy first, then pick the paste's target in the Paths panel: the component selection
        // belongs to the panel row it was made on, so the switch comes after the copy.
        app.run("edit.copy", json!({})).unwrap();
        app.run("path.set", json!({"name": "Sister", "path": {"subpaths": [{"closed": true, "knots": [[150, 20], [190, 20], [190, 60], [150, 60]]}]}}))
            .unwrap();
        app.ui.selected_path = Some("Sister".into());
        // Paste lands in the panel-selected "Sister": appended at the paste centre, no new
        // saved path, and the panel selection stays.
        app.run("edit.paste", json!({"center": [100.0, 100.0]})).unwrap();
        let sister = saved(&app, "Sister").unwrap();
        assert_eq!(sister.subpaths.len(), 2, "the copied component appended to the selected path");
        assert_eq!(sister.subpaths[1].knots[0].anchor, photocraft_geom::Point::new(80.0, 80.0));
        assert!(saved(&app, "Pasted Path").is_none(), "a targeted paste creates no saved path");
        assert_eq!(app.ui.selected_path.as_deref(), Some("Sister"), "the panel selection stays");
    }

    #[test]
    fn cut_takes_the_component_in_one_undoable_step() {
        let (mut app, _) = two_squares_inner_selected();
        let steps = app.session.active().unwrap().history.past_len();
        app.run("edit.cut", json!({})).unwrap();
        assert_eq!(shape(&app).1.path.subpaths.len(), 1, "the cut component left the shape");
        assert!(app.session.path_clipboard.is_some(), "and landed on the clipboard");
        assert!(app.ui.direct_selection.anchors.is_empty(), "the cut cleared the selection");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1, "one history step");
        assert!(app.session.undo());
        assert_eq!(shape(&app).1.path.subpaths.len(), 2, "undo restores the cut component");
    }

    /// The intercept only takes the plain Edit › Copy/Cut: an explicit target (here a colour
    /// channel) keeps its own dispatch — a selected component must not redirect the request.
    #[test]
    fn copy_with_explicit_params_keeps_its_pixel_target() {
        let (mut app, _) = two_squares_inner_selected();
        app.run("edit.copy", json!({"target": {"color": 0}})).unwrap();
        assert!(app.session.path_clipboard.is_none(), "the component was not hijacked");
        assert!(app.session.clipboard.is_some(), "the channel copied as a pixel clip");
        assert_eq!(shape(&app).1.path.subpaths.len(), 2, "the shape is untouched");
    }

    /// Paste precedence, one clipboard: an OS image displaces a path clip only after the app has
    /// a baseline. The first observation (here: the first paste's read) only learns the
    /// baseline — the older OS image cannot displace the fresh copy; once the baseline exists,
    /// a changed OS image wins and an unchanged one never displaces.
    #[test]
    fn paste_prefers_a_newer_os_image() {
        let os = std::sync::Arc::new(std::sync::Mutex::new(None::<(u32, u32, Vec<u8>)>));
        let (a, b) = (os.clone(), os.clone());
        let services = crate::Services { clipboard_get_image: Some(Box::new(move || Ok(b.lock().unwrap().clone()))), ..Default::default() };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.run("file.new", json!({"width": 40, "height": 40})).unwrap();
        app.ui.tool = Tool::Pen;
        app.ui.tool_options.vector_mode = "shape".into();
        square(&mut app, 5.0, 5.0, 10.0);
        let shape_id = shape(&app).0;
        app.ui.tool = Tool::PathSelection;
        let paste = |app: &mut PhotocraftApp| app.run("edit.paste", json!({"center": [20.0, 20.0]})).unwrap();

        // No baseline yet — the OS image (older than the copy) cannot displace the fresh clip;
        // the paste's read only learns the baseline.
        *os.lock().unwrap() = Some((2, 2, [0u8, 255u8, 0, 255].repeat(4)));
        crate::path_components::press(&mut app, [10.0, 10.0], false);
        app.run("edit.copy", json!({})).unwrap();
        app.ui.selected_path = None;
        paste(&mut app);
        assert!(saved(&app, "Pasted Path").is_some(), "the older OS image did not displace the path clip");
        assert!(app.session.clipboard.is_none(), "and it was not installed as a pixel clip");
        // With the baseline, a changed OS image supersedes the clip: ⌘V pastes the pixels.
        *os.lock().unwrap() = Some((2, 2, [255u8, 0, 0, 255].repeat(4)));
        paste(&mut app);
        assert!(app.session.path_clipboard.is_none(), "the newer OS image replaced the path clip");
        assert_eq!(
            app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().map(|s| s.content_bounds().width()),
            Some(2),
            "the OS image pasted as a new layer"
        );
        // An unchanged OS image never displaces a fresh copy. (The pixel paste left its layer
        // active and the panel holds "Pasted Path"; the black arrow targets the panel-selected
        // path, so select the shape and clear the panel for the press to reach it again.)
        app.run("layer.select", json!({"layer": shape_id.0})).unwrap();
        app.ui.selected_path = None;
        crate::path_components::press(&mut app, [10.0, 10.0], false);
        app.run("edit.copy", json!({})).unwrap();
        paste(&mut app);
        assert!(app.session.clipboard.is_none(), "the unchanged image did not install a pixel clip");
        assert!(app.session.path_clipboard.is_some(), "and the fresh path clip survived");
        drop(a);
    }

    /// A pixel clip pastes as pixels even beside a path component selection: the one clipboard
    /// holds the newest kind, and Paste does what matches it (Photoshop pastes the pixels).
    #[test]
    fn a_pixel_clip_pastes_as_pixels_beside_a_path_selection() {
        let (mut app, _) = two_squares_inner_selected();
        // Copy pixels: no component selected, so Edit › Copy takes the layer.
        app.ui.direct_selection = Default::default();
        app.run("edit.copy", json!({})).unwrap();
        assert!(app.session.clipboard.is_some() && app.session.path_clipboard.is_none(), "the pixel copy replaced the path clip");
        // Select the inner square again, so the paste runs beside a component selection.
        crate::path_components::press(&mut app, [80.0, 80.0], false);
        let layers = app.session.active().unwrap().doc.walk().len();
        app.run("edit.paste", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.walk().len(), layers + 1, "the pixel clip pasted as a new layer");
        assert_eq!(shape(&app).1.path.subpaths.len(), 2, "no path was touched");
    }
}
