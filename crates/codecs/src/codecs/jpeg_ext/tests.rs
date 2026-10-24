use super::*;

/// Local check against the conformance samples (never committed): set `PC_JPEG_SAMPLES` to a
/// directory holding testorig.jpg, testorig12.jpg and testimgari.jpg.
#[test]
#[ignore]
fn conformance_samples() {
    let dir = std::env::var("PC_JPEG_SAMPLES").unwrap();
    let read = |n: &str| std::fs::read(format!("{dir}/{n}")).unwrap();
    let reference = crate::decode(&read("testorig.jpg")).unwrap();
    let ref8 = reference.data().to_vec();
    for (name, depth) in [("testorig12.jpg", SampleType::U16), ("testimgari.jpg", SampleType::U8)] {
        let img = crate::decode(&read(name)).unwrap();
        assert_eq!(img.dimensions(), reference.dimensions(), "{name}");
        assert_eq!(img.sample_type(), depth, "{name}");
        assert!(img.warnings.is_empty(), "{name}: {:?}", img.warnings);
        let px: Vec<f32> = match depth {
            SampleType::U16 => img.to_u16_samples().unwrap().iter().map(|&v| v as f32 / 257.0).collect(),
            _ => img.data().iter().map(|&v| v as f32).collect(),
        };
        let mad = px.iter().zip(&ref8).map(|(a, &b)| (a - b as f32).abs()).sum::<f32>() / px.len() as f32;
        eprintln!("{name}: mean abs diff vs testorig.jpg {mad}");
        assert!(mad < 3.0, "{name}: {mad}");
    }
}

// --- A test-only arithmetic encoder (T.81 D.1, F.1.4 and G.1.3), mirroring the decoder ---

struct Enc {
    out: Vec<u8>,
    c: u32,
    a: u32,
    ct: u32,
    stacked: u32,
    b: Option<u8>,
}

impl Enc {
    fn new() -> Self {
        Enc { out: Vec::new(), c: 0, a: 0x10000, ct: 11, stacked: 0, b: None }
    }

    fn emit(&mut self, v: u8) {
        self.out.push(v);
        if v == 0xFF {
            self.out.push(0);
        }
    }

    fn byte_out(&mut self) {
        let t = self.c >> 19;
        if t > 0xFF {
            let b = self.b.unwrap() + 1;
            self.emit(b);
            for _ in 0..self.stacked {
                self.emit(0);
            }
            self.stacked = 0;
            self.b = Some((t & 0xFF) as u8);
        } else if t == 0xFF {
            self.stacked += 1;
        } else {
            if let Some(b) = self.b {
                self.emit(b);
            }
            for _ in 0..self.stacked {
                self.emit(0xFF);
            }
            self.stacked = 0;
            self.b = Some(t as u8);
        }
        self.c &= 0x7FFFF;
    }

    fn renorm(&mut self) {
        while self.a < 0x8000 {
            self.a <<= 1;
            self.c <<= 1;
            self.ct -= 1;
            if self.ct == 0 {
                self.byte_out();
                self.ct = 8;
            }
        }
    }

    fn code(&mut self, st: &mut u8, d: u8) {
        let mps = *st >> 7;
        let (qe, nlps, nmps, switch) = QE[usize::from(*st & 0x7F)];
        self.a -= qe;
        if d == mps {
            if self.a < 0x8000 {
                if self.a < qe {
                    self.c += self.a;
                    self.a = qe;
                }
                *st = nmps | (mps << 7);
                self.renorm();
            }
        } else {
            if self.a >= qe {
                self.c += self.a;
                self.a = qe;
            }
            *st = nlps | ((mps ^ u8::from(switch)) << 7);
            self.renorm();
        }
    }

    fn code_fixed(&mut self, d: u8) {
        let mut st = 0;
        self.code(&mut st, d);
    }

    fn finish(mut self) -> Vec<u8> {
        let mut t = (self.c + self.a - 1) & 0xFFFF_0000;
        if t < self.c {
            t += 0x8000;
        }
        self.c = t << self.ct;
        self.byte_out();
        self.c <<= 8;
        self.byte_out();
        if let Some(b) = self.b {
            self.emit(b);
        }
        for _ in 0..self.stacked {
            self.emit(0xFF);
        }
        self.out
    }
}

