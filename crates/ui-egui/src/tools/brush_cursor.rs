//! The painting cursor of the brush tools, over the framework's `BrushCursor`.
//!
//! The app keeps one [`BrushTip`] for the brush (`PhotocraftApp::brush_tip`), rebuilt by [`sync_tip`] only when
//! a field the outline depends on changes, so the framework reuses its cursor image while the pointer moves.
//! The Pencil and the Eraser in Pencil or Block mode have no held tip: their pixel cell follows the pointer, so
//! [`cursor`] builds it per call.

use craft::behavior::kurbo::Rect;
use craft::behavior::{Cursor, CursorIcon, PointerSample};
use craft::ui::canvas::ToolContext;
use craft::ui::canvas_controls::brush::{BrushCursor, BrushCursorOptions, BrushTip, BrushTipOutline, PaintingCursor, TipPose, TipRings};
use photocraft_engine::prefs::PaintingCursor as PaintingPref;
use photocraft_paint::brush::{BrushSettings, Control, TipShape};
use photocraft_paint::dynamics::{StepInput, apply_pose, control_angle};

use crate::PhotocraftApp;
use crate::state::Tool;

/// The brush fields a tip's outline depends on. The size is part of it only for a plain round tip, whose radii
/// scale with it; a traced outline is in radius units.
#[derive(Clone, PartialEq)]
pub(crate) struct TipKey {
    tip: TipShape,
    angle: f32,
    roundness: f32,
    hardness: f32,
    flip_x: bool,
    flip_y: bool,
    /// The size of a plain round tip, 0 for a traced one.
    size: f32,
}

impl TipKey {
    fn of(b: &BrushSettings) -> Self {
        Self {
            tip: b.tip.clone(),
            angle: b.angle,
            roundness: b.roundness,
            hardness: b.hardness,
            flip_x: b.flip_x,
            flip_y: b.flip_y,
            size: if is_plain(b) { b.size } else { 0.0 },
        }
    }
}

/// A round tip with no angle or flattening: the circle says it all.
fn is_plain(b: &BrushSettings) -> bool {
    matches!(b.tip, TipShape::Round) && b.angle == 0.0 && b.roundness >= 1.0
}

/// The size and kind of the aliased cursor tip the `tool` has: the Pencil's, the Eraser's in Pencil mode, and the
/// Eraser's square Block (`true`), which takes the block size.
fn aliased(app: &PhotocraftApp, tool: Tool) -> Option<(f32, bool)> {
    if crate::eraser_ui::block_mode(app, tool) {
        return Some((crate::eraser_ui::block_size(app), true));
    }
    (tool == Tool::Pencil || crate::eraser_ui::pencil_mode(app, tool)).then_some((app.session.tools.brush.size, false))
}

/// Builds the held tip for the active brush when its outline changed; a size change alone only updates the
/// diameter. Runs once per pass before the tool host, so [`cursor`] only reads.
pub(crate) fn sync_tip(app: &mut PhotocraftApp) {
    if aliased(app, app.ui.tool).is_some() {
        return;
    }
    let brush = &app.session.tools.brush;
    let key = TipKey::of(brush);
    let size = f64::from(brush.size);
    if let Some((held, tip)) = &mut app.brush_tip
        && *held == key
    {
        tip.diameter = size;
        return;
    }
    let tip = build(brush);
    app.brush_tip = Some((key, tip));
}

/// The tip of `brush`: two circles for a plain round tip, else the engine's traced outlines.
fn build(brush: &BrushSettings) -> BrushTip {
    let size = f64::from(brush.size);
    if !is_plain(brush)
        && let (Some(normal), Some(full)) = (photocraft_paint::outline::tip_outline(brush, false), photocraft_paint::outline::tip_outline(brush, true))
    {
        return BrushTip::new(size, BrushTipOutline::traced(TipRings::new(normal), TipRings::new(full)));
    }
    let normal = photocraft_paint::half_coverage_radius(brush.size / 2.0, brush.hardness).max(1.0);
    BrushTip::round(size, f64::from(normal))
}

