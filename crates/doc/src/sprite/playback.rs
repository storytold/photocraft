//! Variable millisecond timing and Aseprite tag traversal, with bounded arithmetic even for
//! a very large elapsed time. A ping-pong repeat counts one traversal in either direction.

use super::Sprite;

impl Sprite {
    /// Frame at an elapsed millisecond. `finished` holds the last frame after a finite tag.
    /// A missing tag means the complete sprite, forward and looping forever.
    pub fn playback_frame(&self, elapsed_ms: u64, tag: Option<usize>) -> Result<(usize, bool), String> {
        if self.frames.is_empty() {
            return Err("sprite has no frames".into());
        }
        let (from, to, direction, repeat) = match tag {
            Some(index) => {
                let t = self.tags.get(index).ok_or("animation tag is out of range")?;
                (t.from, t.to, t.direction, t.repeat)
            }
            None => (0, self.frames.len() - 1, 0, 0),
        };
        if from > to || to >= self.frames.len() || direction > 3 {
            return Err("invalid animation tag range/direction".into());
        }
        let forward: Vec<_> = (from..=to).collect();
        let reverse: Vec<_> = (from..=to).rev().collect();
        let first = if direction == 1 || direction == 3 { &reverse } else { &forward };
        let duration = |seq: &[usize]| -> u64 { seq.iter().filter_map(|i| self.frames.get(*i)).map(|f| u64::from(f.duration_ms.max(1))).sum() };
        let locate = |seq: &[usize], mut ms: u64| -> usize {
            for &i in seq {
                let d = self.frames.get(i).map_or(1, |f| u64::from(f.duration_ms.max(1)));
                if ms < d {
                    return i;
                }
                ms -= d;
            }
            seq.last().copied().unwrap_or(from)
        };
        let first_duration = duration(first);
        if direction < 2 || from == to {
            if repeat != 0 && elapsed_ms >= first_duration.saturating_mul(u64::from(repeat)) {
                return Ok((first.last().copied().unwrap_or(from), true));
            }
            return Ok((locate(first, elapsed_ms % first_duration.max(1)), false));
        }
        if elapsed_ms < first_duration {
            return Ok((locate(first, elapsed_ms), false));
        }
        if repeat == 1 {
            return Ok((first.last().copied().unwrap_or(from), true));
        }
        // Subsequent traversals skip the endpoint just shown by the previous traversal.
        let a = if direction == 2 { &reverse } else { &forward };
        let a = a.get(1..).ok_or("ping-pong range is empty")?;
        let b = first.get(1..).ok_or("ping-pong range is empty")?;
        let (da, db) = (duration(a), duration(b));
        let remaining = elapsed_ms - first_duration;
        if repeat != 0 {
            let legs = u64::from(repeat - 1);
            let total = legs.div_ceil(2).saturating_mul(da) + (legs / 2).saturating_mul(db);
            if remaining >= total {
                return Ok((if legs % 2 == 1 { a } else { b }.last().copied().unwrap_or(from), true));
            }
        }
        let at = remaining % (da + db).max(1);
        Ok((if at < da { locate(a, at) } else { locate(b, at - da) }, false))
    }
}

#[cfg(test)]
mod tests {
    use super::super::{SpriteFrame, SpriteTag};
    use super::*;
    use std::{collections::BTreeMap, sync::Arc};
    #[test]
    fn playback_keeps_variable_delays_reverse_pingpong_and_finite_repeat_endpoints() {
        let mut s = Sprite {
            frames: [17, 83, 29]
                .into_iter()
                .map(|duration_ms| Arc::new(SpriteFrame { duration_ms, palette: Arc::new(vec![]), cels: BTreeMap::new(), metadata: vec![] }))
                .collect(),
            current: 0,
            layers: vec![],
            indexed: false,
            transparent_index: 0,
            pixel_aspect: [1, 1],
            grid: [0; 4],
            tags: vec![],
            profile: None,
        };
        assert_eq!([0, 16, 17, 99, 100, 128, 129].map(|t| s.playback_frame(t, None).unwrap().0), [0, 0, 1, 1, 2, 2, 0]);
        s.tags.push(SpriteTag { name: "Ping".into(), from: 0, to: 2, direction: 2, repeat: 0, color: [0; 3] });
        assert_eq!([128, 129, 211, 212, 228, 229, 311, 312].map(|t| s.playback_frame(t, Some(0)).unwrap().0), [2, 1, 1, 0, 0, 1, 1, 2]);
        s.tags[0].repeat = 1;
        assert_eq!(s.playback_frame(129, Some(0)).unwrap(), (2, true));
        s.tags[0].repeat = 2;
        assert_eq!(s.playback_frame(229, Some(0)).unwrap(), (0, true));
        s.tags[0].direction = 3;
        s.tags[0].repeat = 0;
        assert_eq!([0, 29, 112, 129, 212].map(|t| s.playback_frame(t, Some(0)).unwrap().0), [2, 1, 0, 1, 2]);
        assert!(s.playback_frame(u64::MAX, Some(0)).is_ok());
    }
}
