//! The content versions of the settling regions (`Lazy::content`): the shell's options bar and status bar, and every
//! docked panel body. A version covers the app data its region shows that can change while no input reaches
//! the region: what a command changes, the active document and layer and the tool in every version; the brush, the
//! colours and the type defaults only in the regions that show them, so a drag that edits one (a brush Size slider, a
//! Color slider) runs those regions and leaves the rest settled; what a region shows of the canvas view, a job or the
//! pointer; and what an edit elsewhere changes without a command, such as a box or pin dragged on the canvas or an
//! adjustment's live preview. Input inside a region runs it whatever its version, and so does a repaint it asks for
//! itself.

use std::hash::{Hash, Hasher};

use super::{PhotocraftApp, chrome_ui};
use crate::color_picker_ui::PickWindow;

/// The version of the options bar: every tool option (`app.ui.tool_options`, all of which the bar shows; keys, the
/// canvas menu, the canvas and its task bars, panels and presets write them without a command), the commit and cancel
/// cells and hints of an edit the canvas opens or ends ([`options_bar::edit_key`](super::options_bar::edit_key)), the
/// Zoom tool's view, and an open Free Transform, Puppet Warp or Perspective Warp, which shows its own members in place
/// of the tool's, with what a drag on the canvas changes (the box, the picked pin, the planes; Escape ends them without
/// a command, #15547), the Crop tool's frame state, the type members' caret or selection, the brush and its presets and
/// the colours.
pub(crate) fn options_bar(app: &PhotocraftApp) -> u64 {
    let sessions =
        (crate::transform_tool::bar_key(app), crate::puppet_ui::bar_key(app), crate::perspective_ui::bar_key(app), crate::crop_ui::bar::bar_key(app));
    let shown = (serialized(&app.ui.tool_options), super::options_bar::edit_key(app), text_target(app));
    version(app, (view(app), sessions, shown, brush(app), colours(app)))
}

/// The version of the title bar: the title it shows (the active document's name and its unsaved mark), the language
/// its menu titles are in, whether the menus are in the bar (not in the macOS menu bar) and whether the bar takes the
/// menu keys (F10, Alt, Alt+letter), which a dialog or the shortcut recorder holds back: a dialog closed with Escape
/// runs no command, and the bar registers for the keys again only when it runs. An open menu is an overlay and runs as
/// it is shown.
pub(crate) fn title_bar(app: &PhotocraftApp) -> u64 {
    let (gate, _) = crate::menus::menu_gate(app);
    let keys = (gate.modal, gate.recording_shortcut);
    version(app, (super::title_bar::title_text(app), crate::i18n::current().code(), app.services.native_menu.is_none(), keys))
}

/// The version of what the dock's chrome (each group's header and tab strip, each column's bar) shows of the panels
/// (`DockPanels::spec_version`): their names in the UI language and the toolbar's grid, whose cells are the visible
/// toolbar slots. Every other part of a panel's spec, its permissions and whether its menu has rows are fixed per
/// registered panel. The chrome shows no document data, so this version holds nothing of [`version`].
pub(crate) fn dock_chrome(app: &PhotocraftApp) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (crate::i18n::current().code(), super::toolbar::cells(app)).hash(&mut hasher);
    hasher.finish()
}

/// The version of the status bar: the zoom readout of the active view, the status message and the item it shows, the
/// render-path note, and the progress of the background redraw and of the job in view.
pub(crate) fn status_bar(app: &PhotocraftApp) -> u64 {
    let redraw = app.session.active().and_then(|st| app.redraws.get(&st.doc.id)).map(|redraw| redraw.fraction().to_bits());
    let job = crate::jobs_ui::job_in_view(app).map(|job| (job.id, job.progress.to_bits()));
    let render_path = chrome_ui::render_path_label(app).map(|(label, tip, _)| (label, tip));
    version(app, (view(app), &app.ui.status, app.ui.status_error, &app.ui.chrome.status_info, render_path, redraw, job, app.session.jobs().len()))
}

