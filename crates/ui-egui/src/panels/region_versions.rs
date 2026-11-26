//! The shell's title bar and options bar and every docked panel body as memoized views (#16118). The shell runs a slot
//! every pass and the dock a panel body, and leave memoizing to the app; each reads the whole app, so it is a `MemoView`
//! whose key is a version of what it shows: while the version is the same and no input reaches the slot, the frame adds
//! its last run again instead of running every control in it. The status bar is a memoized view of its own
//! (`status_bar::StatusProps`), over the values it shows. A version covers the app data its slot shows that can change while no
//! input reaches the slot: what a command changes, the active document and layer and the tool in every version; what
//! the slot shows of the canvas view, a job or the brush and colours; and what an edit elsewhere changes without a
//! command, such as a box or pin dragged on the canvas. Input inside a slot runs it whatever its version, and so does a
//! repaint it asks for itself.

use std::hash::{Hash, Hasher};

use craft::ui::contract::{MemoProps, MemoView};

use super::PhotocraftApp;
use crate::color_picker_ui::PickWindow;

/// The props of a shell slot's view: the app the slot reads, which its version stands for. One type per slot, so a
/// trial and the frame's memo log tell the views apart.
macro_rules! slot_props {
    ($($name:ident: $what:literal),*) => {$(
        #[doc = concat!("The ", $what, "'s view: the app it reads, which its version stands for.")]
        #[derive(MemoProps)]
        struct $name<'a> {
            version: u64,
            #[memo_props(covered_by = "version", reason = "the slot's version holds everything the slot shows of the app")]
            app: &'a mut PhotocraftApp,
        }
    )*};
}
slot_props!(TitleSlot: "title bar", OptionsSlot: "options bar");

/// The title bar in its slot, replaying while [`title_bar`]'s version holds.
pub(crate) fn show_title_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let version = title_bar(app);
    let _ = MemoView::new("photocraft.title_bar", TitleSlot { version, app }, |slot: &mut TitleSlot<'_>, ui: &mut egui::Ui| super::title_bar(slot.app, ui))
        .show(ui);
}

/// The options bar in its slot, replaying while [`options_bar`]'s version holds.
pub(crate) fn show_options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let version = options_bar(app);
    let _ =
        MemoView::new("photocraft.options_bar", OptionsSlot { version, app }, |slot: &mut OptionsSlot<'_>, ui: &mut egui::Ui| super::options_bar(slot.app, ui))
            .show(ui);
}

/// A docked panel's body: the panel and the app it reads, which its version stands for.
#[derive(MemoProps)]
struct PanelBody<'a> {
    id: &'a str,
    version: u64,
    #[memo_props(covered_by = "version", reason = "the panel's version holds everything its body shows of the app")]
    app: &'a mut PhotocraftApp,
}

