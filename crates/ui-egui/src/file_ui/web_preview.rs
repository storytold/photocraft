//! Save for Web navigation and indexed palette UI. Draft state is serialized in dialog fields.
use super::*;

/// Export dimensions, using the same percent/width/height precedence as web_document.
pub fn dimensions(doc: &photocraft_doc::Document, p: &Value) -> Result<(f64, f64), String> {
    let (w0, h0) = (f64::from(doc.size.width).max(1.0), f64::from(doc.size.height).max(1.0));
    let positive = |k: &str| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite() && *v > 0.0);
    let (w, h) = match (positive("percent"), positive("width"), positive("height")) {
        (Some(pct), _, _) => (w0 * pct / 100.0, h0 * pct / 100.0),
        (_, Some(w), Some(h)) => (w, h),
        (_, Some(w), None) => (w, h0 * w / w0),
        (_, None, Some(h)) => (w0 * h / h0, h),
        _ => (w0, h0),
    };
    if !w.is_finite() || !h.is_finite() || w > 300_000.0 || h > 300_000.0 {
        return Err("export dimensions exceed the 300,000 px per-side limit".into());
    }
    Ok((w.round().max(1.0), h.round().max(1.0)))
}

/// The method a preview proxy is resampled with: the export's own when the proxy is at full
/// size, so the preview, palette and size estimate show it; bilinear for a reduced proxy.
pub fn proxy_resample(p: &Value, scale: f64) -> &str {
    if scale >= 1.0 { p.get("resample").and_then(Value::as_str).unwrap_or("bicubic") } else { "bilinear" }
}

pub fn prepare_table(app: &PhotocraftApp, f: &mut Map<String, Value>) -> Result<(), String> {
    if !matches!(s(f, "format", "jpeg").as_str(), "gif" | "png8") {
        return Ok(());
    }
    let st = app.session.active().ok_or("no document")?;
    let mut source = params(f);
    let Some(m) = source.as_object_mut() else { return Err("invalid web settings".into()) };
    // Only palette-affecting options invalidate index-based edits. Metadata, interlacing,
    // dithering and output paths must not discard the user's replacement colours.
    m.retain(|k, _| {
        matches!(
            k.as_str(),
            "preset" | "format" | "palette" | "colors" | "transparency" | "matte" | "webSnap" | "convertToSrgb" | "width" | "height" | "percent" | "resample"
        )
    });
    let signature = format!("{}|{}|{}", st.doc.id.0, st.revision, source);
    if f.get("__tableSignature").and_then(Value::as_str) == Some(&signature) {
        return Ok(());
    }
    // A regenerated palette invalidates index-based edits. Explicit tables supplied through
    // ui.dialog.set remain usable until a palette/size setting changes.
    if f.contains_key("__tableSignature") {
        f.remove("colorTable");
        f.remove("colorTableEdits");
        f.remove("__selectedColor");
    }
    if !f.contains_key("colorTable") {
        let settings = WebSettings::from_params(&source, "preview").map_err(|e| e.to_string())?;
        // Bound palette preparation as well as the displayed image. The fixed table is then
        // passed unchanged to full-resolution export, so the swatches are the file's colours.
        let (w, h) = dimensions(&st.doc, &source)?;
        let scale = (2048.0 / w.max(h)).min(1.0);
        let mut proxy = source.clone();
        if let Some(m) = proxy.as_object_mut() {
            m.remove("percent");
            m.insert("width".into(), json!((w * scale).round().max(1.0)));
            m.insert("height".into(), json!((h * scale).round().max(1.0)));
            m.insert("resample".into(), json!(proxy_resample(&source, scale)));
        }
        let (doc, _, _) = web_cmds::web_document(&st.doc, &proxy, &settings).map_err(|e| e.to_string())?;
        f.insert("__paletteApproximate".into(), json!(scale < 1.0));
        let buf = photocraft_compose::flatten(&doc);
        let mut optimized = web_cmds::optimize(&buf.px, doc.size.width as usize, doc.bounds(), &settings, None, None, None, doc.resolution_dpi, false)
            .map_err(|e| e.to_string())?;
        // Tiny transparent features may disappear in the proxy; keep room for their index.
        if settings.transparency {
            optimized.color_table.truncate(255);
        }
        f.insert("colorTable".into(), json!(optimized.color_table.iter().map(|c| web_cmds::rgb_hex(*c)).collect::<Vec<_>>()));
        f.insert("__transparentIndex".into(), json!(optimized.transparent_index));
    }
    f.insert("__tableSignature".into(), json!(signature));
    Ok(())
}

