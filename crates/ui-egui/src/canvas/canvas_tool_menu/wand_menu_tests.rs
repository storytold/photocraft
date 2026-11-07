//! The Magic Wand's canvas right-click menu (#11783), in the reference application's order: the
//! selection-mode rows, Grow and Similar, then the selection actions. The Sample Size rows wait
//! for the Sample Size option (#11777). The other selection tools keep their menus.

use super::*;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app
}

/// The open menu's rows as labels, `―` for a separator.
fn labels(app: &PhotocraftApp) -> Vec<&'static str> {
    rows(app.ui.canvas_tool_menu.as_ref().expect("a menu is open")).iter().map(|r| r.map_or("―", |(l, _)| l)).collect()
}

fn open_wand(app: &mut PhotocraftApp) {
    assert!(open(app, &Context::default(), Tool::MagicWand, [10.0, 10.0]));
}

#[test]
fn with_nothing_selected_the_wand_menu_lists_select_all_then_color_range_and_load_selection() {
    let mut app = app();
    open_wand(&mut app);
    assert_eq!(labels(&app), ["Select All", "―", "Color Range…", "Load Selection…"]);
}

#[test]
fn with_a_selection_the_wand_menu_lists_the_reference_order() {
    let mut app = app();
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 20, "height": 20})).unwrap();
    open_wand(&mut app);
    assert_eq!(
        labels(&app),
        [
            "Add To Selection",
            "Subtract From Selection",
            "Intersect Selection",
            "―",
            "Grow",
            "Similar",
            "―",
            "Deselect",
            "Select Inverse",
            "Feather…",
            "Select and Mask…",
            "―",
            "Save Selection…",
            "Make Work Path…",
            "―",
            "Layer via Copy",
            "Layer via Cut",
            "―",
            "Last Filter",
            "―",
            "Fade…",
        ]
    );
}

#[test]
fn the_mode_rows_set_the_options_bar_mode() {
    for (command, mode) in [(WAND_ADD, 1), (WAND_SUBTRACT, 2), (WAND_INTERSECT, 3)] {
        let mut app = app();
        app.run("select.rect", json!({"x": 4, "y": 4, "width": 20, "height": 20})).unwrap();
        open_wand(&mut app);
        assert!(entry_enabled(&app, app.ui.canvas_tool_menu.as_ref().unwrap(), command), "{command} is enabled");
        choose(&mut app, &Context::default(), command);
        assert_eq!(app.ui.selection_mode, mode, "{command}");
        let st = app.session.active().unwrap();
        assert!(st.doc.selection.is_some(), "{command}: the selection is kept");
    }
}

#[test]
fn grow_and_similar_use_the_wands_tolerance() {
    // Four 8 px columns of grey 100, 110, 140 and 100 on the layer; the first is selected.
    for (command, tolerance, want) in [("select.grow", 12.0, 16), ("select.grow", 5.0, 8), ("select.similar", 12.0, 24)] {
        let mut app = app();
        for (i, g) in [100u8, 110, 140, 100].into_iter().enumerate() {
            app.run("select.rect", json!({"x": 8 * i, "y": 0, "width": 8, "height": 48})).unwrap();
            app.run("edit.fill", json!({"contents": "color", "color": format!("#{g:02x}{g:02x}{g:02x}")})).unwrap();
        }
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 48})).unwrap();
        app.ui.tool_options.tolerance = tolerance;
        open_wand(&mut app);
        choose(&mut app, &Context::default(), command);
        let st = app.session.active().unwrap();
        let b = st.doc.selection.as_ref().unwrap().content_bounds();
        let width: i32 = (b.x0..b.x1).filter(|x| st.doc.selection.as_ref().unwrap().pixel(*x, 20)[0] > 0.5).count() as i32;
        assert_eq!(width, want, "{command} at tolerance {tolerance}: {b:?}");
    }
}

#[test]
fn the_other_selection_tools_keep_their_menus() {
    for tool in [Tool::RectMarquee, Tool::Lasso, Tool::PolygonLasso, Tool::MagneticLasso, Tool::ObjectSelection, Tool::QuickSelection] {
        for selected in [false, true] {
            let mut app = app();
            if selected {
                app.run("select.rect", json!({"x": 4, "y": 4, "width": 20, "height": 20})).unwrap();
            }
            assert!(open(&mut app, &Context::default(), tool, [10.0, 10.0]), "{tool:?}");
            let menu = app.ui.canvas_tool_menu.as_ref().unwrap();
            assert_eq!(rows(menu), selection_rows(selected), "{tool:?} (selection: {selected})");
        }
    }
}
