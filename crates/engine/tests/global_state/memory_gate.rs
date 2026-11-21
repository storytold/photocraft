//! On the web build a failed allocation aborts the module and freezes the page. The big
//! operations therefore ask the allocator first (`photocraft_raster::headroom`) and refuse with
//! an error when their estimated need won't fit (#10024). The simulation switch is process-wide,
//! so every test here holds `LOCK` and resets it when it ends.

use std::sync::MutexGuard;

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_engine::Session;
use photocraft_raster::headroom;
use serde_json::json;

/// Holds the lock and the simulated room; resets the simulation when dropped. It also sets the
/// transform budget to the default Memory Usage, since other tests in this binary lower it through
/// the preferences and do not restore it.
struct Room(#[allow(dead_code)] MutexGuard<'static, ()>);

impl Room {
    fn new(bytes: Option<u64>) -> Room {
        let guard = Room(crate::lock(&crate::PROCESS));
        photocraft_engine::transform_cmds::set_budget(u64::from(photocraft_engine::prefs::DEFAULT_MEMORY_USAGE_MB) << 20);
        headroom::simulate_available(bytes);
        guard
    }
    fn set(&self, bytes: Option<u64>) {
        headroom::simulate_available(bytes);
    }
}

impl Drop for Room {
    fn drop(&mut self) {
        headroom::simulate_available(None);
    }
}

const MIB: u64 = 1 << 20;

/// Enough for the reserve and a small operation, far too little for a big one.
const SMALL: u64 = headroom::RESERVE + 4 * MIB;

fn png(dir: &std::path::Path, name: &str, w: u32, h: u32, color: &str) -> String {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": color})).unwrap();
    let st = s.active().unwrap();
    let bytes = photocraft_io::export(&st.doc, "png", &Default::default()).unwrap().bytes;
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

/// What an operation must not change when the gate refuses it.
fn state(s: &Session) -> (u64, usize, usize, (u32, u32)) {
    let st = s.active().unwrap();
    (st.revision, st.history.past_len(), st.doc.layers.len(), (st.doc.size.width, st.doc.size.height))
}

fn new_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
    s
}

#[test]
fn place_that_does_not_fit_fails_and_leaves_the_document_alone() {
    let room = Room::new(None);
    let dir = tempfile::tempdir().unwrap();
    // 2400 x 1800 px: about 4 MP, about 190 MB by the estimate.
    let path = png(dir.path(), "big \u{e9}.png", 2400, 1800, "#ff0000");
    let mut s = new_doc();
    let before = state(&s);
    room.set(Some(SMALL));
    for cmd in ["file.placeEmbedded", "file.placeLinked"] {
        let err = s.execute(cmd, json!({"path": path})).unwrap_err().to_string();
        assert!(err.contains("Not enough memory to place \u{201c}big \u{e9}.png\u{201d}"), "{err}");
        assert!(err.contains("MB"), "names the need: {err}");
        assert!(err.contains("Edit \u{203a} Purge \u{203a} All"), "says what to do: {err}");
        assert_eq!(state(&s), before, "{cmd}");
    }
    room.set(None);
    s.execute("file.placeEmbedded", json!({"path": path})).unwrap();
    assert_eq!(state(&s).2, before.2 + 1);
}

#[test]
fn image_size_that_does_not_fit_fails_and_leaves_the_document_alone() {
    let room = Room::new(None);
    let mut s = new_doc();
    let before = state(&s);
    room.set(Some(SMALL));
    let err = s.execute("image.imageSize", json!({"width": 12000, "height": 9000})).unwrap_err().to_string();
    assert!(err.contains("Not enough memory to resize the image to 12000 x 9000 px"), "{err}");
    assert_eq!(state(&s), before);
    // Only changing the resolution allocates nothing, so it still goes through.
    s.execute("image.imageSize", json!({"resolution": 150, "resample": "none"})).unwrap();
    room.set(None);
    s.execute("image.imageSize", json!({"width": 800, "height": 600})).unwrap();
    assert_eq!(state(&s).3, (800, 600));
}

/// A smart-object re-render that doesn't fit is refused by name (its own error); the commands
/// that re-render after an edit keep the layer's last appearance instead of failing.
#[test]
fn smart_object_render_that_does_not_fit_is_refused_by_name() {
    let room = Room::new(None);
    let dir = tempfile::tempdir().unwrap();
    let path = png(dir.path(), "src.png", 1500, 1500, "#12ab34");
    let bytes = std::fs::read(&path).unwrap();
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    room.set(Some(SMALL));
    let err = photocraft_engine::smart_cmds::source_image("src.png", &bytes, fmt).unwrap_err().to_string();
    assert!(err.contains("Not enough memory to render the smart object \u{201c}src.png\u{201d}"), "{err}");
    room.set(None);
    assert!(photocraft_engine::smart_cmds::source_image("src.png", &bytes, fmt).is_ok());
}

/// A VectorCraft artboard placed at a large scale is drawn at that scale. The place check counts
/// the artboard at the size it opens at, so the scaled drawing asks for its own room first.
#[test]
fn a_vectorcraft_artboard_drawn_at_a_large_scale_is_refused_when_it_does_not_fit() {
    let room = Room::new(None);
    let content = "0 0 1 rg 0 0 100 200 re f";
    let pdf = format!(
        "%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>\nendobj\n4 0 obj\n<< /Length {} >>\nstream\n{content}\nendstream\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n",
        content.len()
    );
    let mut bytes = format!(
        r#"{{"format":"vectorcraft","version":4,"generator":"VectorCraft 0.1.0","pdf":{{"mime":"application/pdf","blob":[0,{}]}},"document":{{"artboards":[],"layers":[]}}}}"#,
        pdf.len()
    )
    .into_bytes();
    bytes.extend_from_slice(b"\n\0VCBLOBS\0");
    bytes.extend_from_slice(pdf.as_bytes());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Logo.vectorcraft");
    std::fs::write(&path, &bytes).unwrap();
    let mut s = new_doc();
    let before = state(&s);
    // 200 x 200 points fit at 100%; drawn at 800% the artboard is 1600 x 1600 px.
    room.set(Some(SMALL));
    s.execute("file.placeEmbedded", json!({"path": path})).unwrap();
    assert_eq!(state(&s).2, before.2 + 1);
    assert!(s.undo());
    let err = s.execute("file.placeEmbedded", json!({"path": path, "scale": 800})).unwrap_err().to_string();
    assert!(err.contains("Not enough memory to draw the VectorCraft smart object \u{201c}Logo.vectorcraft\u{201d}"), "{err}");
    assert_eq!(state(&s).2, before.2);
    room.set(None);
    s.execute("file.placeEmbedded", json!({"path": path, "scale": 800})).unwrap();
    assert_eq!(state(&s).2, before.2 + 1);
}

#[test]
fn layered_psd_is_refused_before_decode_without_changing_the_document() {
    use craft_psd::{Compression, LayerSpec, PixelData, PsdBuilder};

    let room = Room::new(None);
    const SIDE: u32 = 256;
    const LAYERS: usize = 24;
    let pixels = u64::from(SIDE) * u64::from(SIDE);
    let mut builder = PsdBuilder::new(SIDE, SIDE).compression(Compression::Rle);
    for layer in 0..LAYERS {
        // Nonuniform, opaque pixels populate each full-canvas surface while remaining compressible.
        let rgba: Vec<u8> = (0..SIDE * SIDE).flat_map(|i| [((i / SIDE + layer as u32) % 256) as u8, 80, 160, 255]).collect();
        builder.push_layer(LayerSpec::new(format!("layer {layer}"), 0, 0, SIDE, SIDE, PixelData::Rgba8(rgba)));
    }
    let bytes = builder.to_bytes().unwrap();
    let budget = pixels * photocraft_engine::memory_gate::SMART_RENDER_BYTES_PER_PX + bytes.len() as u64;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layers.psd");
    std::fs::write(&path, &bytes).unwrap();
    let mut s = new_doc();
    let before = state(&s);
    room.set(Some(headroom::RESERVE));
    for cmd in ["file.placeEmbedded", "file.placeLinked"] {
        assert!(s.execute(cmd, json!({"path": path})).unwrap_err().to_string().contains("Not enough memory"));
        assert_eq!(state(&s), before);
    }
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    assert!(photocraft_engine::smart_cmds::source_image("layers.psd", &bytes, fmt).is_err());

    // The layers make the need bigger but bounded, so the error names it.
    let err = s.execute("file.placeEmbedded", json!({"path": path})).unwrap_err().to_string();
    assert!(err.contains("MB"), "{err}");

    // This budget passed the old canvas-only estimate even though the layers retain more.
    room.set(Some(headroom::RESERVE + budget));
    for cmd in ["file.placeEmbedded", "file.placeLinked"] {
        assert!(s.execute(cmd, json!({"path": path})).unwrap_err().to_string().contains("Not enough memory"));
        assert_eq!(state(&s), before);
    }
    assert!(photocraft_engine::smart_cmds::source_image("layers.psd", &bytes, fmt).is_err());
}

#[test]
fn small_flat_psd_still_places_and_renders() {
    let room = Room::new(Some(SMALL));
    // Two-by-two RGB PSD: empty color, resource and layer sections, raw planar pixels.
    let mut bytes = b"8BPS\0\x01\0\0\0\0\0\0\0\x03\0\0\0\x02\0\0\0\x02\0\x08\0\x03".to_vec();
    bytes.extend_from_slice(&[0; 12]);
    bytes.extend_from_slice(&[0, 0]);
    bytes.extend_from_slice(&[128; 12]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("flat.psd");
    std::fs::write(&path, &bytes).unwrap();
    let mut s = new_doc();
    let before = state(&s);
    s.execute("file.placeEmbedded", json!({"path": path})).unwrap();
    assert_eq!(state(&s).2, before.2 + 1);
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    assert!(photocraft_engine::smart_cmds::source_image("flat.psd", &bytes, fmt).is_ok());
    drop(room);
}

/// Every Photoshop file has image resources (resolution, ICC profile, thumbnail); they hold no
/// pixels, so a PSD that has them, flat or with a few layers, still places and renders when the
/// memory is there.
#[test]
fn a_psd_with_image_resources_places_and_renders_when_it_fits() {
    use craft_psd::{LayerSpec, PixelData, PsdBuilder};

    let room = Room::new(Some(headroom::RESERVE + 256 * MIB));
    const SIDE: u32 = 64;
    let rgba = |r: u8| -> Vec<u8> { (0..SIDE * SIDE).flat_map(|_| [r, 80, 160, 255]).collect() };
    let mut flat = PsdBuilder::new(SIDE, SIDE).resolution(72.0);
    flat.composite(PixelData::Rgba8(rgba(10)));
    let mut layered = PsdBuilder::new(SIDE, SIDE).resolution(150.0);
    for (i, r) in [20u8, 30].into_iter().enumerate() {
        layered.push_layer(LayerSpec::new(format!("layer {i}"), 0, 0, SIDE, SIDE, PixelData::Rgba8(rgba(r))));
    }
    let dir = tempfile::tempdir().unwrap();
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, true);
    let mut s = new_doc();
    for (name, builder) in [("resources.psd", flat), ("resources-layers.psd", layered)] {
        let bytes = builder.to_bytes().unwrap();
        // No color-mode data for RGB, then a non-empty image resources section.
        assert_ne!(&bytes[30..34], &[0u8; 4][..], "{name} has image resources");
        assert!(photocraft_engine::smart_cmds::source_image(name, &bytes, fmt).is_ok(), "{name}");
        let path = dir.path().join(name);
        std::fs::write(&path, &bytes).unwrap();
        let before = state(&s);
        s.execute("file.placeEmbedded", json!({"path": path})).unwrap();
        assert_eq!(state(&s).2, before.2 + 1, "{name}");
    }
    drop(room);
}

/// A PSD with no safe bound (here 16-bit) is refused whatever the free memory, so the error
/// doesn't suggest freeing some.
#[test]
fn a_psd_without_a_bound_says_to_flatten_it() {
    let room = Room::new(Some(headroom::RESERVE + 1024 * MIB));
    let mut bytes = b"8BPS\0\x01\0\0\0\0\0\0\0\x03\0\0\0\x02\0\0\0\x02\0\x10\0\x03".to_vec();
    bytes.extend_from_slice(&[0; 14]);
    bytes.extend_from_slice(&[128; 24]);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deep.psd");
    std::fs::write(&path, &bytes).unwrap();
    let mut s = new_doc();
    let before = state(&s);
    let err = s.execute("file.placeEmbedded", json!({"path": path})).unwrap_err().to_string();
    assert!(!err.contains("Purge"), "freeing memory cannot help an unbounded source: {err}");
    assert!(err.contains("flatten"), "{err}");
    assert_eq!(state(&s), before);
    drop(room);
}

#[test]
fn a_smart_object_edit_under_low_memory_keeps_its_appearance() {
    let room = Room::new(None);
    let dir = tempfile::tempdir().unwrap();
    let path = png(dir.path(), "edit.png", 1500, 1500, "#a1b2c3");
    let mut s = new_doc();
    s.execute("file.placeEmbedded", json!({"path": path})).unwrap();
    let before = state(&s);
    room.set(Some(SMALL));
    s.execute("edit.transform", json!({"scaleX": 50, "scaleY": 50})).unwrap();
    assert_eq!(state(&s).2, before.2, "the layer is still there");
}

#[test]
fn everything_passes_without_a_simulation() {
    let _room = Room::new(None);
    assert!(photocraft_engine::memory_gate::ensure("do anything", 64 << 30).is_ok());
    assert!(photocraft_engine::memory_gate::ensure_for_save("save", 64 << 30).is_ok());
}

#[test]
fn the_save_wording_leads_with_freeing_memory_and_keeps_the_document() {
    let room = Room::new(Some(SMALL));
    let err = photocraft_engine::memory_gate::ensure_for_save("save \u{201c}a.psd\u{201d}", 900 * MIB).unwrap_err().to_string();
    assert!(err.starts_with("Not enough memory to save \u{201c}a.psd\u{201d} (it needs about 900 MB)."), "{err}");
    assert!(err.contains("Edit \u{203a} Purge \u{203a} All"), "{err}");
    assert!(err.contains("still open"), "{err}");
    drop(room);
}