struct EncScan {
    dc: [u8; 64],
    ac: [u8; 256],
    ctx: usize,
    pred: i32,
}

impl EncScan {
    fn new() -> Self {
        EncScan { dc: [0; 64], ac: [0; 256], ctx: 0, pred: 0 }
    }

    /// Encode_DC_DIFF with the default conditioning (L = 0, U = 1).
    fn dc(&mut self, e: &mut Enc, value: i32) {
        let diff = value - self.pred;
        self.pred = value;
        let st = &mut self.dc;
        let s0 = self.ctx;
        if diff == 0 {
            e.code(&mut st[s0], 0);
            self.ctx = 0;
            return;
        }
        e.code(&mut st[s0], 1);
        let sign = u8::from(diff < 0);
        e.code(&mut st[s0 + 1], sign);
        let sp = s0 + 2 + usize::from(sign);
        let sz = diff.unsigned_abs() - 1;
        if sz == 0 {
            e.code(&mut st[sp], 0);
        } else {
            e.code(&mut st[sp], 1);
            let mut m = 1u32;
            let mut s = 20;
            while m << 1 <= sz {
                e.code(&mut st[s], 1);
                m <<= 1;
                s += 1;
            }
            e.code(&mut st[s], 0);
            let mut bit = m >> 1;
            while bit != 0 {
                e.code(&mut st[s + 14], u8::from(sz & bit != 0));
                bit >>= 1;
            }
        }
        let mag = diff.unsigned_abs();
        self.ctx = if mag <= 2 { 4 + 4 * usize::from(sign) } else { 12 + 4 * usize::from(sign) };
    }

    /// Encode_AC_coefficients over `ss..=se` of the coefficients scaled down by `al` (Kx = 5).
    fn ac(&mut self, e: &mut Enc, blk: &[i32; 64], ss: usize, se: usize, al: u8) {
        let st = &mut self.ac;
        let v = |k: usize| if blk[k] < 0 { -((-blk[k]) >> al) } else { blk[k] >> al };
        let last = (ss..=se).rev().find(|&k| v(k) != 0);
        let mut k = ss;
        while k <= se {
            if last.is_none_or(|l| k > l) {
                e.code(&mut st[3 * (k - 1)], 1);
                return;
            }
            e.code(&mut st[3 * (k - 1)], 0);
            while v(k) == 0 {
                e.code(&mut st[3 * (k - 1) + 1], 0);
                k += 1;
            }
            e.code(&mut st[3 * (k - 1) + 1], 1);
            e.code_fixed(u8::from(v(k) < 0));
            let sz = v(k).unsigned_abs() - 1;
            let sp = 3 * (k - 1) + 2;
            if sz == 0 {
                e.code(&mut st[sp], 0);
            } else {
                e.code(&mut st[sp], 1);
                if sz == 1 {
                    e.code(&mut st[sp], 0);
                } else {
                    e.code(&mut st[sp], 1);
                    let mut m = 2u32;
                    let mut s = if k <= 5 { 189 } else { 217 };
                    while m << 1 <= sz {
                        e.code(&mut st[s], 1);
                        m <<= 1;
                        s += 1;
                    }
                    e.code(&mut st[s], 0);
                    let mut bit = m >> 1;
                    while bit != 0 {
                        e.code(&mut st[s + 14], u8::from(sz & bit != 0));
                        bit >>= 1;
                    }
                }
            }
            k += 1;
        }
    }

