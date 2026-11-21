use super::*;

fn wrapped_empty_members(count: usize) -> Vec<u8> {
    let empty = crabz2::compress(&[], crabz2::Level::default());
    let last = crabz2::compress(&[MAGIC, &[0; 21]].concat(), crabz2::Level::default());
    let mut bytes = Vec::with_capacity(empty.len() * count + last.len());
    for _ in 0..count {
        bytes.extend_from_slice(&empty);
    }
    bytes.extend_from_slice(&last);
    bytes
}

#[test]
fn bzip_concatenated_empty_members_preserve_header_and_full_decode() {
    let bytes = wrapped_empty_members(10_000);
    let expected = [MAGIC, &[0; 21]].concat();
    assert_eq!(header_prefix("members.xcf.bz2", &bytes, &Interrupt::NONE).unwrap().unwrap(), expected);
    assert_eq!(decode("members.xcf.bz2", &bytes, &Interrupt::NONE).unwrap().unwrap(), expected);
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let cancelled = || calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 3;
    assert!(matches!(header_prefix("members.xcf.bz2", &bytes, &Interrupt::cancel_only(&cancelled)), Err(IoError::Cancelled)));
}

#[test]
fn bzip_multiple_nonempty_blocks_keep_checksums_and_concatenation() {
    // Incompressible deterministic data forces a nonempty block across multiple
    // borrowed-input windows, exercising truncated retries and decoder rebasing.
    let mut state = 0x1234_abcd_u32;
    let noise: Vec<u8> = (0..2_000_000)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect();
    let plain = [MAGIC, &noise].concat();
    let split = 1_100_000;
    let mut compressed = crabz2::compress(&plain[..split], crabz2::Level::default());
    assert!(compressed.len() > CHUNK * 8, "fixture must exercise input-window expansion");
    compressed.extend_from_slice(&crabz2::compress(&[], crabz2::Level::default()));
    compressed.extend_from_slice(&crabz2::compress(&plain[split..], crabz2::Level::default()));
    assert_eq!(decode("blocks.xcf.bz2", &compressed, &Interrupt::NONE).unwrap().unwrap(), plain);
    assert_eq!(header_prefix("blocks.xcf.bz2", &compressed, &Interrupt::NONE).unwrap().unwrap(), plain[..30]);
    assert!(decode("blocks.xcf.bz2", &compressed[..compressed.len() - 1], &Interrupt::NONE).is_err());
    compressed.extend_from_slice(b"junk");
    assert!(decode("blocks.xcf.bz2", &compressed, &Interrupt::NONE).is_err());
}

#[test]
#[ignore = "manual subprocess peak-memory comparison; driven by the resource probe"]
fn bzip_empty_member_resource_probe() {
    use std::io::Write;
    let bytes = wrapped_empty_members(4_000_000);
    println!("BZIP_INPUT_READY");
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    assert_eq!(header_prefix("members.xcf.bz2", &bytes, &Interrupt::NONE).unwrap().unwrap(), [MAGIC, &[0; 21]].concat());
    println!("BZIP_DECODE_DONE");
    std::io::stdout().flush().unwrap();
    std::io::stdin().read_line(&mut line).unwrap();
}