pub fn table(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let p = params(f);
    let Ok(settings) = WebSettings::from_params(&p, "preview") else { return };
    let Some(table) = settings.color_table else { return };
    ui.add_space(8.0);
    ui.label(egui::RichText::new(tl!("Color Table…").trim_end_matches('…')).font(crate::theme::semibold(12.0)));
    ui.scope(|ui| {
        ui.spacing_mut().interact_size = egui::vec2(14.0, 14.0);
        ui.spacing_mut().item_spacing = egui::vec2(1.0, 1.0);
        egui::Grid::new("web-color-table").spacing(egui::vec2(1.0, 1.0)).show(ui, |ui| {
            for (i, source) in table.iter().enumerate() {
                let mut color = settings.color_table_edits.get(&i).copied().unwrap_or(*source);
                let response = ui.push_id(i, |ui| crate::widgets::color_edit_button_srgb(ui, &mut color)).inner;
                response.clone().on_hover_text(format!("Index {i}: {}", web_cmds::rgb_hex(color)));
                if response.clicked() {
                    f.insert("__selectedColor".into(), json!(i));
                }
                if f.get("__selectedColor").and_then(Value::as_u64) == Some(i as u64) {
                    let t = Tokens::get(ui.ctx());
                    ui.painter().rect_stroke(response.rect, t.radius_sm, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
                }
                if response.changed() {
                    let edits = f.entry("colorTableEdits").or_insert_with(|| json!({}));
                    if let Some(edits) = edits.as_object_mut() {
                        edits.insert(i.to_string(), json!(web_cmds::rgb_hex(color)));
                    }
                }
                if i % 16 == 15 {
                    ui.end_row();
                }
            }
            if f.get("__transparentIndex").and_then(Value::as_u64).is_some() {
                let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                crate::widgets::checker(ui.painter(), rect, 4.0);
                response.on_hover_text(tl!("Transparency"));
            }
        });
    });
    ui.horizontal(|ui| {
        ui.weak(format!("{}: {}", tl!("Colors"), table.len()));
        if ui.small_button(tl!("Reset")).clicked() {
            f.remove("colorTableEdits");
        }
    });
}

type EncodedPreview = (Vec<u8>, usize, Option<usize>, &'static str);

/// Palette replacements reuse the cached index buffer; picker drags do not re-quantize or
/// re-encode the proxy. The GIF payload length is unchanged by RGB table replacements.
pub fn optimized(ui: &mut egui::Ui, key: egui::Id, signature: &str, p: &Value, px: &[[f32; 4]], w: u32, h: u32) -> Result<EncodedPreview, String> {
    let settings = WebSettings::from_params(p, "preview").map_err(|e| e.to_string())?;
    let mut base = p.clone();
    if let Some(m) = base.as_object_mut() {
        m.remove("colorTableEdits");
    }
    let signature = format!("{signature}|{base}");
    let pane = key;
    let key = key.with("indexed-source");
    let cached: Option<(String, Arc<Optimized>, usize)> = ui.data(|d| d.get_temp(key));
    let (encoded, bytes) = if let Some((_, encoded, bytes)) = cached.filter(|(sig, ..)| sig == &signature) {
        (encoded, bytes)
    } else {
        let settings = WebSettings::from_params(&base, "preview").map_err(|e| e.to_string())?;
        let (encoded, bytes) = if settings.format == WebFormat::Avif {
            // The estimate shows at once; the real encode replaces it when the worker is done.
            queue_avif(ui, pane, &signature, &settings, px, w, h);
            avif_estimate(px, w, h, &settings)?
        } else {
            let encoded = web_cmds::optimize(px, w as usize, rect(w, h), &settings, None, None, None, 72.0, true).map_err(|e| e.to_string())?;
            let bytes = encoded.bytes.len();
            (encoded, bytes)
        };
        let encoded = Arc::new(encoded);
        ui.data_mut(|d| d.insert_temp(key, (signature, encoded.clone(), bytes)));
        (encoded, bytes)
    };
    let rgba = if settings.color_table_edits.is_empty() || !settings.format.indexed() {
        encoded.preview.clone()
    } else {
        encoded
            .indices
            .iter()
            .flat_map(|index| {
                let index = usize::from(*index);
                let color = settings.color_table_edits.get(&index).copied().or_else(|| encoded.color_table.get(index).copied()).unwrap_or([0; 3]);
                [color[0], color[1], color[2], if Some(index) == encoded.transparent_index { 0 } else { 255 }]
            })
            .collect()
    };
    Ok((rgba, bytes, encoded.colors, encoded.ext))
}

/// AVIF's real encode of the proxy takes seconds, so it runs on a worker while the pane shows
/// [`avif_estimate`]. One worker per pane, and it always takes the newest settings: a slider
/// dragged through ten values encodes the first and the last, not all ten.
/// An encode waiting for the worker: signature, settings, proxy pixels and size.
type AvifRequest = (String, WebSettings, Arc<Vec<[f32; 4]>>, u32, u32);

#[derive(Default)]
struct AvifWork {
    next: Option<AvifRequest>,
    busy: bool,
    /// The newest finished encode, not yet installed in the pane's cache.
    done: Option<(String, Optimized)>,
    /// Bumped whenever a finished encode replaces an estimate (part of the pane's texture key).
    generation: u64,
}

type AvifSlot = Arc<std::sync::Mutex<AvifWork>>;

fn avif_slot(ui: &egui::Ui, pane: egui::Id) -> AvifSlot {
    ui.data_mut(|d| d.get_temp_mut_or_default::<AvifSlot>(pane.with("avif-work")).clone())
}

fn queue_avif(ui: &egui::Ui, pane: egui::Id, signature: &str, settings: &WebSettings, px: &[[f32; 4]], w: u32, h: u32) {
    let slot = avif_slot(ui, pane);
    let Ok(mut work) = slot.lock() else { return };
    work.next = Some((signature.to_string(), settings.clone(), Arc::new(px.to_vec()), w, h));
    if work.busy {
        return;
    }
    work.busy = true;
    drop(work);
    let ctx = ui.ctx().clone();
    let run = move || {
        loop {
            let Some((signature, settings, px, w, h)) = slot.lock().ok().and_then(|mut work| {
                let next = work.next.take();
                work.busy = next.is_some();
                next
            }) else {
                return;
            };
            let encoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                web_cmds::optimize(&px, w as usize, rect(w, h), &settings, None, None, None, 72.0, true)
            }));
            if let (Ok(Ok(encoded)), Ok(mut work)) = (encoded, slot.lock()) {
                work.done = Some((signature, encoded));
            }
            ctx.request_repaint();
        }
    };
    // AVIF is native-only (no rav1e in the wasm build), but stay correct without threads.
    #[cfg(not(target_arch = "wasm32"))]
    if std::thread::Builder::new().name("avif-preview".into()).spawn(run).is_err()
        && let Ok(mut work) = avif_slot(ui, pane).lock()
    {
        // No thread: the pane keeps the estimate.
        work.busy = false;
        work.next = None;
    }
    #[cfg(target_arch = "wasm32")]
    run();
}

