use super::*;

fn precision(bytes: usize, float: bool) -> Precision {
    Precision { bytes, float, linear: true, perceptual: false }
}

fn compare_reference(samples: &[Vec<u8>], precision: Precision, channels: usize, coverage: bool, order: HistoricalXcfSampleOrder) {
    assert!(samples.len().is_multiple_of(channels));
    let mut expected = Vec::new();
    let mut expected_changes = SampleChanges::default();
    let mut invalid = None;
    for (index, bytes) in samples.iter().enumerate() {
        let alpha = channels.is_multiple_of(2) && index % channels == channels - 1;
        match precision.sample_with_conversion(bytes, coverage || alpha) {
            Ok((value, underflow, clamped)) => {
                expected.push(value.to_bits());
                expected_changes.underflow |= underflow;
                expected_changes.coverage_clamped |= clamped;
            }
            Err(_) => {
                invalid = Some(index);
                break;
            }
        }
    }
    let raw: Vec<_> = samples
        .iter()
        .flat_map(|bytes| {
            let mut bytes = bytes.clone();
            if order == HistoricalXcfSampleOrder::LittleEndian {
                bytes.reverse();
            }
            bytes
        })
        .collect();
    let mut output = vec![42.0; samples.len()];
    let result = row_samples(&raw, &mut output, precision, order, channels, coverage);
    if let Some(index) = invalid {
        assert_eq!(result.unwrap_err().sample, index, "{precision:?}, {channels}, {coverage}, {order:?}");
    } else {
        assert_eq!(result.unwrap(), expected_changes, "{precision:?}, {channels}, {coverage}, {order:?}");
        assert_eq!(output.iter().map(|value| value.to_bits()).collect::<Vec<_>>(), expected, "{precision:?}, {channels}, {coverage}, {order:?}");
    }
}

#[test]
fn all_storage_rows_match_scalar_bits_and_flags_for_every_channel_layout_and_order() {
    let cases = [
        (precision(1, false), [0u8, 1, 127, 128, 254, 255].into_iter().map(|value| vec![value]).collect::<Vec<_>>()),
        (precision(2, false), [0u16, 1, 32767, 32768, 65534, 65535].into_iter().map(|value| value.to_be_bytes().to_vec()).collect()),
        (precision(4, false), [0u32, 1, 0x7fff_ffff, 0x8000_0000, u32::MAX - 1, u32::MAX].into_iter().map(|value| value.to_be_bytes().to_vec()).collect()),
        (precision(2, true), [0u16, 0x8000, 1, 0x8001, 0x3c00, 0x3c01, 0xbc00, 0x7bff].into_iter().map(|value| value.to_be_bytes().to_vec()).collect()),
        (
            precision(4, true),
            [0.0f32, -0.0, f32::from_bits(1), -f32::from_bits(1), 1.0, f32::from_bits(1.0f32.to_bits() + 1), -0.25, 1.5, f32::MAX, -f32::MAX]
                .into_iter()
                .map(|value| value.to_be_bytes().to_vec())
                .collect(),
        ),
        (
            precision(8, true),
            [
                0.0f64,
                -0.0,
                f64::from_bits(1),
                -f64::from_bits(1),
                1.0,
                f64::from_bits(1.0f64.to_bits() + 1),
                -0.25,
                1.5,
                f64::from(f32::MAX),
                -f64::from(f32::MAX),
            ]
            .into_iter()
            .map(|value| value.to_be_bytes().to_vec())
            .collect(),
        ),
    ];
    for (precision, values) in cases {
        for channels in 1..=4 {
            // Put every boundary value in every color/alpha position.
            let samples: Vec<_> = values.iter().flat_map(|value| std::iter::repeat_n(value.clone(), channels)).collect();
            for coverage in [false, true] {
                for order in [HistoricalXcfSampleOrder::BigEndian, HistoricalXcfSampleOrder::LittleEndian] {
                    compare_reference(&samples, precision, channels, coverage, order);
                }
            }
        }
    }
}

