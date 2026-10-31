//! View-only environment resources, source-profile-aware PBR maps and native environment picker.
use crate::PhotocraftApp;
use photocraft_uv::{
    lighting::Environment,
    preview::{Channel, Image},
};
use std::sync::Arc;

pub(crate) const MAPS: [Channel; 5] = [Channel::Color, Channel::Metallic, Channel::Roughness, Channel::Ao, Channel::Emissive];
pub(crate) type Key = (u64, u64, u64, u64);
pub(crate) type RegionHint = (Key, Vec<[u32; 4]>);
#[derive(Clone)]
pub(crate) struct GpuMapRequest {
    pub doc: Arc<photocraft_doc::Document>,
    pub key: Key,
    pub linear_srgb: bool,
    /// A partial update is safe only when it was derived from this exact preceding map key.
    pub previous_key: Option<Key>,
    /// Pixels changed since `previous_key`; `None` requires a full recomposite.
    pub damage: Option<photocraft_geom::Rect>,
}
#[derive(Default)]
pub(crate) struct SurfaceMaps {
    pub images: [Vec<Option<Image>>; 5],
    pub gpu: [Vec<Option<GpuMapRequest>>; 5],
    pub keys: [Vec<Key>; 5],
    pub regions: [Vec<Option<RegionHint>>; 5],
    pub use_color_for_pbr: Vec<bool>,
    pub output: Option<(u64, Image)>,
    snapshots: [Vec<Option<Arc<photocraft_doc::Document>>>; 5],
}
pub(crate) fn gpu_color_encoding(doc: &photocraft_doc::Document, channel: Channel) -> Option<bool> {
    if !matches!(channel, Channel::Color | Channel::Emissive) || doc.mode != photocraft_color::ColorMode::Rgb {
        return None;
    }
    let profile = photocraft_engine::color_cmds::composite_profile(doc);
    let hash = profile.content_hash();
    if hash == photocraft_cms::Builtin::Srgb.profile().content_hash() {
        Some(true)
    } else if hash == photocraft_cms::Builtin::LinearSrgb.profile().content_hash() {
        Some(false)
    } else {
        None
    }
}
#[derive(Default)]
pub(crate) struct Cache {
    pub path: String,
    pending_path: String,
    pub surface: SurfaceMaps,
    pub environment: Option<Arc<Environment>>,
    pub name: String,
    preview: Option<egui::TextureHandle>,
    #[cfg(not(target_arch = "wasm32"))]
    pending: Option<std::sync::mpsc::Receiver<Result<(String, Environment), String>>>,
}
pub(crate) fn refresh(app: &mut PhotocraftApp, maps: &mut SurfaceMaps) -> Result<(), String> {
    refresh_impl(app, maps, false)
}
pub(crate) fn refresh_gpu(app: &mut PhotocraftApp, maps: &mut SurfaceMaps) -> Result<(), String> {
    refresh_impl(app, maps, true)
}
fn refresh_impl(app: &mut PhotocraftApp, maps: &mut SurfaceMaps, allow_gpu: bool) -> Result<(), String> {
    let monitor = app.session.active().and_then(|st| app.session.color.canvas_display(&st.doc).ok()).map(|d| d.monitor.clone());
    let monitor = monitor.as_deref().unwrap_or(photocraft_cms::Builtin::Srgb.profile());
    let hash = monitor.content_hash();
    if maps.output.as_ref().is_none_or(|(key, _)| *key != hash) {
        let mut lut = identity_lut();
        let transform = photocraft_cms::transform::cached(photocraft_cms::Builtin::Srgb.profile(), monitor, Default::default()).map_err(|e| e.to_string())?;
        let source = lut.rgba.as_flattened().to_vec();
        transform.convert_f32(&source, 4, lut.rgba.as_flattened_mut(), 4, true);
        maps.output = Some((hash, lut));
    }
    let opacity_bindings = app.session.uv.map_bindings(Channel::Opacity).to_vec();
    let material_count = MAPS.iter().map(|channel| app.session.uv.map_bindings(*channel).len()).max().unwrap_or(0);
    maps.use_color_for_pbr.resize(material_count, false);
    for material in 0..material_count {
        maps.use_color_for_pbr[material] = opacity_bindings.get(material).is_none_or(Option::is_none);
    }
    for (kind, channel) in MAPS.into_iter().enumerate() {
        let bindings = app.session.uv.map_bindings(channel).to_vec();
        maps.images[kind].resize_with(bindings.len(), || None);
        maps.gpu[kind].resize_with(bindings.len(), || None);
        maps.keys[kind].resize(bindings.len(), Key::default());
        maps.regions[kind].resize(bindings.len(), None);
        maps.regions[kind].fill(None);
        maps.snapshots[kind].resize(bindings.len(), None);
        for (material, id) in bindings.into_iter().enumerate() {
            let Some(index) = id.and_then(|id| app.session.documents().iter().position(|s| s.doc.id == id)) else {
                if let Some(v) = maps.gpu[kind].get_mut(material) {
                    *v = None;
                }
                if let Some(v) = maps.images[kind].get_mut(material) {
                    *v = None;
                }
                if let Some(v) = maps.keys[kind].get_mut(material) {
                    *v = Key::default();
                }
                continue;
            };
            if allow_gpu && crate::uv_mask_paint_preview::fill_manipulation_active(app, index) {
                // UV workbench refreshes the bounded mask proxy separately. Keep all source
                // material maps canonical while the Fill controls move; release refreshes them.
                continue;
            }
            let revision = app.session.documents().get(index).map_or(0, |s| s.revision);
            let gpu_candidate =
                allow_gpu.then(|| crate::uv_mask_paint_preview::gpu_display_doc(app, index).unwrap_or_else(|| crate::canvas::display_doc(app, index)));
            let gpu_candidate = gpu_candidate.and_then(|(doc, live)| gpu_color_encoding(&doc, channel).map(|encoding| (doc, live, encoding)));
            // A spatial Fill's GPU snapshot is parameter-current and does not enqueue a CPU
            // materialization. Fall back to the coalesced CPU path only when this map cannot use
            // the GPU planner.
            let (doc, live, gpu_encoding) = if let Some((doc, live, encoding)) = gpu_candidate {
                (doc, live, Some(encoding))
            } else {
                let (doc, live) = crate::canvas::display_doc(app, index);
                (doc, live, None)
            };
            let colored = matches!(channel, Channel::Color | Channel::Emissive);
            // Profile assignment/conversion is a document revision. Avoid parsing an ICC
            // profile for every unchanged slot, including numeric maps, on every refresh.
            let key = (doc.id.0, revision, live, 0);
            let gpu_request = gpu_encoding.map(|linear_srgb| {
                let map_key = (doc.id.0, revision, live, photocraft_engine::color_cmds::composite_profile(&doc).content_hash());
                let previous = maps.gpu[kind]
                    .get(material)
                    .and_then(Option::as_ref)
                    .filter(|old| old.key.0 == map_key.0 && old.key.3 == map_key.3 && old.doc.size == doc.size);
                let last_damage = app.session.documents().get(index).and_then(|st| st.last_damage);
                let damage = previous.and_then(|old| crate::canvas::damage_since(app, index, (old.key.1, old.key.2), (revision, live), 0, last_damage));
                let previous_key = damage.as_ref().and_then(|_| previous.map(|old| old.key));
                GpuMapRequest { doc: doc.clone(), key: map_key, linear_srgb, previous_key, damage }
            });
            if let Some(slot) = maps.gpu[kind].get_mut(material) {
                *slot = gpu_request.clone();
            }
            if gpu_request.is_some() {
                if let Some(slot) = maps.images[kind].get_mut(material) {
                    *slot = None;
                }
                maps.snapshots[kind][material] = None;
                if let Some(slot) = maps.keys[kind].get_mut(material) {
                    *slot = key;
                }
                continue;
            }
            if let Some(slot) = maps.gpu[kind].get_mut(material) {
                *slot = None;
            }
            if maps.keys[kind].get(material) == Some(&key) && maps.images[kind].get(material).is_some_and(Option::is_some) {
                continue;
            }
            let old = maps.keys[kind].get(material).copied().unwrap_or_default();
            let damage = if old.0 == key.0
                && maps.images[kind]
                    .get(material)
                    .and_then(Option::as_ref)
                    .is_some_and(|im| im.width == doc.size.width as usize && im.height == doc.size.height as usize)
            {
                crate::canvas::damage_since(app, index, (old.1, old.2), (revision, live), 0, app.session.documents().get(index).and_then(|st| st.last_damage))
            } else {
                None
            };
            if maps.images[kind][material].is_none() {
                maps.snapshots[kind][material] = None;
            }
            let regions = crate::uv_damage::regions(&mut maps.snapshots[kind][material], &doc, damage);
            let mut hints = Vec::new();
            for rect in regions {
                let buffer = photocraft_compose::render_reduced_damage(&doc, doc.size.width, doc.size.height, rect);
                let rect = buffer.rect;
                let rgba = if colored {
                    let profile = photocraft_engine::color_cmds::composite_profile(&doc);
                    let transform = photocraft_cms::transform::cached(&profile, photocraft_cms::Builtin::LinearSrgb.profile(), Default::default())
                        .map_err(|e| e.to_string())?;
                    let mut dst = vec![[0.0; 4]; buffer.px.len()];
                    transform.convert_f32(buffer.px.as_flattened(), 4, dst.as_flattened_mut(), 4, true);
                    dst
                } else {
                    buffer.px
                };
                if let Some(v) = maps.images[kind].get_mut(material) {
                    crate::uv_ui::patch_image(v, [doc.size.width, doc.size.height], rect, rgba)?;
                }
                hints.push(if rect.is_empty() { [0; 4] } else { [rect.x0 as u32, rect.y0 as u32, rect.width(), rect.height()] });
            }
            if old.0 == key.0
                && let Some(hint) = maps.regions[kind].get_mut(material)
            {
                *hint = Some((old, hints));
            }
            if let Some(v) = maps.keys[kind].get_mut(material) {
                *v = key;
            }
        }
    }
    Ok(())
}
pub(crate) fn identity_lut() -> Image {
    Image {
        width: 17 * 17,
        height: 17,
        rgba: (0..17 * 17 * 17).map(|i| [(i % 17) as f32 / 16.0, ((i / 17) % 17) as f32 / 16.0, (i / (17 * 17)) as f32 / 16.0, 1.0]).collect(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn load(path: &str) -> Result<(String, Environment), String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(256 * 1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > 256 * 1024 * 1024 {
        return Err(tl!("Environment map exceeds 256 MiB").into());
    }
    decode_environment(path, &bytes)
}
#[cfg(not(target_arch = "wasm32"))]
fn decode_environment(path: &str, bytes: &[u8]) -> Result<(String, Environment), String> {
    let format = photocraft_codecs::detect(bytes).ok_or(tl!("Unknown environment map format"))?;
    if !matches!(format, photocraft_codecs::Format::Hdr | photocraft_codecs::Format::OpenExr) {
        return Err(tl!("Choose a linear HDR or EXR panorama").into());
    }
    let opts = photocraft_codecs::DecodeOptions {
        limits: photocraft_codecs::Limits { max_width: 16384, max_height: 8192, max_pixels: 33_554_432, max_alloc: 512 * 1024 * 1024 },
        ..Default::default()
    };
    let decoded = photocraft_codecs::decode_with(bytes, &opts).map_err(|e| e.to_string())?;
    let rgba = decoded.to_rgba_f32().as_chunks::<4>().0.iter().map(|p| p.map(|x| if x.is_finite() { x.clamp(0.0, 65504.0) } else { 0.0 })).collect();
    let source = Image { width: decoded.width() as usize, height: decoded.height() as usize, rgba };
    let env = Environment::prepare(&source).map_err(|e| e.to_string())?;
    let name = std::path::Path::new(path).file_name().map_or_else(|| "HDR".into(), |s| s.to_string_lossy().into_owned());
    Ok((name, env))
}
pub(crate) fn poll(cache: &mut Cache, ctx: &egui::Context) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(rx) = &cache.pending {
        match rx.try_recv() {
            Ok(result) => {
                cache.pending = None;
                match result {
                    Ok((name, env)) => {
                        cache.path = std::mem::take(&mut cache.pending_path);
                        cache.name = name;
                        cache.environment = Some(Arc::new(env));
                        cache.preview = None;
                    }
                    Err(e) => return Some(e),
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                cache.pending = None;
                return Some(tl!("Environment preparation was interrupted").into());
            }
        }
    }
    let _ = (cache, ctx);
    None
}
pub(crate) fn ensure(cache: &mut Cache) -> Arc<Environment> {
    cache.environment.get_or_insert_with(|| Arc::new(Environment::studio())).clone()
}
pub(crate) fn select_environment(app: &mut PhotocraftApp, path: String) -> Result<serde_json::Value, String> {
    if path.is_empty() {
        let cache = &mut app.uv_cache.lighting;
        cache.environment = None;
        cache.preview = None;
        cache.name.clear();
        cache.path.clear();
        cache.pending_path.clear();
        #[cfg(not(target_arch = "wasm32"))]
        {
            cache.pending = None;
        }
        return Ok(serde_json::Value::Null);
    }
    if app.uv_cache.lighting.path == path && app.uv_cache.lighting.environment.is_some() {
        return Ok(serde_json::Value::Null);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let source = path.clone();
        let embedded = app
            .session
            .uv
            .environment
            .clone()
            .filter(|_| app.session.uv.project_view.get("environment").and_then(serde_json::Value::as_str) == Some(path.as_str()));
        std::thread::Builder::new()
            .name("uv-environment".into())
            .spawn(move || {
                let _ = tx.send(if let Some(resource) = embedded { decode_environment(&resource.name, &resource.bytes) } else { load(&source) });
            })
            .map_err(|e| e.to_string())?;
        app.uv_cache.lighting.pending_path = path;
        app.uv_cache.lighting.pending = Some(rx);
        Ok(serde_json::json!({"loading":true}))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (app, path);
        Err(tl!("Environment loading is available in the desktop build").into())
    }
}
pub(crate) fn panel(app: &mut PhotocraftApp, state: &mut crate::uv_ui::State, ui: &mut egui::Ui) {
    egui::CollapsingHeader::new(tl!("Environment / PBR")).default_open(true).show(ui, |ui| {
        ui.spacing_mut().slider_width = (ui.available_width() - 195.0).clamp(60.0, 140.0);
        let cache = &mut app.uv_cache.lighting;
        if let Some(e) = poll(cache, ui.ctx()) {
            state.status = e;
        }
        let env = ensure(cache);
        let settings = &mut state.lighting;
        ui.checkbox(&mut settings.pbr, tl!("PBR · Roughness / Metallic"));
        if cache.preview.is_none()
            && let Some(im) = env.specular.first()
        {
            let bytes: Vec<_> = im
                .rgba
                .iter()
                .flat_map(|p| {
                    let encode = |x: f32| (photocraft_color::convert::linear_to_srgb(x / (1.0 + x)) * 255.0).clamp(0.0, 255.0) as u8;
                    [encode(p[0]), encode(p[1]), encode(p[2]), 255]
                })
                .collect();
            cache.preview = Some(ui.ctx().load_texture(
                "uv-environment-preview",
                egui::ColorImage::from_rgba_unmultiplied([im.width, im.height], &bytes),
                egui::TextureOptions::LINEAR,
            ));
        }
        if let Some(preview) = &cache.preview {
            ui.add(egui::Image::new(preview).fit_to_exact_size(egui::vec2(ui.available_width(), ui.available_width() * 0.5)));
        }
        ui.label(if cache.name.is_empty() { tl!("Studio environment") } else { &cache.name });
        let mut choose = false;
        ui.horizontal(|ui| {
            choose = ui.button(tl!("HDR / EXR…")).clicked();
            if ui.button(tl!("Studio")).clicked() {
                cache.environment = None;
                cache.preview = None;
                cache.name.clear();
                cache.path.clear();
                cache.pending_path.clear();
                #[cfg(not(target_arch = "wasm32"))]
                {
                    cache.pending = None;
                }
            }
        });
        #[cfg(not(target_arch = "wasm32"))]
        if cache.pending.is_some() {
            ui.spinner();
            ui.label(tl!("Preparing lighting…"));
        }
        ui.add(egui::Slider::new(&mut settings.brightness, 0.0..=8.0).text(tl!("Brightness")));
        ui.add(egui::Slider::new(&mut settings.rotation, -180.0..=180.0).suffix("°").text(tl!("Rotation")));
        ui.checkbox(&mut settings.background, tl!("Show environment background"));
        if settings.background {
            ui.add(egui::Slider::new(&mut settings.background_brightness, 0.0..=4.0).text(tl!("Background brightness")));
        }
        ui.add(egui::Slider::new(&mut settings.emissive, 0.0..=8.0).text(tl!("Emissive intensity")));
        egui::CollapsingHeader::new(tl!("Material defaults")).show(ui, |ui| {
            ui.add(egui::Slider::new(&mut settings.roughness, 0.045..=1.0).text(tl!("Roughness")));
            ui.add(egui::Slider::new(&mut settings.metallic, 0.0..=1.0).text(tl!("Metallic")));
        });
        if choose {
            use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
            if let Err(e) = app.ask_file(
                FileDialogRequest::Open { multiple: false, initial_dir: None, extensions: Some(vec!["hdr".into(), "exr".into()]) },
                |app, answer| {
                    let FileDialogAnswer::Paths(paths) = answer else { return Err(tl!("Choose a local HDR / EXR map").into()) };
                    let path = paths.first().ok_or(tl!("No file selected"))?.clone();
                    select_environment(app, path)
                },
            ) {
                state.status = e;
            }
        }
    });
}

pub(crate) fn pending(cache: &Cache) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        cache.pending.is_some()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = cache;
        false
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    #[test]
    fn hdr_environment_loading_keeps_radiance_and_rejects_other_files() {
        let _gpu = photocraft_testkit::gpu_lock();
        let image = photocraft_codecs::Image::from_f32(4, 2, photocraft_codecs::ChannelLayout::Rgb, &[4.0, 2.0, 0.5].repeat(8)).unwrap();
        let bytes = photocraft_codecs::encode(&image, photocraft_codecs::Format::Hdr, &Default::default()).unwrap();
        let path = std::env::temp_dir().join(format!("uv-environment-{}.hdr", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let (_, env) = load(path.to_str().unwrap()).unwrap();
        assert!((env.specular[0].rgba[0][0] - 4.0).abs() < 0.01);
        std::fs::write(&path, b"invalid environment").unwrap();
        assert!(load(path.to_str().unwrap()).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
