//! Offscreen visual check of mask Paint Levels Properties and inactive material checker display.
#[cfg(not(feature = "uv-workbench"))]
fn main() {}

#[cfg(feature = "uv-workbench")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use egui::{Event, Modifiers, PointerButton, vec2};
    use egui_kittest::kittest::Queryable;
    use photocraft_engine::Session;
    use photocraft_ui_egui::{PhotocraftApp, Services, state::Tool};
    use serde_json::json;

    let mut selection_session = Session::new();
    selection_session.execute("file.new", json!({"width":64,"height":64}))?;
    selection_session.execute(
        "select.uvPolygons",
        json!({"polygons":[
            [[8.0,8.0],[24.0,8.0],[24.0,24.0]],
            [[8.0,8.0],[24.0,24.0],[8.0,24.0]]
        ]}),
    )?;
    let triangle_selection = selection_session.active().and_then(|st| st.doc.selection.as_ref()).ok_or("Adjacent UV triangles did not make a selection")?;
    if triangle_selection.sample_channel(16, 16, 0) < 0.95 || triangle_selection.sample_channel(4, 4, 0) > 0.01 {
        return Err("Adjacent UV triangles left a diagonal seam or selected outside the island".into());
    }
    selection_session.execute("select.uvPolygons", json!({"polygons":[[[-50.0,-50.0],[-40.0,-50.0],[-40.0,-40.0]]]}))?;
    if selection_session.active().and_then(|st| st.doc.selection.as_ref()).is_none_or(|selection| selection.sample_channel(16, 16, 0) != 0.0) {
        return Err("Outside-only polygon selection became unrestricted instead of empty".into());
    }
    if !selection_session.undo()
        || selection_session.active().and_then(|st| st.doc.selection.as_ref()).is_none_or(|selection| selection.sample_channel(16, 16, 0) < 0.95)
    {
        return Err("Undo did not restore the adjacent-triangle selection".into());
    }

    let mut session = Session::new();
    session.execute("uv.workspace.activate", json!({"active":true}))?;
    let temp = std::env::temp_dir().join(format!("photocraft-mask-checker-{}.obj", std::process::id()));
    let mtl = temp.with_extension("mtl");
    std::fs::write(&mtl, "newmtl Jacket\nKd 0.8 0.4 0.2\nnewmtl Head\nKd 0.2 0.4 0.8\n")?;
    std::fs::write(&temp, "mtllib photocraft-mask-checker-PLACEHOLDER.mtl\nv -2 -1 0\nv -0.2 -1 0\nv -0.2 1 0\nv -2 1 0\nv 0.2 -1 0\nv 2 -1 0\nv 2 1 0\nv 0.2 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvn 0 0 1\nusemtl Jacket\nf 1/1/1 2/2/1 3/3/1 4/4/1\nusemtl Head\nf 5/1/1 6/2/1 7/3/1 8/4/1\n".replace("photocraft-mask-checker-PLACEHOLDER.mtl", mtl.file_name().and_then(|name| name.to_str()).ok_or("Bad temporary material filename")?))?;
    let source = temp.clone();
    session.execute("uv.model.import", json!({"path":source}))?;
    let scene = session.uv.scene.as_ref().ok_or("No imported material scene")?;
    let jacket = scene.materials.iter().position(|material| material.name == "Jacket").ok_or("Imported Jacket material missing")?;
    let head = scene.materials.iter().position(|material| material.name == "Head").ok_or("Imported Head material missing")?;
    for (material, color) in [(jacket, "#b47c4c"), (head, "#4c75bd")] {
        session.execute("uv.material.select", json!({"material":material}))?;
        session.execute("uv.texture.create", json!({"width":1024,"height":1024}))?;
        session.execute("edit.fill", json!({"color":color}))?;
    }
    session.execute("uv.material.select", json!({"material":jacket}))?;
    session.execute("layer.layerMask.revealAll", json!({}))?;
    session.execute("uv.maskPaint.add", json!({"name":"Mask Levels","kind":"levels"}))?;
    session.execute("uv.maskPaint.modify", json!({"index":0,"input_black":0.12,"gamma":1.45,"input_white":0.91,"output_black":0.04,"output_white":0.96}))?;
    let layer = session.active().and_then(|st| st.active_layer).ok_or("No active mask layer")?;
    session.active().ok_or("No active texture")?;

    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 980.0)).with_pixels_per_point(1.0).with_max_steps(64).wgpu().build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(session, Services::default());
        if let Some(render) = &cc.wgpu_render_state {
            let _ = app.set_uv_wgpu(render.clone());
        }
        app.ui.workspace = "3D".into();
        app.ui.mask_target = true;
        app.ui.tool = Tool::Move;
        app.ui.uv_workbench.view_3d = true;
        app.ui.uv_workbench.split_view = false;
        app.ui.dock_tabs.properties = 0;
        app.ui.dock.heights.insert(photocraft_ui_egui::dock::Group::Properties, 470.0);
        app
    });
    h.run_steps(8);
    h.event(Event::WindowFocused(true));
    photocraft_ui_egui::panels::request_dock_width(&h.ctx, 320.0);
    h.step();

    let Some((thumbs, _)) = photocraft_ui_egui::mask_thumbs_ui::recorded(&h.ctx, layer.0) else {
        return Err("Layer mask thumbnail was not drawn".into());
    };
    let rect = thumbs.first().map(|(_, rect)| *rect).ok_or("No pixel mask thumbnail")?;
    let center = rect.center();
    h.hover_at(center);
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: center, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(2);

    let row = h.get_by_label("Mask Levels").rect();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: row.center(), button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(2);
    if h.state().session.active().and_then(photocraft_engine::mask_view_cmds::current).is_some() {
        return Err("Plain child click unexpectedly enabled grayscale mask view".into());
    }
    if !h.state().ui.mask_target || h.query_by_label("Auto").is_none() {
        return Err("Plain child click did not activate mask Paint Levels".into());
    }

    let levels_path = std::env::args().nth(1).unwrap_or_else(|| "mask-levels-properties.png".into());
    h.render()?.save(&levels_path)?;

    let Some((thumbs, _)) = photocraft_ui_egui::mask_thumbs_ui::recorded(&h.ctx, layer.0) else {
        return Err("Layer mask thumbnail disappeared".into());
    };
    let parent_mask = thumbs.first().map(|(_, rect)| rect.center()).ok_or("No pixel mask thumbnail")?;
    h.hover_at(parent_mask);
    h.step();
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.event(Event::PointerMoved(parent_mask));
    h.run_steps(2);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: parent_mask, button: PointerButton::Primary, pressed, modifiers: Modifiers::ALT });
        h.step();
    }
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(6);
    if h.state().session.active().and_then(photocraft_engine::mask_view_cmds::current).is_none() {
        return Err("Alt-click on parent mask did not display grayscale mask".into());
    }
    if h.query_by_label("Mask Levels").is_none() {
        return Err("Alt-click on parent mask did not reveal the editable stack".into());
    }
    std::thread::sleep(std::time::Duration::from_millis(120));
    h.run_steps(8);
    let active = h.state().session.active().ok_or("No active document after mask click")?;
    let color_bindings = h.state().session.uv.map_bindings(photocraft_uv::preview::Channel::Color);
    if color_bindings.get(jacket) != Some(&Some(active.doc.id)) || color_bindings.get(head).is_none_or(Option::is_none) {
        return Err("Fixture maps are not bound to the imported Jacket and Head materials".into());
    }
    let output = std::path::Path::new(&levels_path).with_file_name("mask-levels-checker.png");
    h.render()?.save(&output)?;
    println!("Mask Levels Properties: {levels_path}");
    println!("Mask view checker preview: {}", output.display());
    println!("Verified normal child activation keeps mask view off; parent Alt-click displays grayscale and expands the stack.");
    let _ = std::fs::remove_file(&source);
    let _ = std::fs::remove_file(&mtl);
    Ok(())
}