/// The tip's axes for the pen's pose, in document space before the view: the brush angle from the shape dynamics
/// (pen tilt or barrel rotation), plus the barrel rotation and the tilt's shortening under Brush Projection. Upright
/// without shape dynamics or a pen.
pub(crate) fn pose(brush: &BrushSettings, pen: Option<PointerSample>) -> TipPose {
    let sd = &brush.shape_dynamics;
    let Some(pen) = pen.filter(|_| sd.enabled) else { return TipPose::upright() };
    let pen = pen.sanitized();
    let point = apply_pose(
        brush,
        photocraft_paint::StrokePoint {
            pressure: pen.pressure.clamp(0.0, 1.0),
            tilt_x: pen.tilt[0].to_degrees(),
            tilt_y: pen.tilt[1].to_degrees(),
            rotation: pen.rotation.to_degrees(),
            ..Default::default()
        },
    );
    let step = StepInput { point, direction: 0.0, initial_direction: 0.0, step: 0 };
    let mut angle = match sd.angle.control {
        Control::PenTilt | Control::Rotation => control_angle(sd.angle.control, sd.angle.fade_steps, &step),
        _ => 0.0,
    };
    if sd.brush_projection {
        angle += point.rotation;
    }
    TipPose::pen(f64::from(angle), sd.brush_projection.then_some([point.tilt_x, point.tilt_y]))
}

/// Preferences › Cursors › Painting Cursors as the framework names it.
fn painting_cursor(pref: PaintingPref) -> PaintingCursor {
    match pref {
        PaintingPref::Standard => PaintingCursor::Standard,
        PaintingPref::Precise => PaintingCursor::Precise,
        PaintingPref::NormalTip => PaintingCursor::NormalBrushTip,
        PaintingPref::FullSizeTip => PaintingCursor::FullSizeBrushTip,
    }
}

/// The cursor of the active painting tool over the canvas. Reads the tip [`sync_tip`] built; with no view or no
/// tip it is the system arrow.
pub(crate) fn cursor(app: &PhotocraftApp, cx: &ToolContext<'_>) -> Cursor {
    let arrow = Cursor::System(CursorIcon::Default);
    let Some(space) = cx.space() else { return arrow };
    let tool = app.ui.tool;
    let prefs = &app.session.prefs().cursors;
    let options = BrushCursorOptions::new(painting_cursor(prefs.painting))
        .show_only_crosshair_while_painting(prefs.show_only_crosshair_while_painting)
        .caps_lock_precise(true);
    let option = cx.mods.alt || app.ui.shell.latched.alt;
    if crate::canvas::cursor_policy::source_target(tool, option, prefs.painting, cx.caps_lock) {
        return crate::canvas::cursor_policy::sample_target();
    }
    let centre = crate::canvas::cursor_policy::brush_tip_centre(tool, option, prefs.show_crosshair_in_brush_tip);
    let painting = crate::canvas::paint_tool::stroking(app).is_some();
    // The framework's Standard cursor is the tool's icon.
    let icon = crate::icons::tool_icon(tool);
    let brush = &app.session.tools.brush;
    if let Some((size, block)) = aliased(app, tool) {
        // The whole-pixel cell the dab fills at the pointer; without a pointer there is nothing to outline.
        let Some(pointer) = cx.pointer.as_ref() else { return arrow };
        let at = space.view.to_doc(pointer.pos);
        let [x0, y0, x1, y1] = photocraft_engine::paint::grid_square(at.x, at.y, size);

        let round = !block && crate::canvas::cursor_policy::pencil_tip_is_round(size);
        let tip = BrushTip::new(f64::from(size), BrushTipOutline::pixels(Rect::new(x0, y0, x1, y1), round));
        return BrushCursor::new(&tip).centre(centre).painting(painting).icon(icon).cursor(cx, &options, &space.view);
    }
    let Some((_, tip)) = &app.brush_tip else { return arrow };
    BrushCursor::new(tip).pose(pose(brush, cx.pointer.as_ref().map(|p| p.sample))).centre(centre).painting(painting).icon(icon).cursor(
        cx,
        &options,
        &space.view,
    )
}
