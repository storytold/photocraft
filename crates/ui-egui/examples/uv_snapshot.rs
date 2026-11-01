//! Deterministic, windowless visual check of the optional UV extension.
#[cfg(not(feature = "uv-workbench"))]
fn main() {
    eprintln!("Build this example with --features uv-workbench");
}

#[cfg(feature = "uv-workbench")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use photocraft_engine::Session;
    use photocraft_ui_egui::{PhotocraftApp, Services};
    use serde_json::json;
    let out = std::env::args().nth(1).unwrap_or_else(|| "uv-workbench.png".into());
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/two-shells.fbx");
    let mut session = Session::new();
    session.execute("uv.model.import", json!({"path":source.to_string_lossy()}))?;
    let material = session.uv.material;
    session.execute("uv.shell.select", json!({"material":material,"shells":[0]}))?;
    session.execute("uv.mask.apply", json!({"guard":8}))?;
    let mut harness =
        egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).with_pixels_per_point(1.0).with_max_steps(64).wgpu().build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(session, Services::default());
            if let Some(rs) = &cc.wgpu_render_state
                && let Err(error) = app.set_uv_wgpu(rs.clone())
            {
                app.ui.uv_workbench.status = error;
            }
            app.ui.uv_workbench.open = false;
            app.ui.uv_workbench.select_shells = true;
            app.ui.uv_workbench.model_path = source.to_string_lossy().into_owned();
            app
        });
    harness.state_mut().sync_views();
    for view in &mut harness.state_mut().ui.views {
        view.zoom = 4.0;
        view.center = [64.0, 64.0];
        view.fit_pending = false;
    }
    for _ in 0..16 {
        harness.step();
    }
    harness.render()?.save(&out)?;
    println!("Saved {out}");
    let ctx = harness.ctx.clone();
    if !photocraft_ui_egui::menus::is_enabled(harness.state(), "window.uvWorkbench") {
        return Err("UV workbench menu is disabled".into());
    }
    photocraft_ui_egui::menus::invoke(harness.state_mut(), &ctx, "window.uvWorkbench", json!({}))?;
    for _ in 0..8 {
        harness.step();
    }
    let controls = std::path::Path::new(&out).with_file_name("uv-controls.png");
    harness.render()?.save(&controls)?;
    println!("Saved {}", controls.display());
    harness.state_mut().ui.uv_workbench.view_3d = true;
    for _ in 0..8 {
        harness.step();
    }
    let model = std::path::Path::new(&out).with_file_name("uv-main-3d.png");
    harness.render()?.save(&model)?;
    if !harness.state().uv_gpu_active() {
        return Err(format!("3D GPU fallback during snapshot: {}", harness.state().ui.uv_workbench.status).into());
    }
    println!("Saved {}", model.display());
    harness.state_mut().ui.uv_workbench.view_3d = false;
    let normal_source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../uv/tests/fixtures/normals-reflected.fbx");
    harness.state_mut().run("uv.model.import", json!({"path":normal_source}))?;
    harness.state_mut().run("uv.texture.create", json!({"channel":"baseColor","width":128,"height":128}))?;
    harness.state_mut().ui.uv_workbench.view_3d = true;
    for _ in 0..12 {
        harness.step();
    }
    let normals = std::path::Path::new(&out).with_file_name("uv-normals-3d.png");
    harness.render()?.save(&normals)?;
    println!("Saved {}", normals.display());
    harness.state_mut().ui.uv_workbench.view_3d = false;
    // Exercise the actual entry point with no texture document open as well.
    photocraft_ui_egui::menus::invoke(harness.state_mut(), &ctx, "window.uvWorkbench", json!({}))?;
    harness.state_mut().session = Session::new();
    harness.state_mut().sync_views();
    if !photocraft_ui_egui::menus::is_enabled(harness.state(), "window.uvWorkbench") {
        return Err("UV import window is unavailable without a document".into());
    }
    photocraft_ui_egui::menus::invoke(harness.state_mut(), &ctx, "window.uvWorkbench", json!({}))?;
    for _ in 0..8 {
        harness.step();
    }
    let empty = std::path::Path::new(&out).with_file_name("uv-empty-workbench.png");
    harness.render()?.save(&empty)?;
    println!("Saved {}", empty.display());
    Ok(())
}
