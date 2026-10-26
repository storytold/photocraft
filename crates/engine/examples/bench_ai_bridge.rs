//! Synthetic 24 MP document, 1 MP AI ROI; no provider or network is contacted.
//! cargo run --release -p photocraft-engine --example bench_ai_bridge
use std::time::Instant;

use photocraft_ai::{AiRequest, AiResult, Artifact, ImageSize, Operation, PromptPreparation, RgbaImage, WorkflowId};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent};
use photocraft_engine::{
    Session,
    ai_cmds::{Snapshot, Source},
};
use photocraft_geom::{Rect, Size, TILE_SIZE, TileCoord};
use photocraft_raster::Surface;
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new("Synthetic benchmark", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U16);
    let mut surface = Surface::new(doc.pixel_format());
    let tile = surface.solid_tile(&[0.2, 0.4, 0.6, 1.0]);
    for y in 0..((4000 + TILE_SIZE - 1) / TILE_SIZE) {
        for x in 0..((6000 + TILE_SIZE - 1) / TILE_SIZE) {
            surface.put_tiles([(TileCoord::new(x, y), tile.clone())]);
        }
    }
    doc.layers.push(Layer::new("Original", LayerContent::Raster(surface)));
    let mut selection = Surface::new(PixelFormat::GRAY8);
    selection.write_region(Rect::new(1024, 1024, 2048, 2048), &vec![1.0; 1024 * 1024]);
    doc.selection = Some(selection);
    let mut session = Session::new();
    session.add_document(doc, None);
    println!("24 MP RGB16 document; exact 1024x1024 ROI, 3 samples after warmup; no network");
    for iteration in 0..4 {
        let state = session.active().ok_or("missing benchmark document")?;
        let start = Instant::now();
        let full = photocraft_compose::render(&state.doc, state.doc.bounds());
        std::hint::black_box(&full);
        let full_ms = start.elapsed().as_secs_f64() * 1000.0;
        drop(full);
        let start = Instant::now();
        let snapshot = Snapshot::capture(state, Source::Composite, 0)?;
        let capture_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let input = snapshot.prepare(AiRequest {
            operation: Operation::Fill,
            workflow: WorkflowId { name: "bench".into(), revision: 1 },
            prompt: "a blue square".into(),
            negative_prompt: None,
            size: ImageSize { width: 1024, height: 1024 },
            variants: 1,
            seed: 0,
            source: None,
            edit_mask: None,
            references: vec![],
            prompt_preparation: PromptPreparation::Manual,
        })?;
        let prepare_ms = start.elapsed().as_secs_f64() * 1000.0;
        let result = AiResult {
            provider_job: None,
            prepared_prompt: None,
            artifacts: vec![Artifact::Image(RgbaImage::new(input.request().size, [0.1, 0.2, 0.8, 1.0].repeat(1024 * 1024))?)],
        };
        let start = Instant::now();
        let prepared = input.finish(result)?;
        let finish_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let id = session.retain_ai_result(prepared)?;
        session.execute("ai.result.apply", json!({"result":id}))?;
        let apply_ms = start.elapsed().as_secs_f64() * 1000.0;
        if iteration > 0 {
            println!(
                "full_composite={full_ms:.2}ms capture={capture_ms:.2}ms roi_prepare={prepare_ms:.2}ms finish={finish_ms:.2}ms retain_apply={apply_ms:.2}ms"
            );
        }
        if !session.undo() {
            return Err("benchmark undo failed".into());
        }
        session.execute("ai.result.discard", json!({"result":id}))?;
    }
    Ok(())
}
