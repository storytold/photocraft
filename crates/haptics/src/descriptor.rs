//! The platform-independent half: reading a HID report descriptor for a Simple Haptic
//! Controller (Haptics page 0x0E), decoding its Waveform List feature report and building the
//! output report that plays a waveform.
//!
//! References:
//!  - HID Usage Tables, Haptics page (0x0E)
//!  - https://learn.microsoft.com/en-us/windows-hardware/design/component-guidelines/input-haptics-implementation-guide
//!
//! The descriptor comes from a device, so it is treated as hostile: every read is
//! bounds checked and every field size is capped.

use std::collections::HashMap;

pub const HAPTICS_USAGE_PAGE: u16 = 0x0E;
pub const SIMPLE_HAPTIC_CONTROLLER_USAGE: u16 = 0x01;

const USAGE_WAVEFORM_LIST: u16 = 0x10;
const USAGE_MANUAL_TRIGGER: u16 = 0x21;
const USAGE_INTENSITY: u16 = 0x23;
const USAGE_WAVEFORM_CUTOFF_TIME: u16 = 0x28;

/// Ordinals 1 and 2 are implicitly None and Stop; the Waveform List starts at 3.
const FIRST_WAVEFORM_ORDINAL: u32 = 3;
/// Waveform usages below this are None (0x1001) and Stop (0x1002).
const FIRST_REAL_WAVEFORM: u32 = 0x1003;
/// Upper end of the standard and vendor waveform range on the Haptics page.
const LAST_REAL_WAVEFORM: u32 = 0x2FFF;

/// Fields wider than this are not something we know how to fill.
const MAX_FIELD_BITS: u32 = 32;
/// HID reports over full-speed USB and Bluetooth are far below this.
const MAX_REPORT_BITS: u32 = 8 * 512;
/// Stops pathological descriptors from growing our state.
const MAX_COLLECTION_DEPTH: usize = 32;
const MAX_LOCAL_USAGES: usize = 256;

/// Where a value lives inside a report, in bits after the report ID byte.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BitField {
    pub offset: u32,
    pub size: u32,
}

/// Everything needed to fire a waveform on one Simple Haptic Controller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HapticsLayout {
    /// Output report that carries Manual Trigger.
    pub trigger_report_id: u8,
    /// Total size of that output report, in bits, without the report ID.
    pub trigger_report_bits: u32,
    pub trigger: BitField,
    pub intensity: Option<BitField>,
    pub intensity_max: i64,
    pub cutoff: Option<BitField>,
    pub cutoff_min: i64,
    /// Feature report that carries the Waveform List.
    pub waveform_report_id: u8,
    /// One field per ordinal, starting at ordinal 3.
    pub waveform_fields: Vec<BitField>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Usage {
    page: u16,
    id: u16,
}

#[derive(Default)]
struct GlobalState {
    usage_page: u16,
    logical_min: i64,
    logical_max: i64,
    report_size: u32,
    report_count: u32,
    report_id: u8,
}

#[derive(Default)]
struct PartialLayout {
    trigger: Option<(u8, BitField)>,
    intensity: Option<(u8, BitField, i64)>,
    cutoff: Option<(u8, BitField, i64)>,
    waveforms: Option<(u8, Vec<BitField>)>,
}

