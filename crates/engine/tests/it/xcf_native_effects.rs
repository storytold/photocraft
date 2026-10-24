//! Original synthetic XCF bytes versus a counterpart created with native commands.
//! No private corpus, GIMP renderer or converted-document pixels define the oracle.

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerMask};
use photocraft_engine::{Session, file_cmds::open_bytes_as, smart_cmds};
use photocraft_geom::{Rect, Size};
use photocraft_raster::Surface;
use serde_json::json;

const PIXELS: [u8; 24] = [40, 100, 180, 255, 180, 60, 30, 255, 90, 160, 50, 128, 20, 220, 150, 255, 230, 100, 10, 255, 120, 60, 200, 200];
struct Wire(Vec<u8>);
impl Wire {
    fn word(&mut self, value: u32) {
        self.0.extend(value.to_be_bytes());
    }
    fn pointer(&mut self, value: usize) {
        self.0.extend((value as u64).to_be_bytes());
    }
    fn slot(&mut self) -> usize {
        let at = self.0.len();
        self.pointer(0);
        at
    }
    fn patch(&mut self, at: usize) {
        let value = self.0.len() as u64;
        self.0[at..at + 8].copy_from_slice(&value.to_be_bytes());
    }
    fn text(&mut self, value: &str) {
        self.word(value.len() as u32 + 1);
        self.0.extend(value.as_bytes());
        self.0.push(0);
    }
    fn property(&mut self, kind: u32, value: &[u8]) {
        self.word(kind);
        self.word(value.len() as u32);
        self.0.extend(value);
    }
    fn end(&mut self) {
        self.word(0);
        self.word(0);
    }
    fn hierarchy(&mut self, width: u32, height: u32, channels: u32, pixels: &[u8]) {
        self.word(width);
        self.word(height);
        self.word(channels);
        let level = self.slot();
        self.pointer(0);
        self.patch(level);
        self.word(width);
        self.word(height);
        let tile = self.slot();
        self.pointer(0);
        self.patch(tile);
        self.0.extend(pixels);
    }
}
struct Step<'a> {
    operation: &'a str,
    opacity: f32,
    visible: bool,
    masked: bool,
    malformed_mask: bool,
    arguments: Vec<(&'a str, u32, Vec<u8>)>,
}
impl<'a> Step<'a> {
    fn new(operation: &'a str) -> Self {
        Self { operation, opacity: 1.0, visible: true, masked: false, malformed_mask: false, arguments: vec![] }
    }
}
fn source(operation: &str, opacity: f32, visible: bool, masked: bool) -> Vec<u8> {
    source_stack(&[Step { opacity, visible, masked, ..Step::new(operation) }])
}
/// The slice is execution order; XCF stores drawable filter pointers in reverse.
fn source_stack(steps: &[Step<'_>]) -> Vec<u8> {
    source_stack_with_owner(steps, 3, 2, &PIXELS)
}
fn source_stack_with_owner(steps: &[Step<'_>], width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    source_stack_with_format(steps, width, height, pixels, false)
}
fn source_stack_with_format(steps: &[Step<'_>], width: u32, height: u32, pixels: &[u8], gray: bool) -> Vec<u8> {
    source_stack_with_format_version(steps, width, height, pixels, gray, 26)
}
fn source_stack_with_format_version(steps: &[Step<'_>], width: u32, height: u32, pixels: &[u8], gray: bool, version: u32) -> Vec<u8> {
    source_stack_with_options(steps, width, height, pixels, (gray, version, false))
}
fn source_stack_with_options(steps: &[Step<'_>], width: u32, height: u32, pixels: &[u8], options: (bool, u32, bool)) -> Vec<u8> {
    let (gray, version, with_sibling) = options;
    assert!([20, 26].contains(&version));
    let mut w = Wire(format!("gimp xcf v{version:03}\0").into_bytes());
    for value in [8, 6, u32::from(gray), 150] {
        w.word(value);
    }
    w.property(17, &[0]);
    w.end();
    let layer = w.slot();
    let sibling = with_sibling.then(|| w.slot());
    w.pointer(0);
    w.pointer(0);
    w.pointer(0);
    w.patch(layer);
    for value in [width, height, if gray { 3 } else { 1 }] {
        w.word(value);
    }
    w.text("Procedural owner");
    w.property(15, &[(-1i32).to_be_bytes(), 2i32.to_be_bytes()].concat());
    w.end();
    let hierarchy = w.slot();
    w.pointer(0);
    let effects: Vec<_> = steps.iter().map(|_| w.slot()).collect();
    w.pointer(0);
    w.patch(hierarchy);
    w.hierarchy(width, height, if gray { 2 } else { 4 }, pixels);
    for (slot, step) in effects.into_iter().zip(steps.iter().rev()) {
        w.patch(slot);
        w.text("Source operation");
        w.text("");
        w.text(step.operation);
        if version >= 22 {
            w.text(if step.operation == "gegl:exposure" { "1:0" } else { "0:0" });
        }
        w.property(33, &step.opacity.to_be_bytes());
        w.property(8, &u32::from(step.visible).to_be_bytes());
        for (name, kind, value) in &step.arguments {
            let mut arg = Wire(vec![]);
            arg.text(name);
            arg.word(*kind);
            arg.0.extend(value);
            w.property(45, &arg.0);
        }
        w.end();
        let mask = w.slot();
        if step.masked || step.malformed_mask {
            w.patch(mask);
            w.word(if step.malformed_mask { 0 } else { 8 });
            w.word(6);
            w.text("Global effect coverage");
            w.end();
            let hierarchy = w.slot();
            w.pointer(0);
            w.patch(hierarchy);
            let mut pixels = [0u8; 48];
            pixels[2 * 8] = 255;
            pixels[3 * 8] = 255;
            w.hierarchy(8, 6, 1, &pixels);
        }
    }
    if let Some(slot) = sibling {
        w.patch(slot);
        for value in [1, 1, if gray { 3 } else { 1 }] {
            w.word(value);
        }
        w.text("Unaffected graph sibling");
        w.property(15, &[6i32.to_be_bytes(), 1i32.to_be_bytes()].concat());
        w.end();
        let hierarchy = w.slot();
        w.pointer(0);
        w.pointer(0);
        w.patch(hierarchy);
        w.hierarchy(1, 1, if gray { 2 } else { 4 }, if gray { &[77, 255] } else { &[17, 83, 201, 255] });
    }
    w.0
}
fn native_counterpart(operation: &str, opacity: f32, visible: bool, masked: bool) -> Document {
    let mut local = Document::new("Native source", Size::new(3, 2), ColorMode::Rgb, SampleType::U8);
    local.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
    let mut layer = Layer::raster("Native pixels", local.pixel_format());
    layer.surface_mut().unwrap().write_region(Rect::new(0, 0, 3, 2), &PIXELS.map(|v| f32::from(v) / 255.0));
    local.layers.push(layer);
    let mut session = Session::new();
    session.add_document(local, None);
    session.execute("filter.convertForSmartFilters", json!({})).unwrap();
    if visible {
        if operation == "gegl:value-invert" {
            session.execute("filter.other.hsbHsl", json!({"inputMode":"rgb","rowOrder":"hsb"})).unwrap();
            session.execute("image.adjustments.curves", json!({"blue":[[0,255],[255,0]]})).unwrap();
            session.execute("filter.other.hsbHsl", json!({"inputMode":"hsb","rowOrder":"rgb"})).unwrap();
        } else {
            session.execute("image.adjustments.invert", json!({})).unwrap();
        }
        let id = session.active().unwrap().doc.layers[0].id;
        if let LayerContent::Smart(smart) = &mut std::sync::Arc::make_mut(&mut session.active_mut().unwrap().doc).layers[0].content {
            let mut surface = Surface::with_default(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false), &[if masked { 0.0 } else { opacity }]);
            if masked {
                surface.write_pixel(1, 0, &[opacity]);
                surface.write_pixel(1, 1, &[opacity]);
            }
            smart.filter_mask = Some(LayerMask { surface, ..LayerMask::reveal_all() });
            smart.cache = None;
        }
        smart_cmds::refresh(std::sync::Arc::make_mut(&mut session.active_mut().unwrap().doc), id).unwrap();
    }
    let pixels = session.active().unwrap().doc.layers[0].surface().unwrap();
    let mut expected = Document::new("Native expected", Size::new(8, 6), ColorMode::Rgb, SampleType::U8);
    let mut output = Layer::raster("Native result", expected.pixel_format());
    *output.surface_mut().unwrap() = photocraft_algo::resample::translate_surface(pixels, -1, 2);
    expected.layers.push(output);
    expected
}

#[test]
fn first_open_compound_stage_uses_native_preparation_whole_opacity_and_global_mask_placement() {
    for (operation, opacity, visible, masked) in [
        ("gegl:value-invert", 0.375, true, false),
        ("gegl:value-invert", 0.375, true, true),
        ("gegl:value-invert", 0.375, false, false),
        ("gegl:invert-gamma", 0.625, true, true),
    ] {
        let bytes = source(operation, opacity, visible, masked);
        let mut session = Session::new();
        session.color.settings.ask_on_missing = false;
        open_bytes_as(&mut session, "synthetic.xcf", &bytes, None, None).unwrap();
        let state = session.active().unwrap();
        assert!(state.source_read_only);
        let LayerContent::Smart(smart) = &state.doc.layers[0].content else { panic!("native smart structure") };
        assert!(smart.cache.is_some());
        let expected = native_counterpart(operation, opacity, visible, masked);
        let actual = photocraft_compose::flatten(&state.doc);
        let wanted = photocraft_compose::flatten(&expected);
        assert!(
            actual.px.iter().zip(&wanted.px).all(|(a, b)| a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 2.0 / 255.0)),
            "{operation} opacity{opacity} visible{visible} masked{masked}"
        );
        let report = state.doc.metadata.import_conversion.clone();
        session.execute("layer.setProps", json!({"opacity":0.5})).unwrap();
        session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(session.active().unwrap().doc.metadata.import_conversion, report);
        let saved = photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap();
        let mut reopened = Session::new();
        reopened.color.settings.ask_on_missing = false;
        open_bytes_as(&mut reopened, "native.pcraft", &saved, None, None).unwrap();
        assert!(!reopened.active().unwrap().source_read_only);
        assert_eq!(photocraft_compose::flatten(&reopened.active().unwrap().doc).px, actual.px);
        assert_eq!(reopened.active().unwrap().doc.metadata.import_conversion, report);
    }
}

/// Native smart replay intentionally tolerates old/unknown filters. Imported
/// mappings must also pass the current strict user-command parameter gate.
fn assert_current_native_commands(document: &Document, operation: &str, depth: usize) {
    assert!(depth < 64, "{operation}: excessive native nesting");
    fn layers(document: &Document, items: &[Layer], operation: &str, depth: usize) {
        for layer in items {
            match &layer.content {
                LayerContent::Group(group) => layers(document, &group.children, operation, depth + 1),
                LayerContent::Smart(smart) => {
                    for filter in &smart.smart_filters {
                        let mut target = Document::new("Native parameter validation", Size::new(3, 2), document.mode, document.depth);
                        target.icc_profile = document.icc_profile.clone();
                        let mut input = Layer::raster("Native input", target.pixel_format());
                        let pixel = photocraft_raster::from_rgba(&target.pixel_format(), [0.25, 0.5, 0.75, 1.0]);
                        input.surface_mut().unwrap().write_pixel(0, 0, &pixel);
                        target.layers.push(input);
                        let mut native = Session::new();
                        native.add_document(target, None);
                        native.execute("filter.convertForSmartFilters", json!({})).unwrap();
                        native
                            .execute(&filter.command, filter.params.clone())
                            .unwrap_or_else(|error| panic!("{operation}: strict native command {} rejected params {}: {error}", filter.command, filter.params));
                    }
                    if let photocraft_doc::SmartSource::Embedded { bytes, .. } = &smart.source {
                        assert!(photocraft_format::is_pcraft(bytes), "{operation}: generated smart source is not self-contained native data");
                        let nested = photocraft_format::load_from_bytes(bytes).unwrap();
                        assert_current_native_commands(&nested, operation, depth + 1);
                    }
                }
                _ => {}
            }
        }
    }
    layers(document, &document.layers, operation, depth);
}

/// Default-argument envelope smoke catches native command ids/parameter schemas,
/// first-open preparation and self-contained native saves. Retained operations
/// are reported explicitly; this does not claim their native mapping exists.
#[test]
fn finite_default_operations_prepare_and_reopen_or_report_retained_source() {
    use photocraft_doc::import_conversion::{ConversionDisposition, ImportConversion};
    // Authored from the frozen 182 canonical identities, five aliases and G001.
    // This explicit list has no runtime dependency on local planning files.
    const OPERATIONS: &[&str] = &[
        "gegl:absolute",
        "gegl:alien-map",
        "gegl:alpha-clip",
        "gegl:antialias",
        "gegl:apply-lens",
        "gegl:bayer-matrix",
        "gegl:bevel",
        "gegl:bilateral-filter",
        "gegl:bloom",
        "gegl:box-blur",
        "gegl:brightness-contrast",
        "gegl:bump-map",
        "gegl:c2g",
        "gegl:cartoon",
        "gegl:cell-noise",
        "gegl:channel-mixer",
        "gegl:checkerboard",
        "gegl:color-assimilation-grid",
        "gegl:color-enhance",
        "gegl:color-exchange",
        "gegl:color-overlay",
        "gegl:color-rotate",
        "gegl:color-temperature",
        "gegl:color-to-alpha",
        "gegl:color-warp",
        "gegl:component-extract",
        "gegl:convolution-matrix",
        "gegl:cubism",
        "gegl:deinterlace",
        "gegl:denoise-dct",
        "gegl:difference-of-gaussians",
        "gegl:diffraction-patterns",
        "gegl:displace",
        "gegl:distance-transform",
        "gegl:dither",
        "gegl:domain-transform",
        "gegl:dropshadow",
        "gegl:edge",
        "gegl:edge-laplace",
        "gegl:edge-neon",
        "gegl:edge-sobel",
        "gegl:emboss",
        "gegl:engrave",
        "gegl:exposure",
        "gegl:fattal02",
        "gegl:focus-blur",
        "gegl:fractal-explorer",
        "gegl:fractal-trace",
        "gegl:gaussian-blur",
        "gegl:gaussian-blur-selective",
        "gegl:grid",
        "gegl:high-pass",
        "gegl:hue-chroma",
        "gegl:illusion",
        "gegl:image-gradient",
        "gegl:inner-glow",
        "gegl:invert-gamma",
        "gegl:invert-linear",
        "gegl:lens-blur",
        "gegl:lens-distortion",
        "gegl:lens-flare",
        "gegl:levels",
        "gegl:linear-sinusoid",
        "gegl:local-threshold",
        "gegl:long-shadow",
        "gegl:mantiuk06",
        "gegl:maze",
        "gegl:mean-curvature-blur",
        "gegl:median-blur",
        "gegl:mirrors",
        "gegl:mono-mixer",
        "gegl:mosaic",
        "gegl:motion-blur-circular",
        "gegl:motion-blur-linear",
        "gegl:motion-blur-zoom",
        "gegl:negative-darkroom",
        "gegl:newsprint",
        "gegl:noise-cie-lch",
        "gegl:noise-hsv",
        "gegl:noise-hurl",
        "gegl:noise-pick",
        "gegl:noise-reduction",
        "gegl:noise-rgb",
        "gegl:noise-slur",
        "gegl:noise-solid",
        "gegl:noise-spread",
        "gegl:normal-map",
        "gegl:oilify",
        "gegl:panorama-projection",
        "gegl:perlin-noise",
        "gegl:photocopy",
        "gegl:pixelize",
        "gegl:plasma",
        "gegl:recursive-transform",
        "gegl:red-eye-removal",
        "gegl:reinhard05",
        "gegl:rgb-clip",
        "gegl:ripple",
        "gegl:saturation",
        "gegl:sepia",
        "gegl:shadows-highlights",
        "gegl:shift",
        "gegl:simplex-noise",
        "gegl:sinus",
        "gegl:slic",
        "gegl:snn-mean",
        "gegl:softglow",
        "gegl:spherize",
        "gegl:spiral",
        "gegl:stereographic-projection",
        "gegl:stress",
        "gegl:stretch-contrast",
        "gegl:stretch-contrast-hsv",
        "gegl:styles",
        "gegl:supernova",
        "gegl:texturize-canvas",
        "gegl:tile-glass",
        "gegl:tile-paper",
        "gegl:tile-seamless",
        "gegl:unpremultiply",
        "gegl:unsharp-mask",
        "gegl:value-invert",
        "gegl:value-propagate",
        "gegl:variable-blur",
        "gegl:vibrance",
        "gegl:video-degradation",
        "gegl:vignette",
        "gegl:waterpixels",
        "gegl:waves",
        "gegl:whirl-pinch",
        "gegl:wind",
        "gimp:addition-legacy",
        "gimp:anti-erase",
        "gimp:behind",
        "gimp:border",
        "gimp:brightness-contrast",
        "gimp:buffer-source-validate",
        "gimp:burn-legacy",
        "gimp:color-balance",
        "gimp:colorize",
        "gimp:compose-crop",
        "gimp:curves",
        "gimp:darken-only-legacy",
        "gimp:desaturate",
        "gimp:difference-legacy",
        "gimp:divide-legacy",
        "gimp:dodge-legacy",
        "gimp:erase",
        "gimp:fill-source",
        "gimp:flood",
        "gimp:gradient",
        "gimp:grain-extract-legacy",
        "gimp:grain-merge-legacy",
        "gimp:grow",
        "gimp:hardlight-legacy",
        "gimp:hsl-color-legacy",
        "gimp:hsv-hue-legacy",
        "gimp:hsv-saturation-legacy",
        "gimp:hsv-value-legacy",
        "gimp:hue-saturation",
        "gimp:layer-mode",
        "gimp:levels",
        "gimp:lighten-only-legacy",
        "gimp:mask-components",
        "gimp:merge",
        "gimp:multiply-legacy",
        "gimp:normal",
        "gimp:overwrite",
        "gimp:pass-through",
        "gimp:posterize",
        "gimp:profile-transform",
        "gimp:replace",
        "gimp:scalar-multiply",
        "gimp:screen-legacy",
        "gimp:semi-flatten",
        "gimp:set-alpha",
        "gimp:shrink",
        "gimp:softlight-legacy",
        "gimp:split",
        "gimp:subtract-legacy",
        "gimp:threshold",
        "gimp:threshold-alpha",
        "gegl:abs",
        "gegl:color-reduction",
        "gegl:invert",
        "gegl:motion-blur",
        "gegl:little-planet",
        "gegl:gegl",
    ];
    for &operation in OPERATIONS {
        let bytes = source(operation, 1.0, true, false);
        let mut session = Session::new();
        session.color.settings.ask_on_missing = false;
        open_bytes_as(&mut session, "synthetic-default.xcf", &bytes, None, None)
            .unwrap_or_else(|error| panic!("{operation}: first-open preparation failed: {error}"));
        let state = session.active().unwrap();
        assert!(state.source_read_only, "{operation}");
        let Some(ImportConversion::Report(report)) = &state.doc.metadata.import_conversion else { panic!("{operation}: missing report") };
        let effect =
            report.entries.iter().find(|entry| entry.feature_code == "xcf.layer_effect").unwrap_or_else(|| panic!("{operation}: missing operation outcome"));
        assert!(
            matches!(effect.disposition, ConversionDisposition::NativeApproximation | ConversionDisposition::RetainedUnapplied),
            "{operation}: unexpected disposition {:?}",
            effect.disposition
        );
        assert!(
            report.entries.iter().any(|entry| entry.feature_code == "xcf.source_property.45" && entry.source == effect.source && !entry.attachments.is_empty())
                && report.attachments.iter().any(|attachment| attachment.owner == effect.source && !attachment.bytes.is_empty()),
            "{operation}: source record was not retained under its operation identity"
        );
        println!("{operation}: {:?}: {}", effect.disposition, effect.native_description);
        if let LayerContent::Smart(smart) = &state.doc.layers[0].content {
            assert!(smart.cache.is_some(), "{operation}: first-open native cache not prepared");
        }
        assert_current_native_commands(&state.doc, operation, 0);
        let pixels = photocraft_compose::flatten(&state.doc).px;
        assert!(pixels.iter().flatten().all(|sample| sample.is_finite()), "{operation}: nonfinite native output");
        let report = state.doc.metadata.import_conversion.clone();
        let saved = photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap();
        drop(session);
        drop(bytes);
        let mut reopened = Session::new();
        reopened.color.settings.ask_on_missing = false;
        open_bytes_as(&mut reopened, "native-default.pcraft", &saved, None, None).unwrap_or_else(|error| panic!("{operation}: native reopen failed: {error}"));
        assert!(!reopened.active().unwrap().source_read_only, "{operation}");
        assert_eq!(photocraft_compose::flatten(&reopened.active().unwrap().doc).px, pixels, "{operation}");
        assert_eq!(reopened.active().unwrap().doc.metadata.import_conversion, report, "{operation}");
    }
}

fn independently_native_chain(commands: &[(&str, serde_json::Value)]) -> Document {
    let mut local = Document::new("Independent native commands", Size::new(3, 2), ColorMode::Rgb, SampleType::U8);
    local.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
    let mut layer = Layer::raster("Original pixels", local.pixel_format());
    layer.surface_mut().unwrap().write_region(Rect::new(0, 0, 3, 2), &PIXELS.map(|v| f32::from(v) / 255.0));
    local.layers.push(layer);
    let mut native = Session::new();
    native.add_document(local, None);
    native.execute("filter.convertForSmartFilters", json!({})).unwrap();
    for (command, params) in commands {
        native.execute(command, params.clone()).unwrap();
    }
    let mut expected = Document::new("Native expected", Size::new(8, 6), ColorMode::Rgb, SampleType::U8);
    let mut layer = Layer::raster("Independent native result", expected.pixel_format());
    *layer.surface_mut().unwrap() = photocraft_algo::resample::translate_surface(native.active().unwrap().doc.layers[0].surface().unwrap(), -1, 2);
    expected.layers.push(layer);
    expected
}

#[test]
fn transparent_native_paper_replaces_full_stage_and_partial_stage_uses_native_underlay() {
    use photocraft_doc::import_conversion::{ConversionDisposition, ImportConversion};
    // A 3x2 owner contains no complete 100x100 tile. Native "omit partial tiles"
    // therefore has a transparent result, independent of seeded tile placement.
    for opacity in [1.0, 0.375] {
        let mut step = Step::new("gegl:tile-paper");
        step.opacity = opacity;
        step.arguments = vec![
            ("tile-width", 1, 100i32.to_be_bytes().to_vec()),
            ("tile-height", 1, 100i32.to_be_bytes().to_vec()),
            ("background-type", 5, 0i32.to_be_bytes().to_vec()),
            ("fractional-type", 5, 0i32.to_be_bytes().to_vec()),
            ("centering", 2, 0u32.to_be_bytes().to_vec()),
        ];
        let bytes = source_stack(&[step]);
        let mut session = Session::new();
        session.color.settings.ask_on_missing = false;
        open_bytes_as(&mut session, "transparent-paper.xcf", &bytes, None, None).unwrap();
        let state = session.active().unwrap();
        let actual = photocraft_compose::flatten(&state.doc).px;
        let expected = if opacity == 1.0 {
            vec![[0.0; 4]; 8 * 6]
        } else {
            // Ordinary native source-over of a transparent result reveals its
            // unchanged native underlay, including the original fractional alpha.
            photocraft_compose::flatten(&independently_native_chain(&[])).px
        };
        assert_eq!(actual, expected, "opacity {opacity}");
        let Some(ImportConversion::Report(report)) = &state.doc.metadata.import_conversion else { panic!("missing report") };
        assert!(report.entries.iter().any(|entry| entry.feature_code == "xcf.layer_effect" && entry.disposition == ConversionDisposition::NativeApproximation));
        assert_current_native_commands(&state.doc, "gegl:tile-paper", 0);
        let saved = photocraft_format::save_to_bytes(&state.doc, &Default::default()).unwrap();
        drop(bytes);
        let mut reopened = Session::new();
        reopened.color.settings.ask_on_missing = false;
        open_bytes_as(&mut reopened, "paper.pcraft", &saved, None, None).unwrap();
        assert_eq!(photocraft_compose::flatten(&reopened.active().unwrap().doc).px, actual);
        reopened.execute("layer.setProps", json!({"opacity":0.6})).unwrap();
        reopened.execute("edit.undo", json!({})).unwrap();
        assert_eq!(photocraft_compose::flatten(&reopened.active().unwrap().doc).px, actual);
    }
}

#[test]
fn noncommuting_stored_order_and_omitted_siblings_match_independent_native_commands() {
    use photocraft_doc::import_conversion::{ConversionDisposition, ImportConversion};
    for omitted in [false, true] {
        let mut exposure = Step::new("gegl:exposure");
        exposure.arguments.push(("exposure", 3, 1.0f32.to_be_bytes().to_vec()));
        let mut steps = vec![exposure];
        if omitted {
            steps.push(Step::new("gegl:unmapped-original-test-operation"));
        }
        steps.push(Step::new("gegl:invert-gamma"));
        let source = source_stack(&steps);
        let mut session = Session::new();
        session.color.settings.ask_on_missing = false;
        open_bytes_as(&mut session, "order.xcf", &source, None, None).unwrap();
        let expected = independently_native_chain(&[
            ("image.adjustments.exposure", json!({"exposure":1.0,"offset":0.0,"gamma":1.0})),
            ("image.adjustments.invert", json!({})),
        ]);
        let reversed = independently_native_chain(&[
            ("image.adjustments.invert", json!({})),
            ("image.adjustments.exposure", json!({"exposure":1.0,"offset":0.0,"gamma":1.0})),
        ]);
        let pixels = photocraft_compose::flatten(&session.active().unwrap().doc).px;
        let expected = photocraft_compose::flatten(&expected).px;
        assert_ne!(expected, photocraft_compose::flatten(&reversed).px, "oracle must distinguish order");
        assert!(pixels.iter().zip(&expected).all(|(a, b)| a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 2.0 / 255.0)), "omitted{omitted}");
        let Some(ImportConversion::Report(report)) = &session.active().unwrap().doc.metadata.import_conversion else { panic!() };
        let outcomes: Vec<_> = report.entries.iter().filter(|entry| entry.feature_code == "xcf.layer_effect").collect();
        assert_eq!(outcomes.len(), if omitted { 3 } else { 2 });
        if omitted {
            assert_eq!(outcomes[1].disposition, ConversionDisposition::RetainedUnapplied);
            assert!(outcomes[2].native_description.contains("earlier source stage was omitted"));
        }
        let report = session.active().unwrap().doc.metadata.import_conversion.clone();
        let saved = photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap();
        drop(source);
        let mut reopened = Session::new();
        reopened.color.settings.ask_on_missing = false;
        open_bytes_as(&mut reopened, "native-order.pcraft", &saved, None, None).unwrap();
        assert_eq!(photocraft_compose::flatten(&reopened.active().unwrap().doc).px, pixels);
        assert_eq!(reopened.active().unwrap().doc.metadata.import_conversion, report);
    }
}

#[test]
fn source_alpha_extraction_becomes_an_ordinary_native_alpha_to_color_filter() {
    use photocraft_doc::import_conversion::{ConversionDisposition, ImportConversion};
    for invert in [false, true] {
        let mut step = Step::new("gegl:component-extract");
        step.arguments.push(("component", 5, 20i32.to_be_bytes().to_vec()));
        step.arguments.push(("invert", 2, u32::from(invert).to_be_bytes().to_vec()));
        let bytes = source_stack(&[step]);
        let mut session = Session::new();
        session.color.settings.ask_on_missing = false;
        open_bytes_as(&mut session, "alpha-extraction.xcf", &bytes, None, None).unwrap();
        let mut commands = Vec::new();
        if invert {
            commands.push(("filter.other.alphaCurve", json!({"values":(0..256).map(|i|1.0-i as f32/255.0).collect::<Vec<_>>()})));
        }
        commands.push(("filter.other.alphaToColor", json!({})));
        let expected = independently_native_chain(&commands);
        let actual = photocraft_compose::flatten(&session.active().unwrap().doc).px;
        assert_eq!(actual, photocraft_compose::flatten(&expected).px);
        let Some(ImportConversion::Report(report)) = &session.active().unwrap().doc.metadata.import_conversion else { panic!() };
        let outcome = report.entries.iter().find(|entry| entry.feature_code == "xcf.layer_effect").unwrap();
        assert_eq!(outcome.disposition, ConversionDisposition::NativeApproximation);
        assert_current_native_commands(&session.active().unwrap().doc, "gegl:component-extract", 0);
        let saved = photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap();
        drop(bytes);
        let mut reopened = Session::new();
        reopened.color.settings.ask_on_missing = false;
        open_bytes_as(&mut reopened, "alpha-extraction.pcraft", &saved, None, None).unwrap();
        assert_eq!(photocraft_compose::flatten(&reopened.active().unwrap().doc).px, actual);
        reopened.execute("layer.setProps", json!({"opacity":0.5})).unwrap();
        reopened.execute("edit.undo", json!({})).unwrap();
        assert_eq!(photocraft_compose::flatten(&reopened.active().unwrap().doc).px, actual);
    }
}

#[test]
fn malformed_optional_effect_mask_and_unknown_component_are_retained_without_applying_stage() {
    use photocraft_doc::import_conversion::{ConversionDisposition, ConversionReason, ImportConversion};
    for unknown_component in [false, true] {
        let mut unsupported = Step::new(if unknown_component { "gegl:component-extract" } else { "gegl:invert-gamma" });
        if unknown_component {
            unsupported.arguments.push(("component", 5, 999i32.to_be_bytes().to_vec()));
        } else {
            unsupported.malformed_mask = true;
        }
        let mut exposure = Step::new("gegl:exposure");
        exposure.arguments.push(("exposure", 3, 1.0f32.to_be_bytes().to_vec()));
        let source = source_stack(&[unsupported, exposure]);
        let mut session = Session::new();
        session.color.settings.ask_on_missing = false;
        open_bytes_as(&mut session, "optional.xcf", &source, None, None).unwrap();
        let expected = independently_native_chain(&[("image.adjustments.exposure", json!({"exposure":1.0,"offset":0.0,"gamma":1.0}))]);
        let pixels = photocraft_compose::flatten(&session.active().unwrap().doc).px;
        assert_eq!(pixels, photocraft_compose::flatten(&expected).px, "unknown_component{unknown_component}");
        let Some(ImportConversion::Report(report)) = &session.active().unwrap().doc.metadata.import_conversion else { panic!() };
        let outcomes: Vec<_> = report.entries.iter().filter(|entry| entry.feature_code == "xcf.layer_effect").collect();
        assert_eq!(outcomes[0].disposition, ConversionDisposition::RetainedUnapplied);
        assert_eq!(outcomes[0].reason, if unknown_component { ConversionReason::MissingMapping } else { ConversionReason::InvalidOptionalData });
        if unknown_component {
            assert!(outcomes[0].native_description.contains("Unknown source extraction component"));
        }
        assert_eq!(outcomes[1].disposition, ConversionDisposition::NativeApproximation);
        let report = session.active().unwrap().doc.metadata.import_conversion.clone();
        let saved = photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap();
        drop(source);
        let mut reopened = Session::new();
        reopened.color.settings.ask_on_missing = false;
        open_bytes_as(&mut reopened, "native-optional.pcraft", &saved, None, None).unwrap();
        assert_eq!(photocraft_compose::flatten(&reopened.active().unwrap().doc).px, pixels);
        assert_eq!(reopened.active().unwrap().doc.metadata.import_conversion, report);
    }
}

/// Whole-source preparation and persistence checks. Numerical native operation
/// counterparts are tested independently in photocraft-io; these assertions
/// deliberately check integration rather than reuse converter output as an oracle.
fn nondefault_native_source_prepares_edits_and_reopens(step: Step<'_>, required_commands: &[&str], requires_mask: bool, required_frame: Option<Size>) {
    nondefault_native_source_in_mode(step, required_commands, requires_mask, required_frame, false);
}
fn nondefault_native_source_in_mode(step: Step<'_>, required_commands: &[&str], requires_mask: bool, required_frame: Option<Size>, gray: bool) {
    use photocraft_doc::import_conversion::{ConversionDisposition, ImportConversion};
    fn inspect(doc: &Document, commands: &mut Vec<String>, masks: &mut usize, frames: &mut Vec<Size>, depth: usize) {
        assert!(depth < 64);
        frames.push(doc.size);
        fn layers(items: &[Layer], commands: &mut Vec<String>, masks: &mut usize, frames: &mut Vec<Size>, depth: usize) {
            for layer in items {
                *masks += usize::from(layer.mask.is_some()) + usize::from(layer.vector_mask.is_some());
                match &layer.content {
                    LayerContent::Group(group) => layers(&group.children, commands, masks, frames, depth + 1),
                    LayerContent::Smart(smart) => {
                        let cache = smart.cache.as_ref().unwrap_or_else(|| panic!("native first-open cache missing for {}", layer.name));
                        let samples = cache.read_region(cache.content_bounds());
                        assert!(samples.iter().all(|sample| sample.is_finite()), "nonfinite native cache for {}", layer.name);
                        for pixel in samples.chunks_exact(cache.channels()) {
                            let rgba = photocraft_raster::to_rgba(&cache.format(), pixel);
                            assert!((0.0..=1.0).contains(&rgba[3]), "invalid native cache alpha for {}", layer.name);
                        }
                        commands.extend(smart.smart_filters.iter().map(|filter| filter.command.clone()));
                        if let photocraft_doc::SmartSource::Embedded { bytes, .. } = &smart.source {
                            inspect(&photocraft_format::load_from_bytes(bytes).unwrap(), commands, masks, frames, depth + 1);
                        }
                    }
                    _ => {}
                }
            }
        }
        layers(&doc.layers, commands, masks, frames, depth);
    }
    // A larger original owner gives native kernels interior samples. All older
    // source_stack callers keep their identical3x2 bytes and assertions.
    let pixels: Vec<u8> =
        (0..5u8).flat_map(|y| (0..7u8).flat_map(move |x| [13 + x * 29, 19 + y * 37, 180, if (x + y) % 2 == 0 { 128 } else { 224 }])).collect();
    let operation = step.operation;
    let pixels = if gray { pixels.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[3]]).collect() } else { pixels };
    let source = source_stack_with_format(&[step], 7, 5, &pixels, gray);
    let mut session = Session::new();
    session.color.settings.ask_on_missing = false;
    open_bytes_as(&mut session, "original-optical.xcf", &source, None, None).unwrap_or_else(|error| panic!("{operation}: first-open failed: {error}"));
    let state = session.active().unwrap();
    assert_eq!(state.doc.mode, if gray { ColorMode::Grayscale } else { ColorMode::Rgb });
    assert!(state.source_read_only);
    let Some(ImportConversion::Report(report)) = &state.doc.metadata.import_conversion else { panic!("missing source report") };
    let outcome = report.entries.iter().find(|entry| entry.feature_code == "xcf.layer_effect").unwrap();
    assert_eq!(outcome.disposition, ConversionDisposition::NativeApproximation, "{operation}: {}", outcome.native_description);
    let LayerContent::Smart(smart) = &state.doc.layers[0].content else { panic!("native optical Smart Object") };
    assert!(smart.cache.is_some(), "{operation}: first-open cache missing");
    assert_current_native_commands(&state.doc, operation, 0);
    let mut commands = Vec::new();
    let mut masks = 0;
    let mut frames = Vec::new();
    inspect(&state.doc, &mut commands, &mut masks, &mut frames, 0);
    if let Some(frame) = required_frame {
        assert!(frames.contains(&frame), "{operation}: missing actual native filter parent frame {frame:?}; {frames:?}");
    }
    for command in required_commands {
        assert!(commands.iter().any(|actual| actual == command), "{operation}: missing native command {command}; {commands:?}");
    }
    assert!(commands.iter().all(|id| !id.starts_with("gegl:") && !id.starts_with("gimp:")));
    if requires_mask {
        assert!(masks > 0, "{operation}: native optical coverage missing");
    }
    let actual = photocraft_compose::flatten(&state.doc).px;
    assert!(actual.iter().flatten().all(|v| v.is_finite()), "{operation}: nonfinite native samples");
    assert!(actual.iter().all(|v| (0. ..=1.).contains(&v[3])), "{operation}: invalid native coverage");
    assert!(actual.iter().any(|v| v[3] > 0.), "{operation}: meaningful source variant unexpectedly empty");
    let report = state.doc.metadata.import_conversion.clone();
    session.execute("layer.setProps", json!({"opacity":0.25})).unwrap();
    assert_ne!(photocraft_compose::flatten(&session.active().unwrap().doc).px, actual, "{operation}: first-open native property edit had no effect");
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(photocraft_compose::flatten(&session.active().unwrap().doc).px, actual);
    assert_eq!(session.active().unwrap().doc.metadata.import_conversion, report);
    let saved = photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap();
    drop(source);
    drop(session);
    let mut reopened = Session::new();
    reopened.color.settings.ask_on_missing = false;
    open_bytes_as(&mut reopened, "self-contained-optical.pcraft", &saved, None, None).unwrap();
    assert!(!reopened.active().unwrap().source_read_only);
    assert_eq!(photocraft_compose::flatten(&reopened.active().unwrap().doc).px, actual);
    assert_eq!(reopened.active().unwrap().doc.metadata.import_conversion, report);
    assert_current_native_commands(&reopened.active().unwrap().doc, operation, 0);
    reopened.execute("layer.setProps", json!({"opacity":0.25})).unwrap();
    assert_ne!(photocraft_compose::flatten(&reopened.active().unwrap().doc).px, actual);
    reopened.execute("edit.undo", json!({})).unwrap();
    assert_eq!(photocraft_compose::flatten(&reopened.active().unwrap().doc).px, actual);
    assert_eq!(reopened.active().unwrap().doc.metadata.import_conversion, report);
}
#[test]
fn nondefault_lens_distortion_source_prepares_native_radial_vignette_and_reopens_without_source() {
    let mut step = Step::new("gegl:lens-distortion");
    step.arguments = vec![("main", 3, 25f32.to_be_bytes().to_vec()), ("edge", 3, 10f32.to_be_bytes().to_vec()), ("brighten", 3, 50f32.to_be_bytes().to_vec())];
    nondefault_native_source_prepares_edits_and_reopens(step, &["filter.distort.displace", "filter.lensCorrection"], true, None);
}
#[test]
fn nondefault_inverse_panorama_source_prepares_native_map_mask_and_reopens_without_source() {
    let mut step = Step::new("gegl:panorama-projection");
    step.arguments = vec![("inverse", 2, 1u32.to_be_bytes().to_vec()), ("zoom", 3, 10f32.to_be_bytes().to_vec())];
    nondefault_native_source_prepares_edits_and_reopens(step, &["filter.distort.displace"], true, None);
}
#[test]
fn nondefault_panorama_output_frame_source_prepares_and_reopens_without_source() {
    let mut step = Step::new("gegl:panorama-projection");
    // FILTER_PROP_INT is kind1 in the actual decoder, not an inferred zero enum.
    step.arguments = vec![("width", 1, 4i32.to_be_bytes().to_vec()), ("height", 1, 3i32.to_be_bytes().to_vec())];
    nondefault_native_source_prepares_edits_and_reopens(step, &["filter.distort.displace"], false, None);
}

