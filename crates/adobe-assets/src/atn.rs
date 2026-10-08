//! Photoshop action files (`.atn`, version 16): one action set of recorded steps.
//!
//! Layout, as observed in files saved by Photoshop (all integers big-endian; a Unicode string is
//! a u32 count of UTF-16 code units, then the units):
//!
//! * set: version u32 (16), name (Unicode string), expanded u8, action count u32, actions;
//! * action: function-key index u16, Shift u8, Command/Ctrl u8, colour index u16, name (Unicode
//!   string), expanded u8, step count u32, steps;
//! * step: expanded u8, enabled u8, show-dialog u8, dialog options u8, then the event id as
//!   `TEXT` + length-prefixed string (`levels`, `hueSaturation`…); a length-prefixed display name;
//!   then a u32 that is 0 (no descriptor) or `0xFFFFFFFF` (an ActionDescriptor follows, without
//!   the version prefix layers and resources use).
//!
//! Also accepted, though not seen in a saved file yet: an event id stored as `long` + a
//! four-character code (the descriptor format's other id form). Descriptors are kept as raw bytes
//! (decode them with `photocraft_psd::Descriptor::from_bytes`), names as stored code units and
//! bytes, so unmodified files write back byte for byte.

use crate::error::{AssetError, Result};
use crate::io::{Reader, WriteExt};

/// The only action-file version this module reads.
pub const VERSION: u32 = 16;
/// Most actions read from one set.
pub const MAX_ACTIONS: usize = 10_000;
/// Most steps read per action.
pub const MAX_STEPS: usize = 10_000;
/// Deepest descriptor nesting walked (the bound `photocraft_psd` decodes to).
pub const MAX_DEPTH: usize = 64;

/// Marker that a descriptor follows a step's display name.
const DESCRIPTOR_PRESENT: u32 = 0xFFFF_FFFF;

/// A step's event id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventId {
    /// `TEXT`: a string id such as `levels` or `hueSaturation`.
    Text(Vec<u8>),
    /// `long`: a four-character code.
    Code([u8; 4]),
}

impl EventId {
    /// The id as text (Latin-1 for string ids, the four characters for codes).
    pub fn to_string_lossy(&self) -> String {
        let b: &[u8] = match self {
            EventId::Text(b) => b,
            EventId::Code(c) => c,
        };
        b.iter().map(|&c| char::from(c)).collect()
    }
}

/// One recorded step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtnStep {
    /// Expanded in the Actions panel.
    pub expanded: bool,
    /// The step's checkbox: disabled steps are skipped on playback.
    pub enabled: bool,
    /// The dialog toggle: playback stops in the step's dialog.
    pub dialog: bool,
    /// The dialog-options byte, kept as stored (2 in every observed step).
    pub dialog_options: u8,
    /// Event id.
    pub event: EventId,
    /// Display name as stored bytes (ASCII in every observed file).
    pub name: Vec<u8>,
    /// The step's parameters: one ActionDescriptor (no version prefix), when it records any.
    pub descriptor: Option<Vec<u8>>,
}

impl AtnStep {
    /// A step with `event`, enabled and without a dialog, a name or parameters.
    pub fn new(event: EventId) -> Self {
        AtnStep { expanded: false, enabled: true, dialog: false, dialog_options: 0, event, name: Vec::new(), descriptor: None }
    }
}

/// One action.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AtnAction {
    /// Function-key index as stored (0 in actions without a shortcut).
    pub function_key: u16,
    /// Shift modifier on the function key.
    pub shift: bool,
    /// Command (macOS) or Ctrl (Windows) modifier on the function key.
    pub command: bool,
    /// Button-mode colour index as stored (0 in actions without a colour).
    pub color: u16,
    /// Name as stored UTF-16 code units (often NUL-terminated); see [`lossy`].
    pub name: Vec<u16>,
    /// Expanded in the Actions panel.
    pub expanded: bool,
    /// Steps in playback order.
    pub steps: Vec<AtnStep>,
}

/// An action set: the content of one `.atn` file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AtnSet {
    /// Name as stored UTF-16 code units; see [`lossy`]. Actions recorded into Photoshop's Default
    /// Actions set are saved under its localization key
    /// (`$$$/Presets/Actions/DefaultActions_atn/DefaultActions=Default Actions`): the text after
    /// the `=` is the display name.
    pub name: Vec<u16>,
    /// Expanded in the Actions panel.
    pub expanded: bool,
    /// Actions in panel order.
    pub actions: Vec<AtnAction>,
}