/// Reads the layout of a Simple Haptic Controller out of its report descriptor.
/// Returns `None` when the device can't be driven this way.
pub fn parse_haptics_descriptor(descriptor: &[u8]) -> Option<HapticsLayout> {
    let mut global = GlobalState::default();
    let mut local_usages: Vec<Usage> = Vec::new();
    let mut collections: Vec<Usage> = Vec::new();
    let mut output_bits: HashMap<u8, u32> = HashMap::new();
    let mut feature_bits: HashMap<u8, u32> = HashMap::new();
    let mut found = PartialLayout::default();

    let mut i = 0usize;
    while i < descriptor.len() {
        let prefix = descriptor[i];
        i += 1;

        if prefix == 0xFE {
            // Long item: data size, long tag, then data. Nothing we need lives in one.
            let data_size = *descriptor.get(i)? as usize;
            i = i.checked_add(2)?.checked_add(data_size)?;
            continue;
        }

        let size = match prefix & 0x03 {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        let bytes = descriptor.get(i..i.checked_add(size)?)?;
        i += size;
        let unsigned = read_unsigned(bytes);
        let signed = read_signed(bytes);

        match prefix & 0xFC {
            // Global items
            0x04 => global.usage_page = unsigned as u16,
            0x14 => global.logical_min = signed,
            0x24 => global.logical_max = signed,
            0x74 => global.report_size = unsigned,
            0x94 => global.report_count = unsigned,
            0x84 => global.report_id = unsigned as u8,
            // Local items
            0x08 => {
                if local_usages.len() < MAX_LOCAL_USAGES {
                    // A four byte usage carries its own page in the high half.
                    let usage = if size == 4 {
                        Usage { page: (unsigned >> 16) as u16, id: unsigned as u16 }
                    } else {
                        Usage { page: global.usage_page, id: unsigned as u16 }
                    };
                    local_usages.push(usage);
                }
            }
            // Main items
            0xA0 => {
                if collections.len() >= MAX_COLLECTION_DEPTH {
                    return None;
                }
                let usage = local_usages.last().copied().unwrap_or(Usage { page: global.usage_page, id: 0 });
                collections.push(usage);
                local_usages.clear();
            }
            0xC0 => {
                collections.pop();
                local_usages.clear();
            }
            0x90 => {
                let offset = output_bits.get(&global.report_id).copied().unwrap_or(0);
                record_output(&mut found, &global, &local_usages, offset);
                let end = advance(offset, &global)?;
                output_bits.insert(global.report_id, end);
                local_usages.clear();
            }
            0xB0 => {
                let offset = feature_bits.get(&global.report_id).copied().unwrap_or(0);
                let in_waveform_list = collections.iter().any(|c| c.page == HAPTICS_USAGE_PAGE && c.id == USAGE_WAVEFORM_LIST);
                if in_waveform_list {
                    record_waveform_list(&mut found, &global, offset);
                }
                let end = advance(offset, &global)?;
                feature_bits.insert(global.report_id, end);
                local_usages.clear();
            }
            0x80 => local_usages.clear(),
            _ => {}
        }
    }

    let (trigger_report_id, trigger) = found.trigger?;
    let (waveform_report_id, waveform_fields) = found.waveforms?;
    if waveform_fields.is_empty() {
        return None;
    }
    let trigger_report_bits = output_bits.get(&trigger_report_id).copied()?;

    // Intensity and cutoff only count when they travel in the trigger's report.
    let intensity = found.intensity.filter(|(id, ..)| *id == trigger_report_id);
    let cutoff = found.cutoff.filter(|(id, ..)| *id == trigger_report_id);

    Some(HapticsLayout {
        trigger_report_id,
        trigger_report_bits,
        trigger,
        intensity: intensity.map(|(_, field, _)| field),
        intensity_max: intensity.map(|(.., max)| max).unwrap_or(0),
        cutoff: cutoff.map(|(_, field, _)| field),
        cutoff_min: cutoff.map(|(.., min)| min).unwrap_or(0),
        waveform_report_id,
        waveform_fields,
    })
}

/// Maps each waveform the device supports to its ordinal, from the Waveform List
/// feature report. `report` starts with the report ID byte, as hidapi returns it.
pub fn parse_waveform_list(layout: &HapticsLayout, report: &[u8]) -> HashMap<u16, u32> {
    let mut ordinals = HashMap::new();
    let body = report.get(1..).unwrap_or(&[]);
    for (index, field) in layout.waveform_fields.iter().enumerate() {
        let Some(value) = read_bits(body, *field) else {
            break; // the device sent a short report
        };
        let Ok(index) = u32::try_from(index) else {
            break;
        };
        let ordinal = FIRST_WAVEFORM_ORDINAL.saturating_add(index);
        if (FIRST_REAL_WAVEFORM..=LAST_REAL_WAVEFORM).contains(&value) {
            ordinals.entry(value as u16).or_insert(ordinal);
        }
    }
    ordinals
}

/// The waveform list for the log, in the device's order: each usage (with its name when it is
/// one of the HID Usage Tables' haptic waveforms, `vendor` for 0x2001 and up) and its ordinal.
pub fn describe_waveforms(ordinals: &HashMap<u16, u32>) -> String {
    let mut list: Vec<(u32, u16)> = ordinals.iter().map(|(usage, ordinal)| (*ordinal, *usage)).collect();
    list.sort_unstable();
    let parts: Vec<String> = list.iter().map(|(ordinal, usage)| format!("{usage:#06x} {} = {ordinal}", waveform_name(*usage))).collect();
    parts.join(", ")
}

fn waveform_name(usage: u16) -> &'static str {
    match usage {
        0x1003 => "click",
        0x1004 => "buzz",
        0x1005 => "rumble",
        0x1006 => "press",
        0x1007 => "release",
        0x1008 => "hover",
        0x1009 => "success",
        0x100A => "error",
        0x100B => "ink",
        0x100C => "pencil",
        0x100D => "marker",
        0x100E => "chisel marker",
        0x100F => "brush",
        0x1010 => "eraser",
        0x1011 => "sparkle",
        0x1012 => "collide",
        0x1013 => "align",
        0x1014 => "step",
        0x1015 => "grow",
        0x2001.. => "vendor",
        _ => "other",
    }
}

