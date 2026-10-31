#[cfg(not(feature = "uv-workbench"))]
fn main() {}
#[cfg(feature = "uv-workbench")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output_dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "mask-paint-snapshots".into()));
    std::fs::create_dir_all(&output_dir)?;
    use egui_kittest::kittest::Queryable;
    use photocraft_engine::Session;
    use photocraft_ui_egui::{PhotocraftApp, Services};
    use serde_json::json;
    let mut session = Session::new();
    session.execute("uv.workspace.activate", json!({"active":true}))?;
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.obj");
    session.execute("uv.model.import", json!({"path":source}))?;
    session.execute("uv.texture.create", json!({"width":128,"height":128}))?;
    session.execute("edit.fill", json!({"color":"#649ecf"}))?;
    session.execute("layer.layerMask.revealAll", json!({}))?;
    session.execute("uv.maskPaint.add", json!({"name":"Paint 1"}))?;
    session.execute("paint.stroke", json!({"target":"mask","points":[[30,30],[60,60]],"size":20,"color":"#000000"}))?;
    session.execute("uv.maskPaint.add", json!({"name":"Paint 2"}))?;
    session.execute("uv.maskPaint.add", json!({"kind":"fill","value":0.0}))?;
    session.execute("uv.maskPaint.modify", json!({"index":2,"opacity":0.25}))?;
    session.execute("uv.maskPaint.add", json!({"kind":"levels","input_black":0.1,"gamma":1.2,"input_white":0.9,"output_black":0.05,"output_white":0.95}))?;
    let mut h =
        egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_pixels_per_point(1.0).with_max_steps(64).wgpu().build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(session, Services::default());
            if let Some(rs) = &cc.wgpu_render_state {
                let _ = app.set_uv_wgpu(rs.clone());
            }
            app.ui.workspace = "3D".into();
            app.ui.mask_target = true;
            app.ui.uv_workbench.select_shells = false;
            app.ui.uv_workbench.split_view = true;
            app
        });
    h.state_mut().sync_views();
    for _ in 0..12 {
        h.step();
    }
    if h.query_by_label("Paint 1").is_none() {
        return Err("Mask Paint layer list did not expand".into());
    }
    h.render()?.save(output_dir.join("mask-paint-expanded.png"))?;
    fn click(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, p: egui::Pos2) {
        h.hover_at(p);
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        h.run_steps(3);
    }
    let paint = h.get_by_label("Paint 1").rect().center();
    click(&mut h, paint);
    if h.query_by_label("Paint 1").is_none() {
        return Err("Paint row click collapsed mask layers".into());
    }
    let second = h.get_by_label("Paint 2").rect();
    let icon = egui::pos2(second.left() - 16.0, second.center().y);
    click(&mut h, icon);
    if h.query_by_label("Paint 1").is_none() {
        return Err("Paint icon click collapsed mask layers".into());
    }
    let row = photocraft_ui_egui::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.thumbnail.is_some()).ok_or("No layer thumbnail")?;
    if let Some(name) = row.name {
        click(&mut h, name.center());
        if h.query_by_label("Paint 1").is_none() {
            return Err("Main layer name click collapsed the Paint list".into());
        }
    }
    click(&mut h, row.thumbnail.ok_or("No thumbnail")?.center());
    if h.query_by_label("Paint 1").is_some() {
        return Err("Mask Paint layer list did not collapse".into());
    }
    h.render()?.save(output_dir.join("mask-paint-collapsed.png"))?;
    println!("Mask Paint snapshots saved.");
    Ok(())
}
