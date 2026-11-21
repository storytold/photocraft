//! #14191: the fill continues straight bands and stripes through the hole.
//!
//! The measured document (both the reference application and PhotoCraft main f143ce5849):
//! 600 x 400, RGB, 8 bit, one Background layer. Top half blue 60,120,200 with a light 120,170,230
//! band 10 px high every 40 px (at y 0, 40, 80, 120, 160); bottom half vertical stripes 20 px wide,
//! 40,140,60 at x = 0 mod 40 and 80,180,90 at x = 20 mod 40; a red 220,30,30 square from 260,160 to
//! 340,240; selection 250,150 to 350,250 (the square plus 10 px). The sampling window is 100,0 to
//! 500,400, which is what [`sampling_window`] gives for that selection, and Auto covers the same
//! area as Rectangular on this document.
//!
//! The reference application keeps the bands and the stripes in their original phase through the
//! hole and keeps the blue / green boundary at y 200. PhotoCraft main runs the blue half down into
//! the green one: 300,165 -> 64,124,201 instead of the light band 120,170,230, and 300,205 ->
//! 65,127,205, 285,220 -> 64,127,205, 300,240 -> 62,124,202 instead of the green stripes.

use super::*;

const DOC_W: usize = 600;
const DOC_H: usize = 400;
/// The measured selection: the red square plus 10 px.
const SEL: (i32, i32, i32, i32) = (250, 150, 350, 250);
/// The red square inside it.
const SQUARE: (usize, usize, usize, usize) = (260, 160, 340, 240);

/// The document's colours without the square, in levels of 255: what the reference application
/// shows through the hole again.
fn background_levels(x: usize, y: usize) -> [i32; 3] {
    if y < 200 {
        if y % 40 < 10 { [120, 170, 230] } else { [60, 120, 200] }
    } else if x % 40 < 20 {
        [40, 140, 60]
    } else {
        [80, 180, 90]
    }
}

/// The measured document as interleaved normalised floats (`ch = 3`).
fn document() -> Vec<f32> {
    let at = |x: usize, y: usize| -> [f32; 3] {
        let v = if (SQUARE.0..SQUARE.2).contains(&x) && (SQUARE.1..SQUARE.3).contains(&y) { [220, 30, 30] } else { background_levels(x, y) };
        [v[0] as f32 / 255.0, v[1] as f32 / 255.0, v[2] as f32 / 255.0]
    };
    (0..DOC_H).flat_map(|y| (0..DOC_W).flat_map(move |x| at(x, y))).collect()
}

/// The sampling window of the measured selection on the measured canvas.
fn window() -> Rect {
    sampling_window(Rect::new(SEL.0, SEL.1, SEL.2, SEL.3), Rect::new(0, 0, DOC_W as i32, DOC_H as i32))
}

/// The measured window's buffer and its hole: `edit.contentAwareFill` fills the sampling window,
/// with the selection as the hole and (for Auto) the area `auto_sampling` keeps within it.
fn window_buffers() -> (Vec<f32>, Vec<bool>, usize) {
    let doc = document();
    let win = window();
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    let mut img = vec![0.0f32; ww * wh * 3];
    let mut hole = vec![false; ww * wh];
    for y in 0..wh {
        let dy = (win.y0 + y as i32) as usize;
        let s = (dy * DOC_W + win.x0 as usize) * 3;
        img[y * ww * 3..(y + 1) * ww * 3].copy_from_slice(&doc[s..s + ww * 3]);
        for x in 0..ww {
            hole[y * ww + x] = (SEL.0..SEL.2).contains(&(win.x0 + x as i32)) && (SEL.1..SEL.3).contains(&(win.y0 + y as i32));
        }
    }
    (img, hole, ww)
}

/// Fill the measured document the way the command does with `opts` and Auto sampling, and return
/// the filled window.
fn fill_measured_document(opts: &FillOptions) -> Vec<f32> {
    let (img, hole, ww) = window_buffers();
    let source = auto_sampling(ww, 400, 3, &img, &hole);
    fill(ww, 400, 3, &img, &hole, &source, opts)
}

/// The filled colour at document coordinates `(x, y)`, in levels of 255.
fn filled_at(win: &[f32], x: usize, y: usize) -> [i32; 3] {
    let w = window().width() as usize;
    let i = ((y - window().y0 as usize) * w + (x - window().x0 as usize)) * 3;
    [0, 1, 2].map(|c| (win[i + c] * 255.0).round() as i32)
}