/// Installs a finished AVIF encode over the pane's estimate when its settings are still the
/// pane's, and says where the pane stands: (generation, still encoding).
pub fn avif_status(ui: &egui::Ui, pane: egui::Id) -> (u64, bool) {
    let slot = avif_slot(ui, pane);
    let Ok(mut work) = slot.lock() else { return (0, false) };
    if let Some((signature, encoded)) = work.done.take() {
        let key = pane.with("indexed-source");
        let current: Option<(String, Arc<Optimized>, usize)> = ui.data(|d| d.get_temp(key));
        if current.is_some_and(|(sig, ..)| sig == signature) {
            let bytes = encoded.bytes.len();
            ui.data_mut(|d| d.insert_temp(key, (signature, Arc::new(encoded), bytes)));
            work.generation += 1;
        }
    }
    let pending = work.busy || work.next.is_some();
    (work.generation, pending)
}

fn rect(w: u32, h: u32) -> photocraft_geom::Rect {
    photocraft_geom::Rect::new(0, 0, w as i32, h as i32)
}

/// AVIF's preview size is estimated from a mosaic of `AVIF_TILES`² tiles, `AVIF_TILE` px square,
/// taken at full resolution from an even grid over the image.
const AVIF_TILES: u32 = 4;
const AVIF_TILE: u32 = 128;

