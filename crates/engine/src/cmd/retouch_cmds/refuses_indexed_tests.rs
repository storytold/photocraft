//! The toning and focus tools refuse Indexed Color and Bitmap documents with the reference
//! application's messages, changing nothing and adding no history; the live stroke is refused the
//! same way. RGB, Grayscale, CMYK and Lab at every depth, Multichannel and Duotone keep working.
//! Dodge #12686, Burn #12673, Sponge #13033, Blur #12656, Smudge #12967, Sharpen #12953.

use photocraft_color::{Color, ColorMode, SampleType};
use photocraft_doc::{Document, Size};
use serde_json::json;

use super::*;

/// What a stroke in a mode that keeps working must show besides its History step.
#[derive(Clone, Copy)]
enum Works {
    /// The stroke changed pixels.
    Changes,
    /// The stroke made the edge of [`Tool::edged`]'s document sharper.
    Sharpens,
    /// Only that it ran: the grey document has no saturation for the Sponge to take away.
    Runs,
}

/// One tool's row: its name in the refusal, its command, its stroke's own params, whether it
/// works on an edge, and how a working stroke shows.
struct Tool {
    name: &'static str,
    command: &'static str,
    params: fn() -> Value,
    edged: bool,
    works: Works,
}

const TOOLS: [Tool; 6] = [
    Tool { name: "dodge", command: "paint.dodge", params: || json!({"range": "midtones", "exposure": 100}), edged: false, works: Works::Changes },
    Tool { name: "burn", command: "paint.burn", params: || json!({"range": "midtones", "exposure": 100}), edged: false, works: Works::Changes },
    Tool { name: "sponge", command: "paint.sponge", params: || json!({"mode": "desaturate"}), edged: false, works: Works::Runs },
    Tool { name: "blur", command: "paint.blur", params: || json!({"strength": 100}), edged: true, works: Works::Changes },
    Tool { name: "smudge", command: "paint.smudge", params: || json!({"strength": 100}), edged: true, works: Works::Changes },
    Tool { name: "sharpen", command: "paint.sharpen", params: || json!({"strength": 100, "protectDetail": false}), edged: true, works: Works::Sharpens },
];

impl Tool {
    /// A 20 px hard stroke across the middle, with the tool's own params.
    fn stroke(&self) -> Value {
        let mut stroke = json!({"points": [[20, 32], [44, 32]], "size": 20, "hardness": 100});
        if let (Some(stroke), Value::Object(own)) = (stroke.as_object_mut(), (self.params)()) {
            stroke.extend(own);
        }
        stroke
    }

    /// The mid-grey document, with its left half black for the tools that work on an edge.
    fn document(&self, mode: ColorMode, depth: SampleType) -> Session {
        let mut s = session(mode, depth);
        if self.edged {
            s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 64})).unwrap();
            s.execute("edit.fill", json!({"contents": "color", "color": "#000000"})).unwrap();
            s.execute("select.deselect", json!({})).unwrap();
        }
        s
    }

    /// The reference application's refusal in a `mode` document ("Indexed Color" or "Bitmap"),
    /// which names the mode to convert to.
    fn refusal(&self, mode: &str, to: &str) -> String {
        format!("Could not use the {} tool because it does not work with {mode} images (convert image to {to} mode to edit).", self.name)
    }
}

/// A session with one 64 × 64 document of `mode` and `depth`, filled mid grey.
fn session(mode: ColorMode, depth: SampleType) -> Session {
    let mut s = Session::new();
    s.add_document(Document::with_background("t", Size::new(64, 64), mode, depth, Color::rgb(0.5, 0.5, 0.5)), None);
    s
}

/// The active document's History length.
fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

/// Every pixel of the background, in native channels.
fn pixels(s: &Session) -> Vec<f32> {
    let st = s.active().unwrap();
    let surf = st.doc.layers[0].surface().unwrap();
    (0..64).flat_map(|y| (0..64).flat_map(move |x| surf.pixel(x, y))).collect()
}

/// How far apart the two pixels across the edge are, summed over the native channels.
fn edge_contrast(s: &Session) -> f32 {
    let surf = s.active().unwrap().doc.layers[0].surface().unwrap();
    surf.pixel(31, 32).iter().zip(surf.pixel(32, 32)).map(|(a, b)| (a - b).abs()).sum()
}

#[test]
fn indexed_and_bitmap_documents_refuse_with_the_reference_words() {
    for tool in &TOOLS {
        // Per mode: the command's error, the live stroke's error, pixels kept, history kept.
        let got = [ColorMode::Indexed, ColorMode::Bitmap].map(|mode| {
            let mut s = tool.document(mode, SampleType::U8);
            let (before, n) = (pixels(&s), steps(&s));
            let err = s.execute(tool.command, tool.stroke()).err().map(|e| e.to_string());
            let live = LiveDab::begin(&s, tool.command, &tool.stroke()).err().map(|e| e.to_string());
            (err, live, pixels(&s) == before, steps(&s) == n)
        });
        let want = [tool.refusal("Indexed Color", "RGB Color"), tool.refusal("Bitmap", "Grayscale")].map(|m| (Some(m.clone()), Some(m), true, true));
        assert_eq!(got, want, "{}", tool.command);
    }
}

/// Every other mode and depth keeps working: one History step and a live stroke.
#[test]
fn other_modes_keep_working_at_every_depth() {
    let depths = [SampleType::U8, SampleType::U16, SampleType::F32];
    let measured = [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab].into_iter().flat_map(|m| depths.map(|d| (m, d)));
    // Not measured in the reference: they keep today's behaviour.
    let unmeasured = [(ColorMode::Multichannel, SampleType::U8), (ColorMode::Duotone, SampleType::U8)];
    for (mode, depth) in measured.chain(unmeasured) {
        for tool in &TOOLS {
            let case = format!("{} {mode:?}/{depth:?}", tool.command);
            let mut s = tool.document(mode, depth);
            let (before, contrast, n) = (pixels(&s), edge_contrast(&s), steps(&s));
            assert!(LiveDab::begin(&s, tool.command, &tool.stroke()).is_ok(), "{case}: live");
            s.execute(tool.command, tool.stroke()).unwrap_or_else(|e| panic!("{case}: {e}"));
            match tool.works {
                Works::Changes => assert_ne!(pixels(&s), before, "{case}: the stroke changed pixels"),
                Works::Sharpens => assert!(edge_contrast(&s) > contrast, "{case}: the edge got sharper ({contrast} -> {})", edge_contrast(&s)),
                Works::Runs => {}
            }
            assert_eq!(steps(&s), n + 1, "{case}: one history step");
        }
    }
}
