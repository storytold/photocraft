//! The command registry. Ids follow Photoshop's menu structure (see the parity checklist).

use photocraft_color::{BlendMode, Color, ColorMode, SampleType};
use photocraft_doc::{Adjustment, Document, Fill, Layer, LayerContent, LayerId, LayerMask, Size};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Map, Value, json};

use crate::{EngineError, Result, Session, inspect, pixels};

type Run = fn(&mut Session, &Value) -> Result<Value>;
type Enabled = fn(&Session) -> std::result::Result<(), String>;

/// Metadata + implementation for one command.
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Menu placement, e.g. `["Layer", "New"]`. Empty = not in menus.
    pub menu: &'static [&'static str],
    /// Default shortcut in egui-style notation (`Cmd+Shift+N`), mapped per platform by the UI.
    pub shortcut: Option<&'static str>,
    /// Human/agent-readable parameter description (JSON-ish).
    pub params: &'static str,
    pub enabled: Enabled,
    pub run: Run,
    /// Record in the session journal (false for queries).
    pub journal: bool,
}

/// Image › Image Rotation by a right angle or a flip (pixels, vectors, guides, … all move).
fn turn(s: &mut Session, label: &str, t: crate::canvas_geom::Turn) -> Result<Value> {
    if let Some(d) = s.active() {
        let a = t.affine(f64::from(d.doc.size.width), f64::from(d.doc.size.height));
        // Sampler points are pixel indices, not pixel edges: map the pixel's centre, so the point
        // lands on the same pixel `Turn::pixel_map` moves the pixels to (`w - 1 - x`).
        let half = photocraft_geom::Affine::translate(0.5, 0.5);
        s.set_canvas_map(a, half.then(&a).then(&photocraft_geom::Affine::translate(-0.5, -0.5)));
    }
    s.edit(label, |doc, _| {
        crate::canvas_geom::turn_canvas(doc, t);
        Ok(())
    })?;
    Ok(Value::Null)
}

macro_rules! cmd {
    ($id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: true }
    };
}

pub mod actions_cmds;
pub mod actions_edit_cmds;
pub mod adjust_cmds;
mod adjust_params;
mod adjustments;
pub mod align_cmds;
pub mod analysis_cmds;
pub mod artboard_cmds;
pub mod atn_cmds;
pub mod automate_cmds;
pub mod background_cmds;
pub mod brush_cmds;
pub mod brush_key_cmds;
pub mod brush_preset_cmds;
pub mod channel_cmds;
pub mod color_cmds;
pub mod color_to_alpha_cmds;
pub mod comps_cmds;
pub mod contour_lib_cmds;
pub mod cutout_cmds;
pub mod distort_cmds;
pub mod document_preset_cmds;
pub mod edit_cmds;
mod edit_menu;
pub mod edit_menu_cmds;
mod enabled;
pub mod eraser_cmds;
pub mod export_preset_cmds;
pub mod exr_cmds;
pub mod extra_cmds;
pub mod face_cmds;
pub mod file_cmds;
mod file_menu;
pub mod fill_cmds;
pub mod fill_key_cmds;
pub mod float_cmds;
pub mod font_library_cmds;
pub(super) mod frame_cmds;
pub mod fx_view_cmds;
pub mod fx_visibility_cmds;
pub mod gallery_cmds;
pub mod gradient_fill_cmds;
pub mod group_view_cmds;
pub mod history_cmds;
pub mod history_menu_cmds;
pub mod history_options_cmds;
pub mod history_select_cmds;
pub mod history_source_cmds;
pub mod image_cmds;
mod image_menu;
pub mod layer_copy_cmds;
pub mod layer_label_cmds;
pub mod layer_list_cmds;
pub mod layer_mask_props_cmds;
mod layer_menu;
pub mod layer_menu_cmds;
pub mod layer_multi_cmds;
pub mod layer_nav_cmds;
mod layer_params;
pub mod layer_xmp_cmds;
pub mod lens_cmds;
pub mod magnetic_cmds;
pub mod mask_view_cmds;
pub(super) mod migrate_cmds;
pub mod mode_cmds;
pub mod multichannel_cmds;
pub mod notes_cmds;
mod ops;
mod paint;
pub mod paint_cmds;
mod params;
pub(super) mod path_edit_cmds;
pub mod pattern_cmds;
pub mod pattern_fill_cmds;
pub mod perspective_crop_cmds;
pub mod photo_cmds;
pub mod pick_cmds;
pub mod plugin_cmds;
pub mod preset_import_cmds;
pub mod print_cmds;
mod queries;
pub mod rasterize_style_cmds;
pub mod redeye_cmds;
pub mod render_cmds;
pub mod retouch_cmds;
pub mod sample_cmds;
pub mod sampler_cmds;
pub mod select_extra_cmds;
mod select_menu;
pub mod selection_cmds;
pub mod slice_cmds;
pub mod smart_cmds;
pub mod smartselect_cmds;
pub mod solid_fill_cmds;
pub mod stamp_cmds;
pub mod style_asl_cmds;
pub mod swatch_cmds;
pub mod symmetry_cmds;
pub(super) mod timeline_cmds;
pub mod transform_cmds;
pub(super) mod trap_cmds;
pub mod type_area_cmds;
pub mod type_caret_cmds;
pub mod type_cmds;
pub mod type_extra_cmds;
pub mod type_mask_cmds;
pub mod type_spell_cmds;
pub mod type_styles_cmds;
pub mod updates_cmds;
pub(super) mod variables_cmds;
pub mod vector_cmds;
pub(super) mod video_cmds;
pub mod vp_cmds;
pub mod warp_cmds;
pub mod web_cmds;
pub(super) mod wia_cmds;

