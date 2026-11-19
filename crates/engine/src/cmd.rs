//! The command registry. Ids follow Photoshop's menu structure (see the parity checklist).

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Adjustment, Document, Fill, Layer, LayerContent, LayerId, LayerMask, Size};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::Surface;
use serde_json::{Value, json};

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

macro_rules! cmd {
    ($id:literal, $label:literal, [$($m:literal),*], $sc:expr, $params:literal, $en:expr, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[$($m),*], shortcut: $sc, params: $params, enabled: $en, run: $run, journal: true }
    };
}

pub mod actions_cmds;
pub mod actions_edit_cmds;
pub mod adjust_cmds;
mod adjust_params;
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
#[cfg(test)]
mod command_order_tests;
pub mod comps_cmds;
pub mod contour_lib_cmds;
#[cfg(test)]
mod count_delete_group_tests;
#[cfg(test)]
mod count_history_names_tests;
#[cfg(test)]
mod count_record_label_tests;
#[cfg(test)]
mod count_resize_tests;
pub mod cutout_cmds;
pub mod distort_cmds;
pub mod document_preset_cmds;
pub mod edit_cmds;
pub mod edit_menu_cmds;
mod enabled;
pub mod eraser_cmds;
pub mod export_preset_cmds;
pub mod exr_cmds;
pub mod extra_cmds;
pub mod face_cmds;
pub mod file_cmds;
pub mod fill_cmds;
pub mod fill_key_cmds;
pub mod float_cmds;
pub mod font_library_cmds;
pub mod frame_cmds;
pub mod fx_view_cmds;
pub mod fx_visibility_cmds;
pub mod gallery_cmds;
#[cfg(test)]
mod gradient_alpha_method_tests;
pub mod gradient_fill_cmds;
pub mod group_view_cmds;
pub mod history_cmds;
pub mod history_menu_cmds;
pub mod history_options_cmds;
pub mod history_select_cmds;
pub mod history_source_cmds;
pub mod image_cmds;
pub mod layer_copy_cmds;
pub mod layer_css_cmds;
pub mod layer_label_cmds;
pub mod layer_list_cmds;
pub mod layer_mask_props_cmds;
pub mod layer_menu_cmds;
pub mod layer_multi_cmds;
pub mod layer_nav_cmds;
mod layer_params;
mod layer_references;
pub mod layer_xmp_cmds;
pub mod lens_cmds;
pub mod magnetic_cmds;
pub mod mask_view_cmds;
pub(super) mod migrate_cmds;
pub mod mode_cmds;
pub mod multichannel_cmds;
#[cfg(test)]
mod new_document_history_name_tests;
pub mod notes_cmds;
#[cfg(test)]
mod notes_stored_values_tests;
mod ops;
pub mod paint_cmds;
mod params;
pub(super) mod path_edit_cmds;
pub mod pattern_cmds;
pub mod pattern_fill_cmds;
pub mod perspective_crop_cmds;
#[cfg(test)]
mod perspective_crop_refuse_tests;
pub mod photo_cmds;
pub mod pick_cmds;
pub mod plugin_cmds;
pub mod preset_import_cmds;
pub mod print_cmds;
pub mod rasterize_style_cmds;
pub mod raw_editor_cmds;
pub mod redeye_cmds;
pub mod render_cmds;
pub mod retouch_cmds;
pub mod sample_cmds;
#[cfg(test)]
mod sample_zoom_box_tests;
pub mod sampler_cmds;
pub mod select_extra_cmds;
pub mod selection_cmds;
pub mod slice_cmds;
pub mod smart_cmds;
pub mod smart_svg_cmds;
pub mod smartselect_cmds;
pub mod solid_fill_cmds;
pub mod stamp_cmds;
pub mod style_asl_cmds;
pub mod swatch_cmds;
pub mod symmetry_cmds;
mod table;
pub(super) mod timeline_cmds;
pub mod transform_cmds;
pub(super) mod trap_cmds;
pub mod type_area_cmds;
pub mod type_caret_cmds;
pub mod type_cmds;
pub mod type_extra_cmds;
#[cfg(test)]
mod type_indexed_mode_tests;
pub mod type_mask_cmds;
pub mod type_spell_cmds;
pub mod type_styles_cmds;
pub(super) mod variables_cmds;
mod vector_clipboard;
pub mod vector_cmds;
pub(super) mod video_cmds;
pub mod vp_cmds;
pub mod warp_cmds;
pub mod web_cmds;
pub(super) mod wia_cmds;

