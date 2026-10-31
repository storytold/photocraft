use egui::Rect;

/// The direction in which the children of a split are placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SplitAxis {
    /// First child on the left, second child on the right.
    Horizontal,
    /// First child above the second child.
    Vertical,
}

/// A transient sizing request. Applications may adapt their own persisted sizing enum.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SplitSize {
    /// Fraction of the space left after the divider is removed, assigned to the first child.
    Ratio(f32),
    /// First child size in logical points.
    FixedFirst(f32),
    /// Second child size in logical points.
    FixedSecond(f32),
}

impl SplitSize {
    fn value(self) -> f32 {
        match self {
            Self::Ratio(v) | Self::FixedFirst(v) | Self::FixedSecond(v) => v,
        }
    }
}

/// Contained, non-overlapping rectangles of a single split, including its divider.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitLayout {
    pub first: Rect,
    pub divider: Rect,
    pub second: Rect,
    first_size: f32,
    available: f32,
}

impl SplitLayout {
    /// The first child's size along the split axis, in logical points.
    pub fn first_size(self) -> f32 {
        self.first_size
    }

    /// Space for both children, excluding the divider, in logical points.
    pub fn available(self) -> f32 {
        self.available
    }

    /// Propose a new size after moving the divider by `delta` points.
    ///
    /// The sizing mode is retained. `minimum` may differ from the minimum used for layout, e.g.
    /// a pane can have a 20-point layout minimum but a 40-point drag minimum. When both minima
    /// cannot fit, each child receives half the available space. Invalid inputs and zero available
    /// space return `None`, so the caller can leave its persisted value unchanged.
    pub fn resized(self, size: SplitSize, delta: f32, minimum: f32) -> Option<SplitSize> {
        if !size.value().is_finite() || !delta.is_finite() || !valid_nonnegative(minimum) || self.available <= 0.0 {
            return None;
        }
        let available = f64::from(self.available);
        let first = clamp_first(f64::from(self.first_size) + f64::from(delta), available, minimum);
        Some(match size {
            SplitSize::Ratio(_) => SplitSize::Ratio((first / available) as f32),
            SplitSize::FixedFirst(_) => SplitSize::FixedFirst(first as f32),
            SplitSize::FixedSecond(_) => SplitSize::FixedSecond((available - first) as f32),
        })
    }
}

/// Divide `rect` into two panes and a divider.
///
/// `gap` and `minimum` are logical points. The divider is capped to the available span; equal
/// pane minima relax to half the remaining span in tiny areas. Zero-sized rectangles are valid.
/// Finite sizing requests outside the available range are clamped. Inverted rectangles,
/// non-finite coordinates or extents, negative/non-finite gap or minimum, and non-finite sizing
/// requests return `None`. No state is stored or changed.
pub fn split_rect(rect: Rect, axis: SplitAxis, size: SplitSize, gap: f32, minimum: f32) -> Option<SplitLayout> {
    if !valid_rect(rect) || !valid_nonnegative(gap) || !valid_nonnegative(minimum) || !size.value().is_finite() {
        return None;
    }
    let total = match axis {
        SplitAxis::Horizontal => rect.width(),
        SplitAxis::Vertical => rect.height(),
    };
    let gap = gap.min(total);
    let available = total - gap;
    let space = f64::from(available);
    // Do intermediate arithmetic in f64: two finite f32 inputs can overflow when multiplied.
    let wanted = match size {
        SplitSize::Ratio(ratio) => space * f64::from(ratio),
        SplitSize::FixedFirst(points) => f64::from(points),
        SplitSize::FixedSecond(points) => space - f64::from(points),
    };
    let first_size = clamp_first(wanted, space, minimum) as f32;
    let mut first = rect;
    let mut divider = rect;
    let mut second = rect;
    match axis {
        SplitAxis::Horizontal => {
            first.max.x = (rect.min.x + first_size).min(rect.max.x);
            divider.min.x = first.max.x;
            divider.max.x = (divider.min.x + gap).min(rect.max.x);
            second.min.x = divider.max.x;
        }
        SplitAxis::Vertical => {
            first.max.y = (rect.min.y + first_size).min(rect.max.y);
            divider.min.y = first.max.y;
            divider.max.y = (divider.min.y + gap).min(rect.max.y);
            second.min.y = divider.max.y;
        }
    }
    Some(SplitLayout { first, divider, second, first_size, available })
}

pub(super) fn valid_rect(rect: Rect) -> bool {
    rect.min.is_finite()
        && rect.max.is_finite()
        && rect.width().is_finite()
        && rect.height().is_finite()
        && rect.min.x <= rect.max.x
        && rect.min.y <= rect.max.y
}

fn valid_nonnegative(value: f32) -> bool {
    value.is_finite() && value >= 0.0
}

fn clamp_first(wanted: f64, available: f64, minimum: f32) -> f64 {
    let low = f64::from(minimum).min(available * 0.5);
    wanted.clamp(low, available - low)
}
