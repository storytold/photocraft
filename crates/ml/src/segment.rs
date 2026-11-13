//! Subject matte from BiRefNet-lite.

use crate::guided::guided_filter;
use crate::resize::bilinear;
use crate::store::Model;
use crate::{Cancel, MlError, Rgb, backend, check_cancel};

/// The network's fixed input side.
pub const SEGMENT_SIZE: usize = 1024;

const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// The subject's soft matte (`w × h`, `0..=1`) of `img`: 1 on the subject, 0 on the background.
pub fn segment_subject(img: &Rgb) -> Result<Vec<f32>, MlError> {
    segment_subject_with(img, &|| false)
}

/// [`segment_subject`] that stops with [`MlError::Cancelled`] between stages once `cancel` says so.
pub fn segment_subject_with(img: &Rgb, cancel: Cancel<'_>) -> Result<Vec<f32>, MlError> {
    Rgb::check_dims(img.w, img.h)?;
    if img.px.len() != img.w * img.h * 3 {
        return Err(MlError::BadInput("the pixel buffer does not match the size".into()));
    }
    let n = SEGMENT_SIZE;
    let small = bilinear(&img.px, img.w, img.h, 3, n, n);
    // NHWC -> NCHW, ImageNet normalisation.
    let mut input = vec![0.0f32; 3 * n * n];
    for (i, px) in small.as_chunks::<3>().0.iter().enumerate() {
        for (c, v) in px.iter().enumerate() {
            if let (Some(o), Some(m), Some(s)) = (input.get_mut(c * n * n + i), MEAN.get(c), STD.get(c)) {
                *o = (v - m) / s;
            }
        }
    }
    check_cancel(cancel)?;
    let logits = backend::run_with(Model::BirefnetLite, &[(&[1, 3, n, n], input)], cancel)?;
    check_cancel(cancel)?;
    if logits.len() != n * n {
        return Err(MlError::Backend(format!("unexpected output of {} values", logits.len())));
    }
    let matte: Vec<f32> = logits.iter().map(|x| 1.0 / (1.0 + (-x).exp())).collect();
    let mut full = bilinear(&matte, n, n, 1, img.w, img.h);
    refine_edges(img, &mut full);
    Ok(full)
}

/// Pulls the matte's uncertain band onto the photo's own edges (guided filter on luminance), so
/// hair and fur get the partial coverage the 1024 px network output only hints at. Confident
/// pixels are left exactly as the network said.
fn refine_edges(img: &Rgb, matte: &mut [f32]) {
    let (w, h) = (img.w, img.h);
    // The network sees 1024 px; below that there is nothing to recover.
    if w.max(h) <= SEGMENT_SIZE / 2 {
        return;
    }
    let luma: Vec<f32> = img.px.as_chunks::<3>().0.iter().map(|[r, g, b]| 0.299 * r + 0.587 * g + 0.114 * b).collect();
    let r = (w.max(h) / 300).clamp(2, 24);
    let q = guided_filter(&luma, matte, w, h, r, 1e-3);
    for (m, g) in matte.iter_mut().zip(q) {
        // 1 at p = 0.5, fading to 0 for confident pixels.
        let band = (4.0 * *m * (1.0 - *m)).clamp(0.0, 1.0).powf(0.5);
        *m += (g - *m) * band;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_input_is_an_error() {
        assert!(matches!(segment_subject(&Rgb { w: 0, h: 0, px: vec![] }), Err(MlError::BadInput(_))));
        assert!(matches!(segment_subject(&Rgb { w: 4, h: 4, px: vec![0.0; 5] }), Err(MlError::BadInput(_))));
    }

    #[test]
    fn missing_model_is_reported() {
        // SAFETY-free: only reads the environment of this process; the folder does not exist.
        let img = Rgb::new(8, 8, vec![0.5; 8 * 8 * 3]).unwrap();
        if crate::model_path(Model::BirefnetLite).is_none() {
            assert!(matches!(segment_subject(&img), Err(MlError::ModelMissing { .. }) | Err(MlError::Unsupported)));
        }
    }

    #[test]
    fn cancelled_before_inference() {
        let img = Rgb::new(8, 8, vec![0.5; 8 * 8 * 3]).unwrap();
        assert_eq!(segment_subject_with(&img, &|| true), Err(MlError::Cancelled));
    }

    #[test]
    fn refinement_leaves_confident_pixels() {
        let (w, h) = (1100usize, 4usize);
        let img = Rgb::new(w, h, vec![0.5; w * h * 3]).unwrap();
        let mut m: Vec<f32> = (0..w * h).map(|i| if i % w < 500 { 0.0 } else { 1.0 }).collect();
        let before = m.clone();
        refine_edges(&img, &mut m);
        assert_eq!(m, before);
    }
}
