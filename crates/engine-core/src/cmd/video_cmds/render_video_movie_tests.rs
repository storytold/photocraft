//! #14165: Render Video writes one movie file (MOV, or H.264 MP4 where the build has the
//! encoder), not only image sequences.
#![cfg(not(target_arch = "wasm32"))]

use super::render_video_range_tests::{distinct_frames, grey};
use super::*;
use craft_media::isobmff::{self, TrackKind};
use serde_json::json;

/// Renders with `params`, keeping the files in memory: their names and contents.
fn render(s: &Session, params: Value) -> Result<Vec<(String, Vec<u8>)>> {
    let mut written = Vec::new();
    render_video_with(
        s,
        &params,
        |name, bytes| {
            written.push((name.to_owned(), bytes.to_vec()));
            Ok(())
        },
        |_, _| true,
    )?;
    Ok(written)
}

/// A written movie's video track: its sample entry's code and size, its frame rate and its
/// samples in order.
struct Movie {
    fourcc: [u8; 4],
    size: (u16, u16),
    fps: f64,
    seconds: f64,
    samples: Vec<Vec<u8>>,
}

fn movie(file: &[u8]) -> Movie {
    let mp4 = isobmff::open(file).unwrap();
    let t = mp4.track_of_kind(TrackKind::Video).unwrap();
    let track = &mp4.tracks[t];
    let entry = &track.entries[0];
    let video = entry.video.as_ref().unwrap();
    let seconds = track.duration as f64 / f64::from(track.timescale);
    Movie {
        fourcc: entry.format.0,
        size: (video.width, video.height),
        fps: track.samples.len() as f64 / seconds,
        seconds,
        samples: (0..track.samples.len()).map(|i| mp4.read_sample(file, t, i).unwrap()).collect(),
    }
}

/// The grey (0..1) at the middle of a ProRes frame, from its limited-range luma.
fn prores_grey(sample: &[u8]) -> f32 {
    let f = craft_video::prores::decode_frame(sample).unwrap();
    let code = f32::from(f.y[(f.height / 2 * f.width + f.width / 2) as usize]) / 2f32.powi(i32::from(f.bit_depth) - 8);
    (code - 16.0) / 219.0
}

#[test]
fn a_mov_holds_every_frame_at_the_timeline_rate() {
    let s = distinct_frames(10);
    let files = render(&s, json!({"format": "mov", "dir": "out"})).unwrap();
    assert_eq!(files.len(), 1, "one file, not a sequence");
    assert_eq!(files[0].0, "rv.mov");
    let m = movie(&files[0].1);
    assert_eq!(&m.fourcc, b"ap4h", "Animation High Quality is ProRes 4444");
    assert_eq!(m.size, (8, 6));
    assert_eq!(m.samples.len(), 10);
    assert!((m.fps - 30.0).abs() < 1e-9 && (m.seconds - 1.0 / 3.0).abs() < 1e-9, "10 frames at 30 fps last 0.333 s: {} fps, {} s", m.fps, m.seconds);
    for (n, sample) in m.samples.iter().enumerate() {
        let got = prores_grey(sample);
        assert!((got - grey(n)).abs() < 2.0 / 255.0, "frame {n}: grey {got} for {}", grey(n));
    }
}

#[test]
fn uncompressed_frames_hold_the_rendered_pixels() {
    let s = distinct_frames(5);
    let files = render(&s, json!({"format": "mov", "preset": "uncompressed"})).unwrap();
    let m = movie(&files[0].1);
    assert_eq!(&m.fourcc, b"raw ");
    assert_eq!(m.samples.len(), 5);
    for (n, sample) in m.samples.iter().enumerate() {
        let v = (grey(n) * 255.0).round() as u8;
        assert_eq!(sample.len(), 8 * 3 * 6);
        assert!(sample.iter().all(|&b| b == v), "frame {n} is grey {v} throughout");
    }
}

#[test]
fn the_range_size_and_rate_shape_the_movie() {
    let s = distinct_frames(30);
    let files = render(&s, json!({"format": "mov", "preset": "uncompressed", "range": {"start": 10, "end": 19}, "fps": 15, "width": 4, "height": 3})).unwrap();
    let m = movie(&files[0].1);
    assert_eq!(m.size, (4, 3));
    assert_eq!(m.samples.len(), 5, "10 frames at 30 fps are 5 at 15");
    assert!((m.fps - 15.0).abs() < 1e-9);
    for (k, sample) in m.samples.iter().enumerate() {
        let v = (grey(10 + 2 * k) * 255.0).round() as u8;
        assert_eq!(sample[0], v, "output frame {k} shows timeline frame {}", 10 + 2 * k);
    }
}

#[test]
fn bad_movie_params_are_refused_and_write_nothing() {
    let s = distinct_frames(3);
    for params in [
        json!({"format": "mov", "preset": "nope"}),
        json!({"format": "mov", "preset": 5}),
        json!({"format": "mp4", "preset": "animationHigh"}),
        json!({"format": "mp4", "preset": "mediumQuality"}),
    ] {
        let mut written = 0;
        let err = render_video_with(
            &s,
            &params,
            |_, _| {
                written += 1;
                Ok(())
            },
            |_, _| true,
        );
        assert!(err.is_err() && written == 0, "{params} is refused before anything is written");
    }
    // Without a timeline there is no movie to write.
    let mut s = crate::session();
    s.execute("file.new", json!({"width": 8, "height": 6})).unwrap();
    let err = render(&s, json!({"format": "mov"})).unwrap_err().to_string();
    assert!(err.contains("timeline"), "{err}");
}

#[test]
fn a_cancelled_movie_fails() {
    let s = distinct_frames(6);
    let mut written = 0;
    let midway = render_video_with(
        &s,
        &json!({"format": "mov"}),
        |_, _| {
            written += 1;
            Ok(())
        },
        |done, _| done < 3,
    );
    assert!(midway.is_err() && written == 0, "cancelled midway, nothing is written");
    // The final report follows the write (total means the file exists): cancelled there, the
    // render still fails, so the caller discards the file, as it does an image sequence's.
    let r = render_video_with(
        &s,
        &json!({"format": "mov"}),
        |_, _| {
            written += 1;
            Ok(())
        },
        |done, total| done < total,
    );
    assert!(r.is_err() && written == 1);
}

/// H.264 is written only by builds with the encoder; the others say why, and write nothing.
#[test]
fn h264_movies_need_the_encoder_in_the_build() {
    let s = distinct_frames(10);
    if photocraft_io::movie_out::MovieCodec::H264.available() {
        let files = render(&s, json!({"format": "mp4"})).unwrap();
        assert_eq!(files[0].0, "rv.mp4");
        let m = movie(&files[0].1);
        assert_eq!(&m.fourcc, b"avc1");
        assert_eq!(m.samples.len(), 10);
        assert!((m.seconds - 1.0 / 3.0).abs() < 1e-9);
    } else {
        let err = render(&s, json!({"format": "mp4"})).unwrap_err().to_string();
        assert!(err.contains("H.264") && err.contains("#14165"), "{err}");
    }
}