/// The output report (report ID first) that plays `ordinal` at `strength` (0..=1),
/// scaled to the device's own intensity range.
pub fn build_trigger_report(layout: &HapticsLayout, ordinal: u32, strength: f64) -> Option<Vec<u8>> {
    let body_len = usize::try_from(layout.trigger_report_bits.div_ceil(8)).ok()?;
    let mut report = vec![0u8; body_len.checked_add(1)?];
    report[0] = layout.trigger_report_id;
    let body = report.get_mut(1..)?;

    write_bits(body, layout.trigger, u32_value(i64::from(ordinal))?)?;

    if let Some(field) = layout.intensity {
        let strength = if strength.is_finite() { strength.clamp(0.0, 1.0) } else { 1.0 };
        let max = layout.intensity_max.max(1) as f64;
        let intensity = (strength * max).round().max(1.0) as i64;
        write_bits(body, field, u32_value(intensity)?)?;
    }

    if let Some(field) = layout.cutoff {
        // The device wants a cutoff; its minimum lets the waveform play out.
        write_bits(body, field, u32_value(layout.cutoff_min.max(0))?)?;
    }

    Some(report)
}

fn record_output(found: &mut PartialLayout, global: &GlobalState, usages: &[Usage], offset: u32) {
    if global.report_size == 0 || global.report_size > MAX_FIELD_BITS {
        return;
    }
    for (index, usage) in usages.iter().enumerate() {
        if usage.page != HAPTICS_USAGE_PAGE {
            continue;
        }
        let Ok(index) = u32::try_from(index) else {
            return;
        };
        if index >= global.report_count {
            return;
        }
        let Some(field_offset) = index.checked_mul(global.report_size).and_then(|bits| bits.checked_add(offset)) else {
            return;
        };
        let field = BitField { offset: field_offset, size: global.report_size };
        match usage.id {
            USAGE_MANUAL_TRIGGER => found.trigger = Some((global.report_id, field)),
            USAGE_INTENSITY => found.intensity = Some((global.report_id, field, global.logical_max)),
            USAGE_WAVEFORM_CUTOFF_TIME => found.cutoff = Some((global.report_id, field, global.logical_min)),
            _ => {}
        }
    }
}

fn record_waveform_list(found: &mut PartialLayout, global: &GlobalState, offset: u32) {
    if global.report_size == 0 || global.report_size > MAX_FIELD_BITS {
        return;
    }
    let (_, fields) = found.waveforms.get_or_insert_with(|| (global.report_id, Vec::new()));
    for index in 0..global.report_count {
        let Some(field_offset) = index.checked_mul(global.report_size).and_then(|bits| bits.checked_add(offset)) else {
            return;
        };
        if field_offset >= MAX_REPORT_BITS {
            return;
        }
        fields.push(BitField { offset: field_offset, size: global.report_size });
    }
}

/// Bit offset just past a main item, or `None` for a report too large to be real.
fn advance(offset: u32, global: &GlobalState) -> Option<u32> {
    let end = global.report_size.checked_mul(global.report_count)?.checked_add(offset)?;
    (end <= MAX_REPORT_BITS).then_some(end)
}

fn read_unsigned(bytes: &[u8]) -> u32 {
    bytes.iter().rev().fold(0u32, |acc, b| (acc << 8) | u32::from(*b))
}

/// Logical Minimum/Maximum are signed, sized by the item.
fn read_signed(bytes: &[u8]) -> i64 {
    let value = i64::from(read_unsigned(bytes));
    match bytes.len() {
        1 if value & 0x80 != 0 => value - 0x100,
        2 if value & 0x8000 != 0 => value - 0x1_0000,
        4 if value & 0x8000_0000 != 0 => value - 0x1_0000_0000,
        _ => value,
    }
}

