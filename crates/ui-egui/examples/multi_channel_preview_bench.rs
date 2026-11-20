//! 24 MP Liquify preview updates: all channels, Red, and Red+Blue.
//! RAYON_NUM_THREADS=8 cargo run --release -p photocraft-ui-egui --example multi_channel_preview_bench
use craft::ui::runtime::windows::WindowHost;
use photocraft_engine::Session;
use photocraft_geom::Rect;
use photocraft_ui_egui::control::{ControlRequest, Reply, handle};
use photocraft_ui_egui::{PhotocraftApp, Services, canvas, dialogs, shortcuts};
use serde_json::{Value, json};
use std::time::Instant;

/// Passes an injected input may take before the bench gives up.
const MAX_PASSES: usize = 64;

/// One headless pass of the app on its own context, in the order the app's frame runs them: the key route, the
/// document area, the hosted dialogs.
fn pass(app: &mut PhotocraftApp) {
    let ctx = app.ctx().clone();
    let time = ctx.input(|i| i.time) + 1.0 / 60.0;
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
    let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), ..Default::default() }, |ui| {
        shortcuts::handle(app, &ctx);
        egui::CentralPanel::default().show(ui, |ui| canvas::document_area(app, ui));
        dialogs::run(app, ui);
    });
    output.textures_delta.clear();
}

/// Runs passes until the window host has delivered everything injected. The first pass always runs: the tool host
/// takes a `ui.pointer` queue in its next canvas pass.
fn settle(app: &mut PhotocraftApp) -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..MAX_PASSES {
        pass(app);
        if WindowHost::of(app.ctx()).injected_idle() {
            return Ok(());
        }
    }
    Err(format!("the injected input was not taken in {MAX_PASSES} passes").into())
}

fn control(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Result<(), Box<dyn std::error::Error>> {
    let (request, _) = ControlRequest::new(method, params);
    match handle(app, ctx, &request) {
        Reply::Done(value) if value.get("ok") != Some(&Value::Bool(false)) => Ok(()),
        Reply::Done(value) => Err(format!("control failed: {value}").into()),
        _ => Err("expected an inline workspace response".into()),
    }
}

fn invoke(app: &mut PhotocraftApp, params: Value) -> Result<(), Box<dyn std::error::Error>> {
    let ctx = app.ctx().clone();
    control(app, &ctx, "ui.menu.invoke", params)
}

/// Sets one field of the top dialog through the dialog control surface.
fn set_field(app: &mut PhotocraftApp, field: &str, value: i64) -> Result<(), Box<dyn std::error::Error>> {
    let ctx = app.ctx().clone();
    control(app, &ctx, "ui.dialog.set", json!({"dialog": "top", "field": field, "value": value}))
}

/// Sends one pointer event in document coordinates through the control surface and runs the passes that deliver it.
fn pointer(app: &mut PhotocraftApp, kind: &str, x: f64, y: f64, pressure: f64) -> Result<(), Box<dyn std::error::Error>> {
    let ctx = app.ctx().clone();
    control(app, &ctx, "ui.pointer", json!({"events": [{"kind": kind, "x": x, "y": y, "pressure": pressure}]}))?;
    settle(app)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = Session::new();
    fixture.execute("file.new", json!({"width":6000,"height":4000,"depth":16,"background":"transparent"}))?;
    fixture.execute("layer.new.layer", json!({}))?;
    fixture.edit("Bands", |doc, id| {
        let surface = doc
            .layer_mut(id.ok_or(photocraft_engine::EngineError::NoDocument)?)
            .and_then(|l| l.surface_mut())
            .ok_or(photocraft_engine::EngineError::NoDocument)?;
        for x in (0..6000).step_by(64) {
            let value = 0.15 + (x % 640) as f32 / 1000.0;
            surface.fill_rect(Rect::new(x, 0, (x + 64).min(6000), 4000), &[value, 0.4, 1.0 - value, 0.8]);
        }
        Ok(())
    })?;
    let st = fixture.active().ok_or(photocraft_engine::EngineError::NoDocument)?;
    let layer = st.active_layer.ok_or(photocraft_engine::EngineError::NoDocument)?;
    let doc = st.doc.clone();
    let mut apps = Vec::new();
    let modes = ["all", "red", "red_blue"];
    let mut opening = Vec::new();
    for mode in modes {
        let mut session = Session::new();
        session.add_document((*doc).clone(), None);
        session.select_layer(layer)?;
        if mode != "all" {
            session.execute("channel.target", json!({"colors":if mode=="red" {vec![0]} else {vec![0,2]}}))?;
        }
        let mut app = PhotocraftApp::new(session, Services::default());
        PhotocraftApp::setup_context(app.ctx(), Default::default());
        let start = Instant::now();
        invoke(&mut app, json!({"id":"filter.liquify"}))?;
        opening.push(start.elapsed().as_secs_f64() * 1000.0);
        set_field(&mut app, "size", 120)?;
        set_field(&mut app, "density", 50)?;
        // The canvas shows once, so the pointer has a view to map through.
        settle(&mut app)?;
        pointer(&mut app, "down", 2000.0, 1500.0, 1.0)?;
        apps.push(app);
    }
    let mut samples = vec![Vec::new(); 3];
    for run in 0..7 {
        for offset in 0..3 {
            let which = (run + offset) % 3;
            let app = apps.get_mut(which).ok_or("missing benchmark app")?;
            let x = 2050.0 + run as f64 * 10.0;
            let start = Instant::now();
            pointer(app, "move", x, 1510.0, 0.8)?;
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            if run > 0
                && let Some(row) = samples.get_mut(which)
            {
                row.push(ms);
            }
        }
    }
    for app in &mut apps {
        let (request, _) = ControlRequest::new("ui.inspect", Value::Null);
        let ctx = app.ctx().clone();
        let Reply::Done(state) = handle(app, &ctx, &request) else { return Err("expected inline state".into()) };
        if state.pointer("/result/distort/liquify/maxDisplacement").and_then(Value::as_f64).is_none_or(|value| value <= 0.0) {
            return Err("the benchmark did not deform the Liquify field".into());
        }
    }
    let rows: Vec<_> = modes
        .iter()
        .zip(samples)
        .zip(opening)
        .map(|((mode, samples), open_ms)| {
            let mut sorted = samples.clone();
            sorted.sort_by(f64::total_cmp);
            json!({"target":mode,"open_ms":open_ms,"samples_ms":samples,"p50_ms":sorted.get(3),"max_ms":sorted.last()})
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"size":[6000,4000],"depth":16,"operation":"Liquify 120px live move; native proxy preview","warmups":1,"repeats":6,"rows":rows})
        )?
    );
    Ok(())
}