#[test]
fn whirl_half_radius_first_open_prepares_actual_native_frame_edits_undo_and_reopens() {
    let mut step = Step::new("gegl:whirl-pinch");
    step.arguments = vec![("radius", 3, 0.5f32.to_be_bytes().to_vec())];
    nondefault_native_source_prepares_edits_and_reopens(step, &["filter.distort.twirl"], true, Some(Size::new(4, 3)));
}

#[test]
fn broad_unsharp_first_open_prepares_native_branches_edits_undo_and_reopens() {
    let mut step = Step::new("gegl:unsharp-mask");
    step.arguments = vec![("std-dev", 3, 1500f32.to_be_bytes().to_vec()), ("threshold", 3, 0f32.to_be_bytes().to_vec())];
    nondefault_native_source_prepares_edits_and_reopens(step, &["filter.blur.gaussianBlur", "image.adjustments.exposure"], false, None);
    let mut thresholded = Step::new("gegl:unsharp-mask");
    thresholded.arguments =
        vec![("std-dev", 3, 1500f32.to_be_bytes().to_vec()), ("scale", 3, 15f32.to_be_bytes().to_vec()), ("threshold", 3, 0.1f32.to_be_bytes().to_vec())];
    nondefault_native_source_prepares_edits_and_reopens(thresholded, &["filter.sharpen.unsharpMask"], false, None);
}

