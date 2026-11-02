use super::{UvTextureBudget, UvTextureKey, cached_uv_texture, import_uv_texture, uv_texture_key, uv_texture_preflight};
use std::{cell::Cell, collections::HashMap, sync::Arc};

#[test]
fn shared_texture_references_decode_once_across_many_material_slots() {
    let path = "textures/shared.png";
    let mut cache = HashMap::new();
    let decode_count = Cell::new(0usize);

    for _material in 0..64 {
        let key = uv_texture_key(Some(path), None).expect("path key");
        let result = cached_uv_texture(&mut cache, key, || {
            decode_count.set(decode_count.get() + 1);
            Ok(7)
        });
        assert_eq!(result, Ok(7));
    }

    assert_eq!(decode_count.get(), 1);
    assert_eq!(cache.len(), 1);
}

#[test]
fn shared_embedded_arc_and_failed_sources_are_cached() {
    let bytes: Arc<[u8]> = Arc::from(&b"texture"[..]);
    let clone = Arc::clone(&bytes);
    let first = uv_texture_key(None, Some(&bytes)).expect("embedded key");
    let second = uv_texture_key(None, Some(&clone)).expect("cloned embedded key");
    assert_eq!(first, second);

    let mut cache = HashMap::<UvTextureKey, Result<usize, String>>::new();
    let decode_count = Cell::new(0usize);
    for _ in 0..32 {
        let result = cached_uv_texture(&mut cache, first.clone(), || {
            decode_count.set(decode_count.get() + 1);
            Err("invalid texture".to_owned())
        });
        assert_eq!(result, Err("invalid texture".to_owned()));
    }
    assert_eq!(decode_count.get(), 1);
}

#[test]
fn texture_preflight_checks_header_dimensions_and_total_reservation() {
    let png = include_bytes!("../../../uv/tests/fixtures/checker.png");
    let reservation = uv_texture_preflight(png, u64::MAX).expect("small PNG preflights");
    assert!(reservation > 0);
    assert!(uv_texture_preflight(png, reservation - 1).is_err());
    assert!(uv_texture_preflight(b"not an image", u64::MAX).is_err());
}

#[test]
fn texture_budget_refuses_a_source_when_memory_reservation_exceeds_remaining() {
    let png = include_bytes!("../../../uv/tests/fixtures/checker.png");
    let mut budget = UvTextureBudget { reserved_bytes: super::UV_TEXTURE_IMPORT_BUDGET_BYTES, source_bytes: 0 };
    assert!(import_uv_texture("checker.png", png, &mut budget).is_err());
    assert_eq!(budget.reserved_bytes, super::UV_TEXTURE_IMPORT_BUDGET_BYTES);
    assert_eq!(budget.source_bytes, png.len() as u64);
}

#[test]
fn aligned_8k_texture_fits_the_budget_without_allocating_pixels() {
    // QOI's fixed 14-byte header is enough for the codec's header-only size probe.
    let mut qoi = b"qoif".to_vec();
    qoi.extend_from_slice(&8192u32.to_be_bytes());
    qoi.extend_from_slice(&8192u32.to_be_bytes());
    qoi.extend_from_slice(&[4, 0]);
    let reserve = uv_texture_preflight(&qoi, super::UV_TEXTURE_IMPORT_BUDGET_BYTES).expect("one aligned 8K image fits");
    assert_eq!(reserve, 8192u64 * 8192 * 40);
    assert!(uv_texture_preflight(&qoi, reserve - 1).is_err());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn many_materials_share_one_decoded_color_normal_and_emissive_texture() {
    let temp = tempfile::tempdir().expect("temporary model directory");
    let mut material_library = String::new();
    let mut model = String::from("mtllib mesh.mtl\n");
    for material in 0..32 {
        material_library.push_str(&format!("newmtl material_{material}\nmap_Kd shared.png\nnorm shared.png\nmap_Ke shared.png\n"));
        let base = material * 3 + 1;
        let uv = material * 3 + 1;
        model.push_str(&format!(
            "v {material} 0 0\nv {} 0 0\nv {material} 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nusemtl material_{material}\nf {base}/{uv} {}/{} {}/{}\n",
            material + 1,
            base + 1,
            uv + 1,
            base + 2,
            uv + 2,
        ));
    }
    std::fs::write(temp.path().join("mesh.obj"), model).expect("write OBJ");
    std::fs::write(temp.path().join("mesh.mtl"), material_library).expect("write MTL");
    std::fs::write(temp.path().join("shared.png"), include_bytes!("../../../uv/tests/fixtures/checker.png")).expect("write texture");

    let mut session = crate::Session::new();
    session.execute("uv.model.import", serde_json::json!({"path":temp.path().join("mesh.obj").to_string_lossy()})).expect("import model");
    assert_eq!(session.documents().len(), 1);
    let shared_id = session.documents()[0].doc.id;
    let mut expected = vec![Some(shared_id); 33];
    expected[0] = None;
    assert_eq!(session.uv.bindings, expected);
    assert_eq!(session.uv.normal_bindings, expected);
    assert_eq!(session.uv.map_bindings.get(&photocraft_uv::preview::Channel::Emissive), Some(&expected));
}