    /// Successive approximation of AC coefficients: bit `al` of each (G.1.3.3).
    fn ac_refine(&mut self, e: &mut Enc, blk: &[i32; 64], ss: usize, se: usize, al: u8) {
        let st = &mut self.ac;
        let prior = |k: usize| blk[k].unsigned_abs() >> (al + 1) != 0;
        let now = |k: usize| blk[k].unsigned_abs() >> al != 0;
        let eob_prev = (ss..=se).rev().find(|&k| prior(k)).map_or(ss, |k| k + 1);
        let eob_now = (ss..=se).rev().find(|&k| now(k)).map_or(ss, |k| k + 1);
        let mut k = ss;
        while k <= se {
            if k >= eob_prev {
                if k >= eob_now {
                    e.code(&mut st[3 * (k - 1)], 1);
                    return;
                }
                e.code(&mut st[3 * (k - 1)], 0);
            }
            loop {
                if prior(k) {
                    e.code(&mut st[3 * (k - 1) + 2], ((blk[k].unsigned_abs() >> al) & 1) as u8);
                    break;
                }
                if now(k) {
                    e.code(&mut st[3 * (k - 1) + 1], 1);
                    e.code_fixed(u8::from(blk[k] < 0));
                    break;
                }
                e.code(&mut st[3 * (k - 1) + 1], 0);
                k += 1;
            }
            k += 1;
        }
    }
}

/// Deterministic pseudo-random coefficients: mostly small, zero runs, a few large.
fn coefficients(n: usize, seed: u32, big: i32) -> Vec<[i32; 64]> {
    let mut s = seed;
    let mut next = move || {
        s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        s >> 8
    };
    (0..n)
        .map(|_| {
            let mut b = [0i32; 64];
            b[0] = (next() % (2 * big as u32 + 1)) as i32 - big;
            for v in b.iter_mut().skip(1) {
                let mag = match next() % 100 {
                    0..=59 => 0,
                    60..=89 => 1 + (next() % 4) as i32,
                    90..=97 => 1 + (next() % 60) as i32,
                    _ => 1 + (next() % big as u32) as i32,
                };
                *v = if next() % 2 == 0 { mag } else { -mag };
            }
            if next() % 3 == 0 {
                for v in &mut b[20..] {
                    *v = 0;
                }
            }
            b
        })
        .collect()
}

fn segment(out: &mut Vec<u8>, marker: u8, body: &[u8]) {
    out.extend_from_slice(&[0xFF, marker]);
    out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(body);
}

/// A gray SOF9 stream (SOF10 when there are several scans) holding `blocks` (`bw` × `bh`),
/// with the scans `(ss, se, ah, al)` and restart interval `ri` (in MCUs).
fn arith_stream(precision: u8, bw: usize, bh: usize, blocks: &[[i32; 64]], scans: &[(usize, usize, u8, u8)], ri: usize) -> Vec<u8> {
    let progressive = scans.len() > 1;
    let mut s = vec![0xFF, 0xD8];
    let mut dqt = vec![0u8];
    dqt.extend([1u8; 64]);
    segment(&mut s, 0xDB, &dqt);
    let (w, h) = ((bw * 8) as u16, (bh * 8) as u16);
    let mut sof = vec![precision];
    sof.extend(h.to_be_bytes());
    sof.extend(w.to_be_bytes());
    sof.extend([1, 1, 0x11, 0]);
    segment(&mut s, if progressive { 0xCA } else { 0xC9 }, &sof);
    if ri > 0 {
        segment(&mut s, 0xDD, &(ri as u16).to_be_bytes());
    }
    for &(ss, se, ah, al) in scans {
        segment(&mut s, 0xDA, &[1, 1, 0x00, ss as u8, se as u8, (ah << 4) | al]);
        let mut e = Enc::new();
        let mut st = EncScan::new();
        let mut rst = 0u8;
        for (n, blk) in blocks.iter().enumerate() {
            if ri > 0 && n > 0 && n % ri == 0 {
                s.extend(std::mem::replace(&mut e, Enc::new()).finish());
                s.extend([0xFF, 0xD0 + rst]);
                rst = (rst + 1) & 7;
                st = EncScan::new();
            }
            match (ss, ah) {
                (0, 0) => {
                    st.dc(&mut e, blk[0] >> al);
                    if !progressive {
                        st.ac(&mut e, blk, 1, 63, 0);
                    }
                }
                (0, _) => e.code_fixed(((blk[0] >> al) & 1) as u8),
                (_, 0) => st.ac(&mut e, blk, ss, se, al),
                _ => st.ac_refine(&mut e, blk, ss, se, al),
            }
        }
        s.extend(e.finish());
    }
    s.extend([0xFF, 0xD9]);
    s
}