/// The nine pixel samples of #14191, measured in the reference application (document coordinates
/// and the reference colour in levels of 255), in the order the issue lists them.
const SAMPLES: [((usize, usize), [i32; 3]); 9] = [
    ((300, 165), [120, 170, 230]),
    ((300, 180), [60, 120, 200]),
    ((300, 195), [60, 120, 200]),
    ((300, 205), [80, 180, 90]),
    ((285, 220), [40, 140, 60]),
    ((265, 220), [77, 177, 88]),
    ((300, 240), [80, 180, 90]),
    ((255, 155), [60, 120, 200]),
    ((345, 245), [80, 180, 90]),
];

/// The measured samples that are more than 12 levels per channel from the reference value.
fn samples_off(out: &[f32]) -> Vec<String> {
    SAMPLES
        .iter()
        .filter_map(|((x, y), want)| {
            let got = filled_at(out, *x, *y);
            let off = (0..3).map(|c| (got[c] - want[c]).abs()).max().unwrap_or(0);
            (off > 12).then(|| format!("({x},{y}) -> {got:?}, reference {want:?}, off by {off}"))
        })
        .collect()
}

/// How far the fill is from the document's own background, over the whole hole: (worst deviation
/// in levels, number of pixels within 12, number of pixels counted). Every pixel of the hole
/// carries the colour the document has at that place, which is what "the bands and stripes run
/// through the hole with their original phase" means.
fn phase_through_the_hole(out: &[f32]) -> (i32, usize, usize) {
    let (mut worst, mut within, mut n) = (0, 0, 0);
    for y in 150..250 {
        for x in 250..350 {
            let want = background_levels(x, y);
            let got = filled_at(out, x, y);
            let off = (0..3).map(|c| (got[c] - want[c]).abs()).max().unwrap_or(0);
            worst = worst.max(off);
            within += usize::from(off <= 12);
            n += 1;
        }
    }
    (worst, within, n)
}

/// The blue / light half and the green half of the document, by the palette colour a filled pixel
/// took: the boundary between them.
const PALETTE: [(&str, [i32; 3]); 5] =
    [("blue", [60, 120, 200]), ("light", [120, 170, 230]), ("green-a", [40, 140, 60]), ("green-b", [80, 180, 90]), ("red", [220, 30, 30])];

fn palette_name(v: [i32; 3]) -> &'static str {
    PALETTE.iter().min_by_key(|(_, c)| c.iter().zip(v).map(|(a, b): (&i32, i32)| (a - b) * (a - b)).sum::<i32>()).map(|(n, _)| *n).unwrap_or("?")
}

fn cold(name: &str) -> bool {
    name == "blue" || name == "light"
}

fn green(name: &str) -> bool {
    name == "green-a" || name == "green-b"
}

/// Over the inside of the hole: how many pixels above y 200 took a colour of the green half, and
/// how many below it took a colour of the blue half. Rows within 3 px of the boundary are left
/// out: there the seams between copied pieces blend.
fn boundary_mismatches(out: &[f32]) -> (usize, usize) {
    let (mut green_above, mut cold_below) = (0, 0);
    for y in 153..248 {
        if (197..204).contains(&y) {
            continue;
        }
        for x in 253..348 {
            let n = palette_name(filled_at(out, x, y));
            if y < 200 && green(n) {
                green_above += 1;
            }
            if y > 200 && cold(n) {
                cold_below += 1;
            }
        }
    }
    (green_above, cold_below)
}

