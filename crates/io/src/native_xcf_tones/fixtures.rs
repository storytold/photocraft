//! Original, specification-framed synthetic sources; no external assets.
#[derive(Default)]
pub struct MaskedExposureOptions {
    pub unknown_policy: bool,
    pub disabled_stage: bool,
    pub invalid_mask: bool,
    pub partial_alpha: bool,
    pub float_owner: bool,
    pub u16_owner: bool,
}
pub struct MaskedExposureFixture {
    pub bytes: Vec<u8>,
    pub mask_offset: u64,
}

pub fn masked_exposure(options: MaskedExposureOptions) -> MaskedExposureFixture {
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
        fn patch(&mut self, slot: usize) {
            let value = self.0.len() as u64;
            self.0[slot..slot + 8].copy_from_slice(&value.to_be_bytes());
        }
        fn text(&mut self, value: &str) {
            self.word(value.len() as u32 + 1);
            self.0.extend_from_slice(value.as_bytes());
            self.0.push(0);
        }
        fn prop(&mut self, kind: u32, data: &[u8]) {
            self.word(kind);
            self.word(data.len() as u32);
            self.0.extend_from_slice(data);
        }
        fn end(&mut self) {
            self.word(0);
            self.word(0);
        }
        fn hierarchy(&mut self, width: u32, height: u32, bpp: u32, data: &[u8]) {
            for value in [width, height, bpp] {
                self.word(value);
            }
            let level = self.slot();
            self.ptr(0);
            self.patch(level);
            self.word(width);
            self.word(height);
            let tile = self.slot();
            self.ptr(0);
            self.patch(tile);
            self.0.extend_from_slice(data);
        }
    }
    let mut w = W(b"gimp xcf v022\0".to_vec());
    for value in [
        4,
        2,
        0,
        if options.float_owner {
            600
        } else if options.u16_owner {
            250
        } else {
            150
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
    for value in [2, 1, u32::from(options.partial_alpha)] {
        w.word(value);
    }
    w.text("Opaque Exposure owner");
    w.prop(15, &[1i32.to_be_bytes(), 0i32.to_be_bytes()].concat());
    w.prop(33, &0.75f32.to_be_bytes());
    w.prop(7, &28u32.to_be_bytes());
    w.end();
    let pixels = w.slot();
    w.ptr(0);
    let effect = w.slot();
    w.ptr(0);
    w.patch(pixels);
    let raw = if options.float_owner {
        [0.125f32, 0.375, 0.625, 0.75, 0.5, 0.25].into_iter().flat_map(f32::to_be_bytes).collect::<Vec<_>>()
    } else if options.u16_owner {
        let words = if options.partial_alpha {
            vec![8193u16, 24578, 40963, 32768, 49156, 32773, 16390, 65535]
        } else {
            vec![8193u16, 24578, 40963, 49156, 32773, 16390]
        };
        words.into_iter().flat_map(u16::to_be_bytes).collect::<Vec<_>>()
    } else if options.partial_alpha {
        vec![32, 96, 160, 128, 192, 128, 64, 255]
    } else {
        vec![32, 96, 160, 192, 128, 64]
    };
    let channels = if options.partial_alpha { 4 } else { 3 };
    let sample_bytes = if options.float_owner {
        4
    } else if options.u16_owner {
        2
    } else {
        1
    };
    w.hierarchy(2, 1, channels * sample_bytes, &raw);
    w.patch(effect);
    for text in ["Verified Exposure", "", "gegl:exposure", "1:0"] {
        w.text(text);
    }
    w.prop(7, &28u32.to_be_bytes());
    w.prop(8, &u32::from(!options.disabled_stage).to_be_bytes());
    w.prop(33, &0.65f32.to_be_bytes());
    if options.unknown_policy {
        w.prop(35, &0x0c7c8251u32.to_be_bytes());
    }
    for (name, value) in [("black-level", 0.03f32), ("exposure", 0.75)] {
        let mut argument = W(Vec::new());
        argument.text(name);
        argument.word(3);
        argument.0.extend_from_slice(&value.to_be_bytes());
        w.prop(45, &argument.0);
    }
    w.end();
    let mask = w.slot();
    w.patch(mask);
    let mask_offset = w.0.len() as u64;
    w.word(if options.invalid_mask { 3 } else { 4 });
    w.word(2);
    w.text("Independent coverage");
    w.prop(33, &0.5f32.to_be_bytes());
    w.prop(14, &1u32.to_be_bytes());
    w.prop(38, &[0; 12]);
    w.end();
    let samples = w.slot();
    w.patch(samples);
    let coverage = if options.float_owner {
        [0f32, 0.25, 0.75, 1., 1., 0.5, 0.125, 0.].into_iter().flat_map(f32::to_be_bytes).collect::<Vec<_>>()
    } else if options.u16_owner {
        [0u16, 16385, 49153, 65535, 65535, 32769, 8195, 0].into_iter().flat_map(u16::to_be_bytes).collect::<Vec<_>>()
    } else {
        vec![0, 64, 192, 255, 255, 128, 32, 0]
    };
    w.hierarchy(4, 2, sample_bytes, &coverage);
    MaskedExposureFixture { bytes: w.0, mask_offset }
}

/// Original scalar Levels source framing used by IO and engine regressions.
pub mod levels;