fn decoded_blocks(stream: &[u8]) -> Vec<[i32; 64]> {
    let (mut frame, _, damaged) = decode_coefficients(stream, &Limits::default()).unwrap();
    assert!(!damaged);
    std::mem::take(&mut frame.comps[0].coefs)
}

#[test]
fn arithmetic_sequential_round_trips_coefficients() {
    for (precision, big, ri) in [(8u8, 1000, 0usize), (12, 16000, 0), (8, 300, 1), (12, 3000, 5)] {
        let blocks = coefficients(12, u32::from(precision) * 7 + ri as u32, big);
        let stream = arith_stream(precision, 4, 3, &blocks, &[(0, 63, 0, 0)], ri);
        assert_eq!(decoded_blocks(&stream), blocks, "P={precision} ri={ri}");
    }
}

#[test]
fn arithmetic_progressive_round_trips_coefficients() {
    let blocks = coefficients(6, 99, 2000);
    let scans = [(0, 0, 0, 1), (1, 5, 0, 2), (6, 63, 0, 1), (1, 5, 2, 1), (0, 0, 1, 0), (1, 5, 1, 0), (6, 63, 1, 0)];
    for ri in [0, 2] {
        let stream = arith_stream(8, 3, 2, &blocks, &scans, ri);
        assert_eq!(decoded_blocks(&stream), blocks, "ri={ri}");
    }
}

#[test]
fn arithmetic_twelve_bit_flat_block_is_sixteen_bit() {
    // DC only: sample = DC / 8 + 2048 with a quantizer of 1; 1000 → 2173.
    let mut blk = [0i32; 64];
    blk[0] = 1000;
    let stream = arith_stream(12, 1, 1, &[blk], &[(0, 63, 0, 0)], 0);
    assert!(handles(&stream));
    let img = crate::decode(&stream).unwrap();
    assert_eq!((img.dimensions(), img.sample_type(), img.layout()), ((8, 8), SampleType::U16, ChannelLayout::Gray));
    let want = (2173.0f32 * 65535.0 / 4095.0).round() as u16;
    assert!(img.to_u16_samples().unwrap().iter().all(|&v| v.abs_diff(want) <= 1));
}

/// A 12-bit SOF1 stream built by hand: one 8×8 gray block with only a DC coefficient. The DC
/// table has one code (`0`, category 13), the AC table one (`0`, end of block).
fn twelve_bit_huffman_block() -> Vec<u8> {
    let mut s = vec![0xFF, 0xD8];
    let mut dqt = vec![0u8];
    dqt.extend([1u8; 64]);
    segment(&mut s, 0xDB, &dqt);
    segment(&mut s, 0xC1, &[12, 0, 8, 0, 8, 1, 1, 0x11, 0]);
    let mut dht = vec![0x00, 1];
    dht.extend([0u8; 15]);
    dht.push(13);
    dht.extend([0x10, 1]);
    dht.extend([0u8; 15]);
    dht.push(0x00);
    segment(&mut s, 0xC4, &dht);
    segment(&mut s, 0xDA, &[1, 1, 0x00, 0, 63, 0]);
    // DC = 7616 → sample 7616 / 8 + 2048 = 3000. Bits: `0`, 7616 in 13 bits, `0`, one pad `1`.
    let word: u16 = (7616 << 2) | 1;
    s.extend(word.to_be_bytes());
    s.extend([0xFF, 0xD9]);
    s
}

#[test]
fn twelve_bit_huffman_block_decodes_to_sixteen_bit() {
    let stream = twelve_bit_huffman_block();
    assert!(handles(&stream));
    let img = crate::decode(&stream).unwrap();
    assert_eq!((img.dimensions(), img.sample_type()), ((8, 8), SampleType::U16));
    let want = (3000.0f32 * 65535.0 / 4095.0).round() as u16;
    let px = img.to_u16_samples().unwrap();
    assert!(px.iter().all(|&v| v.abs_diff(want) <= 1), "{:?} vs {want}", &px[..4]);
}

