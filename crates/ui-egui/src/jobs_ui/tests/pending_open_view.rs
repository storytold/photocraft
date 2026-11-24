use super::*;
use crate::jobs_ui::{OpenSource, finish_open};
use photocraft_doc::{DocId, Document, LayerId};
use photocraft_engine::DocState;
use photocraft_io::NativeImportTarget;

// Original 2x2, uncompressed XCF v11 fixture. Its source properties independently
// select both layers, show the active mask, expose an alpha overlay and sampler.
struct Xcf(Vec<u8>);
impl Xcf {
    fn word(&mut self, word: u32) {
        self.0.extend(word.to_be_bytes());
    }
    fn ptr(&mut self, value: usize) {
        self.0.extend((value as u64).to_be_bytes());
    }
    fn slot(&mut self) -> usize {
        let at = self.0.len();
        self.ptr(0);
        at
    }
    fn patch(&mut self, at: usize) {
        let value = self.0.len() as u64;
        self.0[at..at + 8].copy_from_slice(&value.to_be_bytes());
    }
    fn string(&mut self, text: &str) {
        self.word(text.len() as u32 + 1);
        self.0.extend(text.as_bytes());
        self.0.push(0);
    }
    fn property(&mut self, kind: u32, payload: &[u8]) {
        self.word(kind);
        self.word(payload.len() as u32);
        self.0.extend(payload);
    }
    fn end(&mut self) {
        self.word(0);
        self.word(0);
    }
    fn pixels(&mut self, slot: usize, data: &[u8], components: u32) {
        self.patch(slot);
        self.word(2);
        self.word(2);
        self.word(components);
        let level = self.slot();
        self.ptr(0);
        self.patch(level);
        self.word(2);
        self.word(2);
        let tile = self.slot();
        self.ptr(0);
        self.patch(tile);
        self.0.extend(data);
    }
}

fn saved_view_xcf(target: NativeImportTarget) -> Vec<u8> {
    let mut w = Xcf(b"gimp xcf v011\0".to_vec());
    for word in [2, 2, 0, 150] {
        w.word(word);
    }
    w.property(17, &[0]);
    w.property(39, &[1u32, 1, 0, 0, 0].into_iter().flat_map(u32::to_be_bytes).collect::<Vec<_>>());
    w.end();
    let front = w.slot();
    let back = w.slot();
    w.ptr(0);
    let alpha = w.slot();
    w.ptr(0);
    for (slot, name, selected, color) in [(front, "Active source", true, [0, 255, 0, 255]), (back, "Also selected", false, [255, 0, 0, 255])] {
        w.patch(slot);
        for word in [2, 2, 1] {
            w.word(word);
        }
        w.string(name);
        w.property(2, &[]);
        w.property(7, &28u32.to_be_bytes());
        w.property(8, &1u32.to_be_bytes());
        if selected {
            w.property(11, &1u32.to_be_bytes());
            w.property(12, &u32::from(target == NativeImportTarget::LayerMask).to_be_bytes());
            w.property(13, &1u32.to_be_bytes());
        }
        w.end();
        let pixels = w.slot();
        let mask = w.slot();
        w.pixels(pixels, &color.repeat(4), 4);
        if selected {
            w.patch(mask);
            w.word(2);
            w.word(2);
            w.string("Mask");
            w.end();
            let pixels = w.slot();
            w.pixels(pixels, &[255, 128, 64, 0], 1);
        }
    }
    w.patch(alpha);
    w.word(2);
    w.word(2);
    w.string("Alpha overlay");
    if matches!(target, NativeImportTarget::AlphaChannel(_)) {
        w.property(3, &[]);
    }
    w.property(8, &1u32.to_be_bytes());
    w.end();
    let pixels = w.slot();
    w.pixels(pixels, &[0, 64, 128, 255], 1);
    w.0
}

#[test]
fn later_xcf_waiting_on_an_earlier_prompt_keeps_view_and_source_policy() {
    for damage in [false, true] {
        for target in [NativeImportTarget::Composite, NativeImportTarget::LayerMask, NativeImportTarget::AlphaChannel(0)] {
            assert_pending_open_preserves_view(damage, target);
        }
    }
}

