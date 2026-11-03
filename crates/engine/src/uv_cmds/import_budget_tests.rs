use super::{ModelDirectory, UvTextureBudget, UvTextureKey, cached_uv_texture, import_uv_texture, uv_texture_key, uv_texture_preflight};
use std::{cell::Cell, collections::HashMap, sync::Arc};

#[test]
fn shared_texture_references_decode_once_across_many_material_slots() {
    let temp = tempfile::tempdir().expect("temporary model directory");
    let source = temp.path().join("mesh.obj");
    let texture = temp.path().join("shared.png");
    std::fs::write(&texture, b"image bytes").expect("write shared image");
    let alias = temp.path().join("hardlink.png");
    let alias_path = if std::fs::hard_link(&texture, &alias).is_ok() {
        let directory = ModelDirectory::new(source.to_str().expect("model path")).expect("open model directory");
        let primary = directory.open_texture_file(&texture.to_string_lossy()).expect("open primary file").1;
        let linked = directory.open_texture_file(&alias.to_string_lossy()).expect("open hard-link alias").1;
        assert_eq!(UvTextureKey::File(Arc::new(primary)), UvTextureKey::File(Arc::new(linked)));
        Some(alias.to_string_lossy().into_owned())
    } else {
        None
    };
    let directory = ModelDirectory::new(source.to_str().expect("model path")).expect("open model directory");
    let primary_path = texture.to_str().expect("texture path");
    let mut cache = HashMap::new();
    let decode_count = Cell::new(0usize);
    let mut budget = UvTextureBudget::default();

    for material in 0..64 {
        let path = if material % 2 == 1 { alias_path.as_deref().unwrap_or(primary_path) } else { primary_path };
        let (file, identity, size) = directory.open_texture_file(path).expect("open identity from pinned directory");
        let key = UvTextureKey::File(Arc::new(identity));
        assert!(super::reserve_uv_texture_identity(&mut cache, &mut budget, &key, path).is_none());
        let result = cached_uv_texture(&mut cache, key, || {
            decode_count.set(decode_count.get() + 1);
            super::charge_texture_source_work(&mut budget, size).expect("charge unique source once");
            budget.decode_work_bytes += 1;
            assert_eq!(super::read_opened_file(file, size).expect("read once"), b"image bytes");
            Ok(7)
        });
        assert_eq!(result, Ok(7));
    }

    assert_eq!(decode_count.get(), 1);
    assert_eq!(cache.len(), 1);
    assert_eq!(budget.encoded_work_bytes, 11);
    assert_eq!(budget.decode_work_bytes, 1);
    assert_eq!(budget.identity_handles, 1);
}

#[test]
fn cached_success_is_reused_after_all_new_work_budgets_are_exhausted() {
    let temp = tempfile::tempdir().expect("temporary model directory");
    let source = temp.path().join("mesh.obj");
    let texture = temp.path().join("shared.png");
    std::fs::write(&texture, b"image bytes").expect("write texture");
    let directory = ModelDirectory::new(source.to_str().expect("model path")).expect("open model directory");
    let mut cache = super::UvTextureCache::new();
    let mut budget = UvTextureBudget::default();
    let decode_count = Cell::new(0usize);

    let (file, identity, size) = directory.open_texture_file(&texture.to_string_lossy()).expect("open first reference");
    let key = UvTextureKey::File(Arc::new(identity));
    assert!(super::reserve_uv_texture_identity(&mut cache, &mut budget, &key, "shared.png").is_none());
    assert_eq!(
        cached_uv_texture(&mut cache, key, || {
            decode_count.set(decode_count.get() + 1);
            super::charge_texture_source_work(&mut budget, size)?;
            budget.decode_work_bytes += 1;
            let _ = super::read_opened_file(file, size).map_err(|error| error.to_string())?;
            Ok(9)
        }),
        Ok(9)
    );

    budget.encoded_work_bytes = super::UV_TEXTURE_TOTAL_SOURCE_BYTES;
    budget.decode_work_bytes = super::UV_TEXTURE_DECODE_WORK_BUDGET_BYTES;
    budget.identity_handles = super::UV_TEXTURE_MAX_IDENTITY_HANDLES;
    let (file, identity, _) = directory.open_texture_file(&texture.to_string_lossy()).expect("open cached alias");
    let key = UvTextureKey::File(Arc::new(identity));
    assert!(super::reserve_uv_texture_identity(&mut cache, &mut budget, &key, "shared.png").is_none());
    assert_eq!(
        cached_uv_texture(&mut cache, key, || {
            decode_count.set(decode_count.get() + 1);
            let _file_not_read = file;
            Err("cache hit should not decode".to_owned())
        }),
        Ok(9)
    );
    assert_eq!(decode_count.get(), 1);
    assert_eq!(budget.encoded_work_bytes, super::UV_TEXTURE_TOTAL_SOURCE_BYTES);
    assert_eq!(budget.decode_work_bytes, super::UV_TEXTURE_DECODE_WORK_BUDGET_BYTES);
    assert_eq!(budget.identity_handles, super::UV_TEXTURE_MAX_IDENTITY_HANDLES);
}