/// The measured fill, computed once: Auto and Rectangular give `fill` the same area on this
/// document ([`measured_auto_area_is_the_rectangular_one`]), so one fill stands for both. The
/// checks are collected first, so a failure reports every one that is off.
#[test]
fn measured_fill_continues_the_bands_and_stripes_and_keeps_the_boundary_at_y_200() {
    assert_eq!(window(), Rect::new(100, 0, 500, 400), "the sampling window #14191 measured");
    let out = fill_measured_document(&FillOptions::default());
    let mut failures = Vec::new();
    let off = samples_off(&out);
    if !off.is_empty() {
        failures.push(format!("{} of 9 samples off\n{}", off.len(), off.join("\n")));
    }
    let (green_above, cold_below) = boundary_mismatches(&out);
    if (green_above, cold_below) != (0, 0) {
        failures.push(format!(
            "the blue / green boundary moved: {green_above} pixels above y 200 took a colour of the green half, {cold_below} below it one of the blue half"
        ));
    }
    let (worst, within, n) = phase_through_the_hole(&out);
    if within != n {
        failures.push(format!("{within} of {n} hole pixels carry the document's own colour within 12 levels (worst {worst})"));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Per-channel slack for the unmeasured pins below. The exact values hold bit for bit on Windows
/// (also at RAYON_NUM_THREADS 4, 2 and 1), but not on every C library: the vote that blends the
/// seams between the copied pieces weights the patches with `(-cost / (2 sigma2)).exp()` of the
/// host C library (`inpaint/solver.rs`), and a filled pixel sitting right on a level boundary
/// rounds one level apart between glibc and the Windows runtime. CI run 43833457124 on
/// ubuntu-24.04 showed (300,180) under colour adaptation High at 64 instead of 65, and (285,220)
/// under rotation Low at 118 instead of 119, while all other 52 channels matched exactly. Nothing
/// on main changed these fills: between f143ce5849 and the branch parent eea585958 no commit
/// touches `content_aware` or `inpaint`. 2 levels is still a hard pin against what the guards
/// guard against: if the new patch radius leaked into one of these option sets, its fill would
/// move the way the measured set moves, and at (300,165) alone that is 60 levels (the None pin
/// 60,120,200 against the 120,170,230 the measured fill gives there).
const UNMEASURED_TOLERANCE: i32 = 2;

/// The option sets #14191 marks *not measured* on this document keep today's fill: these are the
/// nine samples as main f143ce5849 (and as this branch without the change) gives them. Guard
/// tests: they pass before and after the change and pin it, so widening the new patch radius later
/// has to say what these settings now do. Compared within [`UNMEASURED_TOLERANCE`] levels per
/// channel, see there for why.
fn unmeasured_colour_adaptation() -> Vec<(&'static str, FillOptions, [i32; 27])> {
    let at = |level: &str| FillOptions { color_adaptation: color_level(level), rotations: Vec::new(), scale: false, mirror: false, seed: 1 };
    vec![
        ("None", at("none"), [60, 120, 200, 60, 120, 200, 60, 120, 200, 60, 120, 200, 60, 120, 200, 80, 180, 90, 60, 120, 200, 60, 120, 200, 80, 180, 90]),
        ("High", at("high"), [65, 122, 191, 65, 119, 186, 64, 117, 183, 48, 132, 70, 77, 162, 98, 70, 163, 91, 47, 129, 76, 60, 120, 198, 80, 180, 90]),
        (
            "Very High",
            at("veryHigh"),
            [63, 121, 192, 64, 119, 187, 64, 117, 182, 80, 155, 103, 78, 152, 112, 65, 152, 94, 73, 146, 122, 60, 120, 199, 80, 180, 90],
        ),
    ]
}

fn unmeasured_geometric_adaptation() -> Vec<(&'static str, FillOptions, [i32; 27])> {
    let base = || FillOptions { color_adaptation: color_level("default"), rotations: Vec::new(), scale: false, mirror: false, seed: 1 };
    vec![
        (
            "rotation Low",
            FillOptions { rotations: rotation_level("low"), ..base() },
            [61, 118, 192, 62, 119, 192, 62, 119, 198, 61, 119, 203, 60, 119, 206, 77, 172, 95, 60, 119, 208, 60, 120, 198, 80, 180, 90],
        ),
        (
            "rotation Full",
            FillOptions { rotations: rotation_level("full"), ..base() },
            [65, 124, 200, 65, 123, 199, 64, 122, 199, 82, 184, 92, 63, 122, 192, 63, 122, 193, 61, 121, 194, 60, 120, 199, 80, 180, 90],
        ),
        (
            "Scale on",
            FillOptions { scale: true, ..base() },
            [115, 163, 219, 58, 117, 190, 59, 118, 190, 77, 172, 88, 63, 126, 206, 76, 172, 94, 62, 124, 203, 60, 119, 197, 80, 179, 90],
        ),
        (
            "Mirror on",
            FillOptions { mirror: true, ..base() },
            [64, 122, 198, 63, 121, 196, 62, 119, 194, 43, 137, 62, 62, 124, 202, 75, 172, 92, 61, 123, 201, 60, 120, 199, 80, 180, 90],
        ),
    ]
}

fn check_unmeasured(what: &str, opts: &FillOptions, want: &[i32; 27]) {
    let out = fill_measured_document(opts);
    let got: Vec<i32> = SAMPLES.iter().flat_map(|((x, y), _)| filled_at(&out, *x, *y)).collect();
    let off: Vec<String> = SAMPLES
        .iter()
        .enumerate()
        .filter_map(|(k, ((x, y), _))| {
            let (g, w) = (&got[k * 3..k * 3 + 3], &want[k * 3..k * 3 + 3]);
            let dev = (0..3).map(|c| (g[c] - w[c]).abs()).max().unwrap_or(0);
            (dev > UNMEASURED_TOLERANCE).then(|| format!("({x},{y}) -> {g:?}, today's fill {w:?}, off by {dev}"))
        })
        .collect();
    assert!(
        off.is_empty(),
        "{what}: the nine samples moved by more than {UNMEASURED_TOLERANCE} levels, although #14191 marks that case not measured\n{}",
        off.join("\n")
    );
}

#[test]
fn unmeasured_colour_adaptation_keeps_todays_fill() {
    for (what, opts, want) in unmeasured_colour_adaptation() {
        check_unmeasured(what, &opts, &want);
    }
}

#[test]
fn unmeasured_geometric_adaptation_keeps_todays_fill() {
    for (what, opts, want) in unmeasured_geometric_adaptation() {
        check_unmeasured(what, &opts, &want);
    }
}

#[test]
fn measured_auto_area_is_the_rectangular_one() {
    // What the measured fill above rests on: on this document Auto keeps the whole window, as in
    // the reference application, so the Auto fill is the Rectangular one.
    let (img, hole, ww) = window_buffers();
    assert_eq!(auto_sampling(ww, 400, 3, &img, &hole), hole.iter().map(|h| !h).collect::<Vec<bool>>());
}

/// #14191 marks Auto on a document where its content-based choice differs from Rectangular as not
/// measured, so that fill keeps today's 7x7 patch even with the measured options. The document: the
/// measured one with a white column 40 px wide at x 440 to 480 in the window, which Auto leaves
/// out. Today's fill is the completion with [`CompleteParams`]' default radius on the same roles.
#[test]
fn auto_fill_whose_area_differs_from_rectangular_keeps_todays_patch_radius() {
    let (mut img, hole, ww) = window_buffers();
    let column = |i: usize| (440..480).contains(&(window().x0 as usize + i % ww));
    for (i, px) in img.chunks_mut(3).enumerate() {
        if column(i) {
            px.copy_from_slice(&[250.0 / 255.0; 3]);
        }
    }
    let source = auto_sampling(ww, 400, 3, &img, &hole);
    let rectangular: Vec<bool> = hole.iter().map(|h| !h).collect();
    assert_ne!(source, rectangular, "Auto must choose an area other than the Rectangular one here");
    let opts = FillOptions::default();
    let out = fill(ww, 400, 3, &img, &hole, &source, &opts);
    let roles: Vec<Px> = (0..ww * 400)
        .map(|i| {
            if hole[i] {
                Px::Hole
            } else if source[i] {
                Px::Source
            } else {
                Px::Known
            }
        })
        .collect();
    let params = CompleteParams { seed: opts.seed, adapt: opts.color_adaptation, ..Default::default() };
    assert_eq!(params.patch_radius, 3, "today's radius");
    let today = complete_masked(ww, 400, 3, &img, &roles, &params, &Interrupt::NONE).unwrap().unwrap();
    let moved = out.iter().zip(&today).filter(|(a, b)| a != b).count();
    assert_eq!(moved, 0, "{moved} channel values differ from today's radius-3 fill");
}

/// A small buffer that carries the measured option set and the Rectangular area, so the patch
/// radius is the only thing under test: vertical stripes (period 4) in two far-apart colours, with
/// the hole the 1 px border of the buffer. An off-centre 2 x 2 hole leaves no 7 x 7 donor either on
/// a buffer this small (every 7 x 7 window takes a hole pixel), so it proves nothing here.
fn small_stripes(n: usize) -> (Vec<f32>, Vec<bool>) {
    let (a, b) = ([10.0 / 255.0, 200.0 / 255.0, 20.0 / 255.0], [200.0 / 255.0, 30.0 / 255.0, 180.0 / 255.0]);
    let img: Vec<f32> = (0..n * n).flat_map(|i| if (i % n) % 4 < 2 { a } else { b }).collect();
    let hole: Vec<bool> = (0..n * n).map(|i| (i % n).min(i / n) < 1 || (i % n).max(i / n) > n - 2).collect();
    (img, hole)
}

/// The completion of the same roles with a given patch radius: `None` when that radius leaves no
/// donor to copy from, which is what sends the fill to the membrane.
fn completion(n: usize, img: &[f32], hole: &[bool], params: &CompleteParams) -> Option<Vec<f32>> {
    let roles: Vec<Px> = hole.iter().map(|h| if *h { Px::Hole } else { Px::Source }).collect();
    complete_masked(n, n, 3, img, &roles, params, &Interrupt::NONE).unwrap()
}

/// The smallest side that still holds a fully interior 11 x 11 donor window.
const PATCH_SIDE: usize = MEASURED_STRUCTURE_PATCH_RADIUS * 2 + 1;

/// Review round 2, real: on a buffer narrower than the 11 x 11 patch there is no window centre for
/// radius 5 at all, so the measured fill found no donor and answered with the smooth membrane where
/// today's radius 3 still copies pieces (#14191 measures 600 x 400 and keeps every other case on
/// today's behaviour, and its "no regression" bullet covers this). Measured at b2764ca1c: on the
/// 9 x 9 buffer the radius-3 completion fills the border hole with 96 channel values differing from
/// the membrane; the radius-5 completion has no donor, so the fill returned the membrane in all 243
/// channel values of that buffer. The fill now gets today's radius-3 answer there again.
#[test]
fn measured_fill_on_a_small_image_keeps_todays_patch_radius() {
    for n in [PATCH_SIDE - 2, PATCH_SIDE - 1] {
        let (img, hole) = small_stripes(n);
        let opts = FillOptions::default();
        let source: Vec<bool> = hole.iter().map(|h| !h).collect();
        assert!(structure_measured(&opts) && rectangular_area(&hole, &source), "n {n}: the measured configuration");
        let params = CompleteParams { seed: opts.seed, adapt: opts.color_adaptation, ..Default::default() };
        assert_eq!(params.patch_radius, 3, "n {n}: today's radius");
        let today = completion(n, &img, &hole, &params).unwrap_or_else(|| panic!("n {n}: today's radius-3 fill has no donor either"));
        let out = fill(n, n, 3, &img, &hole, &source, &opts);
        let moved = out.iter().zip(&today).filter(|(a, b)| a != b).count();
        let membrane = membrane_fill_with(n, n, 3, &img, &hole, &Interrupt::NONE).unwrap();
        let like_membrane = out.iter().zip(&membrane).filter(|(a, b)| *a == *b).count();
        assert_eq!(
            moved,
            0,
            "n {n}: {moved} of {} channel values differ from today's radius-3 fill; {like_membrane} of them equal the membrane fill",
            img.len()
        );
    }
}

/// The other side of that border: when an 11 x 11 donor window does exist, the measured fill uses
/// it, also on a small buffer (18 x 18 with a 1 px border hole: 178 channel values differ from
/// today's radius-3 completion there). Guard test, passes before and after the change.
#[test]
fn measured_fill_uses_the_11x11_patch_when_a_window_fits() {
    let n = 18;
    let (img, hole) = small_stripes(n);
    let opts = FillOptions::default();
    let source: Vec<bool> = hole.iter().map(|h| !h).collect();
    let wide = CompleteParams { patch_radius: MEASURED_STRUCTURE_PATCH_RADIUS, seed: opts.seed, adapt: opts.color_adaptation, ..Default::default() };
    let with_patch = completion(n, &img, &hole, &wide).expect("n 18: an 11 x 11 donor window must exist beside a 1 px border hole");
    let today = completion(n, &img, &hole, &CompleteParams { seed: opts.seed, adapt: opts.color_adaptation, ..Default::default() })
        .expect("n 18: today's fill has donors");
    let out = fill(n, n, 3, &img, &hole, &source, &opts);
    let from_patch = out.iter().zip(&with_patch).filter(|(a, b)| a != b).count();
    let from_today = out.iter().zip(&today).filter(|(a, b)| a != b).count();
    assert_eq!(from_patch, 0, "the measured fill is not the 11 x 11 completion ({from_patch} channel values differ)");
    assert_ne!(from_today, 0, "the 11 x 11 completion is the same as today's ({from_today} differ): the patch was never used");
}

/// The membrane fallback stays where it already is today: a buffer with no room even for the
/// default 7 x 7 patch has no donor at either radius, so the new radius never reaches it.
/// Guard test, passes before and after the change.
#[test]
fn measured_fill_without_room_for_todays_patch_still_falls_back_to_the_membrane() {
    for n in 4..7 {
        let (img, hole) = small_stripes(n);
        let opts = FillOptions { color_adaptation: Adapt::NONE, ..FillOptions::default() };
        let source: Vec<bool> = hole.iter().map(|h| !h).collect();
        let params = CompleteParams { seed: opts.seed, adapt: opts.color_adaptation, ..Default::default() };
        assert!(completion(n, &img, &hole, &params).is_none(), "n {n}: no donor at today's radius either");
        let out = fill(n, n, 3, &img, &hole, &source, &opts);
        let membrane = membrane_fill_with(n, n, 3, &img, &hole, &Interrupt::NONE).unwrap();
        let moved = out.iter().zip(&membrane).filter(|(a, b)| a != b).count();
        assert_eq!(moved, 0, "n {n}: {moved} channel values differ from the membrane fill");
    }
}

/// The same buffer as [`small_stripes`] with an 8 x 8 hole in the middle instead of the border: at
/// 18 x 18 there are 11 x 11 windows (their centres are x and y 5 to 12), but the hole covers every
/// one of them, while a 7 x 7 window still fits in the 8-wide strips left and right of it.
fn stripes_hole_blocking_the_wide_windows(n: usize) -> (Vec<f32>, Vec<bool>) {
    let (a, b) = ([10.0 / 255.0, 200.0 / 255.0, 20.0 / 255.0], [200.0 / 255.0, 30.0 / 255.0, 180.0 / 255.0]);
    let img: Vec<f32> = (0..n * n).flat_map(|i| if (i % n) % 4 < 2 { a } else { b }).collect();
    let hole: Vec<bool> =
        (0..n * n).map(|i| matches!(i % n, x if x >= n / 2 - 1 && x <= n / 2 + 6) && matches!(i / n, y if y >= n / 2 - 1 && y <= n / 2 + 6)).collect();
    (img, hole)
}

/// The other way radius 5 can leave no donor: a buffer with room for 11 x 11 windows, and a hole
/// that uses up all of them. #14191 measured a hole four patches across in a 400 x 400 window, so a
/// fill that completes today with the default radius has to complete the same way here, before the
/// membrane fallback.
#[test]
fn measured_fill_falls_back_to_todays_radius_when_the_hole_leaves_no_room_for_the_patch() {
    let n = 18;
    let (img, hole) = stripes_hole_blocking_the_wide_windows(n);
    let opts = FillOptions::default();
    let source: Vec<bool> = hole.iter().map(|h| !h).collect();
    assert!(structure_measured(&opts) && rectangular_area(&hole, &source), "the measured configuration");
    assert_eq!(hole.iter().filter(|h| **h).count(), 64, "an 8 x 8 hole");
    let wide = CompleteParams { patch_radius: MEASURED_STRUCTURE_PATCH_RADIUS, seed: opts.seed, adapt: opts.color_adaptation, ..Default::default() };
    assert!(completion(n, &img, &hole, &wide).is_none(), "radius 5 must leave no donor here, that is the case under test");
    let params = CompleteParams { seed: opts.seed, adapt: opts.color_adaptation, ..Default::default() };
    assert_eq!(params.patch_radius, 3, "today's radius");
    let today = completion(n, &img, &hole, &params).expect("today's radius-3 fill has donors");
    let out = fill(n, n, 3, &img, &hole, &source, &opts);
    let moved = out.iter().zip(&today).filter(|(a, b)| a != b).count();
    assert_eq!(moved, 0, "{moved} of {} channel values differ from today's radius-3 fill", img.len());
}
