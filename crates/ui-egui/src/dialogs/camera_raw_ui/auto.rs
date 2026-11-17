use std::sync::Arc;

use photocraft_algo::camera_raw::{CameraRaw, auto::Image};
use photocraft_engine::commands::raw_auto_cmds::{Panel, suggest, suggest_full};

use super::CameraRawDialog;

#[derive(Clone, Copy)]
pub(super) enum Control {
    Panel(Panel),
    Full,
}

impl Control {
    pub(super) const KEYS: [(&'static str, Self); 5] = [
        ("autoWhiteBalance", Self::Panel(Panel::WhiteBalance)),
        ("autoLight", Self::Panel(Panel::Light)),
        ("autoColor", Self::Panel(Panel::Color)),
        ("autoEffects", Self::Panel(Panel::Effects)),
        ("fullAuto", Self::Full),
    ];
}

type Previous = [Option<Arc<CameraRaw>>; 4];

#[derive(Clone, Debug, PartialEq)]
struct Full {
    params: Arc<CameraRaw>,
    panels: Previous,
    curve: Option<Vec<[f32; 2]>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct AutoState {
    before: Previous,
    full: Option<Arc<Full>>,
}

impl AutoState {
    pub(super) fn active(&self, control: Control) -> bool {
        match control {
            Control::Panel(panel) => self.before[panel.index()].is_some(),
            Control::Full => self.full.is_some(),
        }
    }

    pub(super) fn remember_panel(&mut self, panel: Panel, params: &CameraRaw) {
        self.before[panel.index()] = Some(Arc::new(params.clone()));
    }

    pub(super) fn set(
        &mut self,
        control: Control,
        enabled: bool,
        image: &Image<'_>,
        params: &mut CameraRaw,
        float: bool,
        render: impl Fn(&mut [[f32; 4]], usize, usize, &CameraRaw),
    ) {
        if self.active(control) == enabled {
            return;
        }
        match control {
            Control::Panel(panel) if enabled => {
                self.remember_panel(panel, params);
                *params = suggest(image, &[panel], params);
            }
            Control::Panel(panel) => {
                if let Some(before) = self.before[panel.index()].take() {
                    panel.copy_settings(&before, params);
                }
            }
            Control::Full if enabled => {
                let missing: Vec<_> = Panel::ALL.into_iter().filter(|p| !self.active(Control::Panel(*p))).collect();
                let before = Arc::new(params.clone());
                let panels = self.before.clone();
                for p in &missing {
                    self.before[p.index()] = Some(before.clone());
                }
                *params = suggest_full(image, &missing, params, float, render);
                let curve = (params.point_curve != before.point_curve).then(|| params.point_curve.clone());
                self.full = Some(Arc::new(Full { params: before, panels, curve }));
            }
            Control::Full => {
                if let Some(before) = self.full.take() {
                    for panel in Panel::ALL {
                        panel.copy_settings(&before.params, params);
                    }
                    if before.curve.as_ref() == Some(&params.point_curve) {
                        params.point_curve.clone_from(&before.params.point_curve);
                    }
                    self.before.clone_from(&before.panels);
                }
            }
        }
    }
}

impl CameraRawDialog {
    pub(super) fn auto_image(&self) -> Option<Image<'_>> {
        Image::new(&self.proxy, self.pw, self.ph, self.selection.as_deref().map(Vec::as_slice)).map(|image| image.with_shadow_noise(self.shadow_noise))
    }

    pub(super) fn auto_renderer(&self) -> impl Fn(&mut [[f32; 4]], usize, usize, &CameraRaw) + '_ {
        move |pixels, w, h, params| {
            let mut p = params.clone();
            p.pixel_scale = w as f32 / self.full_w.max(1) as f32;
            if let Some(curve) = &self.raw_curve {
                super::proxy::retone(pixels, w, h, curve, &mut p);
            }
            photocraft_algo::camera_raw::develop(pixels, w, h, &p, self.float);
        }
    }
}