pub(crate) use adjust_params::inject_kind;
use adjust_params::{adjust_mode, destructive_adjust, doc_mode, new_adjustment};
pub use adjust_params::{adjustment_from_params, adjustment_kind};
pub(crate) use enabled::{always, check_all_unlocked, check_pixels_unlocked, has_paintable, is_mask_target, paint_surface};
use enabled::{can_redo, can_undo, has_doc, has_layer, has_pixel_or_channel, has_selection};
pub(crate) use layer_params::{NEW_LAYER_PATH, explicit_layer, layer_listing, layer_panel_option, layer_param, mask_at_depth, mask_new_layer, selection_mask};
pub(crate) use layer_references::{recorded_drop, recorded_layer};
use ops::{arrange, merge_group, move_mask, set_mask, set_mask_and_deselect};
pub(crate) use ops::{layer_copy, translate_layer, translate_layer_wrapped};
use params::{bad, set_clipped};
pub(crate) use params::{check_f32_values, cmyk_param, color_param, int, int_i32, parse_hex, u32_id, u32_id_param};
pub use photocraft_color::blend::parse_blend;

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
/// Marquee, which an agent searches for as "ellipse" or "marquee". The RAW Develop filter and
/// RAW Defaults keep Photoshop's "Camera Raw" name as a search alias only: the menus and windows
/// say "RAW Develop" (#12403).
pub(crate) const SEARCH_KEYWORDS: &[(&str, &str)] = &[
    ("select.rect", "Rectangular Marquee, Elliptical Marquee (\"ellipse\":true), ellipse, oval, circle, Single Row/Column Marquee (\"line\")"),
    ("filter.cameraRaw", "Camera Raw Filter"),
    ("edit.preferences.rawDefaults", "Camera Raw Defaults"),
];

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

/// The command registry, built on first use and shared for the rest of the process.
fn registry() -> &'static Registry {
    static REGISTRY: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| Registry::new(build()))
}

// ---------- the table ----------