#[test]
fn encoded_source_work_budget_has_an_allocation_free_boundary_check() {
    let mut budget = UvTextureBudget { encoded_work_bytes: super::UV_TEXTURE_TOTAL_SOURCE_BYTES - 1, ..Default::default() };
    super::charge_texture_source_work(&mut budget, 1).expect("last byte fits exactly");
    assert_eq!(budget.encoded_work_bytes, super::UV_TEXTURE_TOTAL_SOURCE_BYTES);
    assert!(super::charge_texture_source_work(&mut budget, 1).is_err());
    assert_eq!(budget.encoded_work_bytes, super::UV_TEXTURE_TOTAL_SOURCE_BYTES);

    budget.encoded_work_bytes = super::UV_TEXTURE_TOTAL_SOURCE_BYTES - 3;
    assert!(super::charge_texture_source_work(&mut budget, 4).is_err());
    assert_eq!(budget.encoded_work_bytes, super::UV_TEXTURE_TOTAL_SOURCE_BYTES - 3, "rejected aggregate metadata is not charged");

    budget.encoded_work_bytes = 10;
    assert!(super::charge_texture_source_work(&mut budget, super::UV_TEXTURE_MAX_SOURCE_BYTES + 1).is_err());
    assert_eq!(budget.encoded_work_bytes, 10, "rejected per-file metadata is not charged");
}

#[test]
fn shared_embedded_arc_and_failed_sources_are_cached() {
    let bytes: Arc<[u8]> = Arc::from(&b"texture"[..]);
    let clone = Arc::clone(&bytes);
    let first = uv_texture_key(Some(&bytes)).expect("embedded key");
    let second = uv_texture_key(Some(&clone)).expect("cloned embedded key");
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
    let mut budget = UvTextureBudget { resident_bytes: super::UV_TEXTURE_RESIDENT_BUDGET_BYTES, ..Default::default() };
    assert!(import_uv_texture("checker.png", png, &mut budget, false).is_err());
    assert_eq!(budget.resident_bytes, super::UV_TEXTURE_RESIDENT_BUDGET_BYTES);
    assert_eq!(budget.encoded_work_bytes, png.len() as u64);
    assert_eq!(budget.decode_work_bytes, 0);
}

#[test]
fn aligned_8k_texture_fits_the_budget_without_allocating_pixels() {
    // QOI's fixed 14-byte header is enough for the codec's header-only size probe.
    let mut qoi = b"qoif".to_vec();
    qoi.extend_from_slice(&8192u32.to_be_bytes());
    qoi.extend_from_slice(&8192u32.to_be_bytes());
    qoi.extend_from_slice(&[4, 0]);
    let reserve = uv_texture_preflight(&qoi, super::UV_TEXTURE_PEAK_BUDGET_BYTES).expect("one aligned 8K image fits");
    assert_eq!(reserve, 8192u64 * 8192 * 40);
    assert!(uv_texture_preflight(&qoi, reserve - 1).is_err());
}

#[test]
fn failed_decode_spends_work_budget_but_successful_document_spends_resident_budget() {
    let valid = include_bytes!("../../../uv/tests/fixtures/checker.png");
    let mut corrupt = valid.to_vec();
    corrupt[41] = 0;
    assert!(uv_texture_preflight(&corrupt, u64::MAX).is_ok());

    let mut budget = UvTextureBudget::default();
    assert!(import_uv_texture("corrupt.png", &corrupt, &mut budget, false).is_err());
    assert_eq!(budget.resident_bytes, 0);
    assert_eq!(budget.encoded_work_bytes, corrupt.len() as u64);
    assert!(budget.decode_work_bytes > 0);

    let (document, _, _, storage) = import_uv_texture("checker.png", valid, &mut budget, false).expect("valid texture imports after failure");
    assert!(document.size.width > 0);
    assert_eq!(budget.resident_bytes, storage);
    assert_eq!(budget.encoded_work_bytes, (corrupt.len() + valid.len()) as u64);
    assert!(budget.decode_work_bytes > 0);
}