#[test]
fn large_stress_first_open_prepares_native_branches_edits_undo_and_reopens() {
    let mut step = Step::new("gegl:stress");
    step.arguments = vec![("radius", 1, 6000i32.to_be_bytes().to_vec())];
    nondefault_native_source_prepares_edits_and_reopens(
        step,
        &["filter.blur.gaussianBlur", "image.adjustments.shadowsHighlights", "image.adjustments.channelMixer"],
        false,
        None,
    );
}

#[test]
fn large_waterpixels_first_open_prepares_native_framed_samples_and_reopens() {
    for (size, fill) in [(301i32, 0i32), (i32::MAX, 1i32)] {
        let mut step = Step::new("gegl:waterpixels");
        step.arguments = vec![("size", 1, size.to_be_bytes().to_vec()), ("fill", 5, fill.to_be_bytes().to_vec())];
        // The random-point policy uses an actual native1x1 frame, not a fake
        // Average command; inspect its placement/cache/persistence below.
        let commands: &[&str] = if fill == 0 { &["filter.blur.average"] } else { &[] };
        nondefault_native_source_prepares_edits_and_reopens(step, commands, false, if fill == 1 { Some(Size::new(1, 1)) } else { None });
    }
}

#[test]
fn native_c2g_local_contrast_prepares_small_and_broad_radii_and_reopens() {
    for radius in [6i32, 6000] {
        let mut step = Step::new("gegl:c2g");
        step.arguments = vec![("radius", 1, radius.to_be_bytes().to_vec())];
        nondefault_native_source_prepares_edits_and_reopens(step, &["image.adjustments.blackWhite"], false, None);
    }
}