/// Stored UTF-16 code units as text, without trailing NULs (unpaired surrogates become U+FFFD).
pub fn lossy(units: &[u16]) -> String {
    let end = units.iter().rposition(|&u| u != 0).map_or(0, |i| i + 1);
    String::from_utf16_lossy(&units[..end])
}

/// Parses a `.atn` file, which must be consumed entirely.
pub fn parse(data: &[u8]) -> Result<AtnSet> {
    let mut r = Reader::new(data);
    let version = r.u32()?;
    if version != VERSION {
        return Err(AssetError::Unsupported(format!("action file version {version} (only {VERSION} is read)")));
    }
    let name = r.unicode()?;
    let expanded = r.u8()? != 0;
    let count = r.u32()? as usize;
    if count > MAX_ACTIONS {
        return Err(AssetError::LimitExceeded("too many actions in the set"));
    }
    // An action without steps takes at least 15 bytes (empty name included).
    r.check_count(count as u64, 15)?;
    let mut actions = Vec::with_capacity(count);
    for _ in 0..count {
        actions.push(read_action(&mut r)?);
    }
    if !r.is_empty() {
        return Err(AssetError::invalid("trailing bytes after the action set"));
    }
    Ok(AtnSet { name, expanded, actions })
}

fn read_action(r: &mut Reader<'_>) -> Result<AtnAction> {
    let function_key = r.u16()?;
    let shift = r.u8()? != 0;
    let command = r.u8()? != 0;
    let color = r.u16()?;
    let name = r.unicode()?;
    let expanded = r.u8()? != 0;
    let count = r.u32()? as usize;
    if count > MAX_STEPS {
        return Err(AssetError::LimitExceeded("too many steps in an action"));
    }
    // A step without a descriptor takes at least 20 bytes (`long` id, empty name).
    r.check_count(count as u64, 20)?;
    let mut steps = Vec::with_capacity(count);
    for _ in 0..count {
        steps.push(read_step(r)?);
    }
    Ok(AtnAction { function_key, shift, command, color, name, expanded, steps })
}

fn read_step(r: &mut Reader<'_>) -> Result<AtnStep> {
    let expanded = r.u8()? != 0;
    let enabled = r.u8()? != 0;
    let dialog = r.u8()? != 0;
    let dialog_options = r.u8()?;
    let event = match &r.array::<4>()? {
        b"TEXT" => EventId::Text(r.byte_string()?.to_vec()),
        b"long" => EventId::Code(r.array::<4>()?),
        other => return Err(AssetError::InvalidSignature { expected: "TEXT or long event id", found: *other }),
    };
    let name = r.byte_string()?.to_vec();
    let descriptor = match r.u32()? {
        0 => None,
        DESCRIPTOR_PRESENT => {
            let start = r.pos();
            skip_descriptor(r, 0)?;
            Some(r.since(start).to_vec())
        }
        other => return Err(AssetError::invalid(format!("step descriptor marker {other:#x}"))),
    };
    Ok(AtnStep { expanded, enabled, dialog, dialog_options, event, name, descriptor })
}

/// A descriptor's class/key/type id: a u32 length, where 0 means a four-character code follows.
fn skip_id(r: &mut Reader<'_>) -> Result<()> {
    match r.u32()? {
        0 => r.skip(4),
        n => r.skip(n as usize),
    }
}

/// A class reference: display name, then class id.
fn skip_class(r: &mut Reader<'_>) -> Result<()> {
    r.unicode()?;
    skip_id(r)
}

/// Walks one ActionDescriptor (name, class id, item count, then key + typed value per item)
/// without building it, with the same types and nesting bound as `photocraft_psd::descriptor`.
fn skip_descriptor(r: &mut Reader<'_>, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(AssetError::LimitExceeded("descriptor nesting too deep"));
    }
    skip_class(r)?;
    let n = r.u32()?;
    // Each item needs at least 5 (1-byte string key) + 4 (type) + 1 (bool).
    r.check_count(u64::from(n), 10)?;
    for _ in 0..n {
        skip_id(r)?;
        skip_value(r, depth + 1)?;
    }
    Ok(())
}