/// The mosaic [`avif_estimate`] encodes, or the image itself when it is no bigger.
fn avif_sample(px: &[[f32; 4]], w: u32, h: u32) -> (u32, u32, Vec<[f32; 4]>) {
    let side = AVIF_TILES * AVIF_TILE;
    if w <= side && h <= side {
        return (w, h, px.to_vec());
    }
    let (tw, th) = (AVIF_TILE.min(w / AVIF_TILES).max(1), AVIF_TILE.min(h / AVIF_TILES).max(1));
    let (sw, sh) = (tw * AVIF_TILES, th * AVIF_TILES);
    // Tile (i, j) starts at the centre of grid cell (i, j), less half a tile.
    let origin = |i: u32, size: u32, tile: u32| ((2 * i + 1) * size / (2 * AVIF_TILES)).saturating_sub(tile / 2).min(size - tile);
    let sample = (0..sh)
        .flat_map(|y| {
            let sy = origin(y / th, h, th) + y % th;
            (0..sw).map(move |x| px[(sy * w + origin(x / tw, w, tw) + x % tw) as usize])
        })
        .collect();
    (sw, sh, sample)
}

/// AVIF encodes at about a megapixel a second, too slow to wait for on every change of a full
/// proxy. Until the worker's real encode lands ([`queue_avif`]), the pane shows the image before
/// compression (matte included) and a size from encoding a mosaic of tiles from across it
/// ([`avif_sample`]), scaled up by area.
fn avif_estimate(px: &[[f32; 4]], w: u32, h: u32, st: &WebSettings) -> Result<(Optimized, usize), String> {
    let (sw, sh, small) = avif_sample(px, w, h);
    let estimate = web_cmds::optimize(&small, sw as usize, rect(sw, sh), st, None, None, None, 72.0, false).map_err(|e| e.to_string())?;
    let bytes = (estimate.bytes.len() as f64 * f64::from(w) * f64::from(h) / (f64::from(sw) * f64::from(sh))).round() as usize;
    // As the file has it: translucent pixels keep their alpha, or everything goes over the matte.
    let alpha = st.transparency && px.iter().any(|p| p[3] < 1.0);
    let m = st.matte.unwrap_or([1.0, 1.0, 1.0]);
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let preview = px
        .iter()
        .flat_map(|p| {
            let over = |i: usize| to8(p[i] * p[3] + m[i] * (1.0 - p[3]));
            if alpha { p.map(to8) } else { [over(0), over(1), over(2), 255] }
        })
        .collect();
    Ok((Optimized { bytes: Vec::new(), width: w, height: h, preview, ..estimate }, bytes))
}

#[derive(Clone, Copy, Debug)]
struct Navigation {
    zoom: Option<f32>,
    center: egui::Vec2,
}
impl Navigation {
    fn read(f: &Map<String, Value>) -> Self {
        let zoom = f.get("__zoom").and_then(Value::as_f64).filter(|z| z.is_finite() && *z > 0.0).map(|z| (z as f32).clamp(0.0001, 32.0));
        let center = f
            .get("__center")
            .and_then(Value::as_array)
            .and_then(|a| Some(egui::vec2(a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32)))
            .filter(|c| c.is_finite())
            .unwrap_or(egui::vec2(0.5, 0.5));
        Self { zoom, center }
    }
    fn write(self, f: &mut Map<String, Value>) {
        f.insert("__zoom".into(), json!(self.zoom));
        f.insert("__center".into(), json!([self.center.x, self.center.y]));
    }
    fn scale(self, viewport: egui::Vec2, source: egui::Vec2, ppp: f32) -> f32 {
        self.zoom.map_or((viewport.x / source.x).min(viewport.y / source.y), |z| z / ppp)
    }
    fn clamp(&mut self, viewport: egui::Vec2, image: egui::Vec2) {
        let half = (viewport / image * 0.5).min(egui::vec2(0.5, 0.5));
        self.center = self.center.clamp(half, egui::Vec2::splat(1.0) - half);
    }
    fn zoom_at(&mut self, factor: f32, at: egui::Vec2, viewport: egui::Vec2, source: egui::Vec2, ppp: f32) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let old = self.scale(viewport, source, ppp);
        self.zoom = Some((old * ppp * factor).clamp(0.0001, 32.0));
        let new = self.scale(viewport, source, ppp);
        self.center += at / source * (1.0 / old - 1.0 / new);
        self.clamp(viewport, source * new);
    }
}

