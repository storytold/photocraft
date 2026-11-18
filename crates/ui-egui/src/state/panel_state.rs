use super::{Deserialize, Serialize};

/// Shared selection and capture for a point curve; document parameters remain with the host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PointCurveState {
    pub selected: Option<usize>,
    /// Captured point (None while removed outside), grab offset, and re-entry eligibility.
    pub drag: Option<PointCurveDrag>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PointCurveDrag {
    pub index: Option<usize>,
    pub offset: [f32; 2],
    pub removed: bool,
}

/// Curves editor memory (egui temp data): the edited channel and the point gesture.
#[derive(Clone, Copy, Debug, Default)]
pub struct CurvesEditorState {
    pub channel: usize,
    pub gesture: PointCurveState,
}