#[test]
fn gradient_direction_rgb_and_gray_first_open_prepares_native_axes_edits_and_reopens() {
    for gray in [false, true] {
        let mut step = Step::new("gegl:image-gradient");
        step.arguments = vec![("output-mode", 5, 1i32.to_be_bytes().to_vec())];
        nondefault_native_source_in_mode(step, &["filter.other.custom", "image.adjustments.colorLookup", "filter.other.convolution"], false, None, gray);
    }
}

#[test]
fn independent_newsprint_plates_prepare_rgb_working_contents_for_gray_and_reopen() {
    for gray in [false, true] {
        for model in [2i32, 3] {
            let mut step = Step::new("gegl:newsprint");
            step.arguments = vec![("color-model", 5, model.to_be_bytes().to_vec())];
            nondefault_native_source_in_mode(step, &["image.adjustments.curves", "filter.other.setAlpha"], false, None, gray);
        }
    }
}

#[test]
fn normal_vectors_prepare_native_rgb_axes_for_gray_and_reopen() {
    for gray in [false, true] {
        nondefault_native_source_in_mode(
            Step::new("gegl:normal-map"),
            &["filter.other.custom", "image.adjustments.colorLookup", "filter.other.setAlpha"],
            false,
            None,
            gray,
        );
    }
}

