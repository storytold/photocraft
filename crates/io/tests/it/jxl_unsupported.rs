//! Without the `jxl` feature a JPEG XL file is still recognised, and opening it is a clear error
//! naming what this build lacks, never a panic or "unrecognized format".

#![cfg(not(feature = "jxl"))]

use photocraft_io::import;

#[test]
fn jxl_without_the_feature_says_it_is_not_in_this_build() {
    let bare = &[0xFF, 0x0A, 0x30, 0x54][..];
    let container = b"\0\0\0\x0cJXL \r\n\x87\n\0\0\0\x14ftypjxl \0\0\0\0jxl ";
    for (name, bytes) in [("IMG_0001.JXL", bare), ("photo.jxl", container), ("noext", bare)] {
        let e = import(name, bytes).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(e.contains("isn't included in this build"), "{name}: {e}");
    }
}