fn u32_value(value: i64) -> Option<u32> {
    u32::try_from(value).ok()
}

fn read_bits(bytes: &[u8], field: BitField) -> Option<u32> {
    if field.size == 0 || field.size > MAX_FIELD_BITS {
        return None;
    }
    let mut value = 0u32;
    for bit in 0..field.size {
        let position = field.offset.checked_add(bit)?;
        let byte = *bytes.get(usize::try_from(position / 8).ok()?)?;
        if byte & (1 << (position % 8)) != 0 {
            value |= 1 << bit;
        }
    }
    Some(value)
}

/// Writes `value` into `field`, or `None` if it doesn't fit.
fn write_bits(bytes: &mut [u8], field: BitField, value: u32) -> Option<()> {
    if field.size == 0 || field.size > MAX_FIELD_BITS {
        return None;
    }
    if field.size < 32 && value >> field.size != 0 {
        return None;
    }
    for bit in 0..field.size {
        let position = field.offset.checked_add(bit)?;
        let byte = bytes.get_mut(usize::try_from(position / 8).ok()?)?;
        let mask = 1u8 << (position % 8);
        if value & (1 << bit) != 0 {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_log_lists_waveforms_in_device_order() {
        let ordinals: HashMap<u16, u32> = [(0x1013, 5), (0x2005, 6), (0x1003, 3), (0x1012, 4)].into_iter().collect();
        assert_eq!(describe_waveforms(&ordinals), "0x1003 click = 3, 0x1012 collide = 4, 0x1013 align = 5, 0x2005 vendor = 6");
        assert_eq!(describe_waveforms(&HashMap::new()), "");
    }

    use super::*;

    /// A trimmed Simple Haptic Controller in the shape of Microsoft's sample:
    /// feature report 0x11 with a four entry Waveform List (16 bit ordinals),
    /// output report 0x12 with Manual Trigger, Intensity (0..=100) and a
    /// Waveform Cutoff Time (5..=100).
    const SAMPLE_DESCRIPTOR: &[u8] = &[
        0x05, 0x0E, // Usage Page (Haptics)
        0x09, 0x01, // Usage (Simple Haptic Controller)
        0xA1, 0x01, // Collection (Application)
        0x85, 0x11, //   Report ID (0x11)
        0x09, 0x10, //   Usage (Waveform List)
        0xA1, 0x02, //   Collection (Logical)
        0x05, 0x0A, //     Usage Page (Ordinal)
        0x19, 0x03, //     Usage Minimum (3)
        0x29, 0x06, //     Usage Maximum (6)
        0x15, 0x03, //     Logical Minimum (3)
        0x27, 0xFF, 0xFF, 0x00, 0x00, // Logical Maximum (65535)
        0x75, 0x10, //     Report Size (16)
        0x95, 0x04, //     Report Count (4)
        0xB1, 0x02, //     Feature (Data, Var, Abs)
        0xC0, //   End Collection
        0x05, 0x0E, //   Usage Page (Haptics)
        0x09, 0x11, //   Usage (Duration List)
        0xA1, 0x02, //   Collection (Logical)
        0x05, 0x0A, //     Usage Page (Ordinal)
        0x19, 0x03, //     Usage Minimum (3)
        0x29, 0x06, //     Usage Maximum (6)
        0x75, 0x08, //     Report Size (8)
        0x95, 0x04, //     Report Count (4)
        0xB1, 0x02, //     Feature (Data, Var, Abs)
        0xC0, //   End Collection
        0x05, 0x0E, //   Usage Page (Haptics)
        0x85, 0x12, //   Report ID (0x12)
        0x09, 0x21, //   Usage (Manual Trigger)
        0x15, 0x01, //   Logical Minimum (1)
        0x25, 0x06, //   Logical Maximum (6)
        0x75, 0x08, //   Report Size (8)
        0x95, 0x01, //   Report Count (1)
        0x91, 0x02, //   Output (Data, Var, Abs)
        0x09, 0x23, //   Usage (Intensity)
        0x15, 0x00, //   Logical Minimum (0)
        0x25, 0x64, //   Logical Maximum (100)
        0x91, 0x02, //   Output (Data, Var, Abs)
        0x09, 0x28, //   Usage (Waveform Cutoff Time)
        0x15, 0x05, //   Logical Minimum (5)
        0x25, 0x64, //   Logical Maximum (100)
        0x91, 0x02, //   Output (Data, Var, Abs)
        0xC0, // End Collection
    ];

    /// Waveform List for the sample: Click, Align, Step, then an unused slot.
    const SAMPLE_WAVEFORM_REPORT: &[u8] = &[0x11, 0x03, 0x10, 0x13, 0x10, 0x14, 0x10, 0x01, 0x10];

    mod descriptor {
        use super::*;

        #[test]
        fn finds_trigger_intensity_cutoff_and_waveform_list() {
            let layout = parse_haptics_descriptor(SAMPLE_DESCRIPTOR).expect("layout");
            assert_eq!(layout.trigger_report_id, 0x12);
            assert_eq!(layout.trigger_report_bits, 24);
            assert_eq!(layout.trigger, BitField { offset: 0, size: 8 });
            assert_eq!(layout.intensity, Some(BitField { offset: 8, size: 8 }));
            assert_eq!(layout.intensity_max, 100);
            assert_eq!(layout.cutoff, Some(BitField { offset: 16, size: 8 }));
            assert_eq!(layout.cutoff_min, 5);
            assert_eq!(layout.waveform_report_id, 0x11);
            assert_eq!(layout.waveform_fields.len(), 4);
            assert_eq!(layout.waveform_fields[1], BitField { offset: 16, size: 16 });
        }

        #[test]
        fn rejects_a_descriptor_without_haptics() {
            let mouse = [0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, 0xC0];
            assert_eq!(parse_haptics_descriptor(&mouse), None);
        }

        #[test]
        fn survives_truncated_and_hostile_input() {
            for len in 0..SAMPLE_DESCRIPTOR.len() {
                let _ = parse_haptics_descriptor(&SAMPLE_DESCRIPTOR[..len]);
            }
            let deep = [0xA1, 0x01].repeat(100);
            assert_eq!(parse_haptics_descriptor(&deep), None);
            let huge = [0x75, 0xFF, 0x97, 0xFF, 0xFF, 0xFF, 0xFF, 0xB1, 0x02];
            assert_eq!(parse_haptics_descriptor(&huge), None);
            let long_item_past_end = [0xFE, 0xFF, 0x00];
            assert_eq!(parse_haptics_descriptor(&long_item_past_end), None);
        }
    }

    mod waveform_list {
        use super::*;

        #[test]
        fn maps_waveforms_to_positional_ordinals() {
            let layout = parse_haptics_descriptor(SAMPLE_DESCRIPTOR).expect("layout");
            let ordinals = parse_waveform_list(&layout, SAMPLE_WAVEFORM_REPORT);
            assert_eq!(ordinals.get(&0x1003), Some(&3));
            assert_eq!(ordinals.get(&0x1013), Some(&4));
            assert_eq!(ordinals.get(&0x1014), Some(&5));
            assert_eq!(ordinals.get(&0x1001), None);
        }

        #[test]
        fn keeps_what_it_can_read_from_a_short_report() {
            let layout = parse_haptics_descriptor(SAMPLE_DESCRIPTOR).expect("layout");
            let ordinals = parse_waveform_list(&layout, &SAMPLE_WAVEFORM_REPORT[..4]);
            assert_eq!(ordinals.len(), 1);
            assert!(parse_waveform_list(&layout, &[]).is_empty());
        }
    }

    mod trigger_report {
        use super::*;

        #[test]
        fn packs_ordinal_scaled_intensity_and_minimum_cutoff() {
            let layout = parse_haptics_descriptor(SAMPLE_DESCRIPTOR).expect("layout");
            assert_eq!(build_trigger_report(&layout, 4, 0.5), Some(vec![0x12, 4, 50, 5]));
        }

        #[test]
        fn never_sends_zero_intensity_or_out_of_range_strength() {
            let layout = parse_haptics_descriptor(SAMPLE_DESCRIPTOR).expect("layout");
            assert_eq!(build_trigger_report(&layout, 4, 0.0), Some(vec![0x12, 4, 1, 5]));
            assert_eq!(build_trigger_report(&layout, 4, 7.0), Some(vec![0x12, 4, 100, 5]));
            assert_eq!(build_trigger_report(&layout, 4, f64::NAN), Some(vec![0x12, 4, 100, 5]));
        }

        #[test]
        fn refuses_an_ordinal_wider_than_its_field() {
            let layout = parse_haptics_descriptor(SAMPLE_DESCRIPTOR).expect("layout");
            assert_eq!(build_trigger_report(&layout, 300, 1.0), None);
        }
    }
}