pub(crate) use adjust_params::inject_kind;
use adjust_params::{adjust_mode, destructive_adjust, doc_mode, gray_tone_params, new_adjustment};
pub use adjust_params::{adjustment_from_params, adjustment_kind};
pub(crate) use enabled::{always, check_all_unlocked, check_pixels_unlocked, has_paintable, is_mask_target, paint_surface};
use enabled::{can_redo, can_undo, has_doc, has_layer, has_pixel_layer, has_pixel_or_channel, has_selection};
pub use layer_params::blend_from_str;
pub(crate) use layer_params::{NEW_LAYER_PATH, explicit_layer, layer_listing, layer_panel_option, layer_param, mask_at_depth, mask_new_layer, selection_mask};
use ops::{arrange, merge_group, set_mask, set_mask_and_deselect};
pub(crate) use ops::{layer_copy, translate_layer};
use params::{bad, set_clipped};
pub(crate) use params::{check_f32_values, color_param, int, int_i32, parse_hex, u32_id, u32_id_param};

pub fn command_specs() -> &'static [CommandSpec] {
    &registry().specs
}

/// One `command.list` / `engine.commands` row for `c`, given why it can't run now (`None` = it can).
/// `disabledReason` is that message; `needsDocument` is true when the reason is that no document is
/// open (#3664), so an agent can tell "open or create a document first" from a precondition that a
/// document alone won't meet (a selection, an empty clipboard).
pub fn list_entry(s: &Session, c: &CommandSpec, reason: Option<String>) -> Value {
    let needs_document =
        s.active().is_none() && reason.as_deref().is_some_and(|r| ["no document", "no active document", "no open document"].iter().any(|p| r.starts_with(p)));
    let mut entry = json!({
        "id": c.id, "label": c.label, "menu": c.menu, "shortcut": c.shortcut, "params": c.params,
        "enabled": reason.is_none(), "disabledReason": reason, "needsDocument": needs_document,
    });
    if let (Some(k), Some(o)) = (search_keywords(c.id), entry.as_object_mut()) {
        o.insert("keywords".into(), json!(k));
    }
    entry
}

/// Other names a command is looked for by, beyond its id and label: the Photoshop tools and
/// options it stands for (#5060). `select.rect` is also the Elliptical and Single Row/Column
/// Marquee, which an agent searches for as "ellipse" or "marquee".
pub(crate) const SEARCH_KEYWORDS: &[(&str, &str)] =
    &[("select.rect", "Rectangular Marquee, Elliptical Marquee (\"ellipse\":true), ellipse, oval, circle, Single Row/Column Marquee (\"line\")")];

/// [`SEARCH_KEYWORDS`] of command `id`, if it has any.
pub fn search_keywords(id: &str) -> Option<&'static str> {
    SEARCH_KEYWORDS.iter().find(|(k, _)| *k == id).map(|(_, words)| *words)
}

/// Does a [`list_entry`] row match a command-list filter (`commands --filter`, `engine.commands`,
/// MCP `command_list`)? Its id, label or keywords contain `needle`, ignoring case.
pub fn list_entry_matches(entry: &Value, needle: &str) -> bool {
    let needle = needle.to_lowercase();
    ["id", "label", "keywords"].iter().any(|k| entry.get(*k).and_then(Value::as_str).is_some_and(|v| v.to_lowercase().contains(&needle)))
}

