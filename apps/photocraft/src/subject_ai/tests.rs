use std::path::Path;

use craft_ai_core::StoreFile;
use craft_ai_native::secrets::MemoryStore;

use super::*;

/// A shared layer in a temporary folder.
struct World {
    dir: tempfile::TempDir,
    model: CraftSubject,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let shared = SharedStore::new(dir.path().join("connections.json"));
    shared.replace(&StoreFile::default()).unwrap();
    let secrets: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let client = AiClient::new(app_id().unwrap(), shared.clone(), Arc::clone(&secrets));
    let (feature, role) = tool().unwrap();
    World { dir, model: CraftSubject { client, shared, secrets, feature, role } }
}

fn picture(width: u32, height: u32) -> SubjectImage {
    let rgb = (0..width * height).flat_map(|i| [(i % 251) as u8, (i % 13) as u8, (i % 7) as u8]).collect();
    SubjectImage { width, height, rgb }
}

/// An 8-bit picture's pixels as RGB: a gray picture repeats its one value, colour keeps its first
/// three channels, and alpha is left out.
fn rgb_of(image: &Image) -> Vec<u8> {
    let channels = image.layout().channels();
    image.data().chunks_exact(channels).flat_map(|px| if channels >= 3 { [px[0], px[1], px[2]] } else { [px[0]; 3] }).collect()
}

#[test]
fn gray_and_colour_test_pictures_become_rgb() {
    let gray = Image::from_u8(2, 1, ChannelLayout::Gray, vec![10, 200]).unwrap();
    assert_eq!(rgb_of(&gray), vec![10, 10, 10, 200, 200, 200]);
    let gray_alpha = Image::from_u8(2, 1, ChannelLayout::GrayA, vec![10, 255, 200, 0]).unwrap();
    assert_eq!(rgb_of(&gray_alpha), vec![10, 10, 10, 200, 200, 200]);
    let rgba = Image::from_u8(1, 1, ChannelLayout::Rgba, vec![1, 2, 3, 4]).unwrap();
    assert_eq!(rgb_of(&rgba), vec![1, 2, 3]);
}

fn gray_png(width: u32, height: u32) -> Vec<u8> {
    let image = Image::from_u8(width, height, ChannelLayout::Gray, vec![200; (width * height) as usize]).unwrap();
    photocraft_codecs::encode(&image, Format::Png, &EncodeOptions::default()).unwrap()
}

#[test]
fn with_nothing_set_up_the_download_is_offered_with_its_size_and_terms() {
    let ModelStatus::Offer(offer) = world().model.status() else { panic!("expected an offer") };
    assert_eq!(offer.model, "InSPyReNet");
    assert_eq!(offer.download_bytes, 367_120_444);
    assert!(offer.terms.contains("clicking Download"), "{}", offer.terms);
    assert!(offer.terms.contains("MIT"), "{}", offer.terms);
    assert!(offer.folder.ends_with("models\\inspyrenet") || offer.folder.ends_with("models/inspyrenet"), "{}", offer.folder);
}

#[test]
fn the_offer_names_the_folder_a_person_already_chose_and_creates_none() {
    let w = world();
    let chosen = w.dir.path().join("my models");
    FolderDesk::new(w.model.shared.clone()).ensure_default_at(&chosen).unwrap();
    let ModelStatus::Offer(offer) = w.model.status() else { panic!("expected an offer") };
    assert_eq!(Path::new(&offer.folder), chosen.join("inspyrenet"));
    assert!(!chosen.join("inspyrenet").exists());
}

#[test]
fn a_file_that_is_set_up_is_ready() {
    let w = world();
    let weights = w.dir.path().join("inspyrenet.safetensors");
    std::fs::write(&weights, b"stands in for the model").unwrap();
    let launcher = AiClient::new(ToolDesk::app().unwrap(), w.model.shared.clone(), Arc::clone(&w.model.secrets));
    ToolDesk::new(launcher, w.model.shared.clone(), None).enable_native(&w.model.feature, &weights).unwrap();
    assert_eq!(w.model.status(), ModelStatus::Ready { model: "InSPyReNet, built in (any graphics card)".into() });
}