#[test]
fn rectangular_glass_and_zero_focus_broad_blur_prepare_and_reopen() {
    let mut glass = Step::new("gegl:tile-glass");
    glass.arguments = vec![("tile-width", 1, 5i32.to_be_bytes().to_vec()), ("tile-height", 1, 500i32.to_be_bytes().to_vec())];
    nondefault_native_source_prepares_edits_and_reopens(glass, &["filter.distort.displace"], false, None);
    let mut focus = Step::new("gegl:focus-blur");
    focus.arguments = vec![("radius", 3, 0f32.to_be_bytes().to_vec()), ("blur-radius", 3, 600f32.to_be_bytes().to_vec())];
    nondefault_native_source_prepares_edits_and_reopens(focus, &["filter.blur.gaussianBlur"], false, None);
    let mut focused = Step::new("gegl:focus-blur");
    focused.arguments = vec![("radius", 3, 0.75f32.to_be_bytes().to_vec()), ("blur-radius", 3, 600f32.to_be_bytes().to_vec())];
    nondefault_native_source_prepares_edits_and_reopens(focused, &["filter.blurGallery.irisBlur"], false, None);
}

#[test]
fn expanded_native_cubism_cells_prepare_edit_undo_and_reopen() {
    let mut collage = Step::new("gegl:cubism");
    collage.arguments = vec![("tile-size", 3, 1.5f32.to_be_bytes().to_vec()), ("tile-saturation", 3, 1.25f32.to_be_bytes().to_vec())];
    nondefault_native_source_prepares_edits_and_reopens(collage, &["filter.blur.average"], false, None);
}

#[test]
fn broad_highpass_prepares_native_encoded_blend_edits_and_reopens() {
    let mut highpass = Step::new("gegl:high-pass");
    highpass.arguments = vec![("std-dev", 3, 1500f32.to_be_bytes().to_vec())];
    nondefault_native_source_prepares_edits_and_reopens(highpass, &["image.adjustments.curves", "filter.blur.gaussianBlur"], false, None);
}

#[test]
fn signed_levels_gray_component_threshold_and_color_literal_prepare_and_reopen() {
    for high in [1.0f32, -1.0] {
        let mut levels = Step::new("gegl:levels");
        levels.arguments = vec![
            ("in-low", 3, (-1f32).to_be_bytes().to_vec()),
            ("in-high", 3, high.to_be_bytes().to_vec()),
            ("out-low", 3, (-0.5f32).to_be_bytes().to_vec()),
            ("out-high", 3, 1.5f32.to_be_bytes().to_vec()),
        ];
        nondefault_native_source_prepares_edits_and_reopens(levels, &["filter.other.convolution"], false, None);
    }
    let mut threshold = Step::new("gimp:threshold");
    threshold.arguments =
        vec![("channel", 5, 2i32.to_be_bytes().to_vec()), ("low", 3, 0.25f32.to_be_bytes().to_vec()), ("high", 3, 0.75f32.to_be_bytes().to_vec())];
    nondefault_native_source_in_mode(threshold, &["image.adjustments.channelMixer", "image.adjustments.curves"], false, None, true);
    let mut literal = Wire(vec![]);
    literal.text("R'G'B'A u8");
    literal.word(4);
    literal.0.extend([0, 255, 0, 64]);
    literal.word(0);
    let mut colorize = Step::new("gimp:colorize");
    colorize.arguments = vec![("color", 8, literal.0)];
    nondefault_native_source_prepares_edits_and_reopens(colorize, &["image.adjustments.hueSaturation"], false, None);
}

// These independent wire/native tests verify local graph lowering. They do not
// claim a saved GIMP graph witness or public corpus acceptance.
fn graph_step(text: &str) -> Step<'static> {
    let mut string = Wire(Vec::new());
    string.text(text);
    let mut step = Step::new("gegl:gegl");
    step.arguments.push(("string", 4, string.0));
    step
}
fn graph_source(steps: &[Step<'_>], gray: bool, version: u32) -> Vec<u8> {
    source_stack_with_options(steps, 3, 2, &graph_source_pixels(gray), (gray, version, true))
}
fn graph_add_sibling(document: &mut Document) {
    let mut sibling = Layer::raster("Independent unaffected sibling", document.pixel_format());
    let samples = if document.mode == ColorMode::Grayscale { vec![77. / 255., 1.] } else { vec![17. / 255., 83. / 255., 201. / 255., 1.] };
    sibling.surface_mut().unwrap().write_pixel(6, 1, &samples);
    document.layers.insert(0, sibling);
}
fn graph_native_input(gray: bool) -> Document {
    let mode = if gray { ColorMode::Grayscale } else { ColorMode::Rgb };
    let mut doc = Document::new("Original native graph pixels", Size::new(3, 2), mode, SampleType::U8);
    doc.icc_profile = Some(if gray { photocraft_cms::Builtin::SGray } else { photocraft_cms::Builtin::Srgb }.profile().to_bytes());
    let mut layer = Layer::raster("Independent native input", doc.pixel_format());
    let pixels = graph_source_pixels(gray).into_iter().map(|v| f32::from(v) / 255.).collect::<Vec<_>>();
    layer.surface_mut().unwrap().write_region(Rect::new(0, 0, 3, 2), &pixels);
    doc.layers.push(layer);
    doc
}
fn graph_source_pixels(gray: bool) -> Vec<u8> {
    if gray { PIXELS.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[3]]).collect() } else { PIXELS.to_vec() }
}
fn graph_native_layer(input: &Document, name: &str) -> Layer {
    Layer::new(
        name,
        LayerContent::Smart(photocraft_doc::SmartObject::new(
            photocraft_doc::SmartSource::Embedded {
                file_name: "ordinary-native.pcraft".into(),
                bytes: std::sync::Arc::new(photocraft_format::save_to_bytes(input, &Default::default()).unwrap()),
            },
            photocraft_geom::Affine::IDENTITY,
            None,
        )),
    )
}
fn graph_native_template(input: &Document) -> Document {
    let mut doc = Document::new("Independent native graph stage", input.size, input.mode, input.depth);
    doc.icc_profile = input.icc_profile.clone();
    doc
}
fn graph_native_prepare(mut document: Document) -> Document {
    Session::new().prepare_imported_document(&mut document, &photocraft_engine::jobs::JobCtx::new()).unwrap();
    document
}
fn graph_native_apply(input: &Document, commands: &[(&str, serde_json::Value)]) -> Document {
    let mut document = graph_native_template(input);
    document.layers.push(graph_native_layer(input, "Independent command input"));
    let mut session = Session::new();
    session.add_document(graph_native_prepare(document), None);
    for (command, params) in commands {
        session.execute(command, params.clone()).unwrap();
    }
    session.active().unwrap().doc.as_ref().clone()
}
fn graph_native_over(below: &Document, above: &Document) -> Document {
    graph_native_over_opacity(below, above, 1.)
}
fn graph_native_over_opacity(below: &Document, above: &Document, opacity: f32) -> Document {
    graph_native_blend(below, above, photocraft_color::BlendMode::Normal, opacity)
}
fn graph_native_blend(below: &Document, above: &Document, blend: photocraft_color::BlendMode, opacity: f32) -> Document {
    let mut document = graph_native_template(below);
    document.layers.push(graph_native_layer(below, "Original lower branch"));
    let mut upper = graph_native_layer(above, "Independent upper branch");
    upper.opacity = opacity;
    upper.blend = blend;
    document.layers.push(upper);
    graph_native_prepare(document)
}
fn graph_native_erase(below: &Document, above: &Document) -> Document {
    let mut document = graph_native_template(below);
    document.layers.push(graph_native_layer(below, "Independent knockout backdrop"));
    let mut upper = graph_native_layer(above, "Independent native shallow knockout");
    upper.opacity = 0.4;
    upper.fill_opacity = 0.;
    upper.advanced.knockout = photocraft_doc::Knockout::Shallow;
    document.layers.push(upper);
    graph_native_prepare(document)
}

