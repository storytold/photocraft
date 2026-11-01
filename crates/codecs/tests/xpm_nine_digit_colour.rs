//! XPM accepts X11 `#RRRGGGBBB` (3 hex digits per channel) colours.

use photocraft_codecs::*;

#[test]
fn xpm_decodes_nine_digit_hex_colour() {
    let file = b"/* XPM */\nstatic char *a[]={\n\"1 1 1 1\",\n\"a c #fffeeeddd\",\n\"a\"};";
    let img = decode_as(Format::Xpm, file).unwrap();
    assert_eq!(&img.data()[..3], &[255, 238, 221]);
}
