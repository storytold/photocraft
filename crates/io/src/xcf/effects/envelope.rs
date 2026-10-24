//! File framing and typed arguments only; mapping is a separate allowlist.

use photocraft_raster::Interrupt;

use super::{Argument, ArgumentValue, ColorLiteral, SourceProperty, StoredEffect, parse_config};
use crate::xcf::{Cursor, MAX_PAYLOAD, MAX_STRING, Result, invalid};

const MAX_EFFECTS: usize = 1024;
const MAX_PROPERTIES: usize = 4096;

fn charge(budget: &mut usize, bytes: usize) -> Result<()> {
    *budget = budget.checked_sub(bytes).ok_or_else(|| invalid("stored effects exceed the document metadata allowance"))?;
    Ok(())
}

fn push_error(effect: &mut StoredEffect<'_>, message: String, budget: &mut usize) -> Result<()> {
    charge(budget, message.capacity().saturating_add(2 * std::mem::size_of::<String>()))?;
    effect.errors.push(message);
    Ok(())
}

/// Pointers stay in stored order. The caller establishes source execution order
/// from owner semantics rather than guessing from array order.
pub(in crate::xcf) fn read_pointers(cursor: &mut Cursor<'_>, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<Vec<usize>> {
    let mut pointers = Vec::new();
    let mut seen = std::collections::HashSet::new();
    loop {
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        let pointer = cursor.ptr()?;
        if pointer == 0 {
            return Ok(pointers);
        }
        if pointers.len() >= MAX_EFFECTS {
            return Err(invalid("more than 1,024 effects on one drawable"));
        }
        if pointer >= cursor.b.len() {
            return Err(invalid("stored effect pointer is outside the file"));
        }
        charge(budget, 4 * std::mem::size_of::<usize>())?;
        if !seen.insert(pointer) {
            return Err(invalid("repeated stored effect pointer on one drawable"));
        }
        pointers.push(pointer);
    }
}

