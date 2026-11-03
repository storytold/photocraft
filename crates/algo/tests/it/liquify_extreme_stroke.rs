//! A project file stores Liquify strokes as plain numbers, so a stroke may name any `f64` for its
//! brush size or its points. Replaying one must never overflow, however absurd the values.

use photocraft_algo::liquify::{LiquifyField, LiquifyStroke, LiquifyTool};
use photocraft_geom::Rect;

const BRUSH_TOOLS: [LiquifyTool; 10] = [
    LiquifyTool::ForwardWarp,
    LiquifyTool::Reconstruct,
    LiquifyTool::Smooth,
    LiquifyTool::TwirlCw,
    LiquifyTool::TwirlCcw,
    LiquifyTool::Pucker,
    LiquifyTool::Bloat,
    LiquifyTool::PushLeft,
    LiquifyTool::Freeze,
    LiquifyTool::Thaw,
];

const EXTREME: [f64; 9] = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX, f64::MIN, 1e300, 3.0e9, -1.0, 0.0];

fn stroke(tool: LiquifyTool, size: f64, points: Vec<Vec<f64>>) -> LiquifyStroke {
    LiquifyStroke { tool, size, density: 50.0, pressure: 50.0, rate: 50.0, points, amount: None }
}

#[test]
fn a_brush_with_an_absurd_size_or_position_replays_without_overflow() {
    let bounds = Rect::new(0, 0, 64, 64);
    for tool in BRUSH_TOOLS {
        for size in EXTREME {
            for p in [0.0, 1e300, f64::NAN, 3.0e9, -3.0e9] {
                let points = vec![vec![p, 10.0], vec![20.0, p], vec![p, p, f64::MAX]];
                let _ = LiquifyField::from_strokes(bounds, 8.0, &[stroke(tool, size, points)]);
            }
        }
    }
}
