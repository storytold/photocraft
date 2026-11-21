//! #14429: Image > Adjustments > Color Lookup is unavailable in Grayscale, Indexed, Multichannel
//! and Bitmap documents and on a hidden pixel layer (the menu entry greyed and the command
//! answering `The command "Color Lookup" is not currently available.`), the same for a step
//! targeting a layer mask; CMYK and Lab stay enabled but fail at apply; an empty pixel layer
//! answers its own message. No refusal adds a history state or touches a pixel. RGB at 8, 16 and
//! 32 bit and a smart object (a smart filter) keep working, and the other adjustments keep today's
//! checks - Brightness/Contrast still runs on a targeted layer mask, and a step naming an alpha
//! channel or the Quick Mask in its own params edits that channel instead of the apply-time layer
//! refusals; the enabled check knows only the active layer and the Channels panel, and admission
//! agrees with it: a hidden layer merely NAMED in the params of such a step does not refuse it
//! (owner decision #14485, restoring main's path), while a hidden ACTIVE layer still greys the
//! entry and refuses the step.

use super::*;
use photocraft_color::ColorMode;
use photocraft_doc::{Color, Document, SampleType, Size};

/// Measured texts (#14429), spelled out here so the test fails if the app ever drifts.
const NOT_AVAILABLE: &str = "The command \"Color Lookup\" is not currently available.";
const EMPTY_LAYER: &str = "Could not complete the Color Lookup command because the active layer is empty.";

/// A 32 x 24 white-background document in `mode`.
fn session(mode: ColorMode) -> Session {
    let d = Document::with_background("lookup", Size::new(32, 24), mode, SampleType::U8, Color::WHITE);
    let mut s = Session::new();
    s.add_document(d, None);
    s
}

/// The Background's pixel at (16, 12) and the History depth, to prove a refusal changes neither.
fn snapshot(s: &Session) -> (Vec<f32>, usize) {
    let st = s.active().unwrap();
    let pixels = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(16, 12).to_vec();
    (pixels, st.history.past_len())
}

/// Run Color Lookup and require the measured refusal `want`, with nothing changed. The engine
/// wraps a greyed command's precondition text in `Disabled` and a run-level error in `Other`;
/// both must carry the measured sentence itself, which is what the reference dialog shows.
fn refused(s: &mut Session, case: &str, want: &str, p: Value) {
    let active = s.active().unwrap().active_layer.unwrap().0;
    refused_on_layer(s, case, want, p, active);
}

/// [`refused`] watching layer `watch` instead of the active one, for a step whose params name a
/// different target: a group or an adjustment layer, the active layer of these cases, has no
/// surface of its own to snapshot.
fn refused_on_layer(s: &mut Session, case: &str, want: &str, p: Value, watch: u64) {
    let (pixels, past) = (pixel(s, watch), s.active().unwrap().history.past_len());
    let got = match s.execute("image.adjustments.colorLookup", p) {
        Ok(v) => panic!("{case}: ran anyway: {v}"),
        Err(EngineError::Disabled(_, why)) | Err(EngineError::Other(why)) => why,
        Err(e) => panic!("{case}: refused with the wrong error kind: {e}"),
    };
    assert_eq!(got, want, "{case}: refused with the wrong text");
    let (after, past_after) = (pixel(s, watch), s.active().unwrap().history.past_len());
    assert_eq!((after.as_slice(), past_after), (pixels.as_slice(), past), "{case}: the refusal changed the document");
}

#[test]
fn unavailable_modes_are_greyed_and_refused() {
    for mode in [ColorMode::Grayscale, ColorMode::Indexed, ColorMode::Multichannel] {
        let mut s = session(mode);
        assert!(!s.is_enabled("image.adjustments.colorLookup"), "{mode:?}: the menu entry is greyed");
        refused(&mut s, &format!("{mode:?} document"), NOT_AVAILABLE, json!({"lut": "warm"}));
    }
}

#[test]
fn bitmap_keeps_the_shared_adjustments_refusal_text() {
    // Bitmap is already refused for every Image > Adjustments but Invert (PR #10545); #14429
    // only asks that this command's test suite pin it too.
    let mut s = session(ColorMode::Bitmap);
    assert!(!s.is_enabled("image.adjustments.colorLookup"), "Bitmap greys the entry");
    refused(&mut s, "Bitmap document", "not available in Bitmap mode", json!({"lut": "warm"}));
}

