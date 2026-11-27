//! Immutable native command geometry captured from the shared framework session.
//! This owns no gestures or undo state; the framework remains the edit authority.

use crate::state::FreeTransform;
use photocraft_geom::warp::Warp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Geometry {
    pub session: u64,
    pub layer: u64,
    pub rect: [f64; 4],
    pub quad: [[f64; 2]; 4],
    pub interpolation: String,
    pub warp: Option<Warp>,
    pub target: Option<Value>,
}

impl Geometry {
    pub(crate) fn of(t: &FreeTransform) -> Self {
        Self {
            session: t.id,
            layer: t.layer,
            rect: t.rect_array(),
            quad: t.quad_array(),
            interpolation: t.session.interpolation().id().into(),
            warp: t.session.warp().map(|w| w.warp.clone()),
            target: t.target.clone(),
        }
    }
}

/// Drive the shared session from a native-geometry fixture without duplicating gesture behavior.
#[cfg(test)]
pub(crate) fn edit(app: &mut crate::PhotocraftApp, change: impl FnOnce(&mut Geometry)) -> Geometry {
    use crate::state::SessionWarp;
    use craft::behavior::geometry::BoxState;
    use craft::behavior::kurbo::{Point, Rect};
    use craft::draw::ui::transform::{Interpolation, TransformEdit, TransformSession};
    let t = app.ui.transform.as_mut().unwrap();
    let mut geometry = Geometry::of(t);
    change(&mut geometry);
    if t.rect_array() != geometry.rect {
        let r = geometry.rect;
        t.session = TransformSession::new(Rect::new(r[0], r[1], r[2], r[3]));
    }
    t.edit_box(|b| *b = BoxState::new(geometry.quad.map(|[x, y]| Point::new(x, y)), b.pivot));
    let interpolation = match geometry.interpolation.as_str() {
        "nearest" => Interpolation::NearestNeighbor,
        "bilinear" => Interpolation::Bilinear,
        _ => Interpolation::Bicubic,
    };
    t.session.apply(TransformEdit::Interpolation(interpolation), 0.0);
    match &geometry.warp {
        Some(w) => {
            t.session.apply(TransformEdit::Warp(SessionWarp::new(w.clone(), false)), 0.0);
        }
        None if t.session.warp().is_some() => {
            t.session.apply(TransformEdit::LeaveWarp, 0.0);
        }
        None => {}
    }
    geometry
}