fn skip_value(r: &mut Reader<'_>, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(AssetError::LimitExceeded("descriptor nesting too deep"));
    }
    match &r.array::<4>()? {
        b"obj " => {
            let n = r.u32()?;
            r.check_count(u64::from(n), 8)?;
            for _ in 0..n {
                skip_reference_item(r)?;
            }
        }
        b"Objc" | b"GlbO" => skip_descriptor(r, depth + 1)?,
        b"VlLs" => {
            let n = r.u32()?;
            r.check_count(u64::from(n), 4)?;
            for _ in 0..n {
                skip_value(r, depth + 1)?;
            }
        }
        b"doub" | b"comp" => r.skip(8)?,
        b"UntF" => r.skip(12)?,
        b"UnFl" => {
            r.skip(4)?;
            let n = r.u32()?;
            r.check_count(u64::from(n), 8)?;
            r.skip(n as usize * 8)?;
        }
        b"TEXT" => {
            r.unicode()?;
        }
        b"enum" => {
            skip_id(r)?;
            skip_id(r)?;
        }
        b"long" => r.skip(4)?,
        b"bool" => r.skip(1)?,
        b"type" | b"GlbC" => skip_class(r)?,
        b"alis" | b"Pth " | b"tdta" => {
            r.byte_string()?;
        }
        b"ObAr" => {
            r.skip(4)?;
            skip_descriptor(r, depth + 1)?;
        }
        other => return Err(AssetError::Unsupported(format!("descriptor OSType {:?}", String::from_utf8_lossy(other)))),
    }
    Ok(())
}

fn skip_reference_item(r: &mut Reader<'_>) -> Result<()> {
    match &r.array::<4>()? {
        b"prop" => {
            skip_class(r)?;
            skip_id(r)
        }
        b"Clss" => skip_class(r),
        b"Enmr" => {
            skip_class(r)?;
            skip_id(r)?;
            skip_id(r)
        }
        b"rele" => {
            skip_class(r)?;
            r.skip(4)
        }
        b"Idnt" | b"indx" => r.skip(4),
        b"name" => {
            skip_class(r)?;
            r.unicode().map(|_| ())
        }
        other => Err(AssetError::Unsupported(format!("reference item type {:?}", String::from_utf8_lossy(other)))),
    }
}