/// Decode one bounded record. Malformed optional argument values remain inert
/// Invalid variants; truncated framing never becomes a successful import.
pub(in crate::xcf) fn decode<'a>(base: &Cursor<'a>, offset: usize, version: u32, budget: &mut usize, ctl: &Interrupt<'_>) -> Result<StoredEffect<'a>> {
    ctl.check().map_err(|_| crate::IoError::Cancelled)?;
    if version < 20 {
        return Err(invalid("stored effect envelopes are not defined before XCF 20"));
    }
    charge(budget, 2 * std::mem::size_of::<StoredEffect<'a>>())?;
    let mut cursor = base.at(offset);
    let name = string(&mut cursor)?;
    let icon = string(&mut cursor)?;
    let operation = string(&mut cursor)?;
    let stored_version = if version >= 22 { Some(string(&mut cursor)?) } else { None };
    charge(budget, name.len().saturating_add(icon.len()).saturating_add(operation.len()).saturating_add(stored_version.map_or(0, str::len)))?;
    let mut effect = StoredEffect {
        name,
        icon,
        operation,
        version: stored_version,
        arguments: Vec::new(),
        visible: true,
        opacity: 1.0,
        mode: 0,
        blend_space: 0,
        composite_space: 0,
        composite_mode: 0,
        clip: false,
        region: 0,
        mask_offset: 0,
        record_offset: offset,
        raw: &[],
        unknown_properties: Vec::new(),
        errors: Vec::new(),
    };
    let mut count = 0usize;
    loop {
        ctl.check().map_err(|_| crate::IoError::Cancelled)?;
        let property_offset = cursor.pos;
        let kind = cursor.u32()?;
        let length = cursor.u32()? as usize;
        if kind == 0 {
            if length != 0 {
                return Err(invalid("stored effect terminator has a nonzero length"));
            }
            break;
        }
        count = count.checked_add(1).ok_or_else(|| invalid("effect property count overflow"))?;
        if count > MAX_PROPERTIES {
            return Err(invalid("more than 4,096 properties on one effect"));
        }
        if length > MAX_PAYLOAD {
            return Err(invalid("stored effect property exceeds 64 MiB"));
        }
        let bytes = cursor.take(length)?;
        match kind {
            45 => {
                charge(budget, 2 * std::mem::size_of::<Argument<'a>>())?;
                let mut argument_cursor = Cursor { b: bytes, pos: 0, wide: base.wide };
                let argument_name = string(&mut argument_cursor)?;
                charge(budget, argument_name.len())?;
                let value_kind = argument_cursor.u32()?;
                let value_bytes = bytes.get(argument_cursor.pos..).ok_or_else(|| invalid("effect argument framing"))?;
                let value = match argument(&mut argument_cursor, value_kind, budget) {
                    Ok(value) => value,
                    Err(error) => {
                        let message = error.to_string();
                        charge(budget, message.capacity())?;
                        ArgumentValue::Invalid { kind: value_kind, bytes: value_bytes, message }
                    }
                };
                effect.arguments.push(Argument { name: argument_name, value });
            }
            7 | 8 | 33 | 35 | 36 | 37 | 44 | 46 => {
                let value = match bytes {
                    [a, b, c, d] => u32::from_be_bytes([*a, *b, *c, *d]),
                    _ => {
                        let message = format!("effect property {kind} is not four bytes");
                        push_error(&mut effect, message, budget)?;
                        continue;
                    }
                };
                match kind {
                    7 => effect.mode = value as i32,
                    8 if value <= 1 => effect.visible = value != 0,
                    33 => {
                        let opacity = f32::from_bits(value);
                        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
                            push_error(&mut effect, "effect opacity is not finite normalized coverage".into(), budget)?;
                        } else {
                            effect.opacity = opacity;
                        }
                    }
                    35 => effect.composite_mode = value as i32,
                    36 => effect.composite_space = value as i32,
                    37 => effect.blend_space = value as i32,
                    44 => effect.region = value,
                    46 if value <= 1 => effect.clip = value != 0,
                    _ => {
                        push_error(&mut effect, format!("effect property {kind} has invalid boolean {value}"), budget)?;
                    }
                }
            }
            _ => {
                charge(budget, 2 * std::mem::size_of::<SourceProperty<'a>>())?;
                effect.unknown_properties.push(SourceProperty { offset: property_offset, kind, bytes });
            }
        }
    }
    effect.mask_offset = cursor.ptr()?;
    if effect.mask_offset != 0 && effect.mask_offset >= base.b.len() {
        return Err(invalid("effect mask pointer is outside the file"));
    }
    effect.raw = base.b.get(offset..cursor.pos).ok_or_else(|| invalid("stored effect record range"))?;
    Ok(effect)
}

fn string<'a>(cursor: &mut Cursor<'a>) -> Result<&'a str> {
    let count = cursor.u32()? as usize;
    if count == 0 {
        return Ok("");
    }
    if count > MAX_STRING {
        return Err(invalid("effect string exceeds 1 MiB"));
    }
    let bytes = cursor.take(count)?;
    let bytes = bytes.strip_suffix(&[0]).ok_or_else(|| invalid("effect string is missing its NUL terminator"))?;
    if bytes.contains(&0) {
        return Err(invalid("effect string contains an embedded NUL"));
    }
    std::str::from_utf8(bytes).map_err(|_| invalid("effect string is not valid UTF-8"))
}

