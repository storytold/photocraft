use super::*;

fn flat_doc(name: &str, v: f32) -> Document {
    let (w, h) = (16usize, 16usize);
    let mut d = Document::new(name, Size::new(w as u32, h as u32), ColorMode::Rgb, SampleType::U8);
    let fmt = d.pixel_format();
    let n = fmt.channels();
    let mut data = vec![0.0f32; w * h * n];
    for o in data.chunks_exact_mut(n) {
        photocraft_raster::from_rgba_into(&fmt, [v, v, v, 1.0], o);
    }
    let mut s = Surface::new(fmt);
    s.write_region(Rect::new(0, 0, w as i32, h as i32), &data);
    d.layers.push(Layer::new("Background", LayerContent::Raster(s)));
    d
}

fn session_with_exposures() -> Session {
    let mut s = Session::new();
    for (k, v) in [0.2f32, 0.5, 0.8].into_iter().enumerate() {
        s.add_document(flat_doc(&format!("e{k}"), v), None);
    }
    s
}

/// Runs Merge to HDR over every open document and returns the new document's index and name.
fn merge(s: &mut Session) -> (usize, String) {
    let evs: Vec<f64> = (0..s.documents().len()).map(|i| i as f64).collect();
    let r = s.execute("file.automate.mergeToHdrPro", json!({"useOpenDocuments": true, "exposures": evs, "align": false})).unwrap();
    let idx = r["document"].as_u64().unwrap() as usize;
    (idx, s.documents()[idx].doc.name.clone())
}

#[test]
fn first_result_is_named_with_a_hyphen() {
    let mut s = session_with_exposures();
    let (_, name) = merge(&mut s);
    assert_eq!(name, "Untitled_HDR-1");
}

#[test]
fn second_result_while_first_is_open_gets_a_different_name() {
    let mut s = session_with_exposures();
    let (_, first) = merge(&mut s);
    let (_, second) = merge(&mut s);
    assert_ne!(first, second);
    assert!(first.starts_with("Untitled_HDR-") && second.starts_with("Untitled_HDR-"), "{first} {second}");
}

#[test]
fn name_stays_unique_after_a_result_is_closed() {
    let mut s = session_with_exposures();
    let (first_idx, first) = merge(&mut s);
    let (_, second) = merge(&mut s);
    assert_ne!(first, second);
    s.close(first_idx);
    let (_, third) = merge(&mut s);
    assert!(third.starts_with("Untitled_HDR-"), "{third}");
    assert_ne!(third, second, "the new result must not share the still-open result's name");
    let names: Vec<&str> = s.documents().iter().map(|d| d.doc.name.as_str()).collect();
    let mut dedup = names.clone();
    dedup.sort_unstable();
    dedup.dedup();
    assert_eq!(dedup.len(), names.len(), "{names:?}");
}