/// Serializes an action set as a version-16 `.atn` file.
pub fn write(set: &AtnSet) -> Vec<u8> {
    let mut out = Vec::new();
    out.put_u32(VERSION);
    out.put_unicode(&set.name);
    out.put_u8(u8::from(set.expanded));
    out.put_u32(set.actions.len() as u32);
    for a in &set.actions {
        out.put_u16(a.function_key);
        out.put_u8(u8::from(a.shift));
        out.put_u8(u8::from(a.command));
        out.put_u16(a.color);
        out.put_unicode(&a.name);
        out.put_u8(u8::from(a.expanded));
        out.put_u32(a.steps.len() as u32);
        for s in &a.steps {
            for flag in [s.expanded, s.enabled, s.dialog] {
                out.put_u8(u8::from(flag));
            }
            out.put_u8(s.dialog_options);
            match &s.event {
                EventId::Text(id) => {
                    out.extend_from_slice(b"TEXT");
                    out.put_byte_string(id);
                }
                EventId::Code(code) => {
                    out.extend_from_slice(b"long");
                    out.extend_from_slice(code);
                }
            }
            out.put_byte_string(&s.name);
            match &s.descriptor {
                None => out.put_u32(0),
                Some(d) => {
                    out.put_u32(DESCRIPTOR_PRESENT);
                    out.extend_from_slice(d);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal descriptor encoders for test data (this crate only walks descriptors).
    fn id(out: &mut Vec<u8>, s: &str) {
        if s.len() == 4 {
            out.put_u32(0);
        } else {
            out.put_u32(s.len() as u32);
        }
        out.extend_from_slice(s.as_bytes());
    }
    fn class(out: &mut Vec<u8>, name: &str, class_id: &str) {
        out.put_unicode(&name.encode_utf16().collect::<Vec<_>>());
        id(out, class_id);
    }
    fn descriptor(class_id: &str, items: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        class(&mut out, "", class_id);
        out.put_u32(items.len() as u32);
        for (key, value) in items {
            id(&mut out, key);
            out.extend_from_slice(value);
        }
        out
    }
    fn typed(ty: &[u8; 4], body: &[u8]) -> Vec<u8> {
        [&ty[..], body].concat()
    }

    /// A Levels step's parameters: `Adjs` = [LvlA { Chnl: Enmr(Chnl, Chnl, Cmps), Gmm: 1.05 }].
    fn levels() -> Vec<u8> {
        let mut channel = Vec::new();
        channel.put_u32(1);
        channel.extend_from_slice(b"Enmr");
        class(&mut channel, "", "Chnl");
        id(&mut channel, "Chnl");
        id(&mut channel, "Cmps");
        let adjustment = descriptor("LvlA", &[("Chnl", typed(b"obj ", &channel)), ("Gmm ", typed(b"doub", &1.05f64.to_be_bytes()))]);
        let mut list = Vec::new();
        list.put_u32(1);
        list.extend_from_slice(&typed(b"Objc", &adjustment));
        descriptor("null", &[("Adjs", typed(b"VlLs", &list))])
    }

    /// One item of every value type and reference form the walker knows.
    fn every_type() -> Vec<u8> {
        let mut reference = Vec::new();
        reference.put_u32(7);
        reference.extend_from_slice(b"prop");
        class(&mut reference, "", "Lyr ");
        id(&mut reference, "Bckg");
        reference.extend_from_slice(b"Clss");
        class(&mut reference, "x", "Dcmn");
        reference.extend_from_slice(b"Enmr");
        class(&mut reference, "", "Lyr ");
        id(&mut reference, "Ordn");
        id(&mut reference, "Trgt");
        reference.extend_from_slice(b"rele");
        class(&mut reference, "", "Lyr ");
        reference.put_u32(u32::MAX);
        reference.extend_from_slice(b"Idnt");
        reference.put_u32(7);
        reference.extend_from_slice(b"indx");
        reference.put_u32(2);
        reference.extend_from_slice(b"name");
        class(&mut reference, "", "Lyr ");
        reference.put_unicode(&[0x4C, 0]);
        let mut floats = b"#Pxl".to_vec();
        floats.put_u32(2);
        floats.extend_from_slice(&[0; 16]);
        let mut text = Vec::new();
        text.put_unicode(&[0x41, 0xD800, 0]);
        let mut enumerated = Vec::new();
        id(&mut enumerated, "Ordn");
        id(&mut enumerated, "artboardSection");
        let mut type_class = Vec::new();
        class(&mut type_class, "", "Lyr ");
        let mut raw = Vec::new();
        raw.put_byte_string(b"\x01\x02\x03");
        let mut object_array = Vec::new();
        object_array.put_u32(16);
        object_array.extend_from_slice(&descriptor("null", &[]));
        let nested = descriptor("Nest", &[("Bool", typed(b"bool", &[1]))]);
        descriptor(
            "null",
            &[
                ("null", typed(b"obj ", &reference)),
                ("Objc", typed(b"Objc", &nested)),
                ("GlbO", typed(b"GlbO", &nested)),
                ("doub", typed(b"doub", &0.5f64.to_be_bytes())),
                ("comp", typed(b"comp", &(-2i64).to_be_bytes())),
                ("UntF", typed(b"UntF", &[b"#Prc".as_slice(), &25.0f64.to_be_bytes()].concat())),
                ("UnFl", typed(b"UnFl", &floats)),
                ("TEXT", typed(b"TEXT", &text)),
                ("enum", typed(b"enum", &enumerated)),
                ("long", typed(b"long", &(-1i32).to_be_bytes())),
                ("type", typed(b"type", &type_class)),
                ("GlbC", typed(b"GlbC", &type_class)),
                ("alis", typed(b"alis", &raw)),
                ("Pth ", typed(b"Pth ", &raw)),
                ("tdta", typed(b"tdta", &raw)),
                ("ObAr", typed(b"ObAr", &object_array)),
            ],
        )
    }

    fn sample() -> AtnSet {
        let mut text = AtnStep::new(EventId::Text(b"levels".to_vec()));
        text.name = b"Levels".to_vec();
        text.descriptor = Some(levels());
        let mut code = AtnStep::new(EventId::Code(*b"FltI"));
        code.enabled = false;
        code.dialog = true;
        code.dialog_options = 2;
        let mut filter = AtnStep::new(EventId::Text(b"AdobeCameraRawFilter".to_vec()));
        filter.descriptor = Some(descriptor("null", &[("Ex12", typed(b"doub", &0.5f64.to_be_bytes()))]));
        let mut all = AtnStep::new(EventId::Text(b"set".to_vec()));
        all.descriptor = Some(every_type());
        let action = AtnAction {
            function_key: 3,
            shift: true,
            command: false,
            color: 2,
            name: "Tone\0".encode_utf16().collect(),
            expanded: true,
            steps: vec![text, code, filter, all],
        };
        AtnSet { name: "Set 1\0".encode_utf16().collect(), expanded: true, actions: vec![action, AtnAction::default()] }
    }

    #[test]
    fn round_trip_is_byte_stable() {
        let set = sample();
        let bytes = write(&set);
        let back = parse(&bytes).expect("parse");
        assert_eq!(back, set);
        assert_eq!(write(&back), bytes);
    }

    #[test]
    fn reads_both_event_id_forms_flags_and_names() {
        let set = parse(&write(&sample())).expect("parse");
        assert_eq!(lossy(&set.name), "Set 1");
        let action = &set.actions[0];
        assert_eq!(lossy(&action.name), "Tone");
        assert_eq!((action.function_key, action.shift, action.command, action.color), (3, true, false, 2));
        let steps = &action.steps;
        assert_eq!(steps[0].event.to_string_lossy(), "levels");
        assert!(steps[0].enabled && !steps[0].dialog);
        assert_eq!(steps[0].descriptor.as_deref(), Some(&levels()[..]));
        assert_eq!(steps[1].event, EventId::Code(*b"FltI"));
        assert!(!steps[1].enabled && steps[1].dialog);
        assert_eq!(steps[1].descriptor, None);
    }

    #[test]
    fn walks_every_descriptor_type_exactly() {
        let set = parse(&write(&sample())).expect("parse");
        // Another action follows this descriptor, so walking one byte too few or too many fails.
        assert_eq!(set.actions[0].steps[3].descriptor.as_deref(), Some(&every_type()[..]));
    }

    #[test]
    fn unknown_value_types_are_errors() {
        let mut step = AtnStep::new(EventId::Text(b"x".to_vec()));
        step.descriptor = Some(descriptor("null", &[("What", typed(b"Wat?", &[]))]));
        let set = AtnSet { actions: vec![AtnAction { steps: vec![step], ..Default::default() }], ..Default::default() };
        assert!(matches!(parse(&write(&set)), Err(AssetError::Unsupported(_))));
    }

    #[test]
    fn every_truncation_is_an_error() {
        let bytes = write(&sample());
        for n in 0..bytes.len() {
            assert!(parse(&bytes[..n]).is_err(), "prefix of {n} bytes parsed");
        }
    }

    #[test]
    fn single_byte_mutations_never_panic_and_stay_stable() {
        let bytes = write(&sample());
        for i in 0..bytes.len() {
            for flip in [0x01, 0x80, 0xFF] {
                let mut m = bytes.clone();
                m[i] ^= flip;
                if let Ok(set) = parse(&m) {
                    let b = write(&set);
                    assert_eq!(write(&parse(&b).expect("re-parse")), b, "byte {i} ^ {flip:#x}");
                }
            }
        }
    }

    #[test]
    fn rejects_bad_headers_markers_and_trailing_bytes() {
        let bytes = write(&sample());
        let mut v12 = bytes.clone();
        v12[..4].copy_from_slice(&12u32.to_be_bytes());
        assert!(matches!(parse(&v12), Err(AssetError::Unsupported(_))));

        let mut trailing = bytes;
        trailing.push(0);
        assert!(matches!(parse(&trailing), Err(AssetError::Invalid(_))));

        let mut step = AtnStep::new(EventId::Code(*b"Fltn"));
        step.name = b"x".to_vec();
        let set = AtnSet { actions: vec![AtnAction { steps: vec![step], ..Default::default() }], ..Default::default() };
        let good = write(&set);
        // The marker is the last four bytes of a step without a descriptor.
        let mut marker = good.clone();
        let at = marker.len() - 4;
        marker[at..].copy_from_slice(&1u32.to_be_bytes());
        assert!(matches!(parse(&marker), Err(AssetError::Invalid(_))));
        let mut tag = good;
        let at = tag.windows(4).position(|w| w == b"long").expect("tag");
        tag[at..at + 4].copy_from_slice(b"bool");
        assert!(matches!(parse(&tag), Err(AssetError::InvalidSignature { .. })));
    }

    #[test]
    fn huge_counts_fail_before_allocating() {
        let mut bytes = Vec::new();
        bytes.put_u32(VERSION);
        bytes.put_unicode(&[]);
        bytes.put_u8(0);
        bytes.put_u32(u32::MAX);
        assert!(matches!(parse(&bytes), Err(AssetError::LimitExceeded(_))));
        let len = bytes.len();
        bytes[len - 4..].copy_from_slice(&(MAX_ACTIONS as u32).to_be_bytes());
        assert!(matches!(parse(&bytes), Err(AssetError::UnexpectedEof { .. })));
    }

    #[test]
    fn descriptor_nesting_is_bounded() {
        let mut d = descriptor("null", &[]);
        for _ in 0..MAX_DEPTH + 2 {
            d = descriptor("null", &[("Nest", typed(b"Objc", &d))]);
        }
        let mut step = AtnStep::new(EventId::Text(b"deep".to_vec()));
        step.descriptor = Some(d);
        let set = AtnSet { actions: vec![AtnAction { steps: vec![step], ..Default::default() }], ..Default::default() };
        assert!(matches!(parse(&write(&set)), Err(AssetError::LimitExceeded(_))));
    }
}