/// A named group of commands: `SPECS` lists them in registry order.
type Fragment = (&'static str, fn() -> Vec<CommandSpec>);

    // Adjustment layers + destructive adjustments, generated from one list.
    // Levels/Curves channel keys follow the document: red/green/blue (RGB), gray (Grayscale),
    // cyan/magenta/yellow/black (CMYK), lightness/a/b (Lab); see `adjust_params`.
    const ADJ: &[(&str, &str, &str)] = &[
        ("brightnessContrast", "Brightness/Contrast…", r##"{"brightness":-150..150=0,"contrast":-50..100=0,"legacy":bool=false}"##),
        (
            "levels",
            "Levels…",
            r##"{"inBlack":0..253=0,"gamma":0.01..9.99=1,"inWhite":2..255=255,"outBlack":0..255=0,"outWhite":0..255=255,"red":json,"green":json,"blue":json,"eyedropper":json} (top level = composite; per channel {"inBlack","gamma","inWhite","outBlack","outWhite"} under red/green/blue, gray, cyan/magenta/yellow/black or lightness/a/b; eyedropper = the dialog's Set Black/Neutral Gray/White Point picker {"point":"black|gray|white","at":[x,y],"size":1..101=1} or {"point",…,"color":[r,g,b] 0..1}: black/white set each channel's input black/white to the sample, gray each channel's gamma so the sample turns neutral at its luma; colour composites: RGB and Grayscale (no gray); Image adjustments targeting a mask, alpha or Quick Mask sample that gray plane in any document mode, applied over the given levels)"##,
        ),
        (
            "curves",
            "Curves…",
            r##"{"points":json,"red":json,"green":json,"blue":json,"pencil":bool=false,"smooth":0..10=0,"eyedropper":json} (curves as [[in,out],…] in 0..255, 2..19 points: points = composite; red/green/blue, gray, cyan/magenta/yellow/black or lightness/a/b per channel; pencil = drawn curves (Draw to modify the curve): each curve is 256 output levels in 0..255 (point curves given are drawn in, and turning pencil off fits points to the drawing); smooth = Smooth passes over the drawn curves; eyedropper = the dialog's Set Black/Neutral Gray/White Point picker {"point":"black|gray|white","at":[x,y],"size":1..101=1} or {"point",…,"color":[r,g,b] 0..1}, colour composites: RGB and Grayscale (no gray); Image adjustments targeting a mask, alpha or Quick Mask sample that gray plane in any document mode, applied over the given curves)"##,
        ),
        ("exposure", "Exposure…", r##"{"exposure":-20..20=0,"offset":-0.5..0.5=0,"gamma":0.01..9.99=1}"##),
        ("vibrance", "Vibrance…", r##"{"vibrance":-100..100=0,"saturation":-100..100=0}"##),
        (
            "hueSaturation",
            "Hue/Saturation…",
            r##"{"hue":-180..180=0,"saturation":-100..100=0,"lightness":-100..100=0,"colorize":bool=false,"reds":json,"yellows":json,"greens":json,"cyans":json,"blues":json,"magentas":json} (per range {"hue","saturation","lightness","range":[4 hue degrees]}; colorize: hue 0..360, saturation 0..100)"##,
        ),
        (
            "colorBalance",
            "Color Balance…",
            r##"{"shadows":json,"midtones":json,"highlights":json,"preserveLuminosity":bool=true} (each tone [cyan-red, magenta-green, yellow-blue] in -100..100)"##,
        ),
        (
            "blackWhite",
            "Black & White…",
            r##"{"reds":-200..300=40,"yellows":-200..300=60,"greens":-200..300=40,"cyans":-200..300=60,"blues":-200..300=20,"magentas":-200..300=80,"tint":bool=false,"tintColor":"#rrggbb"}"##,
        ),
        (
            "photoFilter",
            "Photo Filter…",
            r##"{"filter":"warming85|warmingLBA|warming81|cooling80|coolingLBB|cooling82|red|orange|yellow|green|cyan|blue|violet|magenta|sepia|deepRed|deepBlue|deepEmerald|deepYellow|underwater","color":"#rrggbb","density":0..100=25,"preserveLuminosity":bool=true}"##,
        ),
        (
            "channelMixer",
            "Channel Mixer…",
            r##"{"red":json,"green":json,"blue":json,"gray":json,"monochrome":bool=false} (per output channel [red %, green %, blue %, constant %] in -200..200; gray = the monochrome mix)"##,
        ),
        ("invert", "Invert", "{}"),
        ("posterize", "Posterize…", r##"{"levels":2..255=4}"##),
        ("threshold", "Threshold…", r##"{"level":1..255=128}"##),
        ("gradientMap", "Gradient Map…", r##"{"stops":json,"reverse":bool=false,"dither":bool=false} (stops [[location 0..1, "#rrggbb"], …], 2..64)"##),
        (
            "selectiveColor",
            "Selective Color…",
            r##"{"method":"relative|absolute"="relative","colors":"reds|yellows|greens|cyans|blues|magentas|whites|neutrals|blacks"="reds","cyan":-100..100=0,"magenta":-100..100=0,"yellow":-100..100=0,"black":-100..100=0,"reds":json} (per-range [c,m,y,k] arrays: reds yellows greens cyans blues magentas whites neutrals blacks)"##,
        ),
        (
            "colorLookup",
            "Color Lookup…",
            r##"{"lut":"none|warm|cool|tealOrange|bleachBypass|fadedFilm|dayForNight|monoContrast|crossProcess"="none","file":text,"interpolation":"trilinear|tetrahedral"="trilinear","dither":bool=false,"data":text,"dataEncoding":"text|base64"="text","fileName":text,"lutFormat":"auto|cube|3dl|look"="auto"} (file: LUT path; data: file text or encoded bytes; lutFormat overrides format detection, auto detects it)"##,
        ),
    ];
    for &(kind, label, params) in ADJ {
        let layer_id: &'static str = Box::leak(format!("layer.newAdjustmentLayer.{kind}").into_boxed_str());
        let image_id: &'static str = Box::leak(format!("image.adjustments.{kind}").into_boxed_str());
        v.push(CommandSpec {
            id: layer_id,
            label,
            menu: &["Layer", "New Adjustment Layer"],
            shortcut: None,
            params: Box::leak(format!(r#"{params} + {{{NEW_LAYER_PATH},"opacity":0..1=1,"name":str?=the adjustment's name + number,"mode":blend mode name="normal" (not Pass Through),"clipToPrevious":bool=false}}"#).into_boxed_str()),
            enabled: has_doc,
            run: |s, p| {
                let kind = p.get("__kind").and_then(Value::as_str).unwrap_or("invert").to_string();
                let eyedropped = if matches!(kind.as_str(), "levels" | "curves") { crate::adjust_params::eyedropper_from_params(s, &kind, p)? } else { None };
                let adj = match eyedropped {
                    Some(adj) => adj,
                    None => crate::adjust_params::from_params(&kind, p, None, doc_mode(s))?,
                };
                new_adjustment(s, adj, p, &format!("layer.newAdjustmentLayer.{kind}"))
            },
            journal: true,
        });
        v.push(CommandSpec {
            id: image_id,
            label,
            menu: &["Image", "Adjustments"],
            shortcut: match kind {
                "levels" => Some("Cmd+L"),
                "curves" => Some("Cmd+M"),
                "hueSaturation" => Some("Cmd+U"),
                "colorBalance" => Some("Cmd+B"),
                "invert" => Some("Cmd+I"),
                "blackWhite" => Some("Cmd+Alt+Shift+B"),
                _ => None,
            },
            params,
            enabled: crate::adjust_cmds::has_adjustable,
            run: |s, p| {
                let kind = p.get("__kind").and_then(Value::as_str).unwrap_or("invert").to_string();
                if matches!(kind.as_str(), "levels" | "curves") {
                    crate::channel_cmds::resolve_tone_target(s, false, p)?;
                }
                let eyedropped = if matches!(kind.as_str(), "levels" | "curves") { crate::adjust_params::destructive_eyedropper(s, &kind, p)? } else { None };
                // A smart filter re-runs its params, so it records what an eyedropper resolved
                // to, not the sample (#3127).
                let recorded = eyedropped.as_ref().map(|adj| {
                    let mut q = p.clone();
                    if let (Value::Object(q), Value::Object(resolved)) = (&mut q, crate::adjust_params::to_params(adj)) {
                        q.remove("eyedropper");
                        q.extend(resolved);
                    }
                    q
                });
                let adj = match eyedropped {
                    Some(adj) => adj,
                    None => {
                        // A gray plane target keeps the neutral tone space even over params
                        // carrying RGB, Lab or CMYK records (#5335).
                        let tone = if is_mask_target(p) || crate::channel_cmds::is_channel_target(p) {
                            if matches!(kind.as_str(), "levels" | "curves") { crate::adjust_params::gray_tone_params(p) } else { p.clone() }
                        } else {
                            p.clone()
                        };
                        crate::adjust_params::from_params(&kind, &tone, None, adjust_mode(s, p))?
                    }
                };
                let adj = if matches!(kind.as_str(), "levels" | "curves")
                    && let Some(colors) = crate::color_channels::target(s, p)?
                {
                    crate::color_channels::tone_adjustment(&kind, p, adj, doc_mode(s), colors)?
                } else {
                    adj
                };
                let label = adj.label().to_string();
                // A smart object records the adjustment as a smart filter (Photoshop), which
                // re-runs from the source instead of baking into a copy of the pixels.
                if crate::adjust_cmds::adjust_as_smart_filter(s, &kind, &label, recorded.as_ref().unwrap_or(p))? {
                    return Ok(Value::Null);
                }
                destructive_adjust(s, &label, adj, p)
            },
            journal: true,
        });
    }
    v.extend(crate::layer_style::specs());
    v.extend(crate::contour_lib_cmds::specs());
    v.extend(crate::layer_list_cmds::specs());
    v.extend(crate::filters::specs());
    v.extend(crate::raw_editor_cmds::specs());
    v.extend(crate::filters_ext::specs());
    v.extend(crate::color_to_alpha_cmds::specs());
    v.extend(crate::gallery_cmds::specs());
    v.extend(crate::gradient_fill_cmds::specs());
    v.extend(crate::solid_fill_cmds::specs());
    v.extend(crate::pattern_fill_cmds::specs());
    v.extend(crate::type_cmds::specs());
    v.extend(crate::type_mask_cmds::specs());
    v.extend(crate::transform_cmds::specs());
    v.extend(crate::float_cmds::specs());
    v.extend(crate::vector_cmds::specs());
    v.extend(crate::path_edit_cmds::specs());
    v.extend(crate::smartselect_cmds::specs());
    v.extend(crate::cutout_cmds::specs());
    v.extend(crate::symmetry_cmds::specs());
    v.extend(crate::edit_cmds::specs());
    v.extend(crate::smart_svg_cmds::specs());
    v.extend(crate::color_cmds::specs());
    v.extend(crate::brush_cmds::specs());
    v.extend(crate::brush_preset_cmds::specs());
    v.extend(crate::eraser_cmds::specs());
    v.extend(crate::preset_import_cmds::specs());
    v.extend(crate::style_asl_cmds::specs());
    v.extend(crate::atn_cmds::specs());
    v.extend(crate::actions_edit_cmds::specs());
    v.extend(crate::swatch_cmds::specs());
    v.extend(crate::retouch_cmds::specs());
    v.extend(crate::redeye_cmds::specs());
    v.extend(crate::image_cmds::specs());
    v.extend(crate::perspective_crop_cmds::specs());
    v.extend(crate::selection_cmds::specs());
    v.extend(crate::magnetic_cmds::specs());
    v.extend(crate::select_extra_cmds::specs());
    v.extend(crate::paint_cmds::specs());
    v.extend(crate::extra_cmds::specs());
    v.extend(crate::background_cmds::specs());
    v.extend(crate::file_cmds::specs());
    v.extend(crate::exr_cmds::specs());
    v.extend(crate::type_extra_cmds::specs());
    v.extend(crate::type_area_cmds::specs());
    v.extend(crate::type_styles_cmds::specs());
    v.extend(crate::type_spell_cmds::specs());
    v.extend(crate::type_caret_cmds::specs());
    v.extend(crate::smart_cmds::specs());
    v.extend(crate::layer_multi_cmds::specs());
    v.extend(crate::layer_copy_cmds::specs());
    v.extend(crate::prefs::specs());
    v.extend(crate::edit_menu_cmds::specs());
    v.extend(crate::fill_key_cmds::specs());
    v.extend(crate::sample_cmds::specs());
    v.extend(crate::face_cmds::specs());
    v.extend(crate::sampler_cmds::specs());
    v.extend(crate::brush_key_cmds::specs());
    v.extend(crate::stamp_cmds::specs());
    v.extend(crate::align_cmds::specs());
    v.extend(crate::photo_cmds::specs());
    v.extend(crate::lens_cmds::specs());
    v.extend(crate::vp_cmds::specs());
    v.extend(crate::channel_cmds::specs());
    v.extend(crate::adjust_cmds::specs());
    v.extend(crate::layer_menu_cmds::specs());
    v.extend(crate::layer_label_cmds::specs());
    v.extend(crate::layer_css_cmds::specs());
    v.extend(crate::layer_xmp_cmds::specs());
    v.extend(crate::mode_cmds::specs());
    v.extend(crate::multichannel_cmds::specs());
    v.extend(crate::pattern_cmds::specs());
    v.extend(crate::warp_cmds::specs());
    v.extend(crate::comps_cmds::specs());
    v.extend(crate::artboard_cmds::specs());
    v.extend(crate::distort_cmds::specs());
    v.extend(crate::analysis_cmds::specs());
    v.extend(crate::notes_cmds::specs());
    v.extend(crate::font_library_cmds::specs());
    v.extend(crate::history_cmds::specs());
    v.extend(crate::history_options_cmds::specs());
    v.extend(crate::history_menu_cmds::specs());
    v.extend(crate::history_source_cmds::specs());
    v.extend(crate::history_select_cmds::specs());
    v.extend(crate::proof_sim::specs());
    v.extend(crate::presets::specs());
    v.extend(crate::document_preset_cmds::specs());
    v.extend(crate::export_preset_cmds::specs());
    v.extend(crate::render_cmds::specs());
    v.extend(crate::slice_cmds::specs());
    v.extend(crate::web_cmds::specs());
    v.extend(crate::automate_cmds::specs());
    v.extend(crate::print_cmds::specs());
    v.extend(crate::installed_profiles::specs());
    v.extend(crate::pick_cmds::specs());
    v.extend(crate::layer_nav_cmds::specs());
    v.extend(crate::frame_cmds::specs());
    v.extend(crate::migrate_cmds::specs());
    v.extend(crate::trap_cmds::specs());
    v.extend(crate::timeline_cmds::specs());
    v.extend(crate::video_cmds::specs());
    v.extend(crate::jobs::specs());
    v.extend(crate::wia_cmds::specs());
    v.extend(crate::rasterize_style_cmds::specs());
    v.extend(crate::variables_cmds::specs());
    v.extend(crate::plugin_cmds::specs());
    v.extend(crate::group_view_cmds::specs());
    v.extend(crate::fx_view_cmds::specs());
    v.extend(crate::fx_visibility_cmds::specs());
    v.extend(crate::mask_view_cmds::specs());
    v.extend(crate::layer_mask_props_cmds::specs());
    v.extend(crate::actions_cmds::specs());
/// Every table fragment with its name, in registry order. The order is part of the behaviour (menus
/// list commands in it) and `command_ids.txt` pins it. The `table` fragments are grouped by the
/// crate that will own them; a menu split between owners has one fragment per contiguous run.
const SPECS: &[Fragment] = &[
    ("store::file_menu", table::store::file_menu::specs),
    ("core::file_menu", table::core::file_menu::specs),
    ("core::edit_menu", table::core::edit_menu::specs),
    ("paint::edit_menu", table::paint::edit_menu::specs),
    ("model::edit_menu", table::model::edit_menu::specs),
    ("core::select_menu", table::core::select_menu::specs),
    ("model::layer_menu", table::model::layer_menu::specs_before_gradient),
    ("store::layer_menu", table::store::layer_menu::specs),
    ("model::layer_menu", table::model::layer_menu::specs_after_gradient),
    ("model::image_menu", table::model::image_menu::specs),
    ("image::image_menu", table::image::image_menu::specs),
    ("paint::stroke", table::paint::stroke::specs),
    ("core::tools", table::core::tools::specs),
    ("core::queries", table::core::queries::specs),
    ("image::adjustments", table::image::adjustments::specs),
    ("layer_style", crate::layer_style::specs),
    ("contour_lib_cmds", crate::contour_lib_cmds::specs),
    ("layer_list_cmds", crate::layer_list_cmds::specs),
    ("filters", crate::filters::specs),
    ("filters_ext", crate::filters_ext::specs),
    ("color_to_alpha_cmds", crate::color_to_alpha_cmds::specs),
    ("gallery_cmds", crate::gallery_cmds::specs),
    ("gradient_fill_cmds", crate::gradient_fill_cmds::specs),
    ("solid_fill_cmds", crate::solid_fill_cmds::specs),
    ("pattern_fill_cmds", crate::pattern_fill_cmds::specs),
    ("type_cmds", crate::type_cmds::specs),
    ("type_mask_cmds", crate::type_mask_cmds::specs),
    ("transform_cmds", crate::transform_cmds::specs),
    ("float_cmds", crate::float_cmds::specs),
    ("vector_cmds", crate::vector_cmds::specs),
    ("path_edit_cmds", crate::path_edit_cmds::specs),
    ("smartselect_cmds", crate::smartselect_cmds::specs),
    ("cutout_cmds", crate::cutout_cmds::specs),
    ("symmetry_cmds", crate::symmetry_cmds::specs),
    ("edit_cmds", crate::edit_cmds::specs),
    ("smart_svg_cmds", crate::smart_svg_cmds::specs),
    ("color_cmds", crate::color_cmds::specs),
    ("brush_cmds", crate::brush_cmds::specs),
    ("brush_preset_cmds", crate::brush_preset_cmds::specs),
    ("eraser_cmds", crate::eraser_cmds::specs),
    ("preset_import_cmds", crate::preset_import_cmds::specs),
    ("style_asl_cmds", crate::style_asl_cmds::specs),
    ("atn_cmds", crate::atn_cmds::specs),
    ("actions_edit_cmds", crate::actions_edit_cmds::specs),
    ("swatch_cmds", crate::swatch_cmds::specs),
    ("retouch_cmds", crate::retouch_cmds::specs),
    ("redeye_cmds", crate::redeye_cmds::specs),
    ("image_cmds", crate::image_cmds::specs),
    ("perspective_crop_cmds", crate::perspective_crop_cmds::specs),
    ("selection_cmds", crate::selection_cmds::specs),
    ("magnetic_cmds", crate::magnetic_cmds::specs),
    ("select_extra_cmds", crate::select_extra_cmds::specs),
    ("paint_cmds", crate::paint_cmds::specs),
    ("extra_cmds", crate::extra_cmds::specs),
    ("background_cmds", crate::background_cmds::specs),
    ("file_cmds", crate::file_cmds::specs),
    ("exr_cmds", crate::exr_cmds::specs),
    ("type_extra_cmds", crate::type_extra_cmds::specs),
    ("type_area_cmds", crate::type_area_cmds::specs),
    ("type_styles_cmds", crate::type_styles_cmds::specs),
    ("type_spell_cmds", crate::type_spell_cmds::specs),
    ("type_caret_cmds", crate::type_caret_cmds::specs),
    ("smart_cmds", crate::smart_cmds::specs),
    ("layer_multi_cmds", crate::layer_multi_cmds::specs),
    ("layer_copy_cmds", crate::layer_copy_cmds::specs),
    ("prefs", crate::prefs::specs),
    ("edit_menu_cmds", crate::edit_menu_cmds::specs),
    ("fill_key_cmds", crate::fill_key_cmds::specs),
    ("sample_cmds", crate::sample_cmds::specs),
    ("face_cmds", crate::face_cmds::specs),
    ("sampler_cmds", crate::sampler_cmds::specs),
    ("brush_key_cmds", crate::brush_key_cmds::specs),
    ("stamp_cmds", crate::stamp_cmds::specs),
    ("align_cmds", crate::align_cmds::specs),
    ("photo_cmds", crate::photo_cmds::specs),
    ("lens_cmds", crate::lens_cmds::specs),
    ("vp_cmds", crate::vp_cmds::specs),
    ("channel_cmds", crate::channel_cmds::specs),
    ("adjust_cmds", crate::adjust_cmds::specs),
    ("layer_menu_cmds", crate::layer_menu_cmds::specs),
    ("layer_label_cmds", crate::layer_label_cmds::specs),
    ("layer_css_cmds", crate::layer_css_cmds::specs),
    ("layer_xmp_cmds", crate::layer_xmp_cmds::specs),
    ("mode_cmds", crate::mode_cmds::specs),
    ("multichannel_cmds", crate::multichannel_cmds::specs),
    ("pattern_cmds", crate::pattern_cmds::specs),
    ("warp_cmds", crate::warp_cmds::specs),
    ("comps_cmds", crate::comps_cmds::specs),
    ("artboard_cmds", crate::artboard_cmds::specs),
    ("distort_cmds", crate::distort_cmds::specs),
    ("analysis_cmds", crate::analysis_cmds::specs),
    ("notes_cmds", crate::notes_cmds::specs),
    ("font_library_cmds", crate::font_library_cmds::specs),
    ("history_cmds", crate::history_cmds::specs),
    ("history_options_cmds", crate::history_options_cmds::specs),
    ("history_menu_cmds", crate::history_menu_cmds::specs),
    ("history_source_cmds", crate::history_source_cmds::specs),
    ("history_select_cmds", crate::history_select_cmds::specs),
    ("proof_sim", crate::proof_sim::specs),
    ("presets", crate::presets::specs),
    ("document_preset_cmds", crate::document_preset_cmds::specs),
    ("export_preset_cmds", crate::export_preset_cmds::specs),
    ("render_cmds", crate::render_cmds::specs),
    ("slice_cmds", crate::slice_cmds::specs),
    ("web_cmds", crate::web_cmds::specs),
    ("automate_cmds", crate::automate_cmds::specs),
    ("print_cmds", crate::print_cmds::specs),
    ("installed_profiles", crate::installed_profiles::specs),
    ("pick_cmds", crate::pick_cmds::specs),
    ("layer_nav_cmds", crate::layer_nav_cmds::specs),
    ("frame_cmds", crate::frame_cmds::specs),
    ("migrate_cmds", crate::migrate_cmds::specs),
    ("trap_cmds", crate::trap_cmds::specs),
    ("timeline_cmds", crate::timeline_cmds::specs),
    ("video_cmds", crate::video_cmds::specs),
    ("jobs", crate::jobs::specs),
    ("wia_cmds", crate::wia_cmds::specs),
    ("rasterize_style_cmds", crate::rasterize_style_cmds::specs),
    ("variables_cmds", crate::variables_cmds::specs),
    ("plugin_cmds", crate::plugin_cmds::specs),
    ("group_view_cmds", crate::group_view_cmds::specs),
    ("fx_view_cmds", crate::fx_view_cmds::specs),
    ("fx_visibility_cmds", crate::fx_visibility_cmds::specs),
    ("mask_view_cmds", crate::mask_view_cmds::specs),
    ("layer_mask_props_cmds", crate::layer_mask_props_cmds::specs),
    ("actions_cmds", crate::actions_cmds::specs),
];

/// Every registered command, in the order of SPECS.
fn build() -> Vec<CommandSpec> {
    let specs = SPECS.iter().flat_map(|(_, specs)| specs());
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
