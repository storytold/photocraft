//! Opt-in end-to-end timing on a caller-provided, public-domain 24–36 MP image. No downloads.
//! cargo run --release -p photocraft-engine --features local-ml --example local_models_bench --
//!   /absolute/model-cache /absolute/image.png [/absolute/cutout-directory]
#[cfg(all(feature = "local-ml", not(target_arch = "wasm32")))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use photocraft_engine::Session;
    use serde_json::json;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cache = args.first().ok_or("provide a model-cache directory")?;
    let input = args.get(1).ok_or("provide a public-domain test image")?;
    let decoded = photocraft_io::import(input, &photocraft_format::read_file(std::path::Path::new(input))?)?;
    if let Some(out) = args.get(2) {
        std::fs::create_dir_all(out)?;
    }
    let mut s = Session::new();
    s.configure_local_models(cache.into());
    s.add_document(decoded.document, None);
    let bounds = s.active().ok_or("no document")?.doc.bounds();
    for (command, model) in [
        ("layer.removeBackground", "classical"),
        ("layer.removeBackground", "birefnet-hr-matting"),
        ("select.object", "classical"),
        ("select.object", "sam2.1-large"),
    ] {
        for run in 0..2 {
            let steps = s.active().ok_or("no document")?.history.past_len();
            let params =
                json!({"model":model,"rect":[bounds.width() as f32*0.1,bounds.height() as f32*0.05,bounds.width() as f32*0.85,bounds.height() as f32*0.95]});
            let start = std::time::Instant::now();
            let result = s.execute(command, params);
            println!(
                "{}",
                json!({"command":command,"model":model,"run":run,"width":bounds.width(),"height":bounds.height(),"ms":start.elapsed().as_secs_f64()*1000.0,"ok":result.is_ok()})
            );
            result?;
            if run == 0
                && let Some(out) = args.get(2)
            {
                // Turn a selection into a mask for the comparison export; undo separately below.
                if command == "select.object" {
                    s.execute("layer.layerMask.revealSelection", json!({}))?;
                }
                let d = s.active().ok_or("no document")?;
                let export = photocraft_io::export(&d.doc, "comparison.png", &Default::default())?;
                let path = std::path::Path::new(out).join(format!("{command}-{model}.png"));
                std::fs::write(path, export.bytes)?;
            }
            while s.active().ok_or("no document")?.history.past_len() > steps {
                s.execute("edit.undo", json!({}))?;
            }
        }
    }
    Ok(())
}

#[cfg(any(not(feature = "local-ml"), target_arch = "wasm32"))]
fn main() {
    eprintln!("Enable local-ml on a native target to run this opt-in benchmark.");
}