pub fn find(id: &str) -> Option<&'static CommandSpec> {
    let registry = registry();
    registry.by_id.get(id).and_then(|&index| registry.specs.get(index))
}

struct Registry {
    specs: Vec<CommandSpec>,
    by_id: std::collections::HashMap<&'static str, usize>,
}

impl Registry {
    fn new(specs: Vec<CommandSpec>) -> Self {
        let mut by_id = std::collections::HashMap::with_capacity(specs.len());
        for (index, spec) in specs.iter().enumerate() {
            // Preserve the linear lookup's first-match behavior if an id is duplicated.
            by_id.entry(spec.id).or_insert(index);
        }
        Self { specs, by_id }
    }
}

fn registry() -> &'static Registry {
    static REGISTRY: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| Registry::new(build()))
}

// ---------- the table ----------

const SPECS: &[fn() -> Vec<CommandSpec>] = &[
    file_menu::specs,
    edit_menu::specs,
    select_menu::specs,
    layer_menu::specs,
    image_menu::specs,
    paint::specs,
    queries::specs,
    adjustments::specs,
    crate::layer_style::specs,
    crate::contour_lib_cmds::specs,
    crate::layer_list_cmds::specs,
    crate::filters::specs,
    crate::filters_ext::specs,
    crate::color_to_alpha_cmds::specs,
    crate::gallery_cmds::specs,
    crate::gradient_fill_cmds::specs,
    crate::solid_fill_cmds::specs,
    crate::pattern_fill_cmds::specs,
    crate::type_cmds::specs,
    crate::type_mask_cmds::specs,
    crate::transform_cmds::specs,
    crate::float_cmds::specs,
    crate::vector_cmds::specs,
    crate::path_edit_cmds::specs,
    crate::smartselect_cmds::specs,
    crate::cutout_cmds::specs,
    crate::symmetry_cmds::specs,
    crate::edit_cmds::specs,
    crate::color_cmds::specs,
    crate::brush_cmds::specs,
    crate::brush_preset_cmds::specs,
    crate::eraser_cmds::specs,
    crate::preset_import_cmds::specs,
    crate::style_asl_cmds::specs,
    crate::atn_cmds::specs,
    crate::actions_edit_cmds::specs,
    crate::swatch_cmds::specs,
    crate::retouch_cmds::specs,
    crate::redeye_cmds::specs,
    crate::image_cmds::specs,
    crate::perspective_crop_cmds::specs,
    crate::selection_cmds::specs,
    crate::magnetic_cmds::specs,
    crate::select_extra_cmds::specs,
    crate::paint_cmds::specs,
    crate::extra_cmds::specs,
    crate::background_cmds::specs,
    crate::file_cmds::specs,
    crate::exr_cmds::specs,
    crate::type_extra_cmds::specs,
    crate::type_area_cmds::specs,
    crate::type_styles_cmds::specs,
    crate::type_spell_cmds::specs,
    crate::type_caret_cmds::specs,
    crate::smart_cmds::specs,
    crate::layer_multi_cmds::specs,
    crate::layer_copy_cmds::specs,
    crate::prefs::specs,
    crate::updates_cmds::specs,
    crate::edit_menu_cmds::specs,
    crate::fill_key_cmds::specs,
    crate::sample_cmds::specs,
    crate::face_cmds::specs,
    crate::sampler_cmds::specs,
    crate::brush_key_cmds::specs,
    crate::stamp_cmds::specs,
    crate::align_cmds::specs,
    crate::photo_cmds::specs,
    crate::lens_cmds::specs,
    crate::vp_cmds::specs,
    crate::channel_cmds::specs,
    crate::adjust_cmds::specs,
    crate::layer_menu_cmds::specs,
    crate::layer_label_cmds::specs,
    crate::layer_xmp_cmds::specs,
    crate::mode_cmds::specs,
    crate::multichannel_cmds::specs,
    crate::pattern_cmds::specs,
    crate::warp_cmds::specs,
    crate::comps_cmds::specs,
    crate::artboard_cmds::specs,
    crate::distort_cmds::specs,
    crate::analysis_cmds::specs,
    crate::notes_cmds::specs,
    crate::font_library_cmds::specs,
    crate::history_cmds::specs,
    crate::history_options_cmds::specs,
    crate::history_menu_cmds::specs,
    crate::history_source_cmds::specs,
    crate::history_select_cmds::specs,
    crate::proof_sim::specs,
    crate::presets::specs,
    crate::document_preset_cmds::specs,
    crate::export_preset_cmds::specs,
    crate::render_cmds::specs,
    crate::slice_cmds::specs,
    crate::web_cmds::specs,
    crate::automate_cmds::specs,
    crate::print_cmds::specs,
    crate::installed_profiles::specs,
    crate::pick_cmds::specs,
    crate::layer_nav_cmds::specs,
    crate::frame_cmds::specs,
    crate::migrate_cmds::specs,
    crate::trap_cmds::specs,
    crate::timeline_cmds::specs,
    crate::video_cmds::specs,
    crate::jobs::specs,
    crate::wia_cmds::specs,
    crate::rasterize_style_cmds::specs,
    crate::variables_cmds::specs,
    crate::plugin_cmds::specs,
    crate::group_view_cmds::specs,
    crate::fx_view_cmds::specs,
    crate::fx_visibility_cmds::specs,
    crate::mask_view_cmds::specs,
    crate::layer_mask_props_cmds::specs,
    crate::actions_cmds::specs,
];

