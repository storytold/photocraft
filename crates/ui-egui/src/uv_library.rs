//! Project image shelf: immutable previews and typed drags into native editable layers.
use crate::{PhotocraftApp, theme::Tokens};
use egui::{Pos2, Rect, Sense, TextureHandle, TextureOptions, vec2};
use serde_json::json;
#[derive(Default)]
pub(crate) struct Cache {
    images: std::collections::BTreeMap<u64, (std::sync::Arc<Vec<u8>>, TextureHandle)>,
    target: Option<(Rect, u64, egui::LayerId)>,
    pub(crate) fill_targets: Vec<(Rect, u64, u64, usize)>,
    #[cfg(test)]
    layers_target: Option<Rect>,
}
#[derive(Clone)]
struct Drag {
    id: u64,
}
fn place(app: &mut PhotocraftApp, id: u64) {
    crate::uv_ui::finish_paint(app);
    if app.ui.transform.is_some() {
        crate::transform_tool::commit(app);
    }
    if let Err(error) = app.run("uv.asset.place", json!({"asset":id})) {
        app.ui.uv_workbench.status = error;
    }
}

fn fill_target_is_current(app: &PhotocraftApp, target: (Rect, u64, u64, usize)) -> bool {
    let (_, document, layer_id, index) = target;
    app.session.active().is_some_and(|st| {
        st.doc.id.0 == document
            && st.doc.layer(photocraft_doc::LayerId(layer_id)).is_some_and(|layer| {
                layer
                    .mask
                    .as_ref()
                    .and_then(|mask| mask.paint.as_ref())
                    .and_then(|stack| stack.layers.get(index))
                    .is_some_and(|effect| matches!(&effect.kind, photocraft_doc::mask_paint::EffectKind::Fill { .. }))
            })
    })
}

fn import_dialog_answer(app: &mut PhotocraftApp, answer: crate::file_dialog::FileDialogAnswer) -> Result<serde_json::Value, String> {
    use crate::file_dialog::FileDialogAnswer;

    let mut errors = Vec::new();
    match answer {
        FileDialogAnswer::Paths(paths) => {
            for path in paths {
                if let Err(error) = app.run("uv.asset.import", json!({"path":path})) {
                    errors.push(error);
                }
            }
        }
        FileDialogAnswer::Contents(name, bytes) => {
            if let Err(error) = photocraft_engine::uv_asset_cmds::import_bytes(&mut app.session, &name, bytes) {
                errors.push(format!("{name}: {error}"));
            }
        }
        FileDialogAnswer::ContentsMany(files) => {
            for (name, bytes) in files {
                if let Err(error) = photocraft_engine::uv_asset_cmds::import_bytes(&mut app.session, &name, bytes) {
                    errors.push(format!("{name}: {error}"));
                }
            }
        }
        _ => return Err("The file dialog returned an unexpected answer".into()),
    }
    if errors.is_empty() { Ok(serde_json::Value::Null) } else { Err(errors.join("\n")) }
}

