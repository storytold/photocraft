//! Shape fill and stroke paints shared by the options bar and the Properties panel.

use craft::color_ui::palette::SwatchGroup;
use craft::color_ui::ui::{GradientPreview, Paint, PaintChip, PaintChipOutput, SwatchKey};
use craft::draw::stroke::StrokeStyle;
use craft::draw::ui::GradientRef;
use craft::draw::ui::shape::{ShapeAppearance, ShapePaint};
use craft::theme::Srgba;
use craft::ui::contract::{Bind, Show as _};
use craft::ui::patterns::presets::PresetState;
use serde_json::{Value, json};

use crate::shape_gradient_ui::ShapeGradients;
use crate::shape_stroke_ui::StrokeOptions;

pub(crate) fn color_of(f: &photocraft_doc::Fill) -> Option<[f32; 4]> {
    match f {
        photocraft_doc::Fill::Solid(c) => Some(rgba_of(c)),
        _ => None,
    }
}

pub(crate) fn rgba_of(c: &photocraft_doc::Color) -> [f32; 4] {
    let [r, g, b] = c.to_rgb();
    [r, g, b, c.alpha]
}

/// A float RGBA colour as the framework's colour.
pub(crate) fn srgba(c: [f32; 4]) -> Srgba {
    Srgba::new(c[0], c[1], c[2], c[3])
}

/// The framework's colour as float RGBA.
pub(crate) fn rgba(c: Srgba) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}