#[test]
fn a_download_is_offered_again_when_the_set_up_file_is_gone() {
    let w = world();
    let weights = w.dir.path().join("inspyrenet.safetensors");
    std::fs::write(&weights, b"stands in for the model").unwrap();
    let launcher = AiClient::new(ToolDesk::app().unwrap(), w.model.shared.clone(), Arc::clone(&w.model.secrets));
    ToolDesk::new(launcher, w.model.shared.clone(), None).enable_native(&w.model.feature, &weights).unwrap();
    assert_eq!(native_connection(), w.model.shared.load().unwrap().defaults[0].connection.as_str(), "the id the shared layer saves");
    std::fs::remove_file(&weights).unwrap();
    assert!(matches!(w.model.status(), ModelStatus::Offer(_)), "{:?}", w.model.status());
}

/// A choice of `connection` for the tool.
fn binding(w: &World, connection: &str) -> craft_ai_core::Binding {
    craft_ai_core::Binding {
        role: w.model.role.id.clone(),
        connection: craft_ai_core::ConnectionId::new(connection).unwrap(),
        model: craft_ai_core::ModelId::new("birefnet").unwrap(),
    }
}

#[test]
fn a_download_is_not_offered_over_another_choice() {
    let w = world();
    let mut store = w.model.shared.load().unwrap();
    store.defaults.push(binding(&w, "comfy-at-home"));
    w.model.shared.replace(&store).unwrap();
    let ModelStatus::Unavailable { reason } = w.model.status() else { panic!("expected no offer: {:?}", w.model.status()) };
    assert!(reason.contains("Craft launcher"), "{reason}");
}

#[test]
fn photocrafts_own_choice_comes_before_the_launchers() {
    // PhotoCraft's own choice of another model: no download, whatever the launcher has.
    let w = world();
    let mut store = w.model.shared.load().unwrap();
    store.apps.entry(app_id().unwrap()).or_default().overrides.push(binding(&w, "comfy-at-home"));
    w.model.shared.replace(&store).unwrap();
    assert!(matches!(w.model.status(), ModelStatus::Unavailable { .. }), "{:?}", w.model.status());
    // PhotoCraft's own choice of this build's model goes before another launcher choice.
    let w = world();
    let mut store = w.model.shared.load().unwrap();
    store.defaults.push(binding(&w, "comfy-at-home"));
    store.apps.entry(app_id().unwrap()).or_default().overrides.push(binding(&w, &native_connection()));
    w.model.shared.replace(&store).unwrap();
    assert!(matches!(w.model.status(), ModelStatus::Offer(_)), "{:?}", w.model.status());
}

#[test]
fn a_file_that_is_not_the_model_fails_in_words_a_person_can_read() {
    let w = world();
    let weights = w.dir.path().join("inspyrenet.safetensors");
    std::fs::write(&weights, b"not the model").unwrap();
    let launcher = AiClient::new(ToolDesk::app().unwrap(), w.model.shared.clone(), Arc::clone(&w.model.secrets));
    ToolDesk::new(launcher, w.model.shared.clone(), None).enable_native(&w.model.feature, &weights).unwrap();
    let Err(ModelError::Failed(message)) = w.model.matte(&picture(8, 8), &|| false) else { panic!("expected a failure") };
    assert!(message.contains("Set up AI"), "{message}");
}

#[test]
fn a_small_picture_goes_as_png() {
    let (Payload::Inline { media_type, data }, 16, 16) = encode(&picture(16, 16), u64::MAX).unwrap() else { panic!("expected bytes") };
    assert_eq!(media_type.as_str(), "image/png");
    assert_eq!(photocraft_codecs::decode(&data).unwrap().width(), 16);
}

#[test]
fn a_picture_too_big_for_one_payload_goes_at_half_size() {
    let image = picture(64, 48);
    let img = Image::from_u8(64, 48, ChannelLayout::Rgb, image.rgb.clone()).unwrap();
    let png = photocraft_codecs::encode(&img, Format::Png, &EncodeOptions::default()).unwrap().len();
    let options = EncodeOptions { jpeg_quality: 95, jpeg_chroma_subsampling: false, ..EncodeOptions::default() };
    let jpeg = photocraft_codecs::encode(&img, Format::Jpeg, &options).unwrap().len();
    let limit = png.min(jpeg) as u64 - 1;
    let (Payload::Inline { data, .. }, width, height) = encode(&image, limit).unwrap() else { panic!("expected bytes") };
    assert_eq!((width, height), (32, 24));
    assert!(data.len() as u64 <= limit);
    assert_eq!(photocraft_codecs::decode(&data).unwrap().width(), 32);
    // Nothing fits at all: an error, not an endless loop.
    assert!(encode(&image, 0).is_err());
}

