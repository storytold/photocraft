//! The content versions of the settling regions (`Lazy::content`): the shell's options bar and status bar, and every
//! docked panel body. A version covers the app data its region shows that can change while no input reaches
//! the region: what a command changes, the active document and layer and the tool in every version; the brush and the
//! colours only in the regions that show them, so a drag that edits one (a brush Size slider, a Color slider) runs
//! those regions and leaves the rest settled; and what a region shows of the canvas view, a job or the pointer. Input
//! inside a region runs it whatever its version, and so does a repaint it asks for itself.

use std::hash::{Hash, Hasher};

use super::{PhotocraftApp, chrome_ui};
use crate::color_picker_ui::PickWindow;

/// The version of the options bar: the Zoom tool's members show the active view, and an open Free Transform shows
/// its own members in place of the tool's (Escape ends it without a command, #15547). The painting tools' members
/// show the brush and its presets, the gradient, shape and Mixer Brush members the colours.
pub(crate) fn options_bar(app: &PhotocraftApp) -> u64 {
    version(app, (view(app), app.ui.transform.is_some(), brush(app), colours(app)))
}

/// The version of the status bar: the zoom readout of the active view, the status message and the item it shows, the
/// render-path note, and the progress of the background redraw and of the job in view.
pub(crate) fn status_bar(app: &PhotocraftApp) -> u64 {
    let redraw = app.session.active().and_then(|st| app.redraws.get(&st.doc.id)).map(|redraw| redraw.fraction().to_bits());
    let job = crate::jobs_ui::job_in_view(app).map(|job| (job.id, job.progress.to_bits()));
    let render_path = chrome_ui::render_path_label(app).map(|(label, tip, _)| (label, tip));
    version(app, (view(app), &app.ui.status, app.ui.status_error, &app.ui.chrome.status_info, render_path, redraw, job, app.session.jobs().len()))
}

/// The version of panel `id`'s body. The Navigator shows the active view, the Info panel the pointer's document pixel
/// and the zoom it samples at, the Clone Source panel the pairing at the last hover point, the Modifier Keys panel the
/// latched keys, which their use releases, the Properties panel the colour its well's open Color Picker previews and
/// the answer that picker left. Brush Settings and Brushes show the brush and its presets; Color, Swatches, Gradients
/// and the Gradient Editor show the colours, and so does Properties (a shape layer's fill and stroke swatches, a
/// gradient fill's Foreground and Background stops, the adjustment editors' swatches). Every other panel shows only what
/// every version holds.
pub(crate) fn panel(app: &PhotocraftApp, id: &str) -> u64 {
    match id {
        "navigator" => version(app, view(app)),
        "info" => version(app, (app.hover_doc.map(|at| at.map(f64::to_bits)), app.current_zoom().to_bits())),
        "cloneSource" => version(app, app.ui.presets_ui.clone_cursor.map(|(document, at)| (document, at.map(f64::to_bits)))),
        "modifierKeys" => version(app, app.ui.shell.latched),
        "properties" => version(app, (crate::color_picker_ui::window_picker_state(app, PickWindow::Properties), colours(app))),
        "brushSettings" | "brushes" => version(app, brush(app)),
        "color" | "swatches" | "gradients" | "gradientEditor" => version(app, colours(app)),
        _ => version(app, ()),
    }
}

/// The version of the Toolbar's rail: the toolbar slots (the preferences revision), the remembered group tools, the
/// Edit Toolbar drawer, Quick Mask, the screen mode and the colour swatches. Its flyouts always open on the same side.
/// The active tool is in every version.
pub(crate) fn toolbar(app: &PhotocraftApp) -> u64 {
    toolbar_with(app, 0)
}

/// [`toolbar`] for the rail as drawn in `ui` (#15493): also the resolved tokens it was drawn with (they change with the
/// theme, the UI font size and the UI scale), so a theme change redraws it even without a command.
pub(crate) fn toolbar_drawn(app: &PhotocraftApp, ui: &egui::Ui) -> u64 {
    toolbar_with(app, std::sync::Arc::as_ptr(&craft::ui::runtime::env::Env::of(ui).tokens) as usize)
}

/// [`toolbar`] with the tokens it was drawn with named by `tokens`.
fn toolbar_with(app: &PhotocraftApp, tokens: usize) -> u64 {
    let quick_mask = app.session.active().is_some_and(|st| st.doc.quick_mask.is_some());
    let screen_mode = format!("{:?}", app.ui.view.screen_mode);
    version(app, (app.session.prefs.rev(), &app.ui.group_tools, app.ui.toolbar_drawer, quick_mask, screen_mode, colours(app), tokens))
}

