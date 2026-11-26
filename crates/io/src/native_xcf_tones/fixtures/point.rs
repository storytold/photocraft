//! Original manual point-effect XCF26 framing; no producer assets or evaluator.
use photocraft_color::SampleType;

#[derive(Clone, Copy)]
pub enum WireValue {
    Float(f32),
    Int(i32),
    Bool(bool),
    Enum(i32),
}
/// `refusal` changes source framing/envelope controls only; no expected native
/// pixels or parameters are embedded in this generator.
pub fn fixture(operation: &str, args: &[(&str, WireValue)], depth: SampleType, refusal: Option<&str>) -> Vec<u8> {
    struct W(Vec<u8>);
    impl W {
        fn word(&mut self, v: u32) {
            self.0.extend_from_slice(&v.to_be_bytes());
        }
        fn ptr(&mut self, v: usize) {
            self.0.extend_from_slice(&(v as u64).to_be_bytes());
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
        fn text(&mut self, v: &str) {
            self.word(v.len() as u32 + 1);
            self.0.extend_from_slice(v.as_bytes());
            self.0.push(0);
        }
        fn prop(&mut self, k: u32, data: &[u8]) {
            self.word(k);
            self.word(data.len() as u32);
            self.0.extend_from_slice(data);
        }
        fn end(&mut self) {
            self.word(0);
            self.word(0);
        }
    }
    let mut w = W(b"gimp xcf v026\0".to_vec());
    for value in [
        3,
        3,
        0,
        match depth {
            SampleType::U8 => 150,
            SampleType::U16 => 250,
            SampleType::F32 => 600,
        },
    ] {
        w.word(value);
    }
    w.prop(17, &[0]);
    w.end();
    let layer = w.slot();
    for _ in 0..3 {
        w.ptr(0);
    }
    w.patch(layer);
    for value in [2, 1, 1] {
        w.word(value);
    }
    w.text("Levels owner");
    w.prop(15, &[(-1i32).to_be_bytes(), 1i32.to_be_bytes()].concat());
    w.prop(33, &0.75f32.to_be_bytes());
    w.end();
    let hierarchy = w.slot();
    w.ptr(0);
    let effect = w.slot();
    w.ptr(0);
    w.patch(hierarchy);
    let sample_bytes = match depth {
        SampleType::U8 => 1,
        SampleType::U16 => 2,
        SampleType::F32 => 4,
    };
    for value in [2, 1, 4 * sample_bytes] {
        w.word(value);
    }
    let level = w.slot();
    w.ptr(0);
    w.patch(level);
    w.word(2);
    w.word(1);
    let tile = w.slot();
    w.ptr(0);
    w.patch(tile);
    match depth {
        SampleType::U8 => w.0.extend_from_slice(&[32, 96, 160, 128, 192, 128, 64, 255]),
        SampleType::U16 => {
            for value in [8193u16, 24578, 40963, 32768, 49156, 32773, 16390, 65535] {
                w.0.extend_from_slice(&value.to_be_bytes());
            }
        }
        SampleType::F32 => {
            for value in [-0.25f32, 0.25, 2., 0.5, 0.75, 1.5, -0.5, 1.] {
                w.0.extend_from_slice(&value.to_be_bytes());
            }
        }
    }
    w.patch(effect);
    for text in ["Source point effect", "", operation, if refusal == Some("version") { "99:0" } else { "0:0" }] {
        w.text(text);
    }
    w.prop(7, &28u32.to_be_bytes());
    w.prop(8, &if refusal == Some("disabled") { 0u32 } else { 1 }.to_be_bytes());
    w.prop(33, &if refusal == Some("opacity") { 0.5f32 } else { 1. }.to_be_bytes());
    if refusal == Some("policy") {
        w.prop(35, &0x0c7c8251u32.to_be_bytes());
    }
    for &(name, value) in args {
        let mut arg = W(Vec::new());
        arg.text(name);
        match value {
            WireValue::Float(v) => {
                arg.word(3);
                arg.0.extend_from_slice(&v.to_be_bytes());
            }
            WireValue::Int(v) => {
                arg.word(1);
                arg.word(v as u32);
            }
            WireValue::Bool(v) => {
                arg.word(2);
                arg.word(u32::from(v));
            }
            WireValue::Enum(v) => {
                arg.word(5);
                arg.word(v as u32);
            }
        }
        w.prop(45, &arg.0);
    }
    w.end();
    w.ptr(if refusal == Some("mask") { 8 } else { 0 });
    w.0
}
