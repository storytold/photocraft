/// `f32::round` (half away from zero) as an integer, without the `roundf` call `f32::round`
/// makes on the baseline x86-64 target. The sum is taken in `f64`, where `v ± 0.5` is exact for
/// every `f32` the cast does not saturate (an `f32` sum is not: `0.49999997 + 0.5` rounds up to
/// `1.0`), so the result equals `v.round() as i32` for every `f32`, NaN (0) and the saturating
/// ends included.
#[inline]
pub(super) fn round_i32(v: f32) -> i32 {
    let v = f64::from(v);
    (v + 0.5f64.copysign(v)) as i32
}