fn assert_pending_open_preserves_view(damage: bool, target: NativeImportTarget) {
    let mut app = super::identity_app();
    let unchanged = super::IdentityControl::capture(&app);
    app.session.execute("edit.colorSettings", json!({"askOnMismatch":true,"askOnMissing":false,"policyRgb":"preserve"})).unwrap();
    if damage {
        let imported = photocraft_io::import("earlier.png", &super::identity_png(None)).unwrap();
        crate::open_prompt::arrive(
            &mut app,
            crate::open_prompt::Held {
                doc: imported.document,
                read_only: false,
                initial_view: None,
                protected_import_source: None,
                damage_pending: true,
                name: "earlier.psd".into(),
                path: None,
                warnings: vec![],
                bytes: None,
                slot: None,
                order: None,
                question: None,
                policy_applied: false,
                smart_object: false,
            },
        );
    } else {
        let earlier = super::completed_identity_open(&mut app, "earlier.png", super::identity_png(craft::cms::Builtin::from_id("adobe-rgb-compat")));
        assert!(earlier["held"].is_number());
        finish_open(&mut app, "earlier.png", None, &earlier, None, None).unwrap();
    }
    let bytes = saved_view_xcf(target);
    let decoded = photocraft_io::import("later.xcf", &bytes).unwrap();
    let expected = decoded.initial_view.unwrap();
    assert_eq!(expected.target, target);
    assert!(expected.show_layer_mask);
    assert_eq!(expected.visible_alpha_channels, [0]);
    assert_eq!(expected.samplers, [[1., 1.]]);
    assert_eq!(expected.selected_layers.len(), 2);
    // Complete the actual background engine decode/apply before the UI
    // event is delivered, while the first file still owns its question.
    let folder = tempfile::tempdir().unwrap();
    let source = folder.path().join("later.xcf");
    std::fs::write(&source, &bytes).unwrap();
    let source = source.to_str().unwrap().to_owned();
    let later = match app.session.start_open_for_user_with_background("later.xcf", OpenSource::Path(source.clone()), true).unwrap() {
        Started::Job(job) => app.session.wait_job(job).unwrap(),
        Started::Done(_) => panic!("native path Open must use the background worker"),
    };
    assert!(later.get("held").is_none(), "the later file has no question of its own");
    let id = DocId(later["docId"].as_u64().unwrap());
    let before = app.session.documents().iter().find(|state| state.doc.id == id).unwrap();
    let selected = before.selection().to_vec();
    let active = before.active_layer();
    let pixels = before.doc.clone();
    assert_eq!(selected.len(), 2);
    finish_open(&mut app, "later.xcf", Some(&source), &later, None, None).unwrap();
    assert!(!app.session.documents().iter().any(|state| state.doc.id == id));
    assert_eq!(app.held_opens.len(), 2);
    let queued = app.held_opens.back().unwrap();
    assert_eq!(queued.initial_view.as_ref().unwrap().target, target);
    assert!(queued.read_only);
    assert_eq!(queued.protected_import_source.as_deref(), Some(source.as_str()));
    unchanged.assert_untouched(&app);
    // Drive the actual first dialog; its result pump admits the queued file.
    let inspected = control(&mut app, "ui.inspect", json!({}));
    let prompts = inspected["result"]["dialogs"].as_array().unwrap();
    assert_eq!(prompts.len(), 1);
    let prompt = prompts[0]["id"].clone();
    assert_eq!(control(&mut app, "ui.dialog.confirm", json!({"dialog":prompt}))["ok"], true);
    assert!(app.held_opens.is_empty());
    let state = app.session.documents().iter().find(|state| state.doc.id == id).unwrap();
    assert_readmitted_view(state, selected.as_slice(), active, target, &pixels, &source);
    unchanged.assert_untouched(&app);
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
}

fn assert_readmitted_view(state: &DocState, selected: &[LayerId], active: Option<LayerId>, target: NativeImportTarget, pixels: &Document, source: &str) {
    assert_eq!(state.selection(), selected);
    assert_eq!(state.active_layer(), active);
    assert_eq!(state.color_samplers, [[1., 1.]]);
    assert!(state.channel_view.alpha_shown(0));
    let mask = state.channel_view.layer_mask.unwrap();
    assert_eq!(mask.mode, photocraft_engine::mask_view_cmds::MaskViewMode::Gray);
    assert_eq!(mask.layer, active.unwrap());
    assert!(photocraft_engine::mask_view_cmds::editing(state).is_none(), "source preview must not become mask-edit authority");
    let want = match target {
        NativeImportTarget::Composite => photocraft_engine::channel_cmds::ChannelTarget::Composite,
        NativeImportTarget::LayerMask => photocraft_engine::channel_cmds::ChannelTarget::LayerMask,
        NativeImportTarget::AlphaChannel(index) => photocraft_engine::channel_cmds::ChannelTarget::Alpha(index),
    };
    assert_eq!(state.channel_view.target, want);
    assert_eq!(state.doc.layers, pixels.layers);
    assert!(state.source_read_only && state.path.is_none());
    assert_eq!(state.protected_import_source.as_deref(), Some(source));
    assert!(photocraft_engine::file_cmds::check_import_destination(state, source).is_err());
    assert_eq!(state.history.past_len(), 0);
    assert!(!state.is_dirty());
}

fn control(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    let (request, _reply) = crate::control::ControlRequest::new(method, params);
    let ctx = app.ctx().clone();
    let result = match crate::control::handle(app, &ctx, &request) {
        crate::control::Reply::Done(result) => result,
        _ => panic!("expected immediate control result"),
    };
    ctx.run_ui(Default::default(), |ui| crate::dialogs::run(app, ui)).textures_delta.clear();
    result
}
