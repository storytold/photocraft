use super::*;
#[test]
fn malformed_generation_requests_are_rejected_before_runtime_access() {
    for p in [
        Value::Null,
        json!({}),
        json!({"prompt":" "}),
        json!({"prompt":"x","width":63}),
        json!({"prompt":"x","height":1025}),
        json!({"prompt":"x","width":65}),
        json!({"prompt":"x","steps":0}),
        json!({"prompt":"x","cfg":-1}),
        json!({"prompt":"x","seed":1.5}),
        json!({"prompt":"x","model":false}),
        json!({"prompt":"x".repeat(2001)}),
    ] {
        assert!(Request::parse(&p).is_err(), "{p}");
    }
    let r = Request::parse(&json!({"prompt":"landscape","seed":42})).unwrap();
    assert_eq!((r.width, r.height, r.seed), (512, 512, 42));
}
#[test]
fn untrusted_png_headers_are_bounded_before_decode() {
    assert!(validate_png(&[], 64, 64).is_err());
    let mut header = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    header.extend(64u32.to_be_bytes());
    header.extend(64u32.to_be_bytes());
    assert!(validate_png(&header, 64, 64).is_ok());
    assert!(validate_png(&header, 128, 64).is_err());
    assert!(validate_png(&header, u32::MAX, 64).is_err());
}
#[test]
fn bad_command_leaves_document_and_history_unchanged() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":64,"height":64})).unwrap();
    let before = s.active().unwrap().doc.clone();
    assert!(s.execute(COMMAND, json!({"prompt":""})).is_err());
    assert_eq!(s.active().unwrap().doc.layers, before.layers);
}

#[test]
#[cfg(target_os = "linux")]
fn generated_layers_preserve_working_format_subject_and_single_undo() {
    use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image};
    let image = Image::from_raw(64, 64, ChannelLayout::Rgba, photocraft_codecs::SampleType::U8, [40, 80, 160, 255].repeat(64 * 64)).unwrap();
    let output = Output {
        png: photocraft_codecs::encode(&image, Format::Png, &EncodeOptions::default()).unwrap(),
        width: 64,
        height: 64,
        seed: 42,
        model: "test-model".into(),
    };
    for depth in [8, 16, 32] {
        for mode in ["rgb", "gray", "cmyk", "lab"] {
            for behind in [false, true] {
                let mut s = Session::new();
                s.execute("file.new", json!({"width":64,"height":64,"depth":depth,"mode":mode})).unwrap();
                s.execute("layer.new.layer", json!({"name":"Subject"})).unwrap();
                s.execute("select.rect", json!({"x":16,"y":16,"width":32,"height":32})).unwrap();
                s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
                let st = s.active().unwrap();
                let subject = st.active_layer.unwrap();
                let before = st.doc.clone();
                let history = st.history.past_len();
                let surface =
                    decode(&output, &crate::color_cmds::document_profile(&before), before.pixel_format(), s.color.settings.intent(), s.color.settings.bpc)
                        .unwrap();
                assert_eq!(surface.format(), before.pixel_format());
                let result = apply(&mut s, surface, 42, "test-model", behind.then_some(subject)).unwrap();
                let st = s.active().unwrap();
                assert_eq!(st.doc.layer(subject), before.layer(subject));
                assert_eq!(st.doc.selection, before.selection);
                assert_eq!(st.doc.depth, before.depth);
                assert_eq!(st.doc.mode, before.mode);
                assert_eq!(st.doc.icc_profile, before.icc_profile);
                assert_eq!(st.history.past_len(), history + 1);
                assert_eq!(result["seed"], 42);
                if behind {
                    assert_eq!(st.active_layer, Some(subject));
                }
                let bytes = photocraft_format::save_to_bytes(&st.doc, &Default::default()).unwrap();
                let reopened = photocraft_format::load_from_bytes(&bytes).unwrap();
                assert_eq!(reopened.layers, st.doc.layers);
                assert!(s.undo());
                assert_eq!(s.active().unwrap().doc.layers, before.layers);
                assert!(s.redo());
                assert_eq!(s.active().unwrap().doc.layer(subject), before.layer(subject));
            }
        }
    }
}

#[test]
#[cfg(target_os = "linux")]
fn cancelling_generation_never_changes_the_document() {
    use crate::jobs::Started;
    let mut s = Session::new();
    s.execute("file.new", json!({"width":64,"height":64})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let past = s.active().unwrap().history.past_len();
    let Started::Job(id) = s.start(COMMAND, json!({"prompt":"cancellation test","width":64,"height":64,"model":"missing-test-model"})).unwrap() else {
        panic!("expected background job")
    };
    assert!(s.cancel_job(id));
    assert_eq!(s.active().unwrap().doc.layers, before.layers);
    assert_eq!(s.active().unwrap().history.past_len(), past);
}

#[test]
#[cfg(target_os = "linux")]
#[ignore = "requires installed Enki runtime and an explicitly selected LUMA_LIVE_ENKI_MODEL"]
fn live_generation_imports_a_new_layer_and_undo_restores_original() {
    let model = std::env::var("LUMA_LIVE_ENKI_MODEL").expect("choose an installed test model");
    let mut s = Session::new();
    s.execute("file.new", json!({"width":256,"height":256})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let result=s.execute(COMMAND,json!({"prompt":"A small lighthouse on a rocky island, blue sea, clear daylight","width":256,"height":256,"steps":4,"cfg":1,"seed":42,"model":model})).unwrap();
    assert_eq!(result["seed"], 42);
    assert_eq!(result["model"], model);
    assert_eq!(s.active().unwrap().doc.layers.len(), before.layers.len() + 1);
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap().layers, s.active().unwrap().doc.layers);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layers, before.layers);
}

#[test]
#[cfg(target_os = "linux")]
fn invalid_background_placement_is_rejected_before_generation() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":1100,"height":64})).unwrap();
    s.execute("layer.new.layer", json!({"name":"Subject"})).unwrap();
    let p = json!({"prompt":"landscape","placement":"background"});
    assert!(s.execute(COMMAND, p.clone()).is_err());
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let error = s.execute(COMMAND, p).unwrap_err();
    assert!(error.to_string().contains("1024"));
    assert_eq!(s.active().unwrap().doc.layers, before.layers);
    assert!(s.execute(COMMAND, json!({"prompt":"landscape","placement":false})).is_err());
}

#[test]
#[cfg(target_os = "linux")]
fn palette_and_special_ink_modes_are_refused_before_generation() {
    for mode in [
        photocraft_color::ColorMode::Indexed,
        photocraft_color::ColorMode::Bitmap,
        photocraft_color::ColorMode::Multichannel,
        photocraft_color::ColorMode::Duotone,
    ] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":64,"height":64})).unwrap();
        s.edit("test mode", |doc, _| {
            doc.mode = mode;
            Ok(())
        })
        .unwrap();
        assert!(enabled(&s).is_err());
        assert!(s.execute(COMMAND, json!({"prompt":"landscape"})).is_err());
    }
}
