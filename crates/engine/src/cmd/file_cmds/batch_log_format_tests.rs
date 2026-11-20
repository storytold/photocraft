//! The Batch error log format (#13665): UTF-16 big-endian with BOM `FE FF`, CRLF line ends, the
//! first line `Start Batch`, the last line `End Batch` plus CRLF, and `File:` paths with the
//! platform's separators.

use super::*;

#[cfg(windows)]
fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-batch-log-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

fn utf16_be(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xfe, 0xff];
    for word in text.encode_utf16() {
        bytes.extend_from_slice(&word.to_be_bytes());
    }
    bytes
}

fn decode_be(bytes: &[u8]) -> String {
    assert_eq!(&bytes[..2], &[0xfe, 0xff], "BOM");
    let (pairs, remainder) = bytes[2..].as_chunks::<2>();
    assert!(remainder.is_empty(), "even length");
    String::from_utf16(&pairs.iter().map(|pair| u16::from_be_bytes(*pair)).collect::<Vec<_>>()).unwrap()
}

#[test]
fn the_log_has_a_big_endian_bom_and_big_endian_code_units() {
    let bytes = encode_batch_error_log([("a.png", "boom")]);
    assert_eq!(&bytes[..4], &[0xfe, 0xff, 0x00, b'S'], "BOM then the big-endian S of `Start Batch`");
}

#[test]
fn the_log_starts_with_start_batch_and_ends_with_end_batch_and_crlf() {
    let text = decode_be(&encode_batch_error_log([("a.png", "boom"), ("b.png", "bang")]));
    assert!(text.starts_with("Start Batch\r\n\r\n"), "{text:?}");
    assert!(text.ends_with("\r\nEnd Batch\r\n"), "{text:?}");
    assert_eq!(text, "Start Batch\r\n\r\nFile: \"a.png\"\r\nError: boom\r\n\r\nFile: \"b.png\"\r\nError: bang\r\n\r\nEnd Batch\r\n");
}

#[test]
fn an_error_free_log_is_exactly_bom_start_blank_end() {
    let no_errors: [(&str, &str); 0] = [];
    assert_eq!(encode_batch_error_log(no_errors), utf16_be("Start Batch\r\n\r\nEnd Batch\r\n"));
}

#[cfg(windows)]
#[test]
fn a_folder_given_with_slashes_is_written_with_backslashes_on_windows() {
    let dir = tmp("batch-log-slashes").replace('\\', "/");
    let missing = format!("{dir}/in/missing.png");
    let log = format!("{dir}/errors.txt");
    let result =
        Session::new().execute("file.automate.batch", json!({"steps": [], "input": [missing], "output": format!("{dir}/out"), "errorLog": log})).unwrap();
    assert_eq!(result["errors"].as_array().map(Vec::len), Some(1), "{result}");
    let text = decode_be(&std::fs::read(&log).unwrap());
    let expected = format!("File: \"{}\"\r\n", missing.replace('/', "\\"));
    assert!(text.contains(&expected), "{text:?} should contain {expected:?}");
}

#[cfg(windows)]
#[test]
fn the_encoder_writes_windows_separators_for_every_file_line() {
    let text = decode_be(&encode_batch_error_log([("C:/pc/in\\notes.txt", "x")]));
    assert!(text.contains("File: \"C:\\pc\\in\\notes.txt\"\r\n"), "{text:?}");
}

#[cfg(not(windows))]
#[test]
fn slashes_are_kept_off_windows() {
    let text = decode_be(&encode_batch_error_log([("/pc/in/notes.txt", "x")]));
    assert!(text.contains("File: \"/pc/in/notes.txt\"\r\n"), "{text:?}");
}
