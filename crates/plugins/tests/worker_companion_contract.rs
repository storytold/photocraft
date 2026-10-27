//! Opt-in cross-language contract: fixtures must come from the production C# writer.
//! The companion CI explicitly supplies the fixture directory and runs this with --ignored.
#![cfg(not(target_arch = "wasm32"))]

use std::error::Error;
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;

use photocraft_plugins::adapter::{Context, Image, OutputKind, Request};
use photocraft_plugins::worker::{protocol, WorkerReply, WorkerState};

struct NoContext;
impl Context for NoContext {
    fn cancelled(&self) -> bool {
        false
    }
    fn progress(&self, _fraction: f32) {}
}

fn read_fixture(directory: &Path, name: &str) -> Result<WorkerReply, Box<dyn Error>> {
    let mut file = File::open(directory.join(name))?;
    protocol::read_response(&mut file, &NoContext).map_err(Into::into)
}

#[test]
#[ignore = "requires generated C# fixtures; set PHOTOCRAFT_COMPANION_FIXTURES and run explicitly with --ignored"]
fn csharp_responses_decode_with_strict_rust_protocol() -> Result<(), Box<dyn Error>> {
    // Missing input is a failure when explicitly run, never a successful skip.
    let directory = std::env::var_os("PHOTOCRAFT_COMPANION_FIXTURES")
        .ok_or("PHOTOCRAFT_COMPANION_FIXTURES must name the C#-generated fixture directory")?;
    let directory = Path::new(&directory);
    for (name, state) in [
        ("ready", WorkerState::Ready),
        ("needs_model", WorkerState::NeedsModel),
        ("unavailable", WorkerState::Unavailable),
        ("error", WorkerState::Error),
    ] {
        let reply = read_fixture(directory, &format!("{name}.bin"))?;
        let WorkerReply::Status(status) = reply else {
            return Err(format!("{name} fixture returned an unexpected run response").into());
        };
        assert_eq!(status.state, state, "{name} fixture state");
        assert_eq!(status.message, format!("fixture: {name}"));
    }

    let WorkerReply::Ok(response) = read_fixture(directory, "ok-mask.bin")? else {
        return Err("ok-mask fixture returned an unexpected status response".into());
    };
    assert_eq!(response.output.kind, OutputKind::Mask);
    let image = response.output.image;
    assert_eq!((image.x, image.y, image.width, image.height), (-7, 11, 3, 2));
    assert_eq!(image.channels, 1);
    assert_eq!(image.color_mode, "gray");
    assert_eq!(image.source_depth, 32);
    assert!(!image.alpha);
    assert!(image.icc_profile.is_empty());
    let expected: Vec<f32> = [0_u8, 1, 63, 127, 128, 255].into_iter().map(|value| f32::from(value) / 255.0).collect();
    assert_eq!(image.data, expected, "C# Gray8 normalization and little-endian f32 samples");
    assert_eq!(response.execution.devices, vec!["windows-ai.npu".to_owned()]);
    assert_eq!(response.execution.backend, "windows-ai");
    assert_eq!(response.execution.fallback_reason, None);

    // Preserve the strict Rust boundary: the former C# fields are invalid even if null.
    let mut ready = File::open(directory.join("ready.bin"))?;
    let mut prefix = [0_u8; 12];
    ready.read_exact(&mut prefix)?;
    let size = u32::from_le_bytes(prefix[8..12].try_into()?);
    assert!((1..=protocol::MAX_HEADER_BYTES).contains(&usize::try_from(size)?));
    let mut bytes = vec![0_u8; usize::try_from(size)?];
    ready.read_exact(&mut bytes)?;
    for key in ["ready_state", "descriptor"] {
        let mut header: serde_json::Value = serde_json::from_slice(&bytes)?;
        header.as_object_mut().ok_or("fixture header must be an object")?.insert(key.into(), serde_json::Value::Null);
        let json = serde_json::to_vec(&header)?;
        let mut frame = Vec::new();
        frame.extend_from_slice(protocol::RESPONSE_MAGIC);
        frame.extend_from_slice(&u32::try_from(json.len())?.to_le_bytes());
        frame.extend_from_slice(&json);
        assert!(protocol::read_response(&mut Cursor::new(frame), &NoContext).is_err(), "obsolete {key} field must remain rejected");
    }
    Ok(())
}

#[test]
#[ignore = "requires fixture output directory; set PHOTOCRAFT_COMPANION_FIXTURES and run explicitly with --ignored"]
fn rust_writer_generates_companion_request_fixtures() -> Result<(), Box<dyn Error>> {
    let directory = std::env::var_os("PHOTOCRAFT_COMPANION_FIXTURES").ok_or("PHOTOCRAFT_COMPANION_FIXTURES must name the fixture directory")?;
    let out_dir = Path::new(&directory).join("from-rust");
    std::fs::create_dir_all(&out_dir)?;
    let mut control_file = File::create_new(out_dir.join("control-request.bin"))?;
    protocol::write_request(&mut control_file, None, &NoContext)?;

    for (name, depth, alpha, with_selection, tagged, automatic) in [
        ("rgb-8", 8, false, false, false, true),
        ("rgb-16", 16, false, false, false, false),
        ("rgb-32", 32, false, false, false, false),
        ("rgba-selection", 16, true, true, false, false),
        ("rgb-icc", 16, false, false, true, false),
    ] {
        // Synthetic ICC header only, not an actual characterization profile or a WIC
        // colour-conversion oracle. This suite only checks byte transport and parser fields.
        let icc_profile = if tagged {
            let mut bytes = vec![0_u8; 128];
            bytes[0..4].copy_from_slice(&128_u32.to_be_bytes());
            bytes[12..16].copy_from_slice(b"mntr");
            bytes[16..20].copy_from_slice(b"RGB ");
            bytes[36..40].copy_from_slice(b"acsp");
            bytes
        } else {
            Vec::new()
        };
        let image = Image {
            x: -7,
            y: 11,
            width: 3,
            height: 2,
            channels: if alpha { 4 } else { 3 },
            color_mode: "rgb".into(),
            source_depth: depth,
            alpha,
            icc_profile,
            data: if alpha { [0.0, 0.5, 1.0, 0.25].repeat(6) } else { [0.0, 0.5, 1.0].repeat(6) },
        };
        let selection = with_selection.then(|| Image {
            x: -7,
            y: 11,
            width: 3,
            height: 2,
            channels: 1,
            color_mode: "gray".into(),
            source_depth: 32,
            alpha: false,
            icc_profile: Vec::new(),
            data: vec![0.0, 0.25, 0.5, 0.75, 1.0, 0.5001],
        });
        let request = Request {
            operation: "select-subject".into(),
            resource: None,
            params: if tagged { serde_json::json!({}) } else { serde_json::json!({ "colorPolicy": "assume-srgb" }) },
            device: (!automatic).then(|| "windows-ai.npu".into()),
            allow_fallback: false,
            image,
            selection,
        };
        let mut file = File::create_new(out_dir.join(format!("{name}.bin")))?;
        protocol::write_request(&mut file, Some(&request), &NoContext)?;
    }
    Ok(())
}