#[test]
fn decode_work_budget_is_independent_of_resident_document_budget() {
    let png = include_bytes!("../../../uv/tests/fixtures/checker.png");
    let reserve = uv_texture_preflight(png, u64::MAX).expect("fixture preflights");
    let mut budget = UvTextureBudget { decode_work_bytes: super::UV_TEXTURE_DECODE_WORK_BUDGET_BYTES - reserve + 1, ..Default::default() };
    assert!(import_uv_texture("checker.png", png, &mut budget, false).is_err());
    assert_eq!(budget.resident_bytes, 0);
    assert_eq!(budget.decode_work_bytes, super::UV_TEXTURE_DECODE_WORK_BUDGET_BYTES - reserve + 1);
    assert_eq!(budget.encoded_work_bytes, png.len() as u64);
}

#[test]
fn rejected_decode_estimate_does_not_consume_remaining_work_budget() {
    let mut budget = UvTextureBudget { decode_work_bytes: super::UV_TEXTURE_DECODE_WORK_BUDGET_BYTES - 50, ..Default::default() };
    assert!(super::reserve_texture_decode_work(&mut budget, 51).is_err());
    assert_eq!(budget.decode_work_bytes, super::UV_TEXTURE_DECODE_WORK_BUDGET_BYTES - 50);
    super::reserve_texture_decode_work(&mut budget, 50).expect("smaller request still fits");
    assert_eq!(budget.decode_work_bytes, super::UV_TEXTURE_DECODE_WORK_BUDGET_BYTES);
}

#[test]
fn opened_texture_that_grows_past_its_reserved_size_is_rejected() {
    use std::io::Write;

    let temp = tempfile::tempdir().expect("temporary model directory");
    let source = temp.path().join("mesh.obj");
    let texture = temp.path().join("map.png");
    std::fs::write(&texture, b"small").expect("write initial texture");
    let directory = ModelDirectory::new(source.to_str().expect("model path")).expect("open model directory");
    let (file, _, reserved_size) = directory.open_texture_file(&texture.to_string_lossy()).expect("open texture at original size");
    let mut writer = std::fs::OpenOptions::new().append(true).open(&texture).expect("reopen texture for append");
    writer.write_all(b" grew after metadata").expect("grow texture");
    let mut budget = UvTextureBudget::default();
    assert!(super::read_import_texture_source(file, reserved_size, &mut budget).is_err());
    assert_eq!(budget.encoded_work_bytes, reserved_size, "read attempt charges the original source size");
}

#[test]
fn cached_external_path_failure_does_not_block_embedded_texture_bytes() {
    let mut cache = super::UvTextureCache::new();
    let mut budget = UvTextureBudget::default();
    let missing_path = UvTextureKey::FailedPath(String::new());
    cache.insert(missing_path, Err("missing external texture".to_owned()));

    let embedded = UvTextureKey::Embedded(1);
    assert!(super::reserve_uv_texture_identity(&mut cache, &mut budget, &embedded, "").is_none());
    assert_eq!(super::cached_uv_texture(&mut cache, embedded, || Ok(3)), Ok(3));
}

#[test]
fn handle_limit_failure_is_cached_without_reserving_another_identity() {
    let temp = tempfile::tempdir().expect("temporary model directory");
    let source = temp.path().join("mesh.obj");
    let texture = temp.path().join("map.png");
    std::fs::write(&texture, b"image bytes").expect("write texture");
    let directory = ModelDirectory::new(source.to_str().expect("model path")).expect("open model directory");
    let (_, identity, _) = directory.open_texture_file(&texture.to_string_lossy()).expect("open identity");
    let key = UvTextureKey::File(Arc::new(identity));
    let mut cache = HashMap::new();
    let mut budget = UvTextureBudget { identity_handles: super::UV_TEXTURE_MAX_IDENTITY_HANDLES, ..Default::default() };
    let result = super::reserve_uv_texture_identity(&mut cache, &mut budget, &key, "map.png").expect("limit rejects");
    assert!(result.is_err());
    assert_eq!(budget.identity_handles, super::UV_TEXTURE_MAX_IDENTITY_HANDLES);
    assert_eq!(cache.len(), 1);
    assert!(cache.keys().all(|key| matches!(key, UvTextureKey::FailedPath(_))));
    assert!(super::reserve_uv_texture_identity(&mut cache, &mut budget, &key, "map.png").is_some_and(|result| result.is_err()), "failed path remains cached");
    assert_eq!(budget.identity_handles, super::UV_TEXTURE_MAX_IDENTITY_HANDLES);
}

