//! Align actual prepared native content to actual prepared native target bounds.
//! This is ordinary native affine placement, not source bounding-box emulation.
use super::effects::{MappingFailure, NativeBranch, NativeMapping, NativeStructure};
use photocraft_geom::{Affine, Rect};
use photocraft_raster::{Interrupt, Surface};

#[derive(Clone, Copy, Debug)]
pub(super) struct Controls {
    pub horizontal: f64,
    pub vertical: f64,
    pub margin_x: f64,
    pub margin_y: f64,
    pub snap: bool,
}
pub(super) fn mapping(input: &Surface, target: &Surface, frame: Rect, controls: Controls, ctl: &Interrupt<'_>) -> Result<NativeMapping, MappingFailure> {
    let check = || ctl.check().map_err(|_| MappingFailure::resource("native alignment cancelled"));
    check()?;
    let Controls { horizontal, vertical, margin_x, margin_y, snap } = controls;
    if frame.is_empty()
        || ![horizontal, vertical, margin_x, margin_y].into_iter().all(f64::is_finite)
        || !(-2.0..=3.0).contains(&horizontal)
        || !(-2.0..=3.0).contains(&vertical)
    {
        return Err("native alignment requires a finite nonempty frame and declared finite justification/margins".into());
    }
    // Captured native surfaces have already passed shared pixel/workspace limits.
    // Reuse the authoritative native extent instead of inspecting foreign nodes.
    let target = target.content_bounds().intersect(&frame);
    check()?;
    if target.is_empty() {
        return Err(MappingFailure::resource(
            "actual native alignment target contains no nondefault content within the native frame; no target extent is invented",
        ));
    }
    let input = input.content_bounds().intersect(&frame);
    check()?;
    if input.is_empty() {
        return Ok(NativeMapping {
            notes: vec!["Actual native alignment input is empty; preserve its empty editable contents without inventing source pixels or an extent.".into()],
            ..Default::default()
        });
    }
    let mut dx =
        f64::from(target.x0) - f64::from(input.x0) + (f64::from(target.width()) - f64::from(input.width())) * horizontal + margin_x * (1. - 2. * horizontal);
    let mut dy =
        f64::from(target.y0) - f64::from(input.y0) + (f64::from(target.height()) - f64::from(input.height())) * vertical + margin_y * (1. - 2. * vertical);
    if snap {
        dx = dx.round();
        dy = dy.round();
    }
    if !dx.is_finite() || !dy.is_finite() || dx.abs() > f64::from(i32::MAX) || dy.abs() > f64::from(i32::MAX) {
        return Err(MappingFailure::resource("native alignment translation exceeds finite native coordinate placement"));
    }
    check()?;
    Ok(NativeMapping {structure:Some(NativeStructure::Composite {branches:vec![NativeBranch {transform:Affine::translate(dx,dy),..Default::default()}]}),notes:vec!["Actual prepared native input and auxiliary target content bounds determine an ordinary editable affine translation. Native justification, inward edge margins, integer snapping and fixed-canvas clipping apply; this does not infer dynamic foreign operation extents or retain a live target binding.".into()],..Default::default()})
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, PixelFormat, SampleType};
    fn controls(x: f64, y: f64) -> Controls {
        Controls { horizontal: x, vertical: y, margin_x: 0., margin_y: 0., snap: false }
    }
    fn translation(m: NativeMapping) -> [f64; 6] {
        let Some(NativeStructure::Composite { branches }) = m.structure else { panic!("ordinary placement") };
        branches[0].transform.m
    }
    #[test]
    fn actual_native_extents_justify_and_margin_each_depth_without_canvas_guessing() {
        let frame = Rect::new(-20, -20, 100, 80);
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut input = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
            input.fill_rect(Rect::new(2, 3, 12, 13), &[1., 0., 0., 1.]);
            let mut target = Surface::new(input.format());
            target.fill_rect(Rect::new(30, 20, 70, 50), &[0., 0., 1., 0.5]);
            assert_eq!(translation(mapping(&input, &target, frame, controls(0., 0.), &Interrupt::NONE).unwrap()), [1., 0., 0., 1., 28., 17.]);
            assert_eq!(translation(mapping(&input, &target, frame, controls(0.5, 0.5), &Interrupt::NONE).unwrap()), [1., 0., 0., 1., 43., 27.]);
            let c = Controls { margin_x: 4., margin_y: 2., ..controls(1., 1.) };
            assert_eq!(translation(mapping(&input, &target, frame, c, &Interrupt::NONE).unwrap()), [1., 0., 0., 1., 54., 35.]);
            let c = Controls { snap: true, ..controls(0.25, 0.125) };
            assert_eq!(translation(mapping(&input, &target, frame, c, &Interrupt::NONE).unwrap()), [1., 0., 0., 1., 36., 20.]);
        }
    }
    #[test]
    fn empty_target_and_hostile_controls_are_explicit_and_cancel_is_checked() {
        let empty = Surface::new(PixelFormat::RGBA8);
        let mut solid = empty.clone();
        solid.fill_rect(Rect::new(1, 1, 3, 3), &[1., 1., 1., 1.]);
        let frame = Rect::new(0, 0, 4, 4);
        assert!(mapping(&solid, &empty, frame, controls(0.5, 0.5), &Interrupt::NONE).unwrap_err().message.contains("target contains no"));
        assert!(mapping(&empty, &solid, frame, controls(0.5, 0.5), &Interrupt::NONE).unwrap().structure.is_none());
        assert!(mapping(&solid, &solid, frame, controls(f64::NAN, 0.), &Interrupt::NONE).is_err());
        let c = Controls { margin_x: f64::MAX, ..controls(0., 0.) };
        assert!(mapping(&solid, &solid, frame, c, &Interrupt::NONE).is_err());
        let cancel = || true;
        let progress = |_| {};
        assert!(mapping(&solid, &solid, frame, controls(0., 0.), &Interrupt::new(&cancel, &progress)).is_err());
    }
}
