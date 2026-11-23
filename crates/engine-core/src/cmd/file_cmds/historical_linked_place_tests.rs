//! Linked interpretation is tied to the exact source revision before publication.
use super::*;
use crate::jobs::{JobCtx, StepWork};
use photocraft_doc::{HistoricalXcfSampleOrder as Order, LayerId, SmartObject};
use std::sync::Arc;

fn fixture(little: bool) -> Vec<u8> {
    let mut bytes = crate::jobs::historical_fixture::historical_rgba16();
    if little {
        let start = bytes.len() - 8;
        for word in bytes[start..].chunks_exact_mut(2) {
            word.swap(0, 1);
        }
    }
    bytes
}
fn session() -> Session {
    let mut s = crate::session();
    s.execute("file.new", json!({"width":1,"height":1,"depth":16})).unwrap();
    s
}
fn object(s: &Session, id: LayerId) -> &SmartObject {
    let LayerContent::Smart(object) = &s.active().unwrap().doc.layer(id).unwrap().content else { panic!("smart source") };
    object
}
fn prepared(input: PlaceInput, ctx: &JobCtx) -> Result<super::open::Prepared> {
    let mut steps = PlaceSteps::new(input, true);
    for _ in 0..32 {
        if let Some(value) = steps.step(ctx, web_time::Instant::now())? {
            return Ok(value);
        }
    }
    panic!("one pixel Place must finish bounded steps")
}
#[test]
fn linked_place_retains_both_orders_across_cold_native_and_psd_sources() {
    let _lock = crate::edit_menu_cmds::PURGE_TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = tempfile::tempdir().unwrap();
    for (wire, order, little) in [("little-endian", Order::LittleEndian, true), ("big-endian", Order::BigEndian, false)] {
        let bytes = fixture(little);
        let path = directory.path().join(format!("{wire}.xcf"));
        std::fs::write(&path, &bytes).unwrap();
        for cooperative in [false, true] {
            let mut s = session();
            s.set_cooperative_jobs(cooperative);
            let r=s.execute("file.placeLinked",json!({"path":path,"fit":false,"frame":false,"historicalXcfSampleOrder":wire,"expectedSourceDigest":photocraft_io::linked::historical_xcf_digest(&bytes)})).unwrap();
            let id = LayerId(r["layer"].as_u64().unwrap());
            let object = object(&s, id);
            assert_eq!(object.historical_xcf_sample_order, Some(order));
            assert_eq!(object.historical_xcf_link_digest, Some(photocraft_io::linked::historical_xcf_digest(&bytes)));
            let document = &s.active().unwrap().doc;
            let native = photocraft_format::load_from_bytes(&photocraft_format::save_to_bytes(document, &Default::default()).unwrap()).unwrap();
            let psd = photocraft_io::export(document, "source.psd", &photocraft_io::ExportOptions { read_linked_files: true, ..Default::default() }).unwrap();
            assert!(!psd.warnings.iter().any(|warning| warning.contains("written as pixels")), "{:?}", psd.warnings);
            let psd = photocraft_io::import("source.psd", &psd.bytes).unwrap().document;
            for document in [&**document, &native, &psd] {
                let object = document.walk().into_iter().find_map(|(_, _, l)| if let LayerContent::Smart(sm) = &l.content { Some(sm) } else { None }).unwrap();
                assert_eq!(object.historical_xcf_sample_order, Some(order));
                crate::smart_cmds::purge_source_cache();
                crate::smart_cmds::purge_stack_cache();
                let actual = crate::smart_cmds::render(document, object).unwrap().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 1, 1));
                for (a, b) in actual.iter().zip([0x1234u16, 0x2345, 0x3456, 0xffff].map(|v| f32::from(v) / 65535.)) {
                    assert!((a - b).abs() <= 1. / 65535., "{a} versus {b}");
                }
            }
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
}
#[test]
fn linked_place_stale_pick_changed_before_apply_and_cancellation_are_atomic() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.xcf");
    let bytes = fixture(true);
    std::fs::write(&path, &bytes).unwrap();
    let mut s = session();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    assert!(s.execute("file.placeLinked", json!({"path":path,"frame":false,"fit":false})).unwrap_err().to_string().contains("sample byte order"));
    assert!(matches!(
        s.execute("file.placeLinked", json!({"path":path,"historicalXcfSampleOrder":"little-endian","expectedSourceDigest":vec![0u8;32]})),
        Err(crate::EngineError::SourceChanged { .. })
    ));
    let params = json!({"frame":false,"fit":false,"historicalXcfSampleOrder":"little-endian"});
    let input = PlaceInput::new(&s, "source.xcf", bytes.clone(), Some(path.to_string_lossy().into_owned()), &params).unwrap();
    let output = prepared(input, &JobCtx::new()).unwrap();
    std::fs::write(&path, fixture(false)).unwrap();
    assert!(matches!(apply_place(&mut s, output), Err(crate::EngineError::HistoricalXcfLinkChanged(_))));
    let cancelled = JobCtx::new();
    cancelled.cancel();
    let input = PlaceInput::new(&s, "source.xcf", bytes, Some(path.to_string_lossy().into_owned()), &params).unwrap();
    assert!(matches!(prepared(input, &cancelled), Err(crate::EngineError::Cancelled)));
    assert!(Arc::ptr_eq(&s.active().unwrap().doc, &before));
    assert_eq!(s.active().unwrap().history.past_len(), history);
}