#[cfg(windows)]
#[test]
fn case_variant_paths_share_file_identity_on_case_insensitive_windows_directories() {
    let temp = tempfile::tempdir().expect("temporary model directory");
    let source = temp.path().join("mesh.obj");
    let texture = temp.path().join("Map.png");
    std::fs::write(&texture, b"same file").expect("write texture");
    let directory = ModelDirectory::new(source.to_str().expect("model path")).expect("open model directory");
    let upper = directory.open_texture_file(&texture.to_string_lossy()).expect("open original case").1;
    let lower_path = directory.directory.join("map.png");
    let lower = match directory.open_texture_file(&lower_path.to_string_lossy()) {
        Ok((_, identity, _)) => identity,
        Err(_) => {
            assert!(directory.open_texture_file(&texture.to_string_lossy()).is_ok(), "original spelling remains openable");
            eprintln!("Skipping case-alias identity test: this Windows directory is case-sensitive");
            return;
        }
    };
    assert_eq!(UvTextureKey::File(Arc::new(upper)), UvTextureKey::File(Arc::new(lower)));
    let alias = temp.path().join("hardlink.png");
    if std::fs::hard_link(&texture, &alias).is_ok() {
        let linked = directory.open_texture_file(&alias.to_string_lossy()).expect("open hard-link alias").1;
        let primary = directory.open_texture_file(&texture.to_string_lossy()).expect("open primary file").1;
        assert_eq!(UvTextureKey::File(Arc::new(linked)), UvTextureKey::File(Arc::new(primary)));
    }
}

#[cfg(windows)]
#[test]
fn texture_identity_open_rejects_windows_symlink_and_escape_paths() {
    let temp = tempfile::tempdir().expect("temporary model directory");
    let source = temp.path().join("mesh.obj");
    let outside_dir = tempfile::tempdir().expect("external directory");
    let outside = outside_dir.path().join("outside.png");
    let link = temp.path().join("linked.png");
    std::fs::write(&outside, b"outside").expect("write outside file");
    if let Err(error) = std::os::windows::fs::symlink_file(&outside, &link) {
        if error.raw_os_error() == Some(1314) {
            eprintln!("Skipping Windows symbolic-link test: Developer Mode or elevation is required");
            return;
        }
        panic!("Could not create Windows symbolic link: {error}");
    }
    let directory = ModelDirectory::new(source.to_str().expect("model path")).expect("open model directory");
    assert!(directory.open_texture_file(&link.to_string_lossy()).is_err());
    assert!(directory.open_texture_file(&outside.to_string_lossy()).is_err());
}

#[cfg(unix)]
#[test]
fn case_distinct_files_keep_distinct_identities_and_symlink_escape_is_rejected() {
    let temp = tempfile::tempdir().expect("temporary model directory");
    let source = temp.path().join("mesh.obj");
    let lower_path = temp.path().join("map.png");
    let upper_path = temp.path().join("MAP.png");
    std::fs::write(&lower_path, b"lower").expect("write lower case texture");
    std::fs::write(&upper_path, b"upper").expect("write upper case texture");
    let directory = ModelDirectory::new(source.to_str().expect("model path")).expect("open model directory");
    let lower = directory.open_texture_file(&lower_path.to_string_lossy()).expect("open lower file").1;
    let upper = directory.open_texture_file(&upper_path.to_string_lossy()).expect("open upper file").1;
    assert_ne!(UvTextureKey::File(Arc::new(lower)), UvTextureKey::File(Arc::new(upper)));

    let outside_dir = tempfile::tempdir().expect("external directory");
    let outside = outside_dir.path().join("outside.png");
    std::fs::write(&outside, b"outside").expect("write outside file");
    let link = temp.path().join("linked.png");
    std::os::unix::fs::symlink(&outside, &link).expect("create symlink");
    assert!(directory.open_texture_file(&link.to_string_lossy()).is_err());
    assert!(directory.open_texture_file(&outside.to_string_lossy()).is_err());
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