/// What every region may show, joined with `extra`: the commands run (any of them may change anything), the active
/// document, its revision and its active layers, and the tool.
fn version(app: &PhotocraftApp, extra: impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    app.commands_run.hash(&mut hasher);
    app.session.active_generation().hash(&mut hasher);
    app.session.active_layer_changes().hash(&mut hasher);
    app.session.active().map(|st| st.revision).hash(&mut hasher);
    app.ui.tool.hash(&mut hasher);
    extra.hash(&mut hasher);
    hasher.finish()
}

/// The brush and its presets, for the regions that show them: their revisions move with every change of either.
fn brush(app: &PhotocraftApp) -> (u64, u64) {
    (app.session.tools.brush_rev, app.session.tools.presets_rev)
}

/// The foreground and background colours, for the regions that show them.
fn colours(app: &PhotocraftApp) -> [u32; 8] {
    let tools = &app.session.tools;
    let mut bits = [0; 8];
    for (bit, channel) in bits.iter_mut().zip(tools.foreground.iter().chain(&tools.background)) {
        *bit = channel.to_bits();
    }
    bits
}

/// The active document's view as the canvas showed it: its zoom, centre and turn.
fn view(app: &PhotocraftApp) -> Option<[u64; 5]> {
    crate::canvas::active_view(app)
        .map(|view| [view.zoom.to_bits(), view.center.x.to_bits(), view.center.y.to_bits(), view.rotation.to_bits(), u64::from(view.flip_x)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Tool;

    /// Everything the settled toolbar shows moves its version (#15493): a version that missed one would replay a stale
    /// rail. The same state gives the same version, so the rail settles.
    #[test]
    fn the_toolbar_version_moves_with_what_the_rail_shows() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let base = toolbar_with(&app, 1);
        assert_eq!(toolbar_with(&app, 1), base, "the same state settles");
        assert_ne!(toolbar_with(&app, 2), base, "other tokens (theme, UI font size or scale)");

        let changed = |name: &str, change: &dyn Fn(&mut PhotocraftApp), app: &mut PhotocraftApp| {
            let before = toolbar_with(app, 1);
            change(app);
            assert_ne!(toolbar_with(app, 1), before, "{name} moves the version");
        };
        changed("the tool", &|app| app.ui.tool = Tool::Brush, &mut app);
        changed("a slot's remembered tool", &|app| app.ui.group_tools.insert(0, Tool::EllipseMarquee), &mut app);
        changed("the Edit Toolbar drawer", &|app| app.ui.toolbar_drawer = !app.ui.toolbar_drawer, &mut app);
        changed("the toolbar layout", &|app| app.session.prefs.edit(|prefs| prefs.toolbar.hidden.push("Brush".to_string())), &mut app);
        changed("the colours", &|app| app.session.tools.foreground[0] = 0.25, &mut app);
    }

    /// The brush and the colours move the versions of the regions that show them and of no other, so a drag that edits
    /// one leaves the rest settled; a version that missed one would replay a stale region.
    #[test]
    fn the_brush_and_the_colours_move_only_the_regions_that_show_them() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let regions = |app: &PhotocraftApp| -> Vec<(&'static str, u64)> {
            let mut all: Vec<(&'static str, u64)> =
                ["layers", "history", "navigator", "properties", "brushSettings", "brushes", "color", "swatches", "gradients", "gradientEditor"]
                    .into_iter()
                    .map(|id| (id, panel(app, id)))
                    .collect();
            all.extend([("optionsBar", options_bar(app)), ("statusBar", status_bar(app)), ("toolbar", toolbar_with(app, 1))]);
            all
        };
        let moved = |app: &mut PhotocraftApp, change: &dyn Fn(&mut PhotocraftApp)| -> Vec<&'static str> {
            let before = regions(app);
            change(app);
            regions(app).into_iter().zip(before).filter(|((_, now), (_, was))| now != was).map(|((id, _), _)| id).collect()
        };
        let brush = moved(&mut app, &|app| {
            app.session.tools.brush.size += 1.0;
            app.session.tools.brush_changed();
        });
        assert_eq!(brush, ["brushSettings", "brushes", "optionsBar"], "the brush");
        let presets = moved(&mut app, &|app| app.session.tools.presets_rev += 1);
        assert_eq!(presets, ["brushSettings", "brushes", "optionsBar"], "the brush presets");
        let colours = moved(&mut app, &|app| app.session.tools.background[2] = 0.5);
        assert_eq!(colours, ["properties", "color", "swatches", "gradients", "gradientEditor", "optionsBar", "toolbar"], "the colours");
    }
}