pub fn controls(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let mut nav = Navigation::read(f);
    let effective = n(f, "__effectiveZoom", 1.0) as f32;
    ui.horizontal(|ui| {
        if ui.small_button(tl!("Fit")).clicked() {
            nav.zoom = None;
            nav.center = egui::vec2(0.5, 0.5);
        }
        if ui.small_button("−").clicked() {
            nav.zoom = Some((nav.zoom.unwrap_or(effective) / 1.25).max(0.0001));
        }
        if ui.small_button("+").clicked() {
            nav.zoom = Some((nav.zoom.unwrap_or(effective) * 1.25).min(32.0));
        }
        let mut percent = nav.zoom.unwrap_or(effective) * 100.0;
        if ui.add(egui::DragValue::new(&mut percent).range(0.01..=3200.0).suffix("%").speed(10.0)).accessible_name(tl!("Zoom")).changed() {
            nav.zoom = Some(percent / 100.0);
        }
        if ui.small_button(tl!("100%")).clicked() {
            nav.zoom = Some(1.0);
        }
        ui.add(egui::Label::new(egui::RichText::new(tl!("Drag to pan · Scroll to zoom · Double-click to fit")).weak()).truncate());
    });
    nav.write(f);
}

pub fn paint(ui: &mut egui::Ui, rect: egui::Rect, response: &egui::Response, tex: &egui::TextureHandle, source: egui::Vec2, f: &mut Map<String, Value>) {
    let mut nav = Navigation::read(f);
    let ppp = ui.ctx().pixels_per_point();
    if response.double_clicked() {
        nav.zoom = None;
        nav.center = egui::Vec2::splat(0.5);
    }
    let mut scale = nav.scale(rect.size(), source, ppp);
    if response.dragged() {
        nav.zoom = Some(scale * ppp);
        nav.center -= response.drag_delta() / (source * scale);
    }
    if response.hovered() {
        let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let factor = if pinch != 1.0 { pinch } else { (scroll * 0.005).exp() };
        if factor != 1.0 {
            let at = response.hover_pos().unwrap_or(rect.center()) - rect.center();
            nav.zoom_at(factor, at, rect.size(), source, ppp);
            scale = nav.scale(rect.size(), source, ppp);
            ui.ctx().request_repaint();
        }
    }
    nav.clamp(rect.size(), source * scale);
    let image = egui::Rect::from_min_size(rect.center() - nav.center * source * scale, source * scale);
    ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).image(
        tex.id(),
        image,
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
    // Fit follows resizing; explicit zoom/pan shares the same normalized centre in every pane.
    if nav.zoom.is_some() {
        f.insert("__effectiveZoom".into(), json!(nav.zoom));
    } else {
        f.insert("__effectiveZoom".into(), json!(scale * ppp));
    }
    nav.write(f);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn anchored_zoom_keeps_pixel_under_pointer_and_pan_is_bounded() {
        let mut nav = Navigation { zoom: Some(1.0), center: egui::Vec2::splat(0.5) };
        let viewport = egui::vec2(200.0, 100.0);
        let source = egui::vec2(800.0, 400.0);
        let at = egui::vec2(30.0, 20.0);
        let before = nav.center + at / source;
        nav.zoom_at(2.0, at, viewport, source, 1.0);
        let after = nav.center + at / (source * 2.0);
        assert!((before - after).length() < 0.00001);
        nav.center = egui::vec2(-5.0, 50.0);
        nav.clamp(viewport, source * 2.0);
        assert_eq!(nav.center, egui::vec2(0.0625, 0.9375));
        nav.zoom = None;
        assert_eq!(nav.scale(viewport, source, 1.0), 0.25);
        assert_eq!(nav.scale(viewport * 2.0, source, 1.0), 0.5);
    }

    #[test]
    fn palette_picker_reuses_indices_and_updates_preview_pixels() {
        let key = egui::Id::unique("test-web-preview");
        let p = json!({"format": "gif", "colorTable": ["#ff0000", "#0000ff"], "dither": "none"});
        let mut h = egui_kittest::Harness::builder().build_ui_state(
            move |ui, state: &mut (Value, Vec<u8>)| {
                state.1 = optimized(ui, key, "fixture", &state.0, &[[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]], 2, 1).unwrap().0;
            },
            (p, Vec::new()),
        );
        let cached = |ctx: &egui::Context| ctx.data(|d| d.get_temp::<(String, Arc<Optimized>, usize)>(key.with("indexed-source"))).unwrap().1;
        let before = cached(&h.ctx);
        assert_eq!(h.state().1, [255, 0, 0, 255, 0, 0, 255, 255]);
        h.state_mut().0["colorTableEdits"] = json!({"0": "#00ff00"});
        h.run_steps(3);
        assert!(Arc::ptr_eq(&before, &cached(&h.ctx)), "picker drag must not re-encode");
        assert_eq!(h.state().1, [0, 255, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn absurd_dimensions_and_navigation_fields_are_safe() {
        let doc =
            photocraft_doc::Document::new("fixture", photocraft_geom::Size::new(100, 50), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
        assert!(dimensions(&doc, &json!({"percent": 1e308})).is_err());
        assert!(dimensions(&doc, &json!({"width": 1e300})).is_err());
        let f = serde_json::from_value::<Map<String, Value>>(json!({"__zoom": 1e300, "__center": [1e300, -1e300]})).unwrap();
        let nav = Navigation::read(&f);
        assert_eq!(nav.zoom, Some(32.0));
        assert_eq!(nav.center, egui::Vec2::splat(0.5));
        let mut nav = nav;
        for factor in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
            nav.zoom_at(factor, egui::Vec2::ZERO, egui::vec2(200.0, 100.0), egui::vec2(400.0, 200.0), 1.0);
            assert_eq!(nav.zoom, Some(32.0));
            assert!(nav.center.is_finite());
        }
    }
}

#[cfg(test)]
mod avif_estimate_tests {
    use super::*;

    /// The pane first shows the source with an estimated size, then the worker's real encode:
    /// decoded, so the low-quality artefacts show, with the file's actual size.
    #[test]
    fn avif_preview_shows_the_estimate_then_the_decoded_file() {
        if !WebFormat::Avif.available() {
            return;
        }
        let (w, h) = (160u32, 120u32);
        let px: Vec<[f32; 4]> =
            (0..w * h).map(|i| [((i % w) as f32 / w as f32), (((i / w) * 7 % 255) as f32 / 255.0), (((i * 31) % 97) as f32 / 97.0), 1.0]).collect();
        let source: Vec<u8> = px.iter().flat_map(|p| p.map(|v| (v * 255.0).round() as u8)).collect();
        let ctx = egui::Context::default();
        let pane = egui::Id::unique("avif-pane");
        let p = json!({"format": "avif", "quality": 5});
        let frame = |f: &mut dyn FnMut(&mut egui::Ui)| {
            ctx.run_ui(egui::RawInput::default(), |ui| f(ui)).textures_delta.clear();
        };
        let mut first = None;
        frame(&mut |ui| first = Some((optimized(ui, pane, "t", &p, &px, w, h).unwrap(), avif_status(ui, pane))));
        let ((look, _, _, ext), (generation, pending)) = first.unwrap();
        assert_eq!((ext, generation), ("avif", 0));
        assert!(pending, "the real encode is running");
        assert_eq!(look, source, "until then, the image before compression");

        let start = std::time::Instant::now();
        let mut status = (0, true);
        while status.1 && start.elapsed() < std::time::Duration::from_secs(60) {
            std::thread::sleep(std::time::Duration::from_millis(20));
            frame(&mut |ui| status = avif_status(ui, pane));
        }
        assert_eq!(status, (1, false), "the encode landed once");
        let mut after = None;
        frame(&mut |ui| after = Some(optimized(ui, pane, "t", &p, &px, w, h).unwrap()));
        let (look, bytes, _, _) = after.unwrap();
        let mse = look.iter().zip(&source).map(|(a, b)| f64::from(a.abs_diff(*b)).powi(2)).sum::<f64>() / look.len() as f64;
        assert!(mse > 4.0, "quality 5's artefacts show: MSE {mse}");
        let st = WebSettings::from_params(&p, "t").unwrap();
        let real = web_cmds::optimize(&px, w as usize, rect(w, h), &st, None, None, None, 72.0, false).unwrap().bytes.len();
        assert_eq!(bytes, real, "the size is the file's own");
    }

    /// The mosaic takes full-resolution tiles from an even grid; small images are used whole.
    #[test]
    fn avif_sample_tiles_the_image_at_full_resolution() {
        let (w, h) = (2048u32, 1024u32);
        let px: Vec<[f32; 4]> = (0..w * h).map(|i| [(i % w) as f32, (i / w) as f32, 0.0, 1.0]).collect();
        let (sw, sh, sample) = avif_sample(&px, w, h);
        assert_eq!((sw, sh, sample.len()), (512, 512, 512 * 512));
        // Tile (0, 0) centres on grid cell (0, 0): x 256 ± 64, y 128 ± 64; neighbours stay 1:1.
        assert_eq!((sample[0][0], sample[0][1]), (192.0, 64.0));
        assert_eq!((sample[1][0], sample[512][1]), (193.0, 65.0));
        // The last tile ends inside the image.
        assert_eq!((sample[512 * 512 - 1][0], sample[512 * 512 - 1][1]), (1855.0, 959.0));
        let small: Vec<[f32; 4]> = vec![[0.5; 4]; 300 * 200];
        assert_eq!(avif_sample(&small, 300, 200).2.len(), 300 * 200);
        assert!(web_formats().iter().any(|f| f.0 == "avif") == WebFormat::Avif.available(), "AVIF is offered only when it can be written");
    }

    /// `PHOTOCRAFT_AVIF_SAMPLES=a.jpg:b.jpg cargo test … -- --ignored` prints the estimate next to
    /// a real encode of each image's 2048 px proxy.
    #[test]
    #[ignore = "measures real photos"]
    fn avif_estimate_against_real_sizes() {
        let paths = std::env::var("PHOTOCRAFT_AVIF_SAMPLES").unwrap_or_default();
        for path in paths.split(':').filter(|p| !p.is_empty()) {
            let mut s = photocraft_engine::Session::new();
            photocraft_engine::file_cmds::open_bytes_as(&mut s, "p.jpg", &std::fs::read(path).unwrap(), None, None).unwrap();
            let st = WebSettings::from_params(&json!({"format": "avif"}), "t").unwrap();
            let (wd, _, _) = web_cmds::web_document(s.active().unwrap().doc.as_ref(), &json!({"width": 2048, "resample": "bilinear"}), &st).unwrap();
            let buf = photocraft_compose::flatten(&wd);
            let (w, h) = (wd.size.width, wd.size.height);
            for q in [30, 60, 90] {
                let st = WebSettings { quality: q, ..st.clone() };
                let real = web_cmds::optimize(&buf.px, w as usize, rect(w, h), &st, None, None, None, 72.0, false).unwrap().bytes.len() as f64;
                let t = std::time::Instant::now();
                let est = avif_estimate(&buf.px, w, h, &st).unwrap().1 as f64;
                println!("{path} q{q}: real {real:.0} estimate {est:.0} ({:+.0}%) in {:.0} ms", (est / real - 1.0) * 100.0, t.elapsed().as_secs_f64() * 1000.0);
            }
        }
    }
}