/// The version of panel `id`'s body. The Navigator shows the active view, the Layers panel the layer the Move tool's
/// rollover outlines on the canvas, the Info panel the pointer's document pixel,
/// the zoom and the Eyedropper's Sample Size it samples at and the Crop tool's frame size, the Clone Source panel the pairing at the last hover point, the Modifier Keys panel the
/// latched keys, which their use releases, the Properties panel the colour its well's open Color Picker previews and
/// the answer that picker left, the Histogram the adjustment preview it counts. Brush Settings and Brushes show the
/// brush and its presets; Color, Swatches, Gradients and the Gradient Editor show the colours, and so do Properties (a
/// shape layer's fill and stroke swatches, a gradient fill's Foreground and Background stops, the adjustment editors'
/// swatches) and Character (the colour of new type is the foreground). Character and Paragraph show the type
/// defaults, and they and Properties the values at the caret or selection of the type being edited, which Glyphs
/// inserts at. Every other panel shows only what every version holds.
pub(crate) fn panel(app: &PhotocraftApp, id: &str) -> u64 {
    match id {
        "navigator" => version(app, view(app)),
        "layers" => version(app, app.canvas_hover_layer),
        "info" => {
            let crop = crate::crop_ui::info_size(app).map(|size| size.map(f64::to_bits));
            version(app, (app.hover_doc.map(|at| at.map(f64::to_bits)), app.current_zoom().to_bits(), app.ui.tool_options.eyedropper.size.pixels(), crop))
        }
        "cloneSource" => version(app, app.ui.presets_ui.clone_cursor.map(|(document, at)| (document, at.map(f64::to_bits)))),
        "modifierKeys" => version(app, app.ui.shell.latched),
        "properties" => version(app, (crate::color_picker_ui::window_picker_state(app, PickWindow::Properties), colours(app), text_target(app))),
        "brushSettings" | "brushes" => version(app, brush(app)),
        "histogram" => version(app, adjustment_preview(app)),
        "character" => version(app, (colours(app), type_defaults(app), text_target(app))),
        "paragraph" => version(app, (type_defaults(app), text_target(app))),
        "glyphs" => version(app, text_target(app)),
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
    let screen_mode = app.ui.view.screen_mode;
    version(app, (app.session.prefs.rev(), &app.ui.group_tools, app.ui.toolbar_drawer, quick_mask, screen_mode, colours(app), tokens))
}

/// What every region may show, joined with `extra`: the commands run (any of them may change anything), the active
/// document, its revision, its active layers and which part of them is targeted (the layer mask, the vector mask, a
/// filter mask, the selected path; the Layers, Channels, Paths and Properties panels and the canvas set it without a
/// command), and the tool.
fn version(app: &PhotocraftApp, extra: impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    app.commands_run.hash(&mut hasher);
    app.session.active_generation().hash(&mut hasher);
    app.session.active_layer_changes().hash(&mut hasher);
    app.session.active().map(|st| st.revision).hash(&mut hasher);
    (app.ui.mask_target, app.ui.vector_mask_target, app.ui.filter_mask_target, &app.ui.selected_path).hash(&mut hasher);
    app.ui.tool.hash(&mut hasher);
    extra.hash(&mut hasher);
    hasher.finish()
}

/// A hash of `value` through its serialized form, which holds every stored field: for app state whose type has no
/// `Hash` (floats, framework types).
fn serialized(value: &impl serde::Serialize) -> u64 {
    struct HashWriter<'a>(&'a mut std::collections::hash_map::DefaultHasher);
    impl std::io::Write for HashWriter<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.write(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    // Writing to a hasher cannot fail, and every tool option serializes (they are saved).
    let _ = serde_json::to_writer(HashWriter(&mut hasher), value);
    hasher.finish()
}

/// The brush and its presets, for the regions that show them: their revisions move with every change of either.
fn brush(app: &PhotocraftApp) -> (u64, u64) {
    (app.session.tools.brush_rev, app.session.tools.presets_rev)
}

/// The Type tool's defaults for new type, for the regions that show them with no type target: an edit in one of them
/// with nothing targeted changes the defaults and runs no command.
fn type_defaults(app: &PhotocraftApp) -> impl Hash + '_ {
    let o = &app.ui.tool_options;
    (&o.type_font, &o.type_style, o.type_size.to_bits(), &o.type_aa, &o.type_align, &o.type_direction, &o.type_style_defaults)
}

/// The caret and selection of the type being edited: the type members, Character, Paragraph and Properties show the
/// values there, and a click, a drag or an arrow key on the canvas moves them without a command.
fn text_target(app: &PhotocraftApp) -> Option<(u64, Option<u64>, usize, usize)> {
    app.ui.text_edit.as_ref().map(|edit| (edit.layer, edit.document, edit.caret, edit.anchor))
}