/// Panel `id`'s body, replaying while [`panel`]'s version holds.
pub(crate) fn show_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: &str) {
    let version = panel(app, id);
    let body = PanelBody { id, version, app };
    let _ =
        MemoView::new(("photocraft.panel", id), body, |body: &mut PanelBody<'_>, ui: &mut egui::Ui| super::panel_docking::body(body.app, ui, body.id)).show(ui);
}

/// The version of the options bar: every tool option (`app.ui.tool_options`, all of which the bar shows; keys, the
/// canvas menu, the canvas and its task bars, panels and presets write them without a command), the commit and cancel
/// cells and hints of an edit the canvas opens or ends ([`options_bar::edit_key`](super::options_bar::edit_key)), the
/// Zoom tool's view, an open Free Transform, Puppet Warp or Perspective Warp, which shows its own members in place
/// of the tool's, with what a drag on the canvas changes (the box, the picked pin, the planes; Escape ends them without
/// a command, #15547), the Crop tool's frame state, the type members' caret or selection, the brush and its presets, the
/// colours, and the vector bar's "Make" row's own two ingredients: the Pen draft's anchor count and the Work Path the
/// app hides (#12892), which a Pen drawing click by click changes with no History step (#12890). Which path is
/// selected is already in [`version`]; the hidden path is held as its document's place, since its content can only
/// change with the document's revision, which `version` holds too.
pub(crate) fn options_bar(app: &PhotocraftApp) -> u64 {
    let sessions =
        (crate::transform_tool::bar_key(app), crate::puppet_ui::bar_key(app), crate::perspective_ui::bar_key(app), crate::crop_ui::bar::bar_key(app));
    let shown = (serialized(&app.ui.tool_options), super::options_bar::edit_key(app), text_target(app));
    // A canvas gesture's start and end change what the bar enables (the symmetry menu's Transform entry).
    let gesture = crate::canvas::tools::in_gesture(app);
    let hidden = app.session.active().map(|st| (st.doc.id.0, app.ui.hidden_work_path.as_ref().map(|(id, _)| *id)));
    let paths = (app.ui.pen.as_ref().map(|p| p.draft.knots.len()), hidden);
    version(app, (view(app), sessions, shown, brush(app), colours(app), gesture, paths))
}

/// The version of the title bar: the title it shows (the active document's name and its unsaved mark), the language
/// its menu titles are in, whether the menus are in the bar (not in the macOS menu bar) and whether the bar takes the
/// menu keys (F10, Alt, Alt+letter), which a dialog or the shortcut recorder holds back: a dialog closed with Escape
/// runs no command, and the bar registers for the keys again only when it runs. An open menu is an overlay, so the
/// view runs while one is shown.
pub(crate) fn title_bar(app: &PhotocraftApp) -> u64 {
    let (gate, _) = crate::menus::menu_gate(app);
    let keys = (gate.modal, gate.recording_shortcut);
    version(app, (super::title_bar::title_text(app), crate::i18n::current().code(), app.services.native_menu.is_none(), keys))
}

/// The version of panel `id`'s body. The Navigator shows the active view and the adjustment preview its picture
/// shows, the Layers panel the layer the Move tool's
/// rollover outlines on the canvas, which effects lists and groups are open (view state off the document's revision,
/// the active document's `Tracked` revision, #15709), the Smart Filter row picked on its own and what it is asked to do in its next
/// pass (open a row for renaming, focus its Name filter, reveal a layer, follow the Move tool's pick), the Info panel the pointer's document pixel, the zoom and the Eyedropper's Sample
/// Size it samples at and the Crop tool's frame size, the Clone Source panel the pairing at the last hover point, the
/// Modifier Keys panel the latched keys, which their use releases, the Properties panel the colour its well's open
/// Color Picker previews and the answer that picker left, the Histogram the adjustment preview it counts and the
/// histograms its worker lands (`histogram_panel::update`, with no command). Brush
/// Settings and Brushes show the brush and its presets; Color, Swatches, Gradients and the Gradient Editor show the
/// colours, and so do Properties (a shape layer's fill and stroke swatches, a gradient fill's Foreground and
/// Background stops, the adjustment editors' swatches) and Character (the colour of new type is the foreground).
/// Character and Paragraph show the type defaults, and they and Properties the values at the caret or selection of the
/// type being edited, which Glyphs inserts at. Every other panel shows only what every version holds.
pub(crate) fn panel(app: &PhotocraftApp, id: &str) -> u64 {
    match id {
        "navigator" => version(app, (view(app), adjustment_preview(app))),
        "layers" => {
            let filter_pick = app.session.active().and_then(|st| app.smart_filter_picks.get(&st.doc.id).copied());
            let requests = (app.layers_rename, app.find_layers_focus, app.pending_reveal.as_ref().map(|(layer, _)| *layer), app.move_pick, filter_pick);
            version(app, (app.canvas_hover_layer, app.session.active_tracked().map(|st| st.revision().count), requests))
        }
        "info" => {
            let crop = crate::crop_ui::info_size(app).map(|size| size.map(f64::to_bits));
            version(app, (app.hover_doc.map(|at| at.map(f64::to_bits)), app.current_zoom().to_bits(), app.ui.tool_options.eyedropper.size.pixels(), crop))
        }
        "cloneSource" => version(app, app.ui.presets_ui.clone_cursor.map(|(document, at)| (document, at.map(f64::to_bits)))),
        "modifierKeys" => version(app, app.ui.shell.latched),
        "properties" => version(app, (crate::color_picker_ui::window_picker_state(app, PickWindow::Properties), colours(app), text_target(app))),
        "brushSettings" | "brushes" => version(app, brush(app)),
        "histogram" => version(app, (adjustment_preview(app), histograms(app))),
        "character" => version(app, (colours(app), type_defaults(app), text_target(app))),
        "paragraph" => version(app, (type_defaults(app), text_target(app))),
        "glyphs" => version(app, text_target(app)),
        "color" | "swatches" | "gradients" | "gradientEditor" => version(app, colours(app)),
        _ => version(app, ()),
    }
}

/// What every slot may show, joined with `extra`: the commands run (any of them may change anything), the active
/// document, its revision, its active layers and which part of them is targeted (the layer mask, the vector mask, a
/// filter mask, the selected path; the Layers, Channels, Paths and Properties panels and the canvas set it without a
/// command), the tool and the preferences (Preferences' Apply and OK and other writers change them through the engine
/// or `prefs.edit`, not as a counted command: units, the language, toolbar slots).
fn version(app: &PhotocraftApp, extra: impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    app.commands_run.hash(&mut hasher);
    app.session.prefs.rev().hash(&mut hasher);
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

/// The brush and its presets, for the options bar: their revisions move with every change of either.
fn brush(app: &PhotocraftApp) -> (u64, u64) {
    (app.session.tools.brush_rev, app.session.tools.presets_rev)
}

/// The Type tool's defaults for new type, for the panels that show them with no type target: an edit in one of them
/// with nothing targeted changes the defaults and runs no command.
fn type_defaults(app: &PhotocraftApp) -> impl Hash + '_ {
    let o = &app.ui.tool_options;
    (&o.type_font, &o.type_style, o.type_size.to_bits(), &o.type_aa, &o.type_align, &o.type_direction, &o.type_style_defaults)
}

/// The adjustment preview the Histogram counts while it shows: an adjustment layer's Properties drag (its proposed
/// values) or an Image › Adjustments dialog's preview. Both change on every move with no command until the release.
fn adjustment_preview(app: &PhotocraftApp) -> impl Hash + '_ {
    (app.live_adjust.as_ref(), crate::adjust_preview::shown_key(app))
}