#[test]
fn twelve_bit_jpeg_keeps_its_photoshop_resources() {
    // An APP13 `8BIM` block (paths, Photoshop settings) right after SOI, as on the zune-jpeg path.
    let resources = b"8BIM\x04\x04\0\0\0\0\0\0".to_vec();
    let mut app13 = b"Photoshop 3.0\0".to_vec();
    app13.extend_from_slice(&resources);
    let stream = twelve_bit_huffman_block();
    let mut with = stream[..2].to_vec();
    segment(&mut with, 0xED, &app13);
    with.extend_from_slice(&stream[2..]);
    assert!(handles(&with));
    let img = crate::decode(&with).unwrap();
    assert_eq!(img.meta.photoshop_resources.as_deref(), Some(resources.as_slice()));
}

/// Where the entropy-coded data of the last scan starts.
fn last_scan_data(s: &[u8]) -> usize {
    let sos = s.windows(2).rposition(|p| p == [0xFF, 0xDA]).unwrap();
    sos + 2 + usize::from(u16::from_be_bytes([s[sos + 2], s[sos + 3]]))
}

/// A Huffman scan whose entropy data runs out is not a clean image, even with EOI present:
/// the zero padding supplied at the marker must not be decoded as real coefficients.
#[test]
fn huffman_scan_out_of_data_is_not_clean() {
    let full = twelve_bit_huffman_block();
    let data = last_scan_data(&full);
    for keep in 0..2 {
        let mut s = full[..data + keep].to_vec();
        s.extend([0xFF, 0xD9]);
        assert!(decode(&s, &Limits::default(), None).is_err(), "keep {keep}");
        if let Ok(img) = crate::decode(&s) {
            assert!(!img.warnings.is_empty(), "keep {keep}: clean decode of missing data");
        }
    }
    // A multi-MCU scan cut in half, EOI kept, decodes as damaged.
    let (w, h) = (64usize, 64usize);
    let rgb: Vec<u8> = (0..w * h).flat_map(|i| [(i % w * 4) as u8, (i / w * 4) as u8, (i * 7) as u8]).collect();
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, 90).encode(&rgb, w as u16, h as u16, jpeg_encoder::ColorType::Rgb).unwrap();
    let sof = out.windows(2).position(|p| p == [0xFF, 0xC0]).unwrap();
    out[sof + 1] = 0xC1;
    let data = last_scan_data(&out);
    let end = out.len() - 2;
    let mut cut = out[..data + (end - data) / 2].to_vec();
    if cut.last() == Some(&0xFF) {
        cut.pop();
    }
    cut.extend([0xFF, 0xD9]);
    assert!(!decode(&out, &Limits::default(), None).unwrap().damaged);
    assert!(decode(&cut, &Limits::default(), None).unwrap().damaged);
}

/// A baseline file from our encoder, relabelled SOF1, decodes like zune decodes the original.
#[test]
fn extended_huffman_matches_zune_on_baseline_data() {
    let (w, h) = (37u16, 29u16);
    let rgb: Vec<u8> = (0..usize::from(w) * usize::from(h))
        .flat_map(|i| {
            let (x, y) = (i % usize::from(w), i / usize::from(w));
            [(x * 7) as u8, (y * 9) as u8, ((x + y) * 3) as u8]
        })
        .collect();
    for (sampling, restart) in
        [(jpeg_encoder::SamplingFactor::R_4_2_0, 0u16), (jpeg_encoder::SamplingFactor::R_4_4_4, 3), (jpeg_encoder::SamplingFactor::R_4_2_2, 1)]
    {
        let mut out = Vec::new();
        let mut enc = jpeg_encoder::Encoder::new(&mut out, 90);
        enc.set_sampling_factor(sampling);
        if restart > 0 {
            enc.set_restart_interval(restart);
        }
        enc.encode(&rgb, w, h, jpeg_encoder::ColorType::Rgb).unwrap();
        let zune = crate::decode(&out).unwrap();
        let sof = out.windows(2).position(|p| p == [0xFF, 0xC0]).unwrap();
        out[sof + 1] = 0xC1;
        let ours = decode(&out, &Limits::default(), None).unwrap();
        assert_eq!((ours.width, ours.height, ours.layout, ours.sample), (u32::from(w), u32::from(h), ChannelLayout::Rgb, SampleType::U8));
        assert!(!ours.damaged);
        let mad = ours.data.iter().zip(zune.data()).map(|(&a, &b)| f32::from(a.abs_diff(b))).sum::<f32>() / ours.data.len() as f32;
        assert!(mad < 1.5, "{sampling:?} restart {restart}: mean abs diff {mad}");
    }
}

