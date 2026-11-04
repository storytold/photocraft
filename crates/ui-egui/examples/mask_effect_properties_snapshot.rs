//! Offscreen Russian UI snapshots for editable UV mask effects.
#[cfg(not(feature = "uv-workbench"))]
fn main() {}

#[cfg(feature = "uv-workbench")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use egui::accesskit::Role;
    use egui::{Event, Modifiers, PointerButton, vec2};
    use egui_kittest::kittest::{NodeT, Queryable};
    use photocraft_engine::Session;
    use photocraft_ui_egui::{PhotocraftApp, Services, state::Tool};
    use serde_json::json;

    let output_dir = std::env::args().nth(1).unwrap_or_else(|| "mask-effect-snapshots".into());
    std::fs::create_dir_all(&output_dir)?;

    let mut session = Session::new();
    session.execute("uv.workspace.activate", json!({"active":true}))?;
    session.execute("prefs.set", json!({"path":"interface.language","value":"ru"}))?;
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
    session.execute("uv.model.import", json!({"path":source}))?;
    session.execute("uv.texture.create", json!({"width":256,"height":256}))?;
    session.execute("edit.fill", json!({"color":"#649ecf"}))?;
    session.execute("layer.layerMask.revealAll", json!({}))?;
    session.execute("uv.maskPaint.add", json!({"kind":"paint","name":"Paint","blend":"screen"}))?;
    session.execute("uv.maskPaint.add", json!({"kind":"fill","name":"Fill","value":0.7,"blend":"multiply"}))?;
    session.execute("uv.maskPaint.add", json!({"kind":"blur","name":"Blur","radius":12}))?;
    let layer = session.active().and_then(|state| state.active_layer).ok_or("No active mask layer")?;

    let mut harness =
        egui_kittest::Harness::builder().with_size(vec2(1440.0, 980.0)).with_pixels_per_point(1.0).with_max_steps(64).wgpu().build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(session, Services::default());
            if let Some(render) = &cc.wgpu_render_state {
                let _ = app.set_uv_wgpu(render.clone());
            }
            app.ui.workspace = "3D".into();
            app.ui.mask_target = true;
            app.ui.tool = Tool::Move;
            app.ui.uv_workbench.view_3d = true;
            app.ui.uv_workbench.split_view = true;
            app.ui.panels.properties = true;
            app.ui.dock_tabs.properties = 0;
            app.ui.dock.heights.insert(photocraft_ui_egui::dock::Group::Properties, 440.0);
            app
        });
    harness.state_mut().sync_views();
    harness.run_steps(12);

    fn click(harness: &mut egui_kittest::Harness<'_, PhotocraftApp>, point: egui::Pos2) {
        harness.hover_at(point);
        harness.step();
        for pressed in [true, false] {
            harness.event(Event::PointerButton { pos: point, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            harness.step();
        }
        harness.run_steps(3);
    }

    let Some((initial_masks, _)) = photocraft_ui_egui::mask_thumbs_ui::recorded(&harness.ctx, layer.0) else {
        return Err("Layer mask thumbnail was not drawn".into());
    };
    let mask_thumb = initial_masks.first().map(|(_, rect)| rect.center()).ok_or("No pixel mask thumbnail")?;
    click(&mut harness, mask_thumb);

    if harness.query_by_label("Blur").is_none() {
        return Err("Mask effects did not expand in the Layers panel".into());
    }
    let blur_row = harness.get_by_label("Blur").rect().center();
    click(&mut harness, blur_row);
    if harness.query_all_by_label("Радиус").all(|node| node.accesskit_node().role() != Role::SpinButton) {
        return Err("Blur Properties did not expose Radius".into());
    }
    harness.render()?.save(std::path::Path::new(&output_dir).join("blur-properties-ru.png"))?;

    let Some((mask_thumbnails, _)) = photocraft_ui_egui::mask_thumbs_ui::recorded(&harness.ctx, layer.0) else {
        return Err("Layer mask thumbnail was not drawn".into());
    };
    let mask_point = mask_thumbnails.first().map(|(_, rect)| rect.center()).ok_or("No pixel mask thumbnail")?;
    harness.hover_at(mask_point);
    harness.step();
    for pressed in [true, false] {
        harness.event(Event::PointerButton { pos: mask_point, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
        harness.step();
    }
    harness.run_steps(3);
    if harness.query_by_label("Добавить слой размытия").is_none() {
        return Err("Russian mask context menu or Blur item did not open".into());
    }
    harness.render()?.save(std::path::Path::new(&output_dir).join("mask-add-menu-ru.png"))?;

    harness.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    harness.run_steps(2);
    let fill_row = harness.get_by_label("Fill").rect().center();
    click(&mut harness, fill_row);
    if harness.query_all_by_label("Режим наложения").all(|node| node.accesskit_node().role() != Role::ComboBox)
        || harness.query_all_by_label("Непрозрачность").all(|node| node.accesskit_node().role() != Role::Slider)
    {
        return Err("Fill Properties did not expose Blend mode and Opacity".into());
    }
    harness.render()?.save(std::path::Path::new(&output_dir).join("fill-properties-ru.png"))?;

    let paint_row = harness.get_by_label("Paint").rect().center();
    click(&mut harness, paint_row);
    if harness.query_all_by_label("Режим наложения").all(|node| node.accesskit_node().role() != Role::ComboBox) {
        return Err("Paint Properties did not expose Blend mode".into());
    }
    harness.render()?.save(std::path::Path::new(&output_dir).join("paint-properties-ru.png"))?;
    harness.query_all_by_label("Режим наложения").find(|node| node.accesskit_node().role() == Role::ComboBox).ok_or("No Russian Blend mode ComboBox")?.click();
    harness.run_steps(2);
    if harness.query_all_by_label("Цветовой тон").next().is_some()
        || harness.query_all_by_label("Насыщенность").next().is_some()
        || harness.query_all_by_label("Цветность").next().is_some()
        || harness.query_all_by_label("Яркость").next().is_some()
    {
        return Err("Color-dependent blend choices were shown for grayscale Paint".into());
    }
    harness.render()?.save(std::path::Path::new(&output_dir).join("paint-blend-menu-ru.png"))?;

    println!("Mask effect snapshots saved to {output_dir}");
    Ok(())
}
