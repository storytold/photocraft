use super::*;

const W: usize = 1024;
const H: usize = 512;

/// A sky of `W × H` pixels whose pixel colour is `f(direction)`.
fn render(f: impl Fn([f64; 3]) -> [f64; 3]) -> Vec<[f32; 4]> {
    let mut px = Vec::with_capacity(W * H);
    for y in 0..H {
        for x in 0..W {
            let c = f(direction((x as f64 + 0.5) / W as f64, (y as f64 + 0.5) / H as f64));
            px.push([c[0] as f32, c[1] as f32, c[2] as f32, 1.0]);
        }
    }
    px
}

fn toward(azimuth: f64, elevation: f64) -> [f64; 3] {
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    [e.cos() * a.sin(), e.sin(), e.cos() * a.cos()]
}

fn angle_between(a: [f64; 3], b: [f64; 3]) -> f64 {
    dot(normalize(a), normalize(b)).clamp(-1.0, 1.0).acos().to_degrees()
}

/// Blue sky above, brown ground below, and a disc of `radiance` `radius` degrees across around
/// `sun`.
fn sky_with_sun(sun: [f64; 3], radius: f64, radiance: [f64; 3], sky: [f64; 3]) -> impl Fn([f64; 3]) -> [f64; 3] {
    move |d| {
        let base = if d[1] > 0.0 { sky } else { [0.12, 0.10, 0.08] };
        if angle_between(d, sun) < radius { [base[0] + radiance[0], base[1] + radiance[1], base[2] + radiance[2]] } else { base }
    }
}

#[test]
fn an_hdr_sun_is_found_and_its_light_measured() {
    let sun = toward(40.0, 35.0);
    let radiance = [3000.0, 2700.0, 2200.0];
    let px = render(sky_with_sun(sun, 1.5, radiance, [0.30, 0.45, 0.70]));
    let sky = Sky { px: &px, width: W, height: H, linear: true };

    let found = find(&sky).unwrap();
    assert!(found.clear);
    assert!(angle_between(found.direction, sun) < 0.3, "found {:?}", azimuth_elevation(found.direction));

    // The sun's irradiance over the pixels the disc covers, as the image samples it.
    let mut e = [0.0; 3];
    for y in 0..H {
        let v = (y as f64 + 0.5) / H as f64;
        let solid_angle = (FRAC_PI_2 - v * PI).cos() * (TAU / W as f64) * (PI / H as f64);
        for x in 0..W {
            if angle_between(direction((x as f64 + 0.5) / W as f64, v), sun) < 1.5 {
                for (e, r) in e.iter_mut().zip(radiance) {
                    *e += r * solid_angle;
                }
            }
        }
    }
    let light = measure(&sky, found.direction).unwrap();
    assert!(light.measured && !light.clipped);
    let expected = e[0] / PI;
    assert!((light.intensity - expected).abs() < expected * 0.01, "intensity {} expected {expected}", light.intensity);
    for i in 0..3 {
        assert!((light.colour[i] - e[i] / e[0]).abs() < 0.01, "colour {:?}", light.colour);
    }
}

#[test]
fn a_sun_on_the_left_right_seam_is_found_whole() {
    let sun = toward(179.5, 20.0);
    let px = render(sky_with_sun(sun, 1.5, [500.0; 3], [0.3, 0.4, 0.6]));
    let found = find(&Sky { px: &px, width: W, height: H, linear: true }).unwrap();
    // Half the disc is at each edge; a fill that stopped at the seam would find only one half.
    assert!(angle_between(found.direction, sun) < 0.3, "found {:?}", azimuth_elevation(found.direction));
}

#[test]
fn a_clipped_sun_is_estimated_warmer_near_the_horizon_and_stronger_under_a_brighter_sky() {
    let encoded = |high_sun: bool, sky: f64| {
        let sun = toward(-60.0, if high_sun { 60.0 } else { 6.0 });
        render(move |d| {
            let c = sky_with_sun(sun, 1.0, [10.0; 3], [sky * 0.5, sky * 0.7, sky])(d);
            c.map(|c| linear_to_srgb(c.min(1.0)))
        })
    };
    let light = |high_sun: bool, sky: f64| {
        let px = encoded(high_sun, sky);
        let sky = Sky { px: &px, width: W, height: H, linear: false };
        let found = find(&sky).unwrap();
        assert!(found.clear);
        measure(&sky, found.direction).unwrap()
    };
    let (high, low, bright) = (light(true, 0.4), light(false, 0.4), light(true, 0.8));
    assert!(!high.measured && !high.clipped);
    assert!(high.intensity > 0.0);
    assert!(low.colour[2] / low.colour[0] < high.colour[2] / high.colour[0], "low {:?} high {:?}", low.colour, high.colour);
    assert!(bright.intensity > high.intensity);
}

#[test]
fn a_clipped_32_bit_sun_is_estimated_and_flagged_as_clipped() {
    let sun = toward(10.0, 40.0);
    let px = render(move |d| sky_with_sun(sun, 1.0, [10.0; 3], [0.3, 0.4, 0.6])(d).map(|c| c.min(1.0)));
    let sky = Sky { px: &px, width: W, height: H, linear: true };
    let light = measure(&sky, find(&sky).unwrap().direction).unwrap();
    assert!(!light.measured && light.clipped);
}

#[test]
fn an_overcast_sky_has_no_clear_sun() {
    let px = render(|d| [0.5 + 0.3 * d[1]; 3]);
    let found = find(&Sky { px: &px, width: W, height: H, linear: false }).unwrap();
    assert!(!found.clear);
}

#[test]
fn the_light_shines_away_from_the_sun() {
    fn rotate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
        let [x, y, z, w] = q;
        let u = [x, y, z];
        let uv = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let uuv = [u[1] * uv[2] - u[2] * uv[1], u[2] * uv[0] - u[0] * uv[2], u[0] * uv[1] - u[1] * uv[0]];
        [0, 1, 2].map(|i| v[i] + 2.0 * (w * uv[i] + uuv[i]))
    }
    for (az, el) in [(0.0, 30.0), (-144.0, 61.0), (90.0, 5.0), (170.0, 80.0)] {
        let sun = toward(az, el);
        let (q, euler) = light_rotation(sun);
        let forward = rotate(q, [0.0, 0.0, 1.0]);
        assert!(angle_between(forward, sun.map(|c| -c)) < 1e-6, "{az} {el}: {forward:?}");
        assert!(euler[0] > 0.0, "a sun above the horizon tilts the light down");
    }
}

#[test]
fn a_lone_bright_pixel_is_found_on_black_and_on_a_dim_sky() {
    let (x, y) = (300usize, 100usize);
    for background in [0.0f32, 0.05] {
        let mut px = vec![[background, background, background, 1.0]; W * H];
        px[y * W + x] = [50.0, 50.0, 50.0, 1.0];
        let found = find(&Sky { px: &px, width: W, height: H, linear: true }).expect("a sun");
        let at = direction((x as f64 + 0.5) / W as f64, (y as f64 + 0.5) / H as f64);
        // The window's dark centre would leave nothing to find on black, and seed the sky on a dim one.
        assert!(angle_between(found.direction, at) < 0.5, "background {background}: {:?}", azimuth_elevation(found.direction));
    }
}

#[test]
fn a_colour_far_from_the_black_body_line_has_no_colour_temperature() {
    assert!(linear_to_kelvin([0.0, 1.0, 0.0]).is_none(), "pure green");
    let k = linear_to_kelvin(kelvin_to_linear(5000.0)).expect("a black body's colour");
    assert!((k - 5000.0).abs() < 150.0, "{k}");
}