fn argument<'a>(cursor: &mut Cursor<'a>, kind: u32, budget: &mut usize) -> Result<ArgumentValue<'a>> {
    let value = match kind {
        0 => {
            let bytes = cursor.take(cursor.b.len().saturating_sub(cursor.pos))?;
            ArgumentValue::Unknown { kind, bytes }
        }
        1 => ArgumentValue::Int(cursor.u32()? as i32),
        2 => match cursor.u32()? {
            0 => ArgumentValue::Bool(false),
            1 => ArgumentValue::Bool(true),
            _ => return Err(invalid("effect argument boolean is not 0 or 1")),
        },
        3 => {
            let value = f32::from_bits(cursor.u32()?);
            if !value.is_finite() {
                return Err(invalid("effect argument float is not finite"));
            }
            ArgumentValue::Float(value)
        }
        4 => ArgumentValue::String(string(cursor)?),
        5 => ArgumentValue::Enum(cursor.u32()? as i32),
        6 => {
            let raw = string(cursor)?;
            ArgumentValue::Config(parse_config(raw, budget).map_err(invalid)?)
        }
        7 => ArgumentValue::UInt(cursor.u32()?),
        8 => {
            let encoding = string(cursor)?;
            if encoding.is_empty() {
                ArgumentValue::Color(None)
            } else {
                let count = cursor.u32()? as usize;
                if count > 256 {
                    return Err(invalid("effect color pixel exceeds 256 bytes"));
                }
                let pixels = cursor.take(count)?;
                let count = cursor.u32()? as usize;
                if count > MAX_PAYLOAD {
                    return Err(invalid("effect color ICC exceeds 64 MiB"));
                }
                let profile = cursor.take(count)?;
                ArgumentValue::Color(Some(ColorLiteral { encoding, pixels, profile }))
            }
        }
        _ => {
            let bytes = cursor.take(cursor.b.len().saturating_sub(cursor.pos))?;
            ArgumentValue::Unknown { kind, bytes }
        }
    };
    if cursor.pos != cursor.b.len() {
        return Err(invalid("effect argument has unconsumed trailing bytes"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    fn text(bytes: &mut Vec<u8>, value: &str) {
        word(bytes, value.len() as u32 + 1);
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(0);
    }
    fn property(bytes: &mut Vec<u8>, kind: u32, value: &[u8]) {
        word(bytes, kind);
        word(bytes, value.len() as u32);
        bytes.extend_from_slice(value);
    }
    fn effect(version: u32, values: &[(&str, u32, Vec<u8>)]) -> Vec<u8> {
        let mut bytes = vec![0; 8];
        text(&mut bytes, "Owner effect");
        text(&mut bytes, "");
        text(&mut bytes, "gegl:exposure");
        if version >= 22 {
            text(&mut bytes, "1:0");
        }
        for (name, kind, value) in values {
            let mut payload = Vec::new();
            text(&mut payload, name);
            word(&mut payload, *kind);
            payload.extend_from_slice(value);
            property(&mut bytes, 45, &payload);
        }
        property(&mut bytes, 8, &0u32.to_be_bytes());
        property(&mut bytes, 33, &0.375f32.to_bits().to_be_bytes());
        property(&mut bytes, 7, &62u32.to_be_bytes());
        property(&mut bytes, 36, &(-1i32).to_be_bytes());
        property(&mut bytes, 9876, &[9, 8, 7]);
        word(&mut bytes, 0);
        word(&mut bytes, 0);
        bytes.extend_from_slice(&0u64.to_be_bytes());
        bytes
    }
    fn decode_bytes(bytes: &[u8], version: u32) -> StoredEffect<'_> {
        decode(&Cursor { b: bytes, pos: 0, wide: true }, 8, version, &mut (32 << 20), &Interrupt::NONE).unwrap()
    }

    #[test]
    fn historical_version_presence_and_native_envelope_fields_are_explicit() {
        for version in [20, 21, 22, 26] {
            let bytes = effect(version, &[]);
            let parsed = decode_bytes(&bytes, version);
            assert_eq!(parsed.version, if version >= 22 { Some("1:0") } else { None });
            assert_eq!(parsed.operation, "gegl:exposure");
            assert!(!parsed.visible);
            assert_eq!(parsed.opacity, 0.375);
            assert_eq!(parsed.mode, 62);
            assert_eq!(parsed.composite_space, -1);
            assert_eq!(parsed.mask_offset, 0);
            assert_eq!(parsed.unknown_properties[0].kind, 9876);
            assert_eq!(parsed.unknown_properties[0].bytes, [9, 8, 7]);
            assert_eq!(parsed.raw, &bytes[8..]);
            assert!(parsed.errors.is_empty());
        }
    }

    #[test]
    fn every_documented_argument_type_and_unknown_bytes_remain_ordered_data() {
        let mut string_value = Vec::new();
        text(&mut string_value, "text");
        let mut config = Vec::new();
        text(&mut config, "(channel red)(gamma 1.25)(channel blue)");
        let mut color = Vec::new();
        text(&mut color, "R'G'B'A u8");
        word(&mut color, 4);
        color.extend_from_slice(&[12, 34, 56, 78]);
        word(&mut color, 3);
        color.extend_from_slice(&[1, 2, 3]);
        let bytes = effect(
            26,
            &[
                ("unknown", 0, vec![1, 2]),
                ("signed", 1, (-12i32).to_be_bytes().to_vec()),
                ("bool", 2, 1u32.to_be_bytes().to_vec()),
                ("float", 3, 2.5f32.to_bits().to_be_bytes().to_vec()),
                ("text", 4, string_value),
                ("enum", 5, (-2i32).to_be_bytes().to_vec()),
                ("config", 6, config),
                ("unsigned", 7, u32::MAX.to_be_bytes().to_vec()),
                ("color", 8, color),
                ("future", 900, vec![4, 5, 6]),
            ],
        );
        let parsed = decode_bytes(&bytes, 26);
        assert_eq!(
            parsed.arguments.iter().map(|arg| arg.name).collect::<Vec<_>>(),
            ["unknown", "signed", "bool", "float", "text", "enum", "config", "unsigned", "color", "future"]
        );
        assert_eq!(parsed.number("signed").unwrap(), Some(-12.0));
        assert_eq!(parsed.number("float").unwrap(), Some(2.5));
        assert_eq!(parsed.boolean("bool").unwrap(), Some(true));
        assert_eq!(parsed.enum_value("enum").unwrap(), Some(-2));
        assert_eq!(parsed.number("unsigned").unwrap(), Some(f64::from(u32::MAX)));
        assert_eq!(parsed.config("config").unwrap().unwrap().fields("channel").count(), 2);
        assert!(
            matches!(parsed.argument("color").unwrap(),Some(ArgumentValue::Color(Some(value))) if value.encoding=="R'G'B'A u8"&&value.pixels==[12,34,56,78]&&value.profile==[1,2,3])
        );
        assert!(matches!(parsed.argument("future").unwrap(),Some(ArgumentValue::Unknown{kind:900,bytes}) if *bytes==[4,5,6]));
    }

    #[test]
    fn invalid_optional_values_stay_inert_and_duplicate_names_are_not_ambiguous() {
        let bytes = effect(
            22,
            &[
                ("badbool", 2, 2u32.to_be_bytes().to_vec()),
                ("nan", 3, f32::NAN.to_bits().to_be_bytes().to_vec()),
                ("a", 1, 1u32.to_be_bytes().to_vec()),
                ("a", 1, 2u32.to_be_bytes().to_vec()),
                ("tail", 1, vec![0, 0, 0, 3, 99]),
            ],
        );
        let parsed = decode_bytes(&bytes, 22);
        for name in ["badbool", "nan", "tail"] {
            assert!(matches!(parsed.argument(name).unwrap(), Some(ArgumentValue::Invalid { .. })));
        }
        assert!(parsed.number("a").is_err());
    }

    #[test]
    fn all_record_truncations_and_hostile_lengths_fail_without_panicking() {
        let bytes = effect(26, &[]);
        for length in 8..bytes.len() {
            assert!(decode(&Cursor { b: &bytes[..length], pos: 0, wide: true }, 8, 26, &mut (32 << 20), &Interrupt::NONE).is_err(), "prefix {length}");
        }
        let mut huge = bytes.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode(&Cursor { b: &huge, pos: 0, wide: true }, 8, 26, &mut (32 << 20), &Interrupt::NONE).is_err());
        assert!(decode(&Cursor { b: &bytes, pos: 0, wide: true }, 8, 26, &mut 1, &Interrupt::NONE).is_err());
    }

    #[test]
    fn bounded_pointer_lists_preserve_order_and_reject_repeated_or_outside_records() {
        let mut bytes = vec![0; 64];
        bytes[..24].copy_from_slice(&[16u64.to_be_bytes(), 32u64.to_be_bytes(), 0u64.to_be_bytes()].concat());
        assert_eq!(read_pointers(&mut Cursor { b: &bytes, pos: 0, wide: true }, &mut 1024, &Interrupt::NONE).unwrap(), [16, 32]);
        bytes[8..16].copy_from_slice(&16u64.to_be_bytes());
        assert!(read_pointers(&mut Cursor { b: &bytes, pos: 0, wide: true }, &mut 1024, &Interrupt::NONE).is_err());
        let mut narrow = vec![0; 32];
        narrow[..12].copy_from_slice(&[16u32.to_be_bytes(), 24u32.to_be_bytes(), 0u32.to_be_bytes()].concat());
        assert_eq!(read_pointers(&mut Cursor { b: &narrow, pos: 0, wide: false }, &mut 1024, &Interrupt::NONE).unwrap(), [16, 24]);
        let cancelled = Interrupt::cancel_only(&|| true);
        assert!(matches!(read_pointers(&mut Cursor { b: &narrow, pos: 0, wide: false }, &mut 1024, &cancelled), Err(crate::IoError::Cancelled)));
        assert!(matches!(decode(&Cursor { b: &[], pos: 0, wide: true }, 0, 26, &mut 1024, &cancelled), Err(crate::IoError::Cancelled)));
        bytes[..8].copy_from_slice(&u64::MAX.to_be_bytes());
        assert!(read_pointers(&mut Cursor { b: &bytes, pos: 0, wide: true }, &mut 1024, &Interrupt::NONE).is_err());
    }
}