/// Every registered command, in the order of SPECS.
fn build() -> Vec<CommandSpec> {
    let specs = SPECS.iter().flat_map(|specs| specs());
    // The dispatch guard's unit test needs a command that panics on purpose (REL-3).
    #[cfg(test)]
    let specs = specs.chain(std::iter::once(test_panic_command()));
    specs.collect()
}

/// A command whose `run` panics on purpose, so the last-resort guard in `jobs::dispatch`
/// can be tested (REL-3). Test builds only: `commands::build` pushes it under `cfg(test)`.
#[cfg(test)]
fn test_panic_command() -> CommandSpec {
    #[allow(clippy::panic)]
    fn run(_: &mut Session, _: &Value) -> Result<Value> {
        panic!("test: this command panicked on purpose")
    }
    CommandSpec { id: "test.panic", label: "Test Panic", menu: &[], shortcut: None, params: r##"{}"##, enabled: |_: &Session| Ok(()), run, journal: false }
}

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[test]
    fn duplicate_ids_keep_the_first_spec_and_listing_order() {
        let mut built = build().into_iter();
        let first = built.next().unwrap();
        let mut second = built.next().unwrap();
        let id = first.id;
        second.id = id;
        let registry = Registry::new(vec![first, second]);
        assert_eq!(registry.by_id.get(id), Some(&0));
        assert_eq!(registry.specs.len(), 2);
    }
}

#[cfg(test)]
mod set_props_tests {
    use super::*;

    #[test]
    fn opacity_and_fill_outside_zero_to_one_are_refused_not_clamped() {
        // #5242: `55` (a percentage, as the Layer Style commands take) used to become 100 %.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("layer.setProps", json!({"opacity": 0.55, "fill": 0.4})).unwrap();
        let layer = |s: &Session| {
            let d = s.active().unwrap();
            let l = d.doc.layer(d.active_layer.unwrap()).unwrap();
            (l.opacity, l.fill_opacity)
        };
        assert_eq!(layer(&s), (0.55, 0.4));
        for bad in [json!({"opacity": 55}), json!({"opacity": -0.1}), json!({"fill": 1.5}), json!({"opacity": "50%"}), json!({"opacity": 0.5, "fill": 40})] {
            let e = s.execute("layer.setProps", bad.clone()).unwrap_err().to_string();
            assert!(e.contains("0..1"), "{bad}: {e}");
            assert_eq!(layer(&s), (0.55, 0.4), "{bad} left the layer alone");
        }
        s.execute("layer.setProps", json!({"opacity": 0, "fill": 1})).unwrap();
        assert_eq!(layer(&s), (0.0, 1.0));
    }
}

#[cfg(test)]
mod parse_hex_tests {
    use super::parse_hex;

    #[test]
    fn signs_and_other_non_hex_characters_are_rejected() {
        for bad in ["#+f+f+f", "+f+f+f+f", "#-f0000", "#gg0000", "#0f0f+f"] {
            assert_eq!(parse_hex(bad), None, "{bad:?} is not hex digits");
        }
        let g = 15.0 / 255.0;
        assert_eq!(parse_hex("#0f0f0f"), Some([g, g, g, 1.0]));
        assert_eq!(parse_hex("#0f0f0fff"), Some([g, g, g, 1.0]));
    }
}