/// A solid or gradient paint exactly as `shape.create` / `shape.edit` take it: float RGBA, the
/// gradient's stops as stored colours (a CMYK or Lab stop keeps its model) and every gradient
/// field, so a swatch edit (angle, style…) keeps the rest of the gradient. The swatches never
/// hand it a pattern (they don't offer one).
pub(crate) fn paint_param(f: &photocraft_doc::Fill) -> Value {
    match f {
        photocraft_doc::Fill::Gradient { stops, angle, scale, style, reverse, opacity_stops, midpoints, offset, dither, align } => json!({"gradient": {
            "stops": stops.iter().map(|(t, c)| json!([t, c])).collect::<Vec<_>>(),
            "angle": angle, "scale": scale * 100.0, "style": format!("{style:?}").to_ascii_lowercase(), "reverse": reverse,
            "opacityStops": opacity_stops, "midpoints": midpoints, "offset": [offset.0, offset.1], "dither": dither, "align": align,
        }}),
        other => json!(color_of(other)),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PaintChange {
    Color([f32; 4]),
    /// A `Fill::Gradient` (the chip's Gradient tab).
    Gradient(photocraft_doc::Fill),
    None,
}

/// The chip's preview of a gradient fill: its stops by position, ascending.
pub(crate) fn gradient_preview(fill: &photocraft_doc::Fill) -> GradientPreview {
    let photocraft_doc::Fill::Gradient { stops, reverse, .. } = fill else { return GradientPreview::new(Vec::new()) };
    let mut ramp: Vec<(f32, Srgba)> = stops.iter().map(|(p, c)| (p.clamp(0.0, 1.0), srgba(rgba_of(c)))).collect();
    ramp.sort_by(|a, b| a.0.total_cmp(&b.0));
    GradientPreview::new(ramp).reversed(*reverse)
}

/// The paint a chip shows for `fill` (`None` is No Color). A pattern shows as No Color: shapes
/// do not offer one.
pub(crate) fn paint_of(fill: Option<&photocraft_doc::Fill>) -> Paint {
    match fill {
        Some(photocraft_doc::Fill::Solid(c)) => Paint::Solid(srgba(rgba_of(c))),
        Some(f @ photocraft_doc::Fill::Gradient { .. }) => Paint::Gradient(gradient_preview(f)),
        _ => Paint::None,
    }
}

/// The gradient a chip's `preview` stands for over `base` (the chip's [`ChipLibrary::gradient`]): `base` itself when
/// the preview shows its stops and direction, so the stops keep their colour models; otherwise `base` with the
/// preview's stops and direction, its angle, style and the other fields kept.
pub(crate) fn gradient_fill(preview: &GradientPreview, base: &photocraft_doc::Fill) -> photocraft_doc::Fill {
    if gradient_preview(base) == *preview {
        return base.clone();
    }
    let stops = preview.stops.iter().map(|(t, c)| (*t, photocraft_doc::Color::rgba(c.r, c.g, c.b, c.a))).collect();
    match base {
        photocraft_doc::Fill::Gradient { .. } => {
            let mut merged = base.clone();
            if let photocraft_doc::Fill::Gradient { stops: kept, reverse, .. } = &mut merged {
                *kept = stops;
                *reverse = preview.reverse;
            }
            merged
        }
        _ => photocraft_doc::Fill::gradient(stops, 90.0, 1.0, photocraft_doc::GradientStyle::Linear, preview.reverse),
    }
}

/// The `shape.edit` parameter of a chip's `paint`: a colour as float RGBA, a gradient as [`gradient_fill`] over
/// `base` gives it, `null` for No Color or a paint the shape tools do not offer (a pattern or picture).
pub(crate) fn fill_param(paint: &Paint, base: &photocraft_doc::Fill) -> Value {
    match paint {
        Paint::Solid(c) => json!(rgba(*c)),
        Paint::Gradient(preview) => paint_param(&gradient_fill(preview, base)),
        _ => Value::Null,
    }
}

/// The colour of a solid `paint`, for the Recently Used Colors.
pub(crate) fn solid_rgb(paint: &Paint) -> Option<[f32; 3]> {
    match paint {
        Paint::Solid(c) => Some([c.r, c.g, c.b]),
        _ => None,
    }
}

/// What the Shape Properties panel and the shape options bar show for shape layer `sh`; `locked` disables every field.
pub(crate) fn appearance_of(sh: &photocraft_doc::ShapeLayer, locked: bool) -> ShapeAppearance {
    let width = sh.stroke.as_ref().map_or(0.0, |s| s.width);
    let style = sh.stroke.as_ref().map(|s| StrokeStyle::from(&StrokeOptions::from(s))).unwrap_or_default();
    ShapeAppearance::new(paint_of(sh.fill.as_ref()), paint_of(sh.stroke.as_ref().map(|s| &s.paint)), f64::from(width)).stroke_style(style).locked(locked)
}

/// What the Fill and Stroke chips offer beside the paint, for the shape options bar and the Shape Properties panel
/// alike: the Recently Used Colors, the swatch library, the full Color Picker and the Gradient tab.
pub(crate) struct ChipLibrary<'a> {
    recent: &'a [Srgba],
    swatches: &'a [SwatchGroup],
    folders: &'a mut PresetState<SwatchKey>,
    gradients: ShapeGradients<'a>,
    gradient_state: &'a mut PresetState<GradientRef>,
    /// The chips' paints as the bar or panel shows them, fill then stroke.
    current: [Option<photocraft_doc::Fill>; 2],
    /// What a switch to the Gradient tab sets.
    default_gradient: photocraft_doc::Fill,
    /// The gradients the chips' editors produced this pass, fill then stroke.
    edited: [Option<photocraft_doc::Fill>; 2],
}

impl<'a> ChipLibrary<'a> {
    /// The library over `recent` colours, the swatch `swatches` with their open `folders`, the `gradients` store and
    /// its list state; `current` is the fill and the stroke paint shown.
    pub(crate) fn new(
        recent: &'a [Srgba],
        swatches: &'a [SwatchGroup],
        folders: &'a mut PresetState<SwatchKey>,
        gradients: ShapeGradients<'a>,
        gradient_state: &'a mut PresetState<GradientRef>,
        current: [Option<photocraft_doc::Fill>; 2],
    ) -> Self {
        let default_gradient = gradients.default_fill();
        ChipLibrary { recent, swatches, folders, gradients, gradient_state, current, default_gradient, edited: [None, None] }
    }

    /// The gradient chip `which` stands for: the one its editor produced this pass, else its own gradient, else the
    /// default gradient.
    pub(crate) fn gradient(&self, which: ShapePaint) -> photocraft_doc::Fill {
        let slot = slot(which);
        self.edited[slot]
            .clone()
            .or_else(|| self.current[slot].clone().filter(|f| matches!(f, photocraft_doc::Fill::Gradient { .. })))
            .unwrap_or_else(|| self.default_gradient.clone())
    }
}

/// A chip's place in [`ChipLibrary`]'s pairs: 0 the fill, 1 the stroke.
fn slot(which: ShapePaint) -> usize {
    match which {
        ShapePaint::Stroke => 1,
        _ => 0,
    }
}

/// The Fill or Stroke chip `which` with what `library` offers; a gradient edited in its Gradient tab is kept in the
/// library for [`ChipLibrary::gradient`].
pub(crate) fn decorate_chip<'c>(chip: PaintChip<'c>, which: ShapePaint, library: &'c mut ChipLibrary<'_>, ui: &mut egui::Ui) -> PaintChipOutput {
    let from = library.gradient(which);
    let default_preview = gradient_preview(&library.default_gradient);
    let ChipLibrary { recent, swatches, folders, gradients, gradient_state, edited, .. } = library;
    let edited = &mut edited[slot(which)];
    let folders: &mut PresetState<SwatchKey> = folders;
    chip.recent(recent)
        .swatches(swatches, Bind::bound(folders))
        .full_picker(true)
        .gradient_editor(default_preview, move |ui: &mut egui::Ui| {
            // A second run in the same pass starts from what the first produced.
            let base = edited.clone().unwrap_or_else(|| from.clone());
            let picked = crate::shape_gradient_ui::editor(ui, &base, gradients, Bind::bound(&mut **gradient_state))?;
            let preview = gradient_preview(&picked);
            *edited = Some(picked);
            Some(preview)
        })
        .show(ui)
}
