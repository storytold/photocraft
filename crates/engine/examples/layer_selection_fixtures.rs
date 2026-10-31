//! Original synthetic fixtures for reviewing saved layer targeting (#10195).
//! `cargo run -p photocraft-engine --example layer_selection_fixtures -- /tmp/layer-selection`

use std::error::Error;
use std::io::{self, Write};
use std::path::PathBuf;

use photocraft_doc::{ColorMode, Document, Layer, LayerId, SampleType};
use photocraft_engine::Session;
use photocraft_geom::{Rect, Size};

enum Target {
    Single,
    Multiple,
    NestedChild,
    Group,
    Empty,
}

/// Paint one original rectangle using the document's pixel format; fail if its surface is absent.
fn raster(doc: &Document, name: &str, bounds: Rect, color: [f32; 4]) -> io::Result<Layer> {
    let mut layer = Layer::raster(name, doc.pixel_format());
    let surface = layer.surface_mut().ok_or_else(|| io::Error::other("the fixture layer has no raster surface"))?;
    surface.fill_rect(bounds, &color);
    Ok(layer)
}

/// Build five distinct colored layers, optionally adding a group with two named child targets.
fn scene(nested: bool) -> io::Result<Document> {
    let mut doc = Document::new("Saved layer selection", Size::new(320, 200), ColorMode::Rgb, SampleType::U8);
    // All pixels are original geometric artwork; no external images, fonts or profiles.
    for (name, bounds, color) in [
        ("Layer 1 - canvas", Rect::new(0, 0, 320, 200), [0.08, 0.10, 0.15, 1.0]),
        ("Layer 2 - red", Rect::new(24, 24, 176, 148), [0.85, 0.18, 0.25, 1.0]),
        ("Layer 3 - orange", Rect::new(80, 44, 246, 168), [1.0, 0.58, 0.12, 1.0]),
        ("Layer 4 - blue", Rect::new(148, 22, 294, 130), [0.15, 0.42, 0.92, 1.0]),
        ("Layer 5 - green", Rect::new(190, 114, 302, 186), [0.20, 0.78, 0.47, 1.0]),
    ] {
        let layer = raster(&doc, name, bounds, color)?;
        doc.layers.push(layer);
    }
    if nested {
        let children = vec![
            raster(&doc, "Nested child - pink", Rect::new(48, 98, 136, 182), [0.94, 0.35, 0.65, 1.0])?,
            raster(&doc, "Nested child - yellow", Rect::new(108, 118, 174, 190), [1.0, 0.90, 0.40, 1.0])?,
        ];
        doc.layers.push(Layer::group("Nested group", children));
    }
    Ok(doc)
}

/// Find a scene target by its human-readable name, including children inside the optional group.
fn layer(session: &Session, name: &str) -> io::Result<LayerId> {
    session
        .active()
        .and_then(|state| state.doc.walk().into_iter().find_map(|(_, _, layer)| (layer.name == name).then_some(layer.id)))
        .ok_or_else(|| io::Error::other(format!("fixture layer {name:?} is missing")))
}

/// Translate captured IDs for the review checklist and reject references outside the output tree.
fn name(doc: &Document, id: LayerId) -> io::Result<&str> {
    doc.layer(id).map(|layer| layer.name.as_str()).ok_or_else(|| io::Error::other("the captured selection refers to a missing layer"))
}

/// Export each targeting scenario as PSD, PSB and native files and print its expected selection.
/// An optional output directory makes the same original artwork reproducible for manual review.
fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    let mut out = io::BufWriter::new(io::stdout().lock());
    if first.as_deref().is_some_and(|arg| arg == "--help" || arg == "-h") {
        writeln!(out, "Usage: cargo run -p photocraft-engine --example layer_selection_fixtures -- [output-directory]")?;
        return Ok(());
    }
    let directory = first.map_or_else(|| std::env::temp_dir().join("photocraft-layer-selection-fixtures"), PathBuf::from);
    if args.next().is_some() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "pass at most one output directory").into());
    }
    std::fs::create_dir_all(&directory)?;
    for (slug, target) in [
        ("single-third", Target::Single),
        ("multiple-third", Target::Multiple),
        ("nested-child", Target::NestedChild),
        ("selected-group", Target::Group),
        ("deselected", Target::Empty),
    ] {
        let mut session = Session::new();
        session.add_document(scene(matches!(target, Target::NestedChild | Target::Group))?, None);
        match target {
            Target::Single => session.select_layer(layer(&session, "Layer 3 - orange")?)?,
            Target::Multiple => {
                let (one, three, five) = (layer(&session, "Layer 1 - canvas")?, layer(&session, "Layer 3 - orange")?, layer(&session, "Layer 5 - green")?);
                session.restore_layer_selection(Some(three), &[one, three, five], Some(one))?;
            }
            Target::NestedChild => session.select_layer(layer(&session, "Nested child - pink")?)?,
            Target::Group => session.select_layer(layer(&session, "Nested group")?)?,
            Target::Empty => session.restore_layer_selection(None, &[], None)?,
        }
        let snapshot = session.active().ok_or_else(|| io::Error::other("the fixture document is missing"))?.save_snapshot();
        let selection = snapshot.saved_layer_selection.as_ref().ok_or_else(|| io::Error::other("the save snapshot has no layer targeting"))?;
        let active = selection.active.map(|id| name(&snapshot, id)).transpose()?.unwrap_or("<none>");
        let selected = selection.selected.iter().map(|id| name(&snapshot, *id)).collect::<io::Result<Vec<_>>>()?;
        writeln!(out, "{slug}: expected active = {active:?}; selected = {selected:?}")?;
        for extension in ["psd", "psb", "pcraft"] {
            let filename = format!("{slug}.{extension}");
            let encoded = photocraft_io::export(&snapshot, &filename, &Default::default())?;
            let path = directory.join(filename);
            photocraft_format::atomic_write(&path, &encoded.bytes)?;
            writeln!(out, "  {}", path.display())?;
            for warning in encoded.warnings {
                writeln!(out, "  export warning: {warning}")?;
            }
        }
    }
    writeln!(out, "\nManual review checklist (not an automated Photoshop or screenshot result):")?;
    writeln!(out, "1. Open every file in PhotoCraft and compare its active/selected layers with the expectations above.")?;
    writeln!(out, "2. Open the PSD/PSB files in Photoshop; expand Nested group and inspect the same selected set and primary target.")?;
    writeln!(out, "3. Select another layer in each app, save, close and reopen; confirm the latest target returns.")?;
    writeln!(out, "4. Repeat after reordering layers; confirm names/content stay attached to the selected layers.")?;
    writeln!(out, "5. For deselected files, confirm no layer is selected despite the document having five layers.")?;
    out.flush()?;
    Ok(())
}