/// The adjustment preview the Histogram counts while it shows: an adjustment layer's Properties drag (its proposed
/// values) or an Image › Adjustments dialog's preview. Both change on every move with no command until the release.
fn adjustment_preview(app: &PhotocraftApp) -> impl Hash + '_ {
    (app.live_adjust.as_ref(), crate::adjust_preview::shown_key(app))
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

    /// The brush, the colours, the type defaults, the Crop options and an adjustment's live preview move the versions of
    /// the regions that show them and of no other, so a drag that edits one leaves the rest settled; a version that
    /// missed one would replay a stale region.
    #[test]
    fn the_brush_and_the_colours_move_only_the_regions_that_show_them() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let regions = |app: &PhotocraftApp| -> Vec<(&'static str, u64)> {
            let mut all: Vec<(&'static str, u64)> = [
                "layers",
                "history",
                "navigator",
                "info",
                "histogram",
                "properties",
                "character",
                "paragraph",
                "glyphs",
                "brushSettings",
                "brushes",
                "color",
                "swatches",
                "gradients",
                "gradientEditor",
            ]
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
        assert_eq!(colours, ["properties", "character", "color", "swatches", "gradients", "gradientEditor", "optionsBar", "toolbar"], "the colours");
        let type_defaults = moved(&mut app, &|app| app.ui.tool_options.type_size += 1.0);
        assert_eq!(type_defaults, ["character", "paragraph", "optionsBar"], "the type defaults");
        app.ui.tool = Tool::Crop;
        let ratio = moved(&mut app, &|app| app.ui.tool_options.crop.ratio = craft::draw::ui::crop::CROP_RATIOS[1]);
        assert_eq!(ratio, ["optionsBar"], "a crop ratio picked in the task bar");
        let straighten = moved(&mut app, &|app| crate::crop_ui::toggle_straighten(app));
        assert_eq!(straighten, ["optionsBar"], "Straighten pressed in the task bar");
        let opacity = moved(&mut app, &|app| app.ui.tool_options.fill_opacity = 50.0);
        assert_eq!(opacity, ["optionsBar"], "a tool option a key sets");
        let sample = moved(&mut app, &|app| app.ui.tool_options.eyedropper.size = craft::color_ui::ui::SampleSize::Avg5);
        assert_eq!(sample, ["info", "optionsBar"], "the Sample Size the canvas menu picks");
        app.ui.tool = Tool::Type;
        let edit = |caret| crate::state::TextEdit { layer: 1, document: None, caret, anchor: 0, session: String::new(), created: false, mask: false };
        app.ui.text_edit = Some(edit(0));
        let caret = moved(&mut app, &|app| app.ui.text_edit = Some(edit(2)));
        assert_eq!(caret, ["properties", "character", "paragraph", "glyphs", "optionsBar"], "the caret moved in the type being edited");
        let rollover = moved(&mut app, &|app| app.canvas_hover_layer = Some(photocraft_doc::LayerId(1)));
        assert_eq!(rollover, ["layers"], "the Move tool's rollover on the canvas");
        let all = regions(&app).len();
        let mask = moved(&mut app, &|app| app.ui.mask_target = true);
        assert_eq!(mask.len(), all, "the layer mask targeted in a panel: {mask:?}");
        let path = moved(&mut app, &|app| app.ui.selected_path = Some("work".into()));
        assert_eq!(path.len(), all, "a path selected on the canvas: {path:?}");
        let preview = moved(&mut app, &|app| app.live_adjust = Some((photocraft_doc::LayerId(1), serde_json::json!({"brightness": 10}))));
        assert_eq!(preview, ["histogram"], "an adjustment layer's live preview");
    }

    /// A command a key fires is a command run, as one from a menu is: the opacity and blend keys and the Artboard,
    /// Slice and panel commands change what a settled region shows without going through `menus::invoke`.
    #[test]
    fn a_fired_command_moves_every_version() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let before = (options_bar(&app), panel(&app, "layers"), status_bar(&app));
        let fired = crate::blend_keys::test_fired(craft::draw::ui::artboard::ArtboardCommand::AddNew.into());
        crate::shortcut_dispatch::run_fired(&mut app, &egui::Context::default(), &[fired]);
        assert!(app.ui.artboard_tool.adding, "Add New Artboard ran");
        let after = (options_bar(&app), panel(&app, "layers"), status_bar(&app));
        assert!(after.0 != before.0 && after.1 != before.1 && after.2 != before.2, "every version moved");
    }
}