/// The histograms the Histogram panel shows, by when each landed: a worker lands one with no command or revision.
fn histograms(app: &PhotocraftApp) -> [Option<u64>; 2] {
    [app.doc_hist.as_ref().map(|h| h.at.to_bits()), app.doc_hist_preview.as_ref().map(|h| h.at.to_bits())]
}

/// The caret and selection of the type being edited: the type members show the values there, and a click, a drag or an
/// arrow key on the canvas moves them without a command.
fn text_target(app: &PhotocraftApp) -> Option<(u64, Option<u64>, usize, usize)> {
    app.ui.text_edit.as_ref().map(|edit| (edit.layer, edit.document, edit.caret, edit.anchor))
}

/// The foreground and background colours, for the bars and panels that show them.
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

    /// The brush, the colours, the type defaults, the tool options, the type caret, the Move tool's rollover and an
    /// adjustment's live preview move the versions of the bars and panels that show them and of no other, so a drag that
    /// edits one leaves the rest replaying; a version that missed one would replay a stale bar or panel.
    #[test]
    fn what_a_region_shows_moves_only_its_version() {
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
            all.extend([("optionsBar", options_bar(app)), ("titleBar", title_bar(app))]);
            all
        };
        let moved = |app: &mut PhotocraftApp, change: &dyn Fn(&mut PhotocraftApp)| -> Vec<&'static str> {
            let before = regions(app);
            change(app);
            regions(app).into_iter().zip(before).filter(|((_, now), (_, was))| now != was).map(|((id, _), _)| id).collect()
        };
        assert_eq!(regions(&app), regions(&app), "the same state settles");
        let brush = moved(&mut app, &|app| {
            app.session.tools.brush.size += 1.0;
            app.session.tools.brush_changed();
        });
        assert_eq!(brush, ["brushSettings", "brushes", "optionsBar"], "the brush");
        assert_eq!(moved(&mut app, &|app| app.session.tools.presets_rev += 1), ["brushSettings", "brushes", "optionsBar"], "the brush presets");
        let colours = moved(&mut app, &|app| app.session.tools.background[2] = 0.5);
        assert_eq!(colours, ["properties", "character", "color", "swatches", "gradients", "gradientEditor", "optionsBar"], "the colours");
        assert_eq!(moved(&mut app, &|app| app.ui.tool_options.type_size += 1.0), ["character", "paragraph", "optionsBar"], "the type defaults");
        app.ui.tool = Tool::Crop;
        let ratio = moved(&mut app, &|app| app.ui.tool_options.crop.ratio = craft::draw::ui::crop::CROP_RATIOS[1]);
        assert_eq!(ratio, ["optionsBar"], "a crop ratio picked in the task bar");
        assert_eq!(moved(&mut app, &|app| crate::crop_ui::toggle_straighten(app)), ["optionsBar"], "Straighten pressed in the task bar");
        assert_eq!(moved(&mut app, &|app| app.ui.tool_options.fill_opacity = 50.0), ["optionsBar"], "a tool option a key sets");
        let sample = moved(&mut app, &|app| app.ui.tool_options.eyedropper.size = craft::color_ui::ui::SampleSize::Avg5);
        assert_eq!(sample, ["info", "optionsBar"], "the Sample Size the canvas menu picks");
        app.ui.tool = Tool::Type;
        let edit = |caret| crate::state::TextEdit { layer: 1, document: None, caret, anchor: 0, session: String::new(), created: false, mask: false };
        app.ui.text_edit = Some(edit(0));
        let caret = moved(&mut app, &|app| app.ui.text_edit = Some(edit(2)));
        assert_eq!(caret, ["properties", "character", "paragraph", "glyphs", "optionsBar"], "the caret moved in the type being edited");
        assert_eq!(moved(&mut app, &|app| app.canvas_hover_layer = Some(photocraft_doc::LayerId(1))), ["layers"], "the Move tool's rollover on the canvas");
        let preview = moved(&mut app, &|app| app.live_adjust = Some((photocraft_doc::LayerId(1), serde_json::json!({"brightness": 10}))));
        assert_eq!(preview, ["navigator", "histogram"], "an adjustment layer's live preview");
        let landed = moved(&mut app, &|app| {
            let key = crate::histogram_panel::HistKey { doc: photocraft_doc::DocId(1), revision: 0, source: 0, preview: 0, max_level: 0 };
            app.doc_hist = Some(crate::histogram_panel::PanelHist { key, at: 30.0, hist: Default::default() });
        });
        assert_eq!(landed, ["histogram"], "a histogram the worker landed");
    }

    /// Opening or closing a layer's effects list or a group is view state off the document's revision, and a command
    /// the engine runs directly is no counted command: the Layers panel's version still moves, so it shows the new rows.
    #[test]
    fn a_disclosure_toggle_moves_the_layers_version() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", serde_json::json!({"width": 100, "height": 100})).unwrap();
        let layer = app.session.execute("layer.new.layer", serde_json::json!({})).unwrap()["layer"].as_u64().unwrap();
        app.session.execute("layer.layerStyle.dropShadow", serde_json::json!({"layer": layer})).unwrap();
        let before = panel(&app, "layers");
        app.session.execute("layer.setEffectsExpanded", serde_json::json!({"layer": layer, "expanded": false})).unwrap();
        assert_ne!(panel(&app, "layers"), before, "closing the effects list moves the Layers version");
    }

    /// What every slot may show moves every version: the tool, a targeted layer mask or a selected path (panels and the
    /// canvas set them without a command).
    #[test]
    fn the_tool_and_the_target_move_every_version() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let all = |app: &PhotocraftApp| (options_bar(app), panel(app, "layers"), title_bar(app));
        let every = |app: &mut PhotocraftApp, name: &str, change: &dyn Fn(&mut PhotocraftApp)| {
            let before = all(app);
            change(app);
            let after = all(app);
            assert!(after.0 != before.0 && after.1 != before.1 && after.2 != before.2, "{name} moves every version");
        };
        every(&mut app, "the tool", &|app| app.ui.tool = Tool::Brush);
        every(&mut app, "the layer mask targeted", &|app| app.ui.mask_target = true);
        every(&mut app, "a path selected", &|app| app.ui.selected_path = Some("work".into()));
        every(&mut app, "a preference written", &|app| {
            app.session.prefs.edit(|p| p.general.image_interpolation = photocraft_engine::prefs::Interpolation::Nearest)
        });
    }

    /// The UI language moves the title bar's version (its menu titles).
    #[test]
    fn the_language_moves_the_title_bar() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        if crate::i18n::needs_catalogs() {
            let before = title_bar(&app);
            let fr = crate::i18n::Lang::from_code("fr").expect("registered language");
            let _lang = crate::i18n::language_scope(fr);
            assert_ne!(title_bar(&app), before, "a new UI language");
        }
    }

    /// A command a key fires is a command run, as one from a menu is: the opacity and blend keys and the Artboard,
    /// Slice and panel commands change what a slot shows without going through `menus::invoke`.
    #[test]
    fn a_fired_command_moves_every_version() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let before = (options_bar(&app), panel(&app, "layers"), title_bar(&app));
        let fired = crate::blend_keys::test_fired(craft::draw::ui::artboard::ArtboardCommand::AddNew.into());
        crate::shortcut_dispatch::run_fired(&mut app, &egui::Context::default(), &[fired]);
        assert!(app.ui.artboard_tool.adding, "Add New Artboard ran");
        let after = (options_bar(&app), panel(&app, "layers"), title_bar(&app));
        assert!(after.0 != before.0 && after.1 != before.1 && after.2 != before.2, "every version moved");
    }
}