#[test]
fn cmyk_and_lab_stay_enabled_but_fail_at_apply() {
    for mode in [ColorMode::Cmyk, ColorMode::Lab] {
        let mut s = session(mode);
        assert!(s.is_enabled("image.adjustments.colorLookup"), "{mode:?}: the dialog opens");
        refused(&mut s, &format!("{mode:?} document, OK"), NOT_AVAILABLE, json!({"lut": "cool"}));
    }
}

#[test]
fn a_hidden_pixel_layer_is_unavailable() {
    let mut s = session(ColorMode::Rgb);
    let id = s.execute("layer.new.layer", json!({"name": "ghost"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.hideLayers", json!({"layer": id})).unwrap();
    assert!(!s.is_enabled("image.adjustments.colorLookup"), "a hidden active pixel layer greys the entry");
    refused(&mut s, "hidden layer", NOT_AVAILABLE, json!({"lut": "warm"}));
}

#[test]
fn a_step_targeting_a_layer_mask_is_unavailable() {
    let mut s = session(ColorMode::Rgb);
    // A real layer mask, so the generic mask plumbing is happy and Color Lookup's own
    // refusal is what answers.
    s.execute("layer.layerMask.fromTransparency", json!({})).unwrap();
    let before = snapshot(&s);
    let got = match s.execute("image.adjustments.colorLookup", json!({"lut": "warm", "target": "mask"})) {
        Ok(v) => panic!("the mask step ran anyway: {v}"),
        Err(EngineError::Disabled(_, why)) | Err(EngineError::Other(why)) => why,
        Err(e) => panic!("wrong error kind: {e}"),
    };
    assert_eq!(got, NOT_AVAILABLE, "a targeted layer mask answers the measured text");
    assert_eq!(snapshot(&s), before, "and neither layer nor mask pixels changed");
    // The other adjustments keep today's behaviour: Brightness/Contrast still runs on the mask.
    assert!(
        s.execute("image.adjustments.brightnessContrast", json!({"target": "mask", "brightness": 10})).is_ok(),
        "Brightness/Contrast keeps the mask target"
    );
}

#[test]
fn an_empty_pixel_layer_gets_its_own_message() {
    let mut s = session(ColorMode::Rgb);
    s.execute("layer.new.layer", json!({"name": "empty"})).unwrap();
    let st = s.active().unwrap();
    let empty_is_active = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().is_some_and(|surf| surf.content_bounds().is_empty());
    assert!(empty_is_active, "layer.new.layer leaves an empty pixel layer active");
    assert!(s.is_enabled("image.adjustments.colorLookup"), "the menu entry stays enabled");
    refused(&mut s, "empty layer", EMPTY_LAYER, json!({"lut": "warm"}));
}

#[test]
fn rgb_at_every_depth_and_a_smart_object_keep_working() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let d = Document::with_background("rgb", Size::new(16, 12), ColorMode::Rgb, depth, Color::WHITE);
        let mut s = Session::new();
        s.add_document(d, None);
        assert!(s.is_enabled("image.adjustments.colorLookup"), "{depth:?}: enabled");
        let before = s.active().unwrap().history.past_len();
        s.execute("image.adjustments.colorLookup", json!({"lut": "warm"})).unwrap_or_else(|e| panic!("{depth:?}: {e}"));
        assert_eq!(s.active().unwrap().history.past_len(), before + 1, "{depth:?}: one history step");
    }
    let mut s = session(ColorMode::Rgb);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    s.execute("image.adjustments.colorLookup", json!({"lut": "warm"})).unwrap();
}

#[test]
fn a_targeted_alpha_channel_keeps_todays_behaviour_on_a_hidden_or_empty_layer() {
    // Not measured, so it must not pick up the layer refusals: the layer is not what is edited.
    let mut s = session(ColorMode::Rgb);
    let bg = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.hideLayers", json!({"layer": bg.0})).unwrap();
    s.execute("channel.new", json!({"name": "A", "fill": "white"})).unwrap();
    assert!(s.is_enabled("image.adjustments.colorLookup"), "a hidden layer does not grey a channel edit");
    s.execute("image.adjustments.colorLookup", json!({"lut": "warm"})).expect("a channel edit on a hidden layer runs");
    let mut s = session(ColorMode::Rgb);
    s.execute("layer.new.layer", json!({"name": "empty"})).unwrap();
    s.execute("channel.new", json!({"name": "A", "fill": "white"})).unwrap();
    s.execute("image.adjustments.colorLookup", json!({"lut": "warm"})).expect("a channel edit on an empty layer runs");
}

#[test]
fn an_explicit_alpha_channel_target_ignores_an_empty_active_layer() {
    // Review #14485: the channel exemption must come from the step's own params as well as from the
    // Channels panel. A step naming `{"channel": 0}` edits that alpha channel, so the empty-layer
    // refusal must not stand in its way: the same step changed the channel before #14429.
    let mut s = session(ColorMode::Rgb);
    s.execute("channel.new", json!({"name": "A", "fill": "white"})).unwrap();
    s.execute("channel.target", json!({"channel": "composite"})).unwrap();
    let empty = s.execute("layer.new.layer", json!({"name": "empty"})).unwrap()["layer"].as_u64().unwrap();
    let past = s.active().unwrap().history.past_len();
    let before_channel = s.active().unwrap().doc.channels[0].surface.sample_channel(16, 12, 0);
    s.execute("image.adjustments.colorLookup", json!({"lut": "warm", "target": {"channel": 0}}))
        .expect("a step naming an alpha channel runs although the active layer is empty");
    assert_eq!(s.active().unwrap().history.past_len(), past + 1, "one history step");
    let st = s.active().unwrap();
    assert!(st.doc.layer(photocraft_doc::LayerId(empty)).unwrap().surface().unwrap().content_bounds().is_empty(), "the empty layer was not the edit target");
    assert_ne!(st.doc.channels[0].surface.sample_channel(16, 12, 0), before_channel, "the warm LUT moves the filled channel's value: the step really wrote it");
    assert_eq!(st.doc.channels[0].surface.content_bounds(), st.doc.bounds(), "the alpha channel is what the step wrote");
}

#[test]
fn a_hidden_active_layer_greys_the_entry_even_for_an_explicit_channel_target() {
    // Pinned for review #14485, the other half of the same input: the enabled check takes no
    // params, so it only knows the Channels panel. A step naming its channel target in its params
    // still meets the measured hidden-layer refusal, because the entry is greyed for the hidden
    // active layer. Both halves are asserted: the availability query is false and the executed step
    // answers the same text (wrapped as `Disabled`, from the precondition, not from the apply check).
    // Letting the params exempt it would mean changing `Session::precondition`,
    // outside the files #14429 names.
    let mut s = session(ColorMode::Rgb);
    let bg = s.active().unwrap().active_layer.unwrap();
    s.execute("channel.new", json!({"name": "A", "fill": "white"})).unwrap();
    s.execute("channel.target", json!({"channel": "composite"})).unwrap();
    s.execute("layer.hideLayers", json!({"layer": bg.0})).unwrap();
    let step = json!({"lut": "warm", "target": {"channel": 0}});
    assert!(!s.is_enabled_with("image.adjustments.colorLookup", &step), "the enable check follows the hidden active layer");
    refused(&mut s, "hidden active layer, channel in the params", NOT_AVAILABLE, step);
}

#[test]
fn a_channel_step_naming_a_hidden_layer_reaches_the_channel_edit() {
    // Owner decision on #14485 (option A): main's path is restored. A visible filled "top" layer
    // is active, the Background is hidden, the step names the alpha channel at index 0 as its
    // target and the Background as its `layer`, and the Channels panel shows the composite. The
    // step edits the channel and never touches the layer it names, so the hidden Background must
    // not refuse it: `Session::explicit_layer_enabled` steps aside for a Color Lookup step whose
    // own params name a channel target, admission agrees with `is_enabled_with` (which only knows
    // the active layer), and the step runs as it did on main. The named hidden layer and the
    // visible active layer both stay untouched; the channel is what the edit writes.
    let (mut s, bg, top) = bg_and_top(true);
    s.execute("layer.hideLayers", json!({"layer": bg})).unwrap();
    s.execute("channel.new", json!({"name": "A", "fill": "white"})).unwrap();
    s.execute("channel.target", json!({"channel": "composite"})).unwrap();
    let step = json!({"lut": "warm", "layer": {"background": true}, "target": {"channel": 0}});
    assert!(s.is_enabled_with("image.adjustments.colorLookup", &step), "the availability query says the step can run");
    let (before_bg, before_top, past) = (pixel(&s, bg), pixel(&s, top), s.active().unwrap().history.past_len());
    let before_channel = s.active().unwrap().doc.channels[0].surface.sample_channel(16, 12, 0);
    s.execute("image.adjustments.colorLookup", step).expect("a channel step naming a hidden layer runs, as it did on main");
    assert_eq!(s.active().unwrap().history.past_len(), past + 1, "one history step");
    assert_eq!(pixel(&s, bg), before_bg, "the hidden layer named in the params stays untouched");
    assert_eq!(pixel(&s, top), before_top, "the visible active layer stays untouched: the step edits the channel");
    let st = s.active().unwrap();
    assert_ne!(st.doc.channels[0].surface.sample_channel(16, 12, 0), before_channel, "the warm LUT moves the filled channel's value: the step really wrote it");
    assert_eq!(st.doc.channels[0].surface.content_bounds(), st.doc.bounds(), "the alpha channel is what the step wrote");
}

#[test]
fn a_quick_mask_step_naming_a_hidden_layer_reaches_the_channel_path() {
    // The Quick Mask half of the owner's decision on #14485: the same shape, with
    // `"target": "quickMask"` naming the channel target in the params and the hidden Background
    // as `layer`, is admitted and reaches the channel path, which answers its own text because
    // no Quick Mask is active - as on main. The hidden-layer refusal is not what answers here,
    // and the named hidden layer stays untouched.
    let (mut s, bg, _top) = bg_and_top(true);
    s.execute("layer.hideLayers", json!({"layer": bg})).unwrap();
    let step = json!({"lut": "warm", "layer": {"background": true}, "target": "quickMask"});
    assert!(s.is_enabled_with("image.adjustments.colorLookup", &step), "the availability query says the step can run");
    let before = pixel(&s, bg);
    let got = match s.execute("image.adjustments.colorLookup", step) {
        Ok(v) => panic!("the Quick Mask step reported success without a Quick Mask: {v}"),
        Err(EngineError::Other(why)) => why,
        Err(e) => panic!("wrong error kind: {e}"),
    };
    assert_eq!(got, "not in Quick Mask mode", "the channel path answers, not the hidden-layer refusal");
    assert_eq!(pixel(&s, bg), before, "the hidden layer named in the params stays untouched");
}

#[test]
fn a_step_naming_a_hidden_layer_edits_a_real_quick_mask() {
    // Review #14485 follow-up: the same shape with an actual Quick Mask (entered with a
    // selection). The step names the hidden Background as `layer` and the Quick Mask as its
    // target, so the mask is what the edit writes: its value moves where the selection was,
    // and the hidden named layer (and the visible one) stay untouched.
    let (mut s, bg, top) = bg_and_top(true);
    s.execute("layer.hideLayers", json!({"layer": bg})).unwrap();
    s.execute("select.rect", json!({"x": 4, "y": 4, "width": 16, "height": 12, "mode": "replace"})).unwrap();
    s.execute("select.editInQuickMaskMode", json!({"on": true})).unwrap();
    let step = json!({"lut": "warm", "layer": {"background": true}, "target": "quickMask"});
    assert!(s.is_enabled_with("image.adjustments.colorLookup", &step), "with a real Quick Mask the step is available too");
    let (before_bg, before_top, past) = (pixel(&s, bg), pixel(&s, top), s.active().unwrap().history.past_len());
    let before_mask = s.active().unwrap().doc.quick_mask.as_ref().expect("Quick Mask mode is on").surface.sample_channel(16, 12, 0);
    s.execute("image.adjustments.colorLookup", step).expect("the step edits the Quick Mask, as on main");
    assert_eq!(s.active().unwrap().history.past_len(), past + 1, "one history step");
    let st = s.active().unwrap();
    assert_ne!(st.doc.quick_mask.as_ref().unwrap().surface.sample_channel(16, 12, 0), before_mask, "the Quick Mask is what the step wrote");
    assert_eq!(pixel(&s, bg), before_bg, "the hidden layer named in the params stays untouched");
    assert_eq!(pixel(&s, top), before_top, "the visible active layer stays untouched too");
}

#[test]
fn a_channel_step_naming_a_missing_group_or_malformed_layer_runs_as_on_main() {
    // Shape honesty for the owner's decision (#14485): the admission hook ignores the
    // `"layer"` param for a channel-target step, and upstream/main did the same for ALL of
    // those shapes (independent review checked at c1f1b4290) - a name no document has, a
    // group, or a value that is not a layer selector at all: the step edits the channel and
    // nothing on its path resolves the named value. Pin the three shapes: each runs, writes
    // the channel, and touches no layer.
    let (mut s, bg, top) = bg_and_top(true);
    s.execute("layer.hideLayers", json!({"layer": bg})).unwrap();
    s.execute("channel.new", json!({"name": "A", "fill": "white"})).unwrap();
    s.execute("channel.target", json!({"channel": "composite"})).unwrap();
    let group = s.execute("layer.new.group", json!({"name": "G"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layer": top})).unwrap();
    for (case, layer) in [("a missing name", json!("NoSuchLayer")), ("a group", json!(group)), ("a malformed value", json!(true))] {
        let past = s.active().unwrap().history.past_len();
        let (before_bg, before_top) = (pixel(&s, bg), pixel(&s, top));
        let before_channel = s.active().unwrap().doc.channels[0].surface.sample_channel(16, 12, 0);
        let step = json!({"lut": "warm", "layer": layer, "target": {"channel": 0}});
        s.execute("image.adjustments.colorLookup", step).unwrap_or_else(|e| panic!("{case}: the step runs, as on main: {e}"));
        assert_eq!(s.active().unwrap().history.past_len(), past + 1, "{case}: one history step");
        assert_eq!(pixel(&s, bg), before_bg, "{case}: the hidden Background stays untouched");
        assert_eq!(pixel(&s, top), before_top, "{case}: the filled active layer stays untouched");
        assert_ne!(s.active().unwrap().doc.channels[0].surface.sample_channel(16, 12, 0), before_channel, "{case}: the channel is what the run wrote");
    }
}

#[test]
fn precheck_agrees_with_execute_for_a_channel_step_naming_any_layer() {
    // Shape honesty for the owner's decision (#14485), second half: `Session::precheck`'s
    // named-layer existence check (what the Actions panel and other precheck callers see)
    // refused shapes `execute` accepts - a Color Lookup step whose params name a channel
    // ignores its `"layer"` param at admission, so precheck must ignore it too. Pin the
    // agreement: all three shapes pass precheck, the availability query and execute alike.
    let (mut s, _bg, top) = bg_and_top(true);
    s.execute("channel.new", json!({"name": "A", "fill": "white"})).unwrap();
    s.execute("channel.target", json!({"channel": "composite"})).unwrap();
    let group = s.execute("layer.new.group", json!({"name": "G"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layer": top})).unwrap();
    for (case, layer) in [("a missing name", json!("NoSuchLayer")), ("a group", json!(group)), ("a malformed value", json!(true))] {
        let step = json!({"lut": "warm", "layer": layer, "target": {"channel": 0}});
        s.precheck("image.adjustments.colorLookup", &step).unwrap_or_else(|e| panic!("{case}: precheck refuses a shape execute accepts: {e}"));
        assert!(s.is_enabled_with("image.adjustments.colorLookup", &step), "{case}: the availability query agrees");
        s.execute("image.adjustments.colorLookup", step).unwrap_or_else(|e| panic!("{case}: execute runs it: {e}"));
    }
}

#[test]
fn an_explicit_quick_mask_target_over_a_visible_layer_answers_its_own_message() {
    // The other half of the same rule, with a VISIBLE active layer: `"target": "quickMask"` names
    // the Quick Mask, so Color Lookup's own layer refusals step aside and the channel path answers
    // with its text. Over a hidden ACTIVE layer the enabled check answers first (see
    // a_hidden_active_layer_refuses_an_explicit_quick_mask_target_too and
    // a_hidden_active_layer_greys_the_entry_even_for_an_explicit_channel_target); a hidden layer
    // merely NAMED in the params does not stop the step
    // (a_quick_mask_step_naming_a_hidden_layer_reaches_the_channel_path).
    let mut s = session(ColorMode::Rgb);
    s.execute("layer.new.layer", json!({"name": "empty"})).unwrap();
    let got = match s.execute("image.adjustments.colorLookup", json!({"lut": "warm", "target": "quickMask"})) {
        Ok(v) => panic!("the Quick Mask step ran anyway: {v}"),
        Err(EngineError::Other(why)) => why,
        Err(e) => panic!("wrong error kind: {e}"),
    };
    assert_eq!(got, "not in Quick Mask mode", "the channel path answers, not the layer refusal");
}

#[test]
fn a_hidden_active_layer_refuses_an_explicit_quick_mask_target_too() {
    // The hidden-layer half of the Quick Mask shape: over a hidden active layer the enabled check
    // answers the unavailable text, so the step never reaches the channel path. Pinned as today's
    // behaviour, not measured by #14429 (its Quick Mask case is listed as not measured).
    let mut s = session(ColorMode::Rgb);
    let bg = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.hideLayers", json!({"layer": bg.0})).unwrap();
    let step = json!({"lut": "warm", "target": "quickMask"});
    assert!(!s.is_enabled_with("image.adjustments.colorLookup", &step), "the enabled check answers first");
    refused(&mut s, "hidden active layer, Quick Mask in the params", NOT_AVAILABLE, step);
}

#[test]
fn a_step_targeting_a_layer_mask_stays_available_and_answers_at_apply() {
    // Measured (#14429): with a layer mask targeted the reference runs the command and answers the
    // unavailable text, so the entry is NOT greyed for the mask; the refusal comes from
    // `color_lookup_applicable` at apply. Pinned so the two checks cannot drift apart silently:
    // the enable half for a mask target lives in `channel_cmds::mask_target_enabled`, outside the
    // files #14429 names.
    let mut s = session(ColorMode::Rgb);
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let mask_step = json!({"lut": "warm", "target": "mask"});
    assert!(s.is_enabled_with("image.adjustments.colorLookup", &mask_step), "a mask-targeted step stays available, as the reference runs it");
    refused(&mut s, "mask target through the enable check", NOT_AVAILABLE, mask_step);
}

#[test]
fn a_channel_target_with_no_layer_selected_still_edits_the_channel() {
    // Review #14485 round 2: the apply check must not resolve a layer for a channel target. A
    // document with no layer selected (`select.deselectLayers`) and the Channels panel on an alpha
    // channel: the menu accepts the step, so apply must not answer `no active layer` before the
    // channel branch runs - the step edited the channel there before #14429.
    let mut s = session(ColorMode::Rgb);
    s.execute("channel.new", json!({"name": "A", "fill": "white"})).unwrap();
    s.execute("select.deselectLayers", json!({})).unwrap();
    assert!(s.active().unwrap().active_layer.is_none(), "no layer is selected");
    assert!(s.is_enabled("image.adjustments.colorLookup"), "a channel edit with no selected layer stays enabled");
    let past = s.active().unwrap().history.past_len();
    s.execute("image.adjustments.colorLookup", json!({"lut": "warm"})).expect("the alpha channel is edited, no layer is involved");
    assert_eq!(s.active().unwrap().history.past_len(), past + 1, "one history step");
}

#[test]
fn a_channel_target_only_in_the_params_still_needs_a_selected_layer_as_on_main() {
    // The same document with the Channels panel put back on the composite: the params-level channel
    // target has no selected layer to check, and main refused it in the plain adjustability check.
    // It keeps that answer and text, and does not pick up Color Lookup's own refusal.
    let mut s = session(ColorMode::Rgb);
    s.execute("channel.new", json!({"name": "A", "fill": "white"})).unwrap();
    s.execute("channel.target", json!({"channel": "composite"})).unwrap();
    s.execute("select.deselectLayers", json!({})).unwrap();
    assert!(s.active().unwrap().active_layer.is_none(), "no layer is selected");
    let step = json!({"lut": "warm", "target": {"channel": 0}});
    assert!(!s.is_enabled_with("image.adjustments.colorLookup", &step), "the plain adjustability check still needs a layer here");
    let got = match s.execute("image.adjustments.colorLookup", step) {
        Ok(v) => panic!("the step ran anyway: {v}"),
        Err(EngineError::Disabled(_, why)) | Err(EngineError::Other(why)) => why,
        Err(e) => panic!("wrong error kind: {e}"),
    };
    assert_eq!(got, "no active layer", "the same answer as before #14429");
}

#[test]
fn an_explicit_channel_target_still_answers_the_document_mode_refusal() {
    // The channel exemption is a rule about the target, not a licence to edit a document the
    // reference marks unavailable: the mode refusals come first, whether the Channels panel or the
    // params name the channel.
    let mut s = session(ColorMode::Grayscale);
    s.execute("channel.new", json!({"name": "A", "fill": "white"})).unwrap();
    s.execute("channel.target", json!({"channel": "composite"})).unwrap();
    refused(&mut s, "channel target in a Grayscale document", NOT_AVAILABLE, json!({"lut": "warm", "target": {"channel": 0}}));
}

#[test]
fn a_selection_still_applies_inside_it_with_one_history_step() {
    let mut s = session(ColorMode::Rgb);
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 16, "height": 24, "mode": "replace"})).unwrap();
    let before = s.active().unwrap().history.past_len();
    s.execute("image.adjustments.colorLookup", json!({"lut": "warm"})).expect("Color Lookup runs with a selection");
    assert_eq!(s.active().unwrap().history.past_len(), before + 1, "one history step");
    let st = s.active().unwrap();
    let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    assert_ne!(surf.rgba(4, 12).to_vec(), surf.rgba(28, 12).to_vec(), "inside the selection changed, outside did not");
}

/// The Background (white, so not empty) and a "top" layer above it, which is the active one. The
/// bucket floods "top" when `fill` is set, otherwise it stays an empty pixel layer. Returns the
/// session with both layer ids.
fn bg_and_top(fill: bool) -> (Session, u64, u64) {
    let mut s = session(ColorMode::Rgb);
    let bg = s.active().unwrap().active_layer.unwrap().0;
    let top = s.execute("layer.new.layer", json!({"name": "top"})).unwrap()["layer"].as_u64().unwrap();
    if fill {
        s.execute("paint.bucket", json!({"x": 5, "y": 5})).unwrap();
    }
    (s, bg, top)
}

/// The pixel at (16, 12) of layer `id`.
fn pixel(s: &Session, id: u64) -> Vec<f32> {
    s.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().surface().unwrap().rgba(16, 12).to_vec()
}

#[test]
fn a_hidden_explicit_target_is_refused_although_the_active_layer_is_visible_and_not_empty() {
    // The target the params name is what Color Lookup edits, so it is the one checked: the
    // active layer ("top": visible, filled) must not stand in for it (#14429).
    let (mut s, bg, top) = bg_and_top(true);
    s.execute("layer.hideLayers", json!({"layer": bg})).unwrap();
    assert_eq!(s.active().unwrap().active_layer.unwrap().0, top);
    let before = pixel(&s, bg);
    refused(&mut s, "hidden explicit target", NOT_AVAILABLE, json!({"lut": "warm", "layer": bg}));
    assert_eq!(pixel(&s, bg), before, "the hidden target was not edited");
}

#[test]
fn an_empty_active_layer_does_not_reject_a_nonempty_explicit_target() {
    let (mut s, bg, top) = bg_and_top(false);
    assert_eq!(s.active().unwrap().active_layer.unwrap().0, top);
    let (before, past) = (pixel(&s, bg), s.active().unwrap().history.past_len());
    s.execute("image.adjustments.colorLookup", json!({"lut": "warm", "layer": bg})).expect("a visible, filled target runs while the active layer is empty");
    assert_ne!(pixel(&s, bg), before, "the targeted layer was edited");
    assert_eq!(s.active().unwrap().history.past_len(), past + 1, "one history step");
    assert_eq!(s.active().unwrap().active_layer.unwrap().0, top, "the active layer is unchanged");
}

#[test]
fn a_hidden_active_layer_does_not_reject_a_visible_explicit_target() {
    let (mut s, bg, top) = bg_and_top(true);
    s.execute("layer.hideLayers", json!({"layer": top})).unwrap();
    let before = pixel(&s, bg);
    s.execute("image.adjustments.colorLookup", json!({"lut": "warm", "layer": bg})).expect("a visible target runs while the active layer is hidden");
    assert_ne!(pixel(&s, bg), before, "the targeted layer was edited");
    assert!(!s.is_enabled("image.adjustments.colorLookup"), "the menu entry still follows the active layer");
}

#[test]
fn an_empty_explicit_target_gets_the_empty_layer_message_although_the_active_layer_is_filled() {
    let (mut s, bg, top) = bg_and_top(false);
    s.execute("layer.select", json!({"layer": bg})).unwrap();
    let before = pixel(&s, bg);
    refused(&mut s, "empty explicit target", EMPTY_LAYER, json!({"lut": "warm", "layer": top}));
    assert_eq!(pixel(&s, bg), before, "the filled active layer was not edited");
}

/// The Background, a "top" pixel layer (flooded with the bucket when `fill`), and an empty group
/// above them. Creating a group makes it the active layer, so this is the Layers panel state the
/// reference still runs Color Lookup from when the step names a pixel layer (#14429).
fn top_and_group(fill: bool) -> (Session, u64, u64, u64) {
    let (mut s, bg, top) = bg_and_top(fill);
    let group = s.execute("layer.new.group", json!({"name": "G"})).unwrap()["layer"].as_u64().unwrap();
    (s, bg, top, group)
}

#[test]
fn a_group_active_layer_does_not_reject_a_visible_nonempty_explicit_raster_target() {
    // The apply-time raster check runs on the layer the params name, not on the active one: the
    // menu entry still greys for the group, but the step editing "top" is valid (#14429).
    let (mut s, bg, top, group) = top_and_group(true);
    assert_eq!(s.active().unwrap().active_layer.unwrap().0, group, "the new group is the active layer");
    assert!(!s.is_enabled("image.adjustments.colorLookup"), "the menu entry still follows the active layer");
    let (before, before_bg, past) = (pixel(&s, top), pixel(&s, bg), s.active().unwrap().history.past_len());
    s.execute("image.adjustments.colorLookup", json!({"lut": "warm", "layer": top})).expect("a visible, filled raster target runs while a group is active");
    assert_ne!(pixel(&s, top), before, "the targeted layer was edited");
    assert_eq!(pixel(&s, bg), before_bg, "the Background was left alone");
    assert_eq!(s.active().unwrap().history.past_len(), past + 1, "one history step");
    assert_eq!(s.active().unwrap().active_layer.unwrap().0, group, "the active layer is unchanged");
}

#[test]
fn an_active_adjustment_layer_does_not_reject_a_visible_nonempty_explicit_raster_target() {
    let (mut s, bg, top) = bg_and_top(true);
    let adj = s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap()["layer"].as_u64().unwrap();
    assert_eq!(s.active().unwrap().active_layer.unwrap().0, adj, "the new adjustment layer is the active one");
    assert!(!s.is_enabled("image.adjustments.colorLookup"), "the menu entry still follows the active layer");
    let (before, before_bg, past) = (pixel(&s, top), pixel(&s, bg), s.active().unwrap().history.past_len());
    s.execute("image.adjustments.colorLookup", json!({"lut": "warm", "layer": top}))
        .expect("a visible, filled raster target runs while an adjustment layer is active");
    assert_ne!(pixel(&s, top), before, "the targeted layer was edited");
    assert_eq!(pixel(&s, bg), before_bg, "the Background was left alone");
    assert_eq!(s.active().unwrap().history.past_len(), past + 1, "one history step");
    assert_eq!(s.active().unwrap().active_layer.unwrap().0, adj, "the active layer is unchanged");
}

#[test]
fn a_group_active_layer_does_not_reject_an_explicit_smart_object_target() {
    // The other half of the same check: a smart object with its rendered cache is adjustable too,
    // and its adjustment records a smart filter instead of baking pixels (#14429).
    let (mut s, _bg, _top) = bg_and_top(true);
    let smart = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
    let group = s.execute("layer.new.group", json!({"name": "G"})).unwrap()["layer"].as_u64().unwrap();
    assert_eq!(s.active().unwrap().active_layer.unwrap().0, group, "the new group is the active layer");
    let past = s.active().unwrap().history.past_len();
    s.execute("image.adjustments.colorLookup", json!({"lut": "warm", "layer": smart})).expect("a smart object target runs while a group is active");
    assert_eq!(s.active().unwrap().history.past_len(), past + 1, "one history step");
    let photocraft_doc::LayerContent::Smart(sm) = &s.active().unwrap().doc.layer(photocraft_doc::LayerId(smart)).unwrap().content else {
        panic!("the target is still a smart object");
    };
    assert_eq!(sm.smart_filters.len(), 1, "the adjustment became the smart filter");
    assert_eq!(sm.smart_filters[0].command, "image.adjustments.colorLookup", "Color Lookup itself");
}

#[test]
fn a_group_active_layer_keeps_the_measured_refusal_for_a_hidden_explicit_target() {
    // The target stays the thing that is checked: the group does not add its own complaint.
    let (mut s, bg, _top, _group) = top_and_group(true);
    s.execute("layer.hideLayers", json!({"layer": bg})).unwrap();
    refused_on_layer(&mut s, "hidden target, group active", NOT_AVAILABLE, json!({"lut": "warm", "layer": bg}), bg);
}

#[test]
fn a_group_active_layer_keeps_the_empty_layer_message_for_an_empty_explicit_target() {
    let (mut s, _bg, top, _group) = top_and_group(false);
    refused_on_layer(&mut s, "empty target, group active", EMPTY_LAYER, json!({"lut": "warm", "layer": top}), top);
}

#[test]
fn an_explicit_group_target_is_still_refused_while_a_raster_layer_is_active() {
    // The mirror case, unchanged by the fix: a group is not adjustable, named by the params or not.
    let (mut s, _bg, top) = bg_and_top(true);
    let group = s.execute("layer.new.group", json!({"name": "G"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layer": top})).unwrap();
    assert_eq!(s.active().unwrap().active_layer.unwrap().0, top, "the filled pixel layer is the active one");
    let want = "active layer is a Group layer, not a pixel layer";
    refused_on_layer(&mut s, "group as the explicit target", want, json!({"lut": "warm", "layer": group}), top);
}
