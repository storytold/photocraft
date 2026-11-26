//! Translate shared source view evidence to one-time native editing hints.

use craft_xcf::records::{self, Budget, metadata};
use photocraft_color::Color;
use photocraft_doc::{AlphaChannel, ColorIndicates, Document, Layer, LayerId};
use photocraft_raster::Interrupt;

use crate::initial_view::{MAX_INITIAL_SAMPLERS, NativeImportTarget, NativeInitialView, path_identity};

use super::{Result, Warnings, map_error};

#[derive(Default)]
pub(super) struct SavedView {
    /// Keep the shared source ordinal so a bottom-up native traversal cannot
    /// change which of several active source layers supplies the edit target.
    selected: Vec<(usize, LayerId)>,
    active: Option<(usize, bool, bool)>,
    shown_masks: usize,
    incomplete: bool,
    channel_incomplete: bool,
    alpha_target: Option<usize>,
    extra_alpha_targets: bool,
    visible_alpha_channels: Vec<usize>,
}

impl SavedView {
    pub(super) fn layer(
        &mut self,
        properties: &records::Scan<records::Property<'_>>,
        index: usize,
        layer: &Layer,
        budget: &mut Budget,
        ctl: &Interrupt<'_>,
    ) -> Result<()> {
        if self.incomplete {
            return Ok(());
        }
        let limits = metadata::Limits::default();
        let keep_going = || !ctl.cancelled();
        let source = metadata::layer(properties, limits, budget, &keep_going).map_err(map_error)?;
        if !source.complete() {
            self.incomplete = true;
            return Ok(());
        }
        let shown = layer.mask.is_some() && source.show_mask.is_some_and(|evidence| evidence.value != 0);
        self.shown_masks += usize::from(shown);
        if source.selected.is_some() {
            if self.selected.try_reserve(1).is_err() {
                self.incomplete = true;
                return Ok(());
            }
            self.selected.push((index, layer.id));
            if self.active.is_none_or(|(active, _, _)| index < active) {
                let edit = layer.mask.is_some() && source.edit_mask.is_some_and(|evidence| evidence.value != 0);
                self.active = Some((index, edit, shown));
            }
        }
        Ok(())
    }

    pub(super) fn channel(
        &mut self,
        xcf: &craft_xcf::Xcf<'_>,
        index: usize,
        native_index: usize,
        channel: &mut AlphaChannel,
        budget: &mut Budget,
        ctl: &Interrupt<'_>,
    ) -> Result<()> {
        if self.channel_incomplete {
            return Ok(());
        }
        let limits = metadata::Limits::default();
        let keep_going = || !ctl.cancelled();
        let properties = xcf.channel_properties(index, limits.records, budget, &keep_going).map_err(map_error)?;
        let source = metadata::channel(&properties, limits, budget, &keep_going).map_err(map_error)?;
        if !source.complete() {
            self.channel_incomplete = true;
            return Ok(());
        }
        if let Some([r, g, b]) =
            source.float_color.map(|value| value.value).or_else(|| source.byte_color.map(|value| value.value.map(|component| f32::from(component) / 255.0)))
        {
            channel.color = Color::rgb(r, g, b);
        }
        channel.opacity = source.float_opacity.map_or_else(|| source.byte_opacity.map_or(1.0, |value| value.value as f32 / 255.0), |value| value.value);
        if let Some(indicates) = source.indicates {
            channel.indicates = match indicates.value {
                metadata::ChannelColorIndicates::SelectedAreas => ColorIndicates::SelectedAreas,
                metadata::ChannelColorIndicates::MaskedAreas => ColorIndicates::MaskedAreas,
            };
        }
        if source.selected.is_some() {
            if self.alpha_target.is_some() {
                self.extra_alpha_targets = true;
            } else {
                self.alpha_target = Some(native_index);
            }
        }
        if source.visible.is_some_and(|value| value.value != 0) {
            if self.visible_alpha_channels.try_reserve(1).is_err() {
                self.channel_incomplete = true;
            } else {
                self.visible_alpha_channels.push(native_index);
            }
        }
        Ok(())
    }