#[test]
fn malformed_streams_return_errors_not_panics() {
    let blocks = coefficients(6, 5, 500);
    let mut streams = vec![
        arith_stream(12, 3, 2, &blocks, &[(0, 63, 0, 0)], 2),
        arith_stream(8, 3, 2, &blocks, &[(0, 0, 0, 1), (1, 63, 0, 0), (0, 0, 1, 0)], 0),
        twelve_bit_huffman_block(),
    ];
    // Header damage: zero sizes, bad component counts, sampling factors and tables, bad DAC.
    let base = arith_stream(8, 1, 1, &blocks[..1], &[(0, 63, 0, 0)], 0);
    let sof = base.windows(2).position(|p| p == [0xFF, 0xC9]).unwrap();
    for (offset, value) in [(4, 16u8), (6, 0), (8, 0), (9, 7), (10, 9), (11, 0x50), (12, 9)] {
        let mut s = base.clone();
        s[sof + offset] = value;
        assert!(crate::decode(&s).is_err(), "SOF byte {offset} = {value}");
        streams.push(s);
    }
    let mut dac = base.clone();
    dac.splice(2..2, [0xFF, 0xCC, 0, 4, 0x00, 0x13]);
    assert!(crate::decode(&dac).is_err());
    streams.push(dac);
    for s in &streams {
        for cut in 0..s.len() {
            let _ = crate::decode(&s[..cut]);
        }
        for i in 2..s.len() {
            for v in [0x00, 0xFF, 0x5A] {
                let mut t = s.clone();
                t[i] = v;
                let _ = crate::decode(&t);
            }
        }
    }
    assert!(crate::decode(&streams[0][..40]).is_err());
}

/// Scans past `MAX_SCANS` aren't decoded (each one walks the whole MCU grid, with or without
/// data): the image keeps what the earlier scans decoded and is reported damaged.
#[test]
fn scans_past_the_limit_are_not_decoded() {
    let blocks = coefficients(1, 3, 500);
    for (count, damaged) in [(MAX_SCANS, false), (MAX_SCANS + 1, true)] {
        let stream = arith_stream(8, 1, 1, &blocks, &vec![(0, 0, 0, 0); count], 0);
        let (frame, _, d) = decode_coefficients(&stream, &Limits::default()).unwrap();
        assert_eq!(d, damaged, "{count} scans");
        assert_eq!(frame.comps[0].coefs[0][0], blocks[0][0], "{count} scans");
    }
}

#[test]
fn limits_are_checked_before_allocating() {
    let mut s = vec![0xFF, 0xD8];
    segment(&mut s, 0xC9, &[8, 0xFF, 0xFF, 0xFF, 0xFF, 3, 1, 0x44, 0, 2, 0x11, 0, 3, 0x11, 0]);
    let small = Limits { max_alloc: 1 << 20, ..Limits::default() };
    assert!(matches!(decode(&s, &small, None), Err(CodecError::LimitExceeded(_))));
}

#[test]
fn routing() {
    assert!(!handles(&[0xFF, 0xD8, 0xFF, 0xC0, 0, 11, 8, 0, 8, 0, 8, 1, 1, 0x11, 0]));
    assert!(handles(&[0xFF, 0xD8, 0xFF, 0xC9, 0, 11, 8, 0, 8, 0, 8, 1, 1, 0x11, 0]));
    assert!(handles(&[0xFF, 0xD8, 0xFF, 0xC1, 0, 11, 12, 0, 8, 0, 8, 1, 1, 0x11, 0]));
    assert!(!handles(&[0xFF, 0xD8]));
}