#[test]
fn half_size_takes_the_mean_and_rounds_up() {
    let image = SubjectImage { width: 3, height: 1, rgb: vec![0, 0, 0, 100, 50, 10, 7, 8, 9] };
    assert_eq!(half(&image), SubjectImage { width: 2, height: 1, rgb: vec![50, 25, 5, 7, 8, 9] });
}

#[test]
fn a_smaller_mask_is_scaled_back_to_the_picture() {
    let mask = vec![0, 255, 0, 255];
    assert_eq!(fit_mask(mask.clone(), (2, 2), (2, 2)), mask);
    let big = fit_mask(mask, (2, 2), (4, 4));
    assert_eq!(big.len(), 16);
    assert_eq!((big[0], big[3], big[12], big[15]), (0, 255, 0, 255));
    assert!(big[1] > 0 && big[1] < big[2] && big[2] < 255, "{big:?}");
}

#[test]
fn a_mask_of_the_wrong_size_or_kind_is_refused() {
    let image = picture(4, 4);
    assert_eq!(decode_mask(&gray_png(4, 4), image.width, image.height).unwrap(), vec![200; 16]);
    assert!(decode_mask(&gray_png(3, 4), image.width, image.height).is_err());
    let rgb = photocraft_codecs::encode(&Image::from_u8(4, 4, ChannelLayout::Rgb, vec![9; 48]).unwrap(), Format::Png, &EncodeOptions::default()).unwrap();
    assert!(decode_mask(&rgb, image.width, image.height).is_err());
    assert!(decode_mask(b"not a picture", image.width, image.height).is_err());
}

/// The real thing, for a person to run: a real download (or the file in the model folder), then a
/// real picture through the real engine. `CRAFT_TEST_PICTURE` names a PNG or JPEG, and
/// `CRAFT_TEST_MODEL_FOLDER` a folder for the model (kept between runs).
#[test]
#[ignore = "downloads 367 MB; needs CRAFT_TEST_PICTURE and CRAFT_TEST_MODEL_FOLDER"]
fn the_real_model_downloads_and_answers() {
    let (Some(picture_path), Some(folder)) = (std::env::var_os("CRAFT_TEST_PICTURE"), std::env::var_os("CRAFT_TEST_MODEL_FOLDER")) else {
        eprintln!("skipped: set CRAFT_TEST_PICTURE and CRAFT_TEST_MODEL_FOLDER");
        return;
    };
    let w = world();
    let desk = FolderDesk::new(w.model.shared.clone());
    desk.ensure_default_at(Path::new(&folder)).unwrap();
    let started = std::time::Instant::now();
    let last = std::sync::Mutex::new(0.0f32);
    w.model
        .install(
            &|fraction, message| {
                let mut seen = last.lock().unwrap();
                if fraction - *seen >= 0.1 || fraction < *seen {
                    eprintln!("{message}: {:.0} %", fraction * 100.0);
                    *seen = fraction;
                }
            },
            &|| false,
        )
        .unwrap();
    eprintln!("installed in {:.1} s", started.elapsed().as_secs_f32());
    assert!(matches!(w.model.status(), ModelStatus::Ready { .. }), "{:?}", w.model.status());

    let decoded = photocraft_codecs::decode(&std::fs::read(picture_path).unwrap()).unwrap();
    let (width, height) = (decoded.width(), decoded.height());
    assert_eq!(decoded.sample_type(), SampleType::U8);
    let rgb = rgb_of(&decoded);
    let image = SubjectImage { width, height, rgb };
    for run in 1..=2 {
        let started = std::time::Instant::now();
        let mask = w.model.matte(&image, &|| false).unwrap();
        let subject = mask.iter().filter(|v| **v >= 128).count() as f32 / mask.len() as f32;
        eprintln!("run {run}: {width} × {height} in {:.1} s, subject covers {:.1} %", started.elapsed().as_secs_f32(), subject * 100.0);
        assert_eq!(mask.len(), (width * height) as usize);
        assert!(subject > 0.001 && subject < 0.999);
    }
}
