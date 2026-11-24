//! The Vertical Type tool's pointer (#12063): an I-beam lying on its side inside a dotted box, drawn
//! as rings around the hotspot, which sits in the middle of the I-beam's stem.

use egui::{Pos2, pos2};

/// Closed outline of an I-beam on its side (stem across, serifs at the ends), centred on the hotspot.
fn sideways_i_beam() -> Vec<Pos2> {
    [(-6.0, -2.5), (-5.0, -2.5), (-5.0, -0.5), (5.0, -0.5), (5.0, -2.5), (6.0, -2.5), (6.0, 2.5), (5.0, 2.5), (5.0, 0.5), (-5.0, 0.5), (-5.0, 2.5), (-6.0, 2.5)]
        .into_iter()
        .map(|(x, y)| pos2(x, y))
        .collect()
}

/// The dotted box: its top-left corner 9 points left of and 4 above the hotspot (the measured hotspot
/// is (9,4) in the reference cursor image), 18 wide and 16 tall, so the column the text runs down
/// sits below the sideways I-beam.
const BOX: [f32; 4] = [-9.0, -4.0, 9.0, 12.0];

/// A 1.5-point square centred on `c`.
fn dot(c: Pos2) -> Vec<Pos2> {
    [(-0.75, -0.75), (0.75, -0.75), (0.75, 0.75), (-0.75, 0.75)].into_iter().map(|(x, y)| pos2(c.x + x, c.y + y)).collect()
}

/// The rings of the pointer: the sideways I-beam and the dots of its box.
pub(super) fn rings() -> Vec<Vec<Pos2>> {
    let [x0, y0, x1, y1] = BOX;
    let mut rings = vec![sideways_i_beam()];
    // Four dots along each of the top and bottom edges, corners shared, spaced so none touches the serifs.
    rings.extend((0..4).map(|i| x0 + (x1 - x0) * i as f32 / 3.0).flat_map(|x| [dot(pos2(x, y0)), dot(pos2(x, y1))]));
    rings.extend((1..3).map(|i| y0 + (y1 - y0) * i as f32 / 3.0).flat_map(|y| [dot(pos2(x0, y)), dot(pos2(x1, y))]));
    rings
}