/// Author a static native mask through ordinary Apply Image, not the import bridge.
fn graph_native_aux_mask(input: &Document, auxiliary: &Document) -> Document {
    let mut document = graph_native_template(input);
    // Apply Image's ordinary target is a pixel layer. Author it from the already
    // prepared native counterpart, without calling the import snapshot helper.
    let mut layer = Layer::raster("Independent current input with native pixel mask", input.pixel_format());
    let buffer = photocraft_compose::flatten(input);
    let pixels: Vec<_> = buffer.px.iter().flat_map(|pixel| photocraft_raster::from_rgba(&input.pixel_format(), *pixel)).collect();
    layer.surface_mut().unwrap().write_region(input.bounds(), &pixels);
    layer.mask = Some(LayerMask { surface: Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false)), ..LayerMask::hide_all() });
    document.layers.push(layer);
    let source = auxiliary.layers.last().unwrap().id;
    let mut session = Session::new();
    session.add_document(graph_native_prepare(document), None);
    session.add_document(auxiliary.clone(), None);
    session.set_active(0);
    session
        .execute("image.applyImage", json!({"source":{"document":1,"layer":source.0,"channel":"composite"},"target":"mask","blending":"normal","opacity":100}))
        .unwrap();
    session.active().unwrap().doc.as_ref().clone()
}

fn graph_native_aux_channel(input: &Document) -> Document {
    let masked = graph_native_aux_mask(input, input);
    let mask = masked.layers[0].mask.as_ref().unwrap();
    let mut document = graph_native_template(input);
    let mut layer = Layer::raster("Independent opaque native composite channel", input.pixel_format());
    let values = mask.surface.read_region(input.bounds());
    let pixels: Vec<_> = values.iter().flat_map(|&v| [v, v, v, 1.]).collect();
    layer.surface_mut().unwrap().write_region(input.bounds(), &pixels);
    document.layers.push(layer);
    document
}

/// Choose ordinary native scale1 for known 0..1 native offsets, then embed that resource.
fn graph_native_aux_displace_params(auxiliary: &Surface, bounds: Rect) -> serde_json::Value {
    let colors = auxiliary.format().mode.color_channels();
    let raw = auxiliary.read_region(bounds);
    let map: Vec<_> = raw
        .chunks_exact(auxiliary.channels())
        .flat_map(|pixel| {
            let x = pixel[0];
            let y = pixel[usize::from(colors >= 2)];
            assert!((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y));
            [(0.5 + f64::from(x) / 2.56) as f32, (0.5 + f64::from(y) / 2.56) as f32, 0.5]
        })
        .collect();
    photocraft_io::smart_map::displace_embed_map(
        &json!({"horizontal":1,"vertical":1,"fit":"stretch","undefinedAreas":"wrap"}),
        &photocraft_algo::Image { rect: bounds, ch: 3, data: map },
    )
    .unwrap()
}

