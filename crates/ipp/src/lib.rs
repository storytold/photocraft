//! Re-exports [`craft_ipp`] for backward compatibility with `photocraft-ipp`.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub use craft_ipp::*;

#[cfg(test)]
mod language_text {
    use super::{Attribute, TAG_NAME, TAG_PRINTER_GROUP, TAG_TEXT};

    fn language_value(language: &str, text: &[u8]) -> Vec<u8> {
        let mut value = (language.len() as u16).to_be_bytes().to_vec();
        value.extend_from_slice(language.as_bytes());
        value.extend_from_slice(&(text.len() as u16).to_be_bytes());
        value.extend_from_slice(text);
        value
    }

    #[test]
    fn language_tagged_text_uses_byte_lengths_and_keeps_plain_text_lossy() {
        for tag in [0x35, 0x36] {
            let value = language_value("de", "Büro 東京".as_bytes());
            let attribute = Attribute { group: TAG_PRINTER_GROUP, tag, name: "text".into(), values: vec![value, b"ignored".to_vec()] };
            assert_eq!(attribute.text(), "Büro 東京");
            let attribute = Attribute { values: vec![language_value("en", b"bad\xfftext")], ..attribute };
            assert_eq!(attribute.text(), "bad\u{fffd}text");
        }
        for tag in [TAG_TEXT, TAG_NAME] {
            let attribute = Attribute { group: TAG_PRINTER_GROUP, tag, name: "text".into(), values: vec![b"plain\xfftext".to_vec()] };
            assert_eq!(attribute.text(), "plain\u{fffd}text");
            assert_eq!(Attribute { values: Vec::new(), ..attribute }.text(), "");
        }
    }

    #[test]
    fn malformed_language_lengths_do_not_become_printer_text() {
        let malformed: &[&[u8]] = &[
            b"",
            b"\0",
            b"\xff\xff",
            b"\x80\0",
            b"\0\x02e",
            b"\0\x02en",
            b"\0\x02en\0",
            b"\0\x02en\xff\xff",
            b"\0\x02en\x80\0",
            b"\0\x02en\0\x02x",
            b"\0\x02en\0\x01xy",
            b"\x7f\xffen\0\0",
            b"\0\x02en\x7f\xffx",
        ];
        for tag in [0x35, 0x36] {
            for value in malformed {
                let attribute = Attribute { group: TAG_PRINTER_GROUP, tag, name: "text".into(), values: vec![value.to_vec()] };
                assert_eq!(attribute.text(), "", "tag {tag:#x}: {value:?}");
            }
            let attribute = Attribute { group: TAG_PRINTER_GROUP, tag, name: "text".into(), values: vec![language_value("en", b"")] };
            assert_eq!(attribute.text(), "");
        }
    }
}