pub(crate) fn layers_drop(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> bool {
    if app.ui.workspace != "3D" {
        app.uv_cache.library.fill_targets.clear();
        return false;
    }
    let rect = ui.available_rect_before_wrap().intersect(ui.clip_rect());
    let fill_targets = std::mem::take(&mut app.uv_cache.library.fill_targets);
    #[cfg(test)]
    {
        app.uv_cache.library.layers_target = Some(rect);
    }
    let pointer = ui.input(|i| i.pointer.latest_pos());
    let over_panel = pointer.is_some_and(|p| rect.contains(p) && ui.ctx().layer_id_at(p).is_none_or(|top| top == ui.layer_id()));
    let fill_row = pointer
        .filter(|_| over_panel && egui::DragAndDrop::has_payload_of_type::<Drag>(ui.ctx()))
        .and_then(|p| fill_targets.iter().rev().find(|(target, _, _, _)| target.contains(p)).copied());
    let fill_target = fill_row.filter(|target| fill_target_is_current(app, *target));
    let over = over_panel;
    if egui::DragAndDrop::has_payload_of_type::<Drag>(ui.ctx()) && over {
        let highlight = fill_target.map_or(rect, |(target, _, _, _)| target);
        ui.painter().rect_stroke(highlight, 2.0, egui::Stroke::new(1.0, Tokens::get(ui.ctx()).accent), egui::StrokeKind::Inside);
        if ui.input(|i| i.pointer.any_released())
            && let Some(payload) = egui::DragAndDrop::take_payload::<Drag>(ui.ctx())
        {
            if let Some((_, _, layer, index)) = fill_target {
                crate::uv_ui::finish_paint(app);
                if let Err(error) = app.run("uv.maskPaint.fillSource", json!({"layer":layer,"index":index,"asset":payload.id})) {
                    app.ui.uv_workbench.status = error;
                }
            } else if fill_row.is_some() {
                app.ui.uv_workbench.status = tl!("Fill target is no longer available").into();
            } else {
                place(app, payload.id);
            }
            return true;
        }
    }
    false
}
pub(crate) fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if app.ui.workspace != "3D" {
        ui.weak(tl!("Project images are available in the 3D workspace"));
        return;
    }
    if app.session.uv.scene.is_none() {
        app.uv_cache.library.target = None;
        ui.weak(tl!("Import a 3D model first"));
        return;
    }
    let bounds = ui.available_rect_before_wrap().intersect(ui.clip_rect());
    app.uv_cache.library.target = Some((bounds, ui.ctx().cumulative_frame_nr(), ui.layer_id()));
    let t = Tokens::get(ui.ctx());
    ui.weak(tl!("Drop images here, then drag them into Layers"));
    if ui.button(tl!("Add images…")).clicked() {
        let r = app.ask_file(
            crate::file_dialog::FileDialogRequest::Open {
                multiple: true,
                initial_dir: None,
                extensions: Some(["png", "jpg", "jpeg", "tif", "tiff", "tga", "exr", "bmp", "dds", "pcraft"].into_iter().map(str::to_owned).collect()),
            },
            |app, answer| import_dialog_answer(app, answer),
        );
        if let Err(e) = r {
            app.ui.uv_workbench.status = e;
        }
    }
    let assets = app.session.uv.assets.clone();
    app.uv_cache.library.images.retain(|id, _| assets.iter().any(|a| a.id == *id));
    let mut remove = None;
    for asset in assets {
        let handle =
            if let Some((_, handle)) = app.uv_cache.library.images.get(&asset.id).filter(|(bytes, _)| std::sync::Arc::ptr_eq(bytes, &asset.source.bytes)) {
                Some(handle.clone())
            } else {
                let sz = asset.document.size;
                let scale = (64.0 / sz.width.max(sz.height).max(1) as f32).min(1.0);
                let (w, h) = ((sz.width as f32 * scale).round().max(1.0) as u32, (sz.height as f32 * scale).round().max(1.0) as u32);
                let px = photocraft_compose::render_reduced(&asset.document, w, h);
                app.session.color.canvas_display(&asset.document).ok().map(|display| {
                    let rgba = display.to_rgba8(&px);
                    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba.pixels);
                    let handle = ui.ctx().load_texture(format!("uv-asset-{}", asset.id), image, TextureOptions::LINEAR);
                    app.uv_cache.library.images.insert(asset.id, (asset.source.bytes.clone(), handle.clone()));
                    handle
                })
            };
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 61.0), Sense::click_and_drag());
        ui.painter().rect_filled(rect, 3.0, t.chrome);
        if let Some(image) = handle {
            let size = image.size_vec2();
            let s = (51.0 / size.x.max(size.y)).min(1.0);
            let area = Rect::from_center_size(rect.left_center() + vec2(30.6, 0.0), size * s);
            ui.painter().image(image.id(), area, Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, crate::i18n::fmt(tl!("Image: {name}"), &[("name", &asset.source.name)])));
        response.dnd_set_drag_payload(Drag { id: asset.id });
        if response.double_clicked() && app.session.active().is_some() {
            place(app, asset.id);
        }
        response.context_menu(|ui| {
            if ui.add_enabled(app.session.active().is_some(), egui::Button::new(tl!("Add to Layers"))).clicked() {
                place(app, asset.id);
                ui.close();
            }
            let fill_target = app
                .session
                .active()
                .and_then(|st| {
                    st.active_layer.and_then(|id| st.doc.layer(id)).map(|layer| {
                        let effect = layer.mask.as_ref().and_then(|mask| mask.paint.as_ref()).and_then(|stack| {
                            stack
                                .layers
                                .get(stack.active)
                                .map(|paint| (stack.active, matches!(&paint.kind, photocraft_doc::mask_paint::EffectKind::Fill { .. })))
                        });
                        (layer.id.0, effect)
                    })
                })
                .filter(|(_, effect)| effect.is_some_and(|(_, is_fill)| is_fill));
            if let Some((layer, Some((index, _)))) = fill_target
                && ui.button(tl!("Use as Fill image")).clicked()
            {
                if let Err(error) = app.run("uv.maskPaint.fillSource", json!({"layer":layer,"index":index,"asset":asset.id})) {
                    app.ui.uv_workbench.status = error;
                }
                ui.close();
            }
            if ui.button(tl!("Remove from library")).clicked() {
                remove = Some(asset.id);
                ui.close();
            }
        });
    }
    if let Some(id) = remove
        && let Err(e) = app.run("uv.asset.remove", json!({"asset":id}))
    {
        app.ui.uv_workbench.status = e;
    }
}
pub(crate) fn file_drop(app: &mut PhotocraftApp, ctx: &egui::Context, at: Option<Pos2>, files: &mut Vec<egui::DroppedFileHandle>) {
    if app.ui.workspace != "3D" || ctx.viewport_id() != egui::ViewportId::ROOT {
        return;
    }
    let Some((rect, frame, layer)) = app.uv_cache.library.target else {
        return;
    };
    let at = at.or_else(|| ctx.input(|i| i.pointer.latest_pos()));
    if at.is_some_and(|p| ctx.layer_id_at(p).is_some_and(|top| top != layer)) {
        return;
    }
    if frame.saturating_add(1) < ctx.cumulative_frame_nr() || !at.or_else(|| ctx.input(|i| i.pointer.latest_pos())).is_some_and(|p| rect.contains(p)) {
        return;
    }
    let mut errors = Vec::new();
    for file in files.drain(..) {
        let path = file.path();
        let result = if path.is_absolute() {
            app.run("uv.asset.import", json!({"path":path}))
        } else {
            crate::read_dropped(&*file)
                .and_then(|bytes| photocraft_engine::uv_asset_cmds::import_bytes(&mut app.session, &path.to_string_lossy(), bytes).map_err(|e| e.to_string()))
        };
        if let Err(e) = result {
            errors.push(e);
        }
    }
    app.ui.uv_workbench.status = errors.join("\n");
    if !errors.is_empty() {
        app.ui.status = app.ui.uv_workbench.status.clone();
        app.ui.status_error = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    #[derive(Debug)]
    struct Drop(std::path::PathBuf);
    impl egui::DroppedFile for Drop {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            Err("uses native path".into())
        }
    }
    #[test]
    fn uv_library_os_drop_preview_and_drag_into_editable_layers() {
        let _gpu = photocraft_testkit::gpu_lock();
        let mut session = photocraft_engine::Session::new();
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.fbx");
        session.execute("uv.model.import", json!({"path":fixture})).unwrap();
        let mut app = PhotocraftApp::new(session, Default::default());
        app.ui.workspace = "3D".into();
        crate::menus::apply_workspace(&mut app);
        app.ui.dock_tabs.properties = 3;
        let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            app.set_wgpu(cc.wgpu_render_state.clone().unwrap());
            app
        });
        h.run_steps(4);
        let target = h.state().uv_cache.library.target.unwrap().0.center();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/checker.png");
        let docs = h.state().session.documents().len();
        h.hover_at(target);
        h.run_steps(1);
        h.input_mut().dropped_files.push(std::sync::Arc::new(Drop(source)));
        h.run_steps(4);
        assert_eq!(h.state().session.uv.assets.len(), 1);
        assert_eq!(h.state().session.documents().len(), docs, "shelf import does not open extra texture tabs");
        assert_eq!(h.state().uv_cache.library.images.len(), 1);
        let before = h.state().session.active().unwrap().doc.layers.len();
        let start = h.get_by_label("Image: checker.png").rect().center();
        h.hover_at(start);
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        h.event(egui::Event::PointerMoved(start + vec2(-30.0, 10.0)));
        h.run_steps(2);
        assert!(egui::DragAndDrop::has_payload_of_type::<Drag>(&h.ctx));
        let end = h.state().uv_cache.library.layers_target.unwrap().center();
        h.event(egui::Event::PointerMoved(end));
        h.run_steps(2);
        h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(4);
        let st = h.state().session.active().unwrap();
        assert_eq!(st.doc.layers.len(), before + 1);
        assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "checker");
        assert_eq!(st.selected_layers().len(), 1);
        assert!(matches!(st.doc.layer(st.active_layer.unwrap()).unwrap().content, photocraft_doc::LayerContent::Raster(_)));
        h.state_mut().run("layer.layerMask.revealAll", json!({})).unwrap();
        h.state_mut().run("uv.maskPaint.add", json!({"kind":"fill"})).unwrap();
        h.state_mut().run("uv.maskPaint.add", json!({"kind":"paint"})).unwrap();
        h.state_mut().ui.mask_target = true;
        let state = h.state().session.active().unwrap();
        assert_eq!(
            state.doc.layer(state.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().paint.as_ref().unwrap().active,
            1,
            "the Paint row is active while the Fill row remains editable"
        );
        crate::uv_mask_paint::expand_effects(&h.ctx, state.doc.id.0, state.active_layer.unwrap().0);
        h.run_steps(4);
        let fill_row = h.state().uv_cache.library.fill_targets.first().map(|(rect, _, _, _)| *rect).unwrap();
        let layers_bounds = h.state().uv_cache.library.layers_target.unwrap();
        assert!(fill_row.left() <= layers_bounds.left() + 1.0, "Fill drop target spans the row's left edge");
        assert!(fill_row.right() >= layers_bounds.right() - 1.0, "Fill drop target spans the row's right edge");
        let fill_target = egui::pos2(layers_bounds.right() - 1.0, fill_row.center().y);
        h.hover_at(start);
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        h.event(egui::Event::PointerMoved(start + vec2(-30.0, 10.0)));
        h.run_steps(2);
        assert!(egui::DragAndDrop::has_payload_of_type::<Drag>(&h.ctx));
        h.event(egui::Event::PointerMoved(fill_target));
        h.run_steps(2);
        h.event(egui::Event::PointerButton { pos: fill_target, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(4);
        let fill = h
            .state()
            .session
            .active()
            .unwrap()
            .doc
            .layer(h.state().session.active().unwrap().active_layer.unwrap())
            .unwrap()
            .mask
            .as_ref()
            .unwrap()
            .paint
            .as_ref()
            .unwrap()
            .layers[0]
            .clone();
        assert!(fill.fill_map.is_some(), "dropping on an inactive Fill row assigns the image source");
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), before + 1, "Fill drop does not place another raster layer");
        assert!(h.state().ui.transform.is_none(), "Fill drop assigns directly without starting a layer transform");
        h.state_mut().ui.dock_tabs.properties = 0;
        h.run_steps(2);
        if let Ok(path) = std::env::var("PHOTOCRAFT_UV_LIBRARY_SCREENSHOT") {
            h.render().unwrap().save(path).unwrap();
        }
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(h.state().session.active().unwrap().doc.layers.len(), before + 1);
        let restored_fill = h
            .state()
            .session
            .active()
            .unwrap()
            .doc
            .layer(h.state().session.active().unwrap().active_layer.unwrap())
            .unwrap()
            .mask
            .as_ref()
            .unwrap()
            .paint
            .as_ref()
            .unwrap()
            .layers[0]
            .clone();
        assert!(restored_fill.fill_map.is_none(), "undo removes the dropped image source");
    }

    #[test]
    fn add_images_imports_web_contents_and_multi_select_files() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/checker.png");
        let bytes = std::fs::read(fixture).unwrap();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());

        import_dialog_answer(&mut app, crate::file_dialog::FileDialogAnswer::Contents("one.png".into(), bytes.clone())).unwrap();
        import_dialog_answer(
            &mut app,
            crate::file_dialog::FileDialogAnswer::ContentsMany(vec![("two.png".into(), bytes.clone()), ("three.png".into(), bytes)]),
        )
        .unwrap();

        let assets = &app.session.uv.assets;
        assert_eq!(assets.len(), 3);
        assert_eq!(assets.iter().map(|asset| asset.source.name.as_str()).collect::<Vec<_>>(), ["one.png", "two.png", "three.png"]);
    }
}