fn graph_native_aux_displace(input: &Document, auxiliary: &Document) -> Document {
    let mut captured = Surface::new(auxiliary.pixel_format());
    let buffer = photocraft_compose::flatten(auxiliary);
    let pixels: Vec<_> = buffer.px.iter().flat_map(|pixel| photocraft_raster::from_rgba(&auxiliary.pixel_format(), *pixel)).collect();
    captured.write_region(auxiliary.bounds(), &pixels);
    let params = graph_native_aux_displace_params(&captured, auxiliary.bounds());
    graph_native_apply(input, &[("filter.distort.displace", params)])
}
fn graph_native_grain_aux(input: &Document) -> Document {
    let mut working = graph_native_template(input);
    working.mode = ColorMode::Rgb;
    working.icc_profile = Some(photocraft_cms::Builtin::Srgb.profile().to_bytes());
    working.layers.push(graph_native_layer(input, "Independent RGB working source"));
    let working = graph_native_prepare(working);
    let result = graph_native_apply(
        &working,
        &[(
            "image.adjustments.channelMixer",
            json!({
                "red":[50.,0.,0.,25.],"green":[0.,50.,0.,25.],"blue":[0.,0.,50.,25.],"monochrome":false
            }),
        )],
    );
    if input.mode == ColorMode::Grayscale {
        let mut gray = graph_native_template(input);
        gray.layers.push(graph_native_layer(&result, "Independent native RGB to Gray conversion"));
        graph_native_prepare(gray)
    } else {
        result
    }
}
fn graph_native_exposure(input: &Document, exposure: f64) -> Document {
    graph_native_apply(input, &[("image.adjustments.exposure", json!({"exposure":exposure,"offset":0.,"gamma":1.}))])
}
fn graph_native_invert(input: &Document) -> Document {
    graph_native_apply(input, &[("image.adjustments.invert", json!({}))])
}
fn graph_native_clip(input: &Document) -> Document {
    let mut document = graph_native_template(input);
    document.layers.push(graph_native_layer(input, "Original clip input"));
    for (value, blend) in [(0.25, photocraft_color::BlendMode::Lighten), (0.75, photocraft_color::BlendMode::Darken)] {
        let color = if input.mode == ColorMode::Grayscale { photocraft_color::Color::gray(value) } else { photocraft_color::Color::rgb(value, value, value) };
        let mut fill = Layer::new("Independent native clip bound", LayerContent::Fill(photocraft_doc::Fill::Solid(color)));
        fill.blend = blend;
        fill.clipped = true;
        document.layers.push(fill);
    }
    graph_native_prepare(document)
}
fn graph_native_gradient(input: &Document) -> Document {
    let mut document = graph_native_template(input);
    let style = photocraft_doc::GradientStyle::Linear;
    let (angle, scale, offset) = photocraft_compose::gradient_fill::from_handles(style, [0., 0.], [4., 0.], input.bounds(), 0.);
    let mut fill = photocraft_doc::Fill::gradient(vec![(0., photocraft_color::Color::BLACK), (1., photocraft_color::Color::WHITE)], angle, scale, style, false)
        .in_mode(input.mode);
    if let photocraft_doc::Fill::Gradient { offset: position, align, .. } = &mut fill {
        *position = offset;
        *align = false;
    }
    document.layers.push(Layer::new("Independent native gradient", LayerContent::Fill(fill)));
    graph_native_prepare(document)
}
fn graph_native_red(input: &Document) -> photocraft_color::Color {
    let profile = photocraft_cms::Profile::parse(input.icc_profile.as_ref().unwrap()).unwrap();
    let transform =
        photocraft_cms::Transform::new(photocraft_cms::Builtin::Srgb.profile(), &profile, photocraft_cms::Intent::RelativeColorimetric, false).unwrap();
    let mut c = [0.; 4];
    transform.convert_f32(&[1., 0., 0.], 3, &mut c, if input.mode == ColorMode::Grayscale { 1 } else { 3 }, false);
    photocraft_color::Color { mode: input.mode, c, alpha: 1. }
}
fn graph_native_text(input: &Document) -> Document {
    use photocraft_doc::text::{ParagraphRun, ParagraphStyle, TextShape};
    let mut text = photocraft_doc::TextLayer {
        text: "A".into(),
        font_family: "Sans".into(),
        size_pt: 4.,
        color: graph_native_red(input),
        shape: TextShape::Box { x: 0., y: 0., width: 1_000_000., height: 1_000_000. },
        paragraphs: vec![ParagraphRun { len: 1, style: ParagraphStyle::default() }],
        ..Default::default()
    };
    let layout = photocraft_text::shared().lock().unwrap_or_else(std::sync::PoisonError::into_inner).layout(&text, 72.);
    if let Some([x, y, ..]) = layout.bounds() {
        text.transform = photocraft_geom::Affine::translate(-f64::from(x), -f64::from(y));
    }
    let mut document = graph_native_template(input);
    document.layers.push(Layer::new("Independent editable text", LayerContent::Text(text)));
    graph_native_prepare(document)
}
fn graph_native_path(input: &Document) -> Document {
    use photocraft_doc::{Knot, Path, ShapeLayer, Subpath};
    use photocraft_geom::Point;
    let mut second = Knot::corner(2., 0.);
    second.out_ctrl = Point::new(3., 0.);
    let mut third = Knot::corner(2., 2.);
    third.in_ctrl = Point::new(3., 2.);
    let shape = ShapeLayer {
        path: Path::new(vec![Subpath { knots: vec![Knot::corner(0., 0.), second, third, Knot::corner(0., 2.)], closed: true, ..Default::default() }]),
        fill: Some(photocraft_doc::Fill::Solid(graph_native_red(input))),
        ..Default::default()
    };
    let mut document = graph_native_template(input);
    document.layers.push(Layer::new("Independent editable cubic shape", LayerContent::Shape(shape)));
    graph_native_prepare(document)
}
fn graph_prepared_object(document: &Document, text: bool, depth: usize) -> bool {
    assert!(depth < 64);
    document.layers.iter().any(|layer| match &layer.content {
        LayerContent::Text(t) if text => t.text == "A" && t.cache.is_some(),
        LayerContent::Shape(s) if !text => s.path.subpaths.len() == 1 && s.path.subpaths[0].knots.len() == 4 && s.cache.is_some(),
        LayerContent::Group(group) => {
            let mut children = graph_native_template(document);
            children.layers = group.children.clone();
            graph_prepared_object(&children, text, depth + 1)
        }
        LayerContent::Smart(s) => {
            if let photocraft_doc::SmartSource::Embedded { bytes, .. } = &s.source {
                graph_prepared_object(&photocraft_format::load_from_bytes(bytes).unwrap(), text, depth + 1)
            } else {
                false
            }
        }
        _ => false,
    })
}
fn graph_native_scene(input: &Document) -> Document {
    let mut scene = Document::new("Independent native scene", Size::new(8, 6), input.mode, input.depth);
    scene.icc_profile = input.icc_profile.clone();
    let mut layer = graph_native_layer(input, "Independent placed result");
    let LayerContent::Smart(s) = &mut layer.content else { panic!("ordinary native Smart") };
    s.transform = photocraft_geom::Affine::translate(-1., 2.);
    scene.layers.push(layer);
    graph_add_sibling(&mut scene);
    graph_native_prepare(scene)
}
fn graph_pixels_match(actual: &Document, expected: &Document, label: &str) {
    let actual = photocraft_compose::flatten(actual);
    let expected = photocraft_compose::flatten(expected);
    assert_eq!(actual.rect, expected.rect);
    let worst = actual.px.iter().zip(&expected.px).flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs())).fold(0f32, f32::max);
    assert!(worst <= 2. / 255., "{label}: maximum ordinary native difference {worst}");
}
fn graph_lifecycle(mut session: Session, expected: &Document, label: &str) {
    let actual = photocraft_compose::flatten(&session.active().unwrap().doc).px;
    graph_pixels_match(&session.active().unwrap().doc, expected, label);
    assert_current_native_commands(&session.active().unwrap().doc, label, 0);
    let report = session.active().unwrap().doc.metadata.import_conversion.clone();
    session.execute("layer.setProps", json!({"opacity":0.25})).unwrap();
    assert_ne!(photocraft_compose::flatten(&session.active().unwrap().doc).px, actual, "ordinary edit must change visible native pixels");
    session.execute("edit.undo", json!({})).unwrap();
    assert!(photocraft_compose::flatten(&session.active().unwrap().doc).px == actual);
    let saved = photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap();
    drop(session);
    let mut reopened = Session::new();
    reopened.color.settings.ask_on_missing = false;
    open_bytes_as(&mut reopened, "self-contained-graph.pcraft", &saved, None, None).unwrap();
    assert!(!reopened.active().unwrap().source_read_only);
    assert!(photocraft_compose::flatten(&reopened.active().unwrap().doc).px == actual, "native graph reopen changed prepared pixels");
    assert_eq!(reopened.active().unwrap().doc.metadata.import_conversion, report);
    assert_current_native_commands(&reopened.active().unwrap().doc, label, 0);
    reopened.execute("layer.setProps", json!({"opacity":0.5})).unwrap();
    reopened.execute("edit.undo", json!({})).unwrap();
    assert!(photocraft_compose::flatten(&reopened.active().unwrap().doc).px == actual);
}
#[test]
fn finite_graph_chain_forks_and_structural_postfilters_prepare_against_independent_native_documents() {
    let cases = [
        "invert-linear",
        "exposure exposure=-1 invert-linear exposure exposure=1",
        "id=in exposure exposure=-1 over aux=[ref=in invert-linear] exposure exposure=1",
        "id=in exposure exposure=-1 id=dark over aux=[ref=in invert-linear over aux=[ref=dark]] exposure exposure=1",
        "id=in over aux=[ref=in invert-linear] over aux=[ref=in exposure exposure=-1]",
        "rgb-clip clip-low=true low-limit=.25 clip-high=true high-limit=.75 invert-linear",
        "id=in gimp:normal opacity=0.4 aux=[ref=in invert-linear]",
        "id=in gimp:behind opacity=0.4 aux=[ref=in invert-linear]",
        "id=in gimp:normal opacity=0.4 aux=[linear-gradient start-x=0 start-y=0 end-x=4 end-y=0 start-color=black end-color=white]",
        "id=in over aux=[text string='A' font='Sans' size=4 wrap=-1 vertical-wrap=-1 alignment=0 vertical-alignment=0 color=red]",
        "fill-path d='M 0 0 L 2 0 C 3 0 3 2 2 2 L 0 2 z' color=red opacity=1 fill-rule=nonzero transform=''",
        "id=in multiply value=.25 aux=[ref=in invert-linear]",
        "id=in gimp:grain-merge-legacy aux=[ref=in invert-linear]",
        "id=in gimp:erase opacity=0.4 aux=[ref=in invert-linear]",
    ];
    for gray in [false, true] {
        for version in [20, 26] {
            for (index, text) in cases.iter().enumerate() {
                let original = graph_native_input(gray);
                // A graph ref binds to its actual input AFTER the previous source effect.
                let input = graph_native_exposure(&original, -0.5);
                let inverted = graph_native_invert(&input);
                let dark = graph_native_exposure(&input, -1.);
                let expected = match index {
                    0 => inverted,
                    1 => graph_native_apply(
                        &input,
                        &[
                            ("image.adjustments.exposure", json!({"exposure":-1.,"offset":0.,"gamma":1.})),
                            ("image.adjustments.invert", json!({})),
                            ("image.adjustments.exposure", json!({"exposure":1.,"offset":0.,"gamma":1.})),
                        ],
                    ),
                    2 => graph_native_exposure(&graph_native_over(&dark, &inverted), 1.),
                    3 => graph_native_exposure(&graph_native_over(&dark, &graph_native_over(&inverted, &dark)), 1.),
                    4 => graph_native_over(&graph_native_over(&input, &inverted), &dark),
                    5 => graph_native_invert(&graph_native_clip(&input)),
                    6 => graph_native_over_opacity(&input, &inverted, 0.4),
                    7 => {
                        let scaled_aux = graph_native_apply(
                            &inverted,
                            &[("filter.other.alphaCurve", json!({"values":(0..256).map(|i|i as f64/255.*0.4).collect::<Vec<_>>()}))],
                        );
                        graph_native_over(&scaled_aux, &input)
                    }
                    8 => graph_native_over_opacity(&input, &graph_native_gradient(&input), 0.4),
                    9 => graph_native_over(&input, &graph_native_text(&input)),
                    10 => graph_native_over(&input, &graph_native_path(&input)),
                    11 => graph_native_blend(&input, &inverted, photocraft_color::BlendMode::Multiply, 1.),
                    12 => graph_native_blend(&input, &graph_native_grain_aux(&inverted), photocraft_color::BlendMode::LinearLight, 1.),
                    13 => graph_native_erase(&input, &inverted),
                    _ => panic!("finite test case"),
                };
                if index == 13 {
                    let before = photocraft_compose::flatten(&input);
                    let erased = photocraft_compose::flatten(&expected);
                    assert!(
                        erased.px.iter().zip(&before.px).any(|(after, before)| after[3] < before[3]),
                        "independent ordinary shallow knockout must remove actual source coverage"
                    );
                }
                if matches!(index, 9 | 10) {
                    assert!(
                        photocraft_compose::flatten(&expected).px != photocraft_compose::flatten(&input).px,
                        "independent prepared native generator must visibly affect the actual input"
                    );
                }
                let expected = graph_native_scene(&graph_native_exposure(&expected, 0.25));
                let mut before = Step::new("gegl:exposure");
                before.arguments.push(("exposure", 3, (-0.5f32).to_be_bytes().to_vec()));
                let mut after = Step::new("gegl:exposure");
                after.arguments.push(("exposure", 3, 0.25f32.to_be_bytes().to_vec()));
                let bytes = graph_source(&[before, graph_step(text), after], gray, version);
                let mut session = Session::new();
                session.color.settings.ask_on_missing = false;
                open_bytes_as(&mut session, "original-graph.xcf", &bytes, None, None).unwrap();
                drop(bytes);
                assert!(session.active().unwrap().source_read_only);
                let Some(photocraft_doc::import_conversion::ImportConversion::Report(report)) = &session.active().unwrap().doc.metadata.import_conversion
                else {
                    panic!("typed graph conversion report")
                };
                let graph = report
                    .entries
                    .iter()
                    .find(|entry| entry.feature_code == "xcf.layer_effect" && entry.source_description.contains("(gegl:gegl)"))
                    .expect("actual graph source outcome");
                assert_eq!(
                    graph.disposition,
                    photocraft_doc::import_conversion::ConversionDisposition::NativeApproximation,
                    "v{version} gray{gray} graph{index}: {}",
                    graph.native_description
                );
                let LayerContent::Smart(s) = &session.active().unwrap().doc.layers.last().unwrap().content else { panic!("prepared ordinary graph contents") };
                assert!(s.cache.is_some());
                if matches!(index, 9 | 10) {
                    assert!(
                        graph_prepared_object(&session.active().unwrap().doc, index == 9, 0),
                        "v{version} gray{gray} graph{index}: generated object must remain editable and receive its first native cache"
                    );
                }
                graph_lifecycle(session, &expected, &format!("v{version} gray{gray} graph{index}"));
            }
        }
    }
}