#[test]
fn every_finite_half_matches_scalar_bits_and_flags_in_both_orders() {
    let samples: Vec<_> = (0u16..=u16::MAX).filter(|bits| bits & 0x7c00 != 0x7c00).map(|bits| bits.to_be_bytes().to_vec()).collect();
    for coverage in [false, true] {
        for order in [HistoricalXcfSampleOrder::BigEndian, HistoricalXcfSampleOrder::LittleEndian] {
            compare_reference(&samples, precision(2, true), 1, coverage, order);
        }
    }
}

#[test]
fn float_rows_report_first_nonfinite_sample_in_color_and_alpha() {
    for (precision, bad) in [
        (precision(2, true), [0x7c00u16, 0xfc00, 0x7e01].into_iter().map(|bits| bits.to_be_bytes().to_vec()).collect::<Vec<_>>()),
        (precision(4, true), [f32::INFINITY, f32::NEG_INFINITY, f32::NAN].into_iter().map(|value| value.to_be_bytes().to_vec()).collect()),
        (precision(8, true), [f64::INFINITY, f64::NEG_INFINITY, f64::NAN].into_iter().map(|value| value.to_be_bytes().to_vec()).collect()),
    ] {
        for channels in 1..=4 {
            for index in 0..channels * 2 {
                for bytes in &bad {
                    let mut samples = vec![vec![0; precision.bytes]; channels * 2];
                    samples[index] = bytes.clone();
                    // A second bad sample must not replace the first error index.
                    *samples.last_mut().unwrap() = bytes.clone();
                    for coverage in [false, true] {
                        for order in [HistoricalXcfSampleOrder::BigEndian, HistoricalXcfSampleOrder::LittleEndian] {
                            compare_reference(&samples, precision, channels, coverage, order);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn double_coverage_normalizes_source_before_narrowing_and_preserves_underflow_and_zero() {
    let values = [-f64::from_bits(1), f64::from_bits(1.0f64.to_bits() + 1), f64::MAX, -f64::MAX];
    for order in [HistoricalXcfSampleOrder::BigEndian, HistoricalXcfSampleOrder::LittleEndian] {
        for values in [[0.0f64, f64::MAX, -f64::MAX], [0.0, 0.0, -f64::MAX]] {
            let samples: Vec<_> = values.into_iter().map(|value| value.to_be_bytes().to_vec()).collect();
            compare_reference(&samples, precision(8, true), 3, false, order);
            compare_reference(&samples, precision(8, true), 3, true, order);
        }
        for value in values {
            // Non-coverage colors accept signed HDR within range, reject overflow;
            // the same finite source in alpha normalizes before F32 narrowing.
            compare_reference(&[value.to_be_bytes().to_vec()], precision(8, true), 1, false, order);
            compare_reference(&[value.to_be_bytes().to_vec()], precision(8, true), 1, true, order);
            let samples = vec![0.5f64.to_be_bytes().to_vec(), value.to_be_bytes().to_vec()];
            compare_reference(&samples, precision(8, true), 2, false, order);
        }
    }
    let values = [f64::from_bits(1), -0.0, -f64::from_bits(1), -0.0];
    let raw: Vec<_> = values.into_iter().flat_map(f64::to_be_bytes).collect();
    let mut output = [1.0; 4];
    let changes = row_samples(&raw, &mut output, precision(8, true), HistoricalXcfSampleOrder::BigEndian, 2, false).unwrap();
    assert_eq!(changes, SampleChanges { underflow: true, coverage_clamped: false });
    assert_eq!(output.map(f32::to_bits), [0, (-0.0f32).to_bits(), (-0.0f32).to_bits(), (-0.0f32).to_bits()]);
}

#[test]
fn malformed_row_layouts_fail_at_sample_zero() {
    for (raw_len, output_len, bytes, channels) in [(4, 1, 4, 0), (20, 5, 4, 5), (12, 3, 4, 2), (3, 1, 4, 1), (5, 1, 4, 1), (3, 1, 3, 1)] {
        let error = row_samples(&vec![0; raw_len], &mut vec![0.0; output_len], precision(bytes, true), HistoricalXcfSampleOrder::BigEndian, channels, false)
            .unwrap_err();
        assert_eq!(error.sample, 0);
    }
}
