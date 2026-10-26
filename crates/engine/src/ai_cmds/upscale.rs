use super::*;

impl Snapshot {
    /// Validate the native destination before a server request or large allocation.
    pub fn validate_upscale_destination(&self, output: ImageSize) -> Result<()> {
        let source = self.canvas_size();
        let ratio_error = (u64::from(output.width) * u64::from(source.height)).abs_diff(u64::from(output.height) * u64::from(source.width));
        if output.width < source.width
            || output.height < source.height
            || output == source
            || output.width > i32::MAX as u32
            || output.height > i32::MAX as u32
            || ratio_error > u64::from(source.width.max(source.height))
        {
            return Err(error("AI Upscale requires a larger canvas preserving the image aspect ratio"));
        }
        scaling::check_destination(Rect::from_xywh(0, 0, output.width, output.height), self.doc.pixel_format(), false)?;
        Ok(())
    }
    /// Upscale the original native composite; only individual model patches enter
    /// SDR transport. The source is never reduced to the generation preview size.
    pub fn finish_upscale(self, output: ImageSize, mut upscale: impl FnMut(&RgbaImage) -> Result<RgbaImage>) -> Result<PreparedResult> {
        self.validate_upscale_destination(output)?;
        let source_size = self.canvas_size();
        let roi = Rect::from_xywh(0, 0, output.width, output.height);
        let format = self.doc.pixel_format();
        scaling::check_destination(roi, format, false)?;
        let profile = document_profile(&self.doc)?;
        let source = scaling::native_composite(&self.doc, self.doc.bounds())?;
        let surface =
            scaling::detail_native(format, &profile, roi, scaling::DetailSource { surface: &source, size: source_size, profile: &profile }, &mut upscale)?;
        let layer = Layer::new("AI Upscale", LayerContent::Raster(surface));
        let mut preview_doc = Document::new("AI Upscale", photocraft_geom::Size::new(output.width, output.height), self.doc.mode, self.doc.depth);
        preview_doc.icc_profile = self.doc.icc_profile.clone();
        preview_doc.layers.push(layer.clone());
        let preview = scaling::preview(&preview_doc, roi)?;
        // The retained transport artifact is the actual bounded result preview;
        // full-resolution model pixels live in the prepared native layer.
        let artifact = RgbaImage::new(preview.size, preview.rgba.iter().map(|v| f32::from(*v) / 255.0).collect()).map_err(error)?;
        let result = AiResult { provider_job: None, prepared_prompt: None, artifacts: vec![Artifact::Image(artifact)] };
        let target = Target {
            document: self.doc.id,
            revision: self.revision,
            snapshot: Arc::downgrade(&self.doc),
            original: self.doc,
            incarnation: self.incarnation,
            comparison: std::sync::Mutex::new(None),
            roi,
            operation: Operation::Upscale,
            layers: vec![layer],
            canvas: Some((output, 0, 0)),
            previews: vec![preview],
            working_size: source_size,
            detail_upscaler: Some("realesrgan-x4plus"),
        };
        if target.bytes()?.checked_add(result.bytes().map_err(error)?).is_none_or(|n| n > CACHE_BYTES) {
            return Err(error(AiError::StorageFull));
        }
        Ok(PreparedResult { result, target })
    }
}