#[test]
fn actual_auxiliary_graph_mask_piecewise_and_displace_use_current_native_inputs_and_survive_source_history_removal() {
    let cases = [
        "id=in opacity value=0.5 aux=[ref=in invert-linear]",
        "id=in piecewise-blend levels=2 aux1=[ref=in] aux2=[ref=in invert-linear]",
        "id=in map-relative scaling=1 abyss-policy=loop aux=[ref=in invert-linear]",
    ];
    for (index, text) in cases.iter().enumerate() {
        let original = graph_native_input(false);
        let input = graph_native_exposure(&original, -0.5);
        let auxiliary = graph_native_invert(&input);
        let expected = match index {
            0 => graph_native_apply(
                &graph_native_aux_mask(&input, &auxiliary),
                &[(
                    "filter.other.alphaCurve",
                    json!({
                        "values":(0..256).map(|sample| sample as f64 / 255. * 0.5).collect::<Vec<_>>()
                    }),
                )],
            ),
            1 => {
                let channel = graph_native_aux_channel(&input);
                let membership = graph_native_apply(&channel, &[("image.adjustments.curves", json!({"pencil":true,"points":(0..256).collect::<Vec<_>>()}))]);
                graph_native_over(&input, &graph_native_aux_mask(&auxiliary, &membership))
            }
            2 => graph_native_aux_displace(&input, &auxiliary),
            _ => panic!("finite native auxiliary case"),
        };
        assert_ne!(photocraft_compose::flatten(&expected).px, photocraft_compose::flatten(&input).px, "actual auxiliary must visibly affect native output");
        let expected = graph_native_scene(&graph_native_exposure(&expected, 0.25));
        let mut before = Step::new("gegl:exposure");
        before.arguments.push(("exposure", 3, (-0.5_f32).to_be_bytes().to_vec()));
        let mut after = Step::new("gegl:exposure");
        after.arguments.push(("exposure", 3, 0.25_f32.to_be_bytes().to_vec()));
        let source = graph_source(&[before, graph_step(text), after], false, 26);
        let mut session = Session::new();
        session.color.settings.ask_on_missing = false;
        open_bytes_as(&mut session, "actual-native-auxiliary.xcf", &source, None, None).unwrap();
        drop(source);
        let Some(photocraft_doc::import_conversion::ImportConversion::Report(report)) = &session.active().unwrap().doc.metadata.import_conversion else {
            panic!("typed auxiliary graph report")
        };
        let outcome = report.entries.iter().find(|entry| entry.feature_code == "xcf.layer_effect" && entry.source_description.contains("(gegl:gegl)")).unwrap();
        assert_eq!(
            outcome.disposition,
            photocraft_doc::import_conversion::ConversionDisposition::NativeApproximation,
            "case{index}: {}",
            outcome.native_description
        );
        // The native lifecycle must not need even the inert original graph configuration.
        std::sync::Arc::make_mut(&mut session.active_mut().unwrap().doc).metadata.import_conversion = None;
        graph_lifecycle(session, &expected, &format!("actual auxiliary native case{index}"));
    }
}

fn graph_saved_channel_source(text: &str, pixels: &[u8]) -> Vec<u8> {
    let mut w = Wire(b"gimp xcf v026\0".to_vec());
    for value in [3, 2, 0, 150] {
        w.word(value);
    }
    w.property(17, &[0]);
    w.end();
    let layer = w.slot();
    w.pointer(0);
    let channel = w.slot();
    w.pointer(0);
    w.pointer(0);
    w.patch(layer);
    for value in [3, 2, 1] {
        w.word(value);
    }
    w.text("Unaffected native pixels");
    w.end();
    let hierarchy = w.slot();
    w.pointer(0);
    w.pointer(0);
    w.patch(hierarchy);
    w.hierarchy(3, 2, 4, &PIXELS);
    w.patch(channel);
    w.word(3);
    w.word(2);
    w.text("Actual scalar channel");
    w.end();
    let hierarchy = w.slot();
    let effect = w.slot();
    w.pointer(0);
    w.patch(hierarchy);
    w.hierarchy(3, 2, 1, pixels);
    w.patch(effect);
    w.text("Original scalar graph");
    w.text("");
    w.text("gegl:gegl");
    w.text("0:0");
    let mut argument = Wire(Vec::new());
    argument.text("string");
    argument.word(4);
    argument.text(text);
    w.property(45, &argument.0);
    w.end();
    w.pointer(0);
    w.0
}

#[test]
fn actual_auxiliary_graph_displace_on_saved_scalar_channel_is_native_editable_and_source_independent() {
    let bounds = Rect::new(0, 0, 3, 2);
    let original = [0_u8, 64, 128, 192, 255, 64];
    let mut scalar = Surface::new(PixelFormat::GRAY8);
    scalar.write_region(bounds, &original.map(|value| f32::from(value) / 255.));
    let mut inverted = Surface::new(PixelFormat::GRAY8);
    inverted.write_region(bounds, &original.map(|value| f32::from(255 - value) / 255.));
    let params = graph_native_aux_displace_params(&inverted, bounds);
    let expected = photocraft_engine::filters::apply_filter_to_surface("filter.distort.displace", &params, &scalar, bounds, None, bounds).unwrap().unwrap();
    assert_ne!(expected, scalar, "actual auxiliary map must move nonuniform scalar native data");
    let source = graph_saved_channel_source("id=in map-relative scaling=1 abyss-policy=loop aux=[ref=in invert-linear]", &original);
    let mut session = Session::new();
    session.color.settings.ask_on_missing = false;
    open_bytes_as(&mut session, "actual-scalar-map.xcf", &source, None, None).unwrap();
    drop(source);
    let doc = &session.active().unwrap().doc;
    let actual = doc.channels[0].surface.read_region(bounds);
    for (got, want) in actual.iter().zip(expected.read_region(bounds)) {
        assert!((*got - want).abs() <= 1. / 255.);
    }
    assert_eq!(doc.layers[0].surface().unwrap().read_region(bounds), PIXELS.map(|value| f32::from(value) / 255.));
    let Some(photocraft_doc::import_conversion::ImportConversion::Report(report)) = &doc.metadata.import_conversion else {
        panic!("typed scalar graph outcome")
    };
    let outcome = report.entries.iter().find(|entry| entry.feature_code == "xcf.coverage_effect").unwrap();
    assert_eq!(outcome.disposition, photocraft_doc::import_conversion::ConversionDisposition::NativeApproximation, "{}", outcome.native_description);
    std::sync::Arc::make_mut(&mut session.active_mut().unwrap().doc).metadata.import_conversion = None;
    session.execute("image.adjustments.invert", json!({"target":{"channel":0}})).unwrap();
    assert_ne!(session.active().unwrap().doc.channels[0].surface.read_region(bounds), actual);
    session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(session.active().unwrap().doc.channels[0].surface.read_region(bounds), actual);
    let bytes = photocraft_format::save_to_bytes(&session.active().unwrap().doc, &Default::default()).unwrap();
    drop(session);
    let mut reopened = Session::new();
    open_bytes_as(&mut reopened, "ordinary-scalar.pcraft", &bytes, None, None).unwrap();
    assert!(!reopened.active().unwrap().source_read_only);
    assert_eq!(reopened.active().unwrap().doc.channels[0].surface.read_region(bounds), actual);
    assert!(reopened.active().unwrap().doc.metadata.import_conversion.is_none());
    reopened.execute("image.adjustments.invert", json!({"target":{"channel":0}})).unwrap();
    reopened.execute("edit.undo", json!({})).unwrap();
    assert_eq!(reopened.active().unwrap().doc.channels[0].surface.read_region(bounds), actual);
}
#[test]
fn finite_graph_whole_linear_envelope_and_global_mask_apply_once_before_native_save() {
    for version in [20, 26] {
        for (opacity, visible, masked) in [(0.375, true, false), (0.375, true, true), (0.375, false, true)] {
            let mut step = graph_step("invert-linear");
            step.opacity = opacity;
            step.visible = visible;
            step.masked = masked;
            let bytes = graph_source(&[step], false, version);
            let mut session = Session::new();
            session.color.settings.ask_on_missing = false;
            open_bytes_as(&mut session, "graph-envelope.xcf", &bytes, None, None).unwrap();
            drop(bytes);
            let mut expected = native_counterpart("gegl:invert-gamma", opacity, visible, masked);
            graph_add_sibling(&mut expected);
            graph_lifecycle(session, &expected, "whole graph envelope");
        }
    }
}
#[test]
fn invalid_graph_fields_preserve_native_neighbors_and_report_unapplied_without_partial_execution() {
    use photocraft_doc::import_conversion::{ConversionDisposition, ImportConversion};
    for text in [
        "invert-linear future-control=1",
        "id=in over aux=[ref=missing invert-linear]",
        "id=in over aux=[invert-linear]",
        "invert-linear unknown-operation",
        "gegl:gegl string=invert-linear",
    ] {
        let mut before = Step::new("gegl:exposure");
        before.arguments.push(("exposure", 3, (-0.5f32).to_be_bytes().to_vec()));
        let after = Step::new("gegl:invert-gamma");
        let bytes = graph_source(&[before, graph_step(text), after], false, 26);
        let mut session = Session::new();
        session.color.settings.ask_on_missing = false;
        open_bytes_as(&mut session, "retained-graph.xcf", &bytes, None, None).unwrap();
        drop(bytes);
        let expected = graph_native_scene(&graph_native_invert(&graph_native_exposure(&graph_native_input(false), -0.5)));
        let ImportConversion::Report(report) = session.active().unwrap().doc.metadata.import_conversion.as_ref().unwrap() else { panic!("typed graph report") };
        assert!(
            report.entries.iter().any(|entry| entry.source_description.contains("gegl:gegl") && entry.disposition == ConversionDisposition::RetainedUnapplied)
        );
        graph_lifecycle(session, &expected, "invalid graph with native neighbors");
    }
}

#[test]
fn finite_fork_graph_whole_envelope_uses_native_underlay_mask_and_opacity_once() {
    for version in [20, 26] {
        for (opacity, visible, masked) in [(0.375, true, false), (0.375, true, true), (0.375, false, true)] {
            let input = graph_native_input(false);
            let computed = graph_native_over(&input, &graph_native_invert(&input));
            let mut expected = graph_native_template(&input);
            expected.layers.push(graph_native_layer(&input, "Independent unchanged graph input"));
            let mut output = graph_native_layer(&computed, "Independent complete graph output");
            output.opacity = opacity;
            output.visible = visible;
            if masked {
                let mut mask = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
                mask.write_pixel(1, 0, &[1.]);
                mask.write_pixel(1, 1, &[1.]);
                output.mask = Some(LayerMask { surface: mask, ..LayerMask::reveal_all() });
            }
            expected.layers.push(output);
            let expected = graph_native_scene(&graph_native_prepare(expected));
            let mut step = graph_step("id=in over aux=[ref=in invert-linear]");
            step.opacity = opacity;
            step.visible = visible;
            step.masked = masked;
            let bytes = graph_source(&[step], false, version);
            let mut session = Session::new();
            session.color.settings.ask_on_missing = false;
            open_bytes_as(&mut session, "whole-fork-envelope.xcf", &bytes, None, None).unwrap();
            drop(bytes);
            graph_lifecycle(session, &expected, "whole fork graph native envelope");
        }
    }
}
