//! Color Table › Load… with swatches (`.aco`) and RIFF palettes (`.pal`) besides `.act` (#14372):
//! the file's first 256 colors in file order become the table; a file without colors is refused.
use super::*;

/// `n` distinct colors, each channel a multiple of 1/255 so it survives the `.aco` encoding.
fn colors(n: usize) -> Vec<[u8; 3]> {
    (0..n).map(|i| [(i % 256) as u8, (i / 256) as u8 * 100 + 7, 255 - (i % 256) as u8]).collect()
}

fn hexes(colors: &[[u8; 3]]) -> Vec<String> {
    colors.iter().map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}")).collect()
}

/// A swatches file of RGB `colors`, as the Swatches panel saves it.
fn aco(colors: &[[u8; 3]]) -> Vec<u8> {
    let entries = colors.iter().enumerate().map(|(i, c)| SwatchEntry::new(format!("c{i}"), SwatchColor::Rgb(c.map(|v| f32::from(v) / 255.0)))).collect();
    write_swatches(&[SwatchGroup { name: "t".into(), entries }], SwatchFormat::Aco).unwrap().0
}

/// A RIFF palette declaring `count` entries and holding `colors`, with an unrelated chunk of odd
/// length (so padded) before `data`.
fn pal(colors: &[[u8; 3]], count: u16) -> Vec<u8> {
    let mut data = vec![0x00, 0x03];
    data.extend_from_slice(&count.to_le_bytes());
    for c in colors {
        data.extend_from_slice(&[c[0], c[1], c[2], 0]);
    }
    let mut body = b"PAL ".to_vec();
    body.extend_from_slice(b"note");
    body.extend_from_slice(&3u32.to_le_bytes());
    body.extend_from_slice(b"abc\0");
    body.extend_from_slice(b"data");
    body.extend_from_slice(&(data.len() as u32).to_le_bytes());
    body.extend_from_slice(&data);
    let mut b = b"RIFF".to_vec();
    b.extend_from_slice(&(body.len() as u32).to_le_bytes());
    b.extend_from_slice(&body);
    b
}

#[test]
fn aco_and_pal_files_load_their_colors_in_file_order() {
    let s = Session::new();
    let want = colors(3);
    for (name, bytes) in [("swatches.aco", aco(&want)), ("SWATCHES.ACO", aco(&want)), ("palette.pal", pal(&want, 3)), ("Palette.PAL", pal(&want, 3))] {
        assert_eq!(color_table_file(&s, name, &bytes), Ok((hexes(&want), None)), "{name}");
    }
    // An `.act` still loads as before, transparent index included.
    let mut act = act_bytes(&want, Some(1)).unwrap();
    assert_eq!(color_table_file(&s, "table.act", &act), act_table(&act));
    act.truncate(10);
    assert!(color_table_file(&s, "table.act", &act).is_err());
}

#[test]
fn colors_past_256_are_dropped() {
    let s = Session::new();
    let all = colors(300);
    let first = hexes(&all[..ACT_COLORS]);
    for (name, bytes) in [("big.aco", aco(&all)), ("big.pal", pal(&all, 300))] {
        let (got, transparent) = color_table_file(&s, name, &bytes).unwrap();
        assert_eq!((got, transparent), (first.clone(), None), "{name}");
    }
}

#[test]
fn unreadable_or_empty_files_are_refused() {
    let s = Session::new();
    let three = colors(3);
    let mut no_data = pal(&three, 3);
    let at = no_data.windows(4).position(|w| w == b"data").unwrap();
    no_data[at..at + 4].copy_from_slice(b"junk");
    let cases: [(&str, Vec<u8>); 9] = [
        ("empty.aco", Vec::new()),
        ("none.aco", vec![0, 1, 0, 0]),
        ("text.aco", b"GIMP Palette\nName: x\n#\n255 0 0 red\n".to_vec()),
        ("act.aco", act_bytes(&three, None).unwrap()),
        ("empty.pal", Vec::new()),
        ("none.pal", pal(&[], 0)),
        ("short.pal", pal(&three[..2], 3)),
        ("nodata.pal", no_data),
        ("act.pal", act_bytes(&three, None).unwrap()),
    ];
    for (name, bytes) in cases {
        assert!(color_table_file(&s, name, &bytes).is_err(), "{name} should be refused");
    }
}
