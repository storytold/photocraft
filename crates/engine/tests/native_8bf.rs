#![cfg(all(not(target_arch = "wasm32"), unix))]
use photocraft_color::{ColorMode, SampleType};
use photocraft_engine::{Session, jobs::Started};
use photocraft_geom::Rect;
use photocraft_plugins::native_8bf::{self, client::Launch};
use serde_json::json;
use std::{path::PathBuf, time::Duration};
fn pe(path: &std::path::Path) {
    let mut b = vec![0u8; 1024];
    b[..2].copy_from_slice(b"MZ");
    b[60..64].copy_from_slice(&128u32.to_le_bytes());
    b[128..132].copy_from_slice(b"PE\0\0");
    b[132..134].copy_from_slice(&0x14cu16.to_le_bytes());
    b[134..136].copy_from_slice(&1u16.to_le_bytes());
    b[148..150].copy_from_slice(&224u16.to_le_bytes());
    b[150..152].copy_from_slice(&0x2000u16.to_le_bytes());
    b[152..154].copy_from_slice(&0x10bu16.to_le_bytes());
    b[248..252].copy_from_slice(&4096u32.to_le_bytes());
    b[252..256].copy_from_slice(&40u32.to_le_bytes());
    b[384..388].copy_from_slice(&512u32.to_le_bytes());
    b[388..392].copy_from_slice(&4096u32.to_le_bytes());
    b[392..396].copy_from_slice(&512u32.to_le_bytes());
    b[396..400].copy_from_slice(&512u32.to_le_bytes());
    b[536..540].copy_from_slice(&1u32.to_le_bytes());
    b[544..548].copy_from_slice(&4160u32.to_le_bytes());
    b[576..580].copy_from_slice(&4176u32.to_le_bytes());
    b[592..603].copy_from_slice(b"PluginMain\0");
    std::fs::write(path, b).unwrap();
}
fn fixture(name: &str, script: &str) -> (String, PathBuf) {
    let path = std::env::temp_dir().join(format!("pc-engine-8bf-{}-{name}.8bf", std::process::id()));
    pe(&path);
    let launch = Launch { program: "python3".into(), args: vec!["-c".into(), script.into()], environment: Default::default(), timeout: Duration::from_secs(3) };
    let p = native_8bf::install_with_launch(&path, launch).unwrap();
    (p.descriptor.id.clone(), path)
}
const INVERT: &str = r#"import sys,struct,json,time
r=sys.stdin.buffer;w=sys.stdout.buffer
r.read(8);n,p=struct.unpack('<IQ',r.read(12));m=json.loads(r.read(n));d=r.read(p)
time.sleep(.1)
h=json.dumps({'status':'ok','message':'','width':m['width'],'height':m['height'],'channels':3,'depth':8}).encode();w.write(b'PC8BF001'+struct.pack('<IQ',len(h),p)+h+bytes(255-v for v in d))
"#;
fn doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":3,"height":1})).unwrap();
    s.edit("fixture", |d, a| {
        d.layer_mut(a.unwrap())
            .unwrap()
            .surface_mut()
            .unwrap()
            .write_region(Rect::new(0, 0, 3, 1), &[0.2, 0.4, 0.6, 0.5, 0.2, 0.4, 0.6, 0.5, 0.2, 0.4, 0.6, 0.5]);
        let mut mask = photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8);
        mask.write_region(Rect::new(0, 0, 3, 1), &[0.0, 0.5, 1.0]);
        d.selection = Some(mask);
        Ok(())
    })
    .unwrap();
    s
}
fn pixels(s: &Session) -> Vec<f32> {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().read_region(Rect::new(0, 0, 3, 1))
}
#[test]
fn applies_background_result_as_one_undo_with_alpha_and_selection() {
    let (id, path) = fixture("invert", INVERT);
    let mut s = doc();
    let before = pixels(&s);
    let depth = s.active().unwrap().history.past_len();
    let started = s.start("plugin.8bf.run", json!({"id":id,"show_ui":false})).unwrap();
    let Started::Job(job) = started else { panic!("expected background job") };
    assert_eq!(pixels(&s), before);
    s.wait_job(job).unwrap();
    let after = pixels(&s);
    assert_eq!(&after[..4], &before[..4]);
    assert_eq!(after[7], before[7]);
    assert_eq!(after[11], before[11]);
    assert!((after[8] - 0.8).abs() < 0.01);
    assert_eq!(s.active().unwrap().history.past_len(), depth + 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(pixels(&s), before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(pixels(&s), after);
    s.execute("plugin.8bf.remove", json!({"id":id})).unwrap();
    std::fs::remove_file(path).unwrap();
}
#[test]
fn errors_and_invalid_targets_leave_document_and_history_intact() {
    let (id, path) = fixture("crash", "import sys;sys.exit(19)");
    let mut s = doc();
    let before = pixels(&s);
    let depth = s.active().unwrap().history.past_len();
    for p in [json!({"id":id,"target":"mask"}), json!({"id":id,"show_ui":"yes"}), json!({"id":id,"show_ui":false})] {
        assert!(s.execute("plugin.8bf.run", p).is_err());
        assert_eq!(pixels(&s), before);
        assert_eq!(s.active().unwrap().history.past_len(), depth);
    }
    s.execute("plugin.8bf.remove", json!({"id":id})).unwrap();
    std::fs::remove_file(path).unwrap();
}

#[test]
fn cancelling_background_native_filter_keeps_document_and_history_unchanged() {
    let (id, path) = fixture("cancel", "import time;time.sleep(60)");
    let mut s = doc();
    let before = pixels(&s);
    let depth = s.active().unwrap().history.past_len();
    let Started::Job(job) = s.start("plugin.8bf.run", json!({"id":id,"show_ui":false})).unwrap() else { panic!("expected background job") };
    assert!(s.cancel_job(job));
    assert!(s.wait_job(job).is_err());
    assert_eq!(pixels(&s), before);
    assert_eq!(s.active().unwrap().history.past_len(), depth);
    native_8bf::remove(&id);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn explicit_rgb_raster_target_runs_while_a_vector_layer_is_active() {
    let (id, path) = fixture("explicit-target", INVERT);
    let mut s = doc();
    let target = s.active().unwrap().active_layer.unwrap();
    s.execute("shape.create", json!({"rect":[0,0,2,1]})).unwrap();
    let active_vector = s.active().unwrap().active_layer.unwrap();
    assert_ne!(active_vector, target);
    assert!(!s.is_enabled("plugin.8bf.run"), "active vector layer cannot run a pixel filter");
    s.execute("plugin.8bf.run", json!({"id":id,"layer":target.0,"show_ui":false})).unwrap();
    assert_eq!(s.active().unwrap().active_layer, Some(active_vector));
    let surface = s.active().unwrap().doc.layer(target).unwrap().surface().unwrap();
    assert!((surface.pixel(2, 0)[0] - 0.8).abs() < 0.01);
    native_8bf::remove(&id);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn non_rgb8_documents_are_rejected_before_worker_launch_even_with_rgb8_storage() {
    let marker = std::env::temp_dir().join(format!("pc-engine-8bf-{}-unsupported-marker", std::process::id()));
    let script = format!("from pathlib import Path;Path({}).touch()", serde_json::to_string(&marker.to_string_lossy()).unwrap());
    let (id, path) = fixture("unsupported-document", &script);
    let run = photocraft_engine::plugin_8bf_cmds::specs().into_iter().find(|spec| spec.id == "plugin.8bf.run").unwrap().run;
    for (mode, sample) in [
        (ColorMode::Indexed, SampleType::U8),
        (ColorMode::Multichannel, SampleType::U8),
        (ColorMode::Grayscale, SampleType::U8),
        (ColorMode::Rgb, SampleType::U16),
        (ColorMode::Rgb, SampleType::F32),
    ] {
        let mut s = doc();
        if mode == ColorMode::Indexed {
            s.execute("image.mode.indexedColor", json!({})).unwrap();
        } else {
            s.edit("document metadata", |d, _| {
                d.mode = mode;
                d.depth = sample;
                Ok(())
            })
            .unwrap();
        }
        let target = s.active().unwrap().active_layer.unwrap();
        let surface = s.active().unwrap().doc.layer(target).unwrap().surface().unwrap();
        assert_eq!(surface.format().mode, ColorMode::Rgb);
        assert_eq!(surface.format().sample, SampleType::U8);
        let before = pixels(&s);
        let depth = s.active().unwrap().history.past_len();
        let revision = s.active().unwrap().revision;
        assert!(!s.is_enabled("plugin.8bf.run"), "unsupported document {mode:?}/{sample:?} should disable native filters");
        for params in [json!({"id":id,"show_ui":false}), json!({"id":id,"show_ui":false,"layer":target.0})] {
            assert!(s.execute("plugin.8bf.run", params.clone()).is_err());
            assert!(run(&mut s, &params).is_err(), "the command handler must independently reject unsupported documents");
            assert_eq!(pixels(&s), before);
            assert_eq!(s.active().unwrap().history.past_len(), depth);
            assert_eq!(s.active().unwrap().revision, revision);
            assert!(!marker.exists(), "unsupported document must not launch the native worker");
        }
    }
    native_8bf::remove(&id);
    std::fs::remove_file(path).unwrap();
}

const FORMAT_INVERT: &str = r#"import sys,struct,json
r=sys.stdin.buffer;w=sys.stdout.buffer
r.read(8);n,p=struct.unpack('<IQ',r.read(12));m=json.loads(r.read(n));d=r.read(p)
if m['depth']==16:
    values=struct.unpack('<'+'H'*(len(d)//2),d);d=struct.pack('<'+'H'*len(values),*(65535-v for v in values))
else:d=bytes(255-v for v in d)
m.update(status='ok',message='')
h=json.dumps(m).encode();w.write(b'PC8BF001'+struct.pack('<IQ',len(h),len(d))+h+d)
"#;

#[test]
fn native_gray_and_rgb_16bit_edits_preserve_precision_selection_alpha_and_undo() {
    let (id, path) = fixture("sixteen-bit", FORMAT_INVERT);
    for mode in ["gray", "rgb"] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":3,"height":1,"mode":mode,"depth":16,"background":"transparent"})).unwrap();
        s.edit("16-bit fixture", |d, a| {
            let surface = d.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            let fmt = surface.format();
            let color = fmt.mode.color_channels();
            let channels = fmt.channels();
            let bytes: Vec<u8> = (0..3)
                .flat_map(|x| {
                    (0..channels).flat_map(move |c| {
                        let value: u16 = if c == color { 65534 } else { 1 + x * 8192 + c as u16 * 1003 };
                        value.to_ne_bytes()
                    })
                })
                .collect();
            surface.write_interleaved(Rect::new(0, 0, 3, 1), &bytes);
            let mut selection = photocraft_raster::Surface::new(photocraft_color::PixelFormat::GRAY8);
            selection.write_region(Rect::new(0, 0, 3, 1), &[0.0, 128.0 / 255.0, 1.0]);
            d.selection = Some(selection);
            Ok(())
        })
        .unwrap();
        let target = s.active().unwrap().active_layer.unwrap();
        let before = s.active().unwrap().doc.layer(target).unwrap().surface().unwrap().clone();
        let format = before.format();
        let color = format.mode.color_channels();
        let channels = format.channels();
        let original = before.to_interleaved(Rect::new(0, 0, 3, 1));
        let depth = s.active().unwrap().history.past_len();
        assert!(s.is_enabled("plugin.8bf.run"), "{mode}16 should allow native filters");
        s.execute("plugin.8bf.run", json!({"id":id,"show_ui":false})).unwrap();
        let after = s.active().unwrap().doc.layer(target).unwrap().surface().unwrap();
        assert_eq!(after.format(), format);
        let result = after.to_interleaved(Rect::new(0, 0, 3, 1));
        assert_eq!(&result[..channels * 2], &original[..channels * 2]);
        for x in 0..3 {
            assert_eq!(
                &result[(x * channels + color) * 2..(x * channels + color + 1) * 2],
                &original[(x * channels + color) * 2..(x * channels + color + 1) * 2]
            );
        }
        for c in 0..color {
            let at = (2 * channels + c) * 2;
            let a = u16::from_ne_bytes([original[at], original[at + 1]]);
            let b = u16::from_ne_bytes([result[at], result[at + 1]]);
            assert_eq!(b, 65535 - a, "{mode}16 lost color precision");
        }
        assert_eq!(s.active().unwrap().history.past_len(), depth + 1);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.layer(target).unwrap().surface().unwrap().to_interleaved(Rect::new(0, 0, 3, 1)), original);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.layer(target).unwrap().surface().unwrap().to_interleaved(Rect::new(0, 0, 3, 1)), result);
    }
    native_8bf::remove(&id);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn gray8_is_enabled_and_keeps_grayscale_storage() {
    let (id, path) = fixture("gray-eight", FORMAT_INVERT);
    let mut s = Session::new();
    s.execute("file.new", json!({"width":3,"height":1,"mode":"gray","depth":8})).unwrap();
    assert!(s.is_enabled("plugin.8bf.run"));
    s.execute("plugin.8bf.run", json!({"id":id,"show_ui":false})).unwrap();
    let st = s.active().unwrap();
    let fmt = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().format();
    assert_eq!(st.doc.mode, ColorMode::Grayscale);
    assert_eq!(st.doc.depth, SampleType::U8);
    assert_eq!(fmt.mode, ColorMode::Grayscale);
    assert_eq!(fmt.sample, SampleType::U8);
    native_8bf::remove(&id);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn turning_preference_folders_off_unregisters_discovered_native_filters_only() {
    let folder = std::env::temp_dir().join(format!("pc-engine-8bf-prune-{}", std::process::id()));
    std::fs::create_dir_all(folder.join("nested")).unwrap();
    pe(&folder.join("nested/discovered.8bf"));
    pe(&folder.join("explicit.8bf"));
    let id = |name: &str| format!("8bf:{}", std::fs::canonicalize(&folder).unwrap().join(name).display());
    let mut s = Session::new();
    s.execute("prefs.set", json!({"values":{"plugIns.additionalPluginsFolders":[folder],"plugIns.useAdditionalPluginsFolder":true}})).unwrap();
    assert!(native_8bf::get(&id("nested/discovered.8bf")).is_some());
    s.execute("plugin.8bf.install", json!({"path":folder.join("explicit.8bf")})).unwrap();
    s.execute("prefs.set", json!({"path":"plugIns.useAdditionalPluginsFolder","value":false})).unwrap();
    assert!(native_8bf::get(&id("nested/discovered.8bf")).is_none(), "folder-discovered filters follow the preference");
    assert!(native_8bf::get(&id("explicit.8bf")).is_some(), "an explicit install stays");
    native_8bf::remove(&id("explicit.8bf"));
    std::fs::remove_dir_all(folder).unwrap();
}