    pub(super) fn finish(
        mut self,
        document: &Document,
        image: &metadata::ImageMetadata<'_>,
        selected_path: Option<usize>,
        warnings: &mut Warnings,
    ) -> Option<NativeInitialView> {
        let mut view = NativeInitialView {
            selected_path,
            selected_path_identity: selected_path.and_then(|index| document.paths.get(index)).map(path_identity),
            ..Default::default()
        };
        if self.incomplete {
            warnings.note("the saved XCF layer selection could not be read completely; the default layer target was used");
        } else {
            self.selected.sort_unstable_by_key(|(index, _)| *index);
            view.selected_layers = self.selected.into_iter().map(|(_, id)| id).collect();
            if let Some((_, edit, shown)) = self.active {
                view.active_layer = view.selected_layers.first().copied();
                if edit {
                    view.target = NativeImportTarget::LayerMask;
                }
                view.show_layer_mask = shown;
            }
            if view.show_layer_mask {
                warnings.note("the saved XCF layer-mask preview is shown as grayscale outside the original layer stack");
            }
            if self.shown_masks > usize::from(view.show_layer_mask) {
                warnings.note("a saved XCF layer-mask preview on a non-active layer could not be restored");
            }
        }
        if image.complete() {
            let points = if image.has_modern_sample_points { &image.sample_points } else { &image.legacy_sample_points };
            view.samplers = points
                .iter()
                .filter_map(|point| {
                    let point = point.value;
                    let x = u32::try_from(point.x).ok()?;
                    let y = u32::try_from(point.y).ok()?;
                    (x < document.size.width && y < document.size.height).then_some([f64::from(x), f64::from(y)])
                })
                .take(MAX_INITIAL_SAMPLERS)
                .collect();
        }
        if self.channel_incomplete {
            warnings.note("the saved XCF channel view could not be read completely; the default channel overlay was used");
        } else {
            view.visible_alpha_channels = self.visible_alpha_channels;
            if let Some(index) = self.alpha_target {
                view.target = NativeImportTarget::AlphaChannel(index);
            }
            if self.extra_alpha_targets {
                warnings.note("only the first selected XCF alpha channel is used as the editing target");
            }
        }
        (view != NativeInitialView::default()).then_some(view)
    }
}

#[cfg(test)]
mod tests {
    use craft_xcf::Compression;

    use super::*;
    use crate::xcf::tests::{TestImage, TestLayer, W, hierarchy};

    #[test]
    fn source_layer_and_sampler_view_is_applied_once() {
        let mut fixture = TestImage::rgb8(4, 3);
        let mut front = TestLayer::rgba("Active", 1, 1, vec![0, 255, 0, 255]);
        front.mask = Some(vec![255]);
        front.props = vec![(2, Vec::new()), (12, 1u32.to_be_bytes().to_vec()), (13, 1u32.to_be_bytes().to_vec())];
        let mut back = TestLayer::rgba("Also selected", 1, 1, vec![255, 0, 0, 255]);
        back.props.push((2, Vec::new()));
        fixture.layers = vec![front, back];
        fixture.image_props.push((27, [0_i32, 0].into_iter().flat_map(i32::to_be_bytes).collect()));
        fixture.image_props.push((39, [3_i32, 2, 0, 0, 0, 4, 1, 0, 0, 0].into_iter().flat_map(i32::to_be_bytes).collect()));

        let imported = crate::import("saved view.xcf", &fixture.write()).unwrap();
        let doc = imported.document;
        let view = imported.initial_view.unwrap();
        assert_eq!(view.selected_layers, [doc.layers[1].id, doc.layers[0].id]);
        assert_eq!(view.active_layer, Some(doc.layers[1].id));
        assert_eq!(view.target, NativeImportTarget::LayerMask);
        assert!(view.show_layer_mask);
        assert_eq!(view.samplers, [[3.0, 2.0]]);
        view.validate(&doc).unwrap();
        let saved = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        let reopened = crate::import("saved view.pcraft", &saved).unwrap();
        assert!(reopened.initial_view.is_none());
    }

    #[test]
    fn saved_channel_target_and_overlay_use_shared_evidence() {
        let mut w = W::new(11);
        for word in [1, 1, 0, 150] {
            w.u32(word);
        }
        w.prop(17, &[0]);
        w.end();
        w.ptr(0); // No layers.
        let channel_slot = w.slot();
        w.ptr(0);
        w.patch(channel_slot, w.out.len());
        w.u32(1);
        w.u32(1);
        w.string("Channel overlay");
        w.prop(3, &[]);
        w.prop_u32(8, 1);
        w.prop(38, &[0.25_f32, 0.5, 0.75].into_iter().flat_map(f32::to_be_bytes).collect::<Vec<_>>());
        w.prop(16, &[255, 0, 0]);
        w.prop(33, &0.25_f32.to_be_bytes());
        w.prop_u32(6, 255);
        w.prop_u32(14, 0);
        w.end();
        let data_slot = w.slot();
        let pixels = hierarchy(&mut w, 1, 1, 1, &[191], Compression::None);
        w.patch(data_slot, pixels);
        let imported = crate::import("saved channel.xcf", &w.out).unwrap();
        let view = imported.initial_view.unwrap();
        let channel = &imported.document.channels[0];
        assert_eq!(view.visible_alpha_channels, [0]);
        assert_eq!(view.target, NativeImportTarget::AlphaChannel(0));
        assert_eq!(channel.color, Color::rgb(0.25, 0.5, 0.75));
        assert_eq!(channel.opacity, 0.25);
        assert_eq!(channel.indicates, ColorIndicates::SelectedAreas);
        view.validate(&imported.document).unwrap();
    }
}
