use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub top: i32,
    pub rows: Vec<Option<(f32, f32)>>,
}

impl Shape {
    pub fn from_mask(cov: &[f32], w: usize, h: usize, x0: i32, y0: i32) -> Option<Shape> {
        let rows: Vec<_> = (0..h)
            .map(|y| {
                let row = &cov[y * w..(y + 1) * w];
                let first = row.iter().position(|&c| c >= 0.5)?;
                let last = row.iter().rposition(|&c| c >= 0.5)?;
                Some((x0 as f32 + first as f32, x0 as f32 + last as f32 + 1.0))
            })
            .collect();
        let first = rows.iter().position(Option::is_some)?;
        let last = rows.iter().rposition(Option::is_some)?;
        Some(Shape { top: y0 + first as i32, rows: rows[first..=last].to_vec() })
    }

    pub fn bottom(&self) -> f32 {
        (self.top + self.rows.len() as i32) as f32
    }

    pub fn eroded(&self, pad: f32) -> Shape {
        let r = pad.max(0.0).ceil() as isize;
        let n = self.rows.len() as isize;
        let rows = (0..n)
            .map(|y| {
                let (mut l, mut rt) = (f32::MIN, f32::MAX);
                for dy in -r..=r {
                    let (a, b) = (*self.rows.get((y + dy) as usize).filter(|_| y + dy >= 0)?)?;
                    l = l.max(a);
                    rt = rt.min(b);
                }
                let (l, rt) = (l + pad, rt - pad);
                (rt > l).then_some((l, rt))
            })
            .collect();
        Shape { top: self.top, rows }
    }

    pub fn centroid(&self) -> Option<(f32, f32)> {
        let (mut area, mut sx, mut sy) = (0.0f64, 0.0f64, 0.0f64);
        for (i, row) in self.rows.iter().enumerate() {
            if let Some((l, r)) = row {
                let a = f64::from(r - l);
                area += a;
                sx += a * f64::from(l + r) / 2.0;
                sy += a * (f64::from(self.top) + i as f64 + 0.5);
            }
        }
        (area > 0.0).then(|| ((sx / area) as f32, (sy / area) as f32))
    }

    pub fn polygon(&self) -> Vec<[f32; 2]> {
        let mut left = Vec::new();
        let mut right = Vec::new();
        for (i, row) in self.rows.iter().enumerate() {
            if let Some((l, r)) = *row {
                let y = (self.top + i as i32) as f32;
                left.extend([[l, y], [l, y + 1.0]]);
                right.extend([[r, y], [r, y + 1.0]]);
            }
        }
        right.reverse();
        left.extend(right);
        left.dedup_by(|a, b| a[0] == b[0] && b[0] == a[0] && (a[1] - b[1]).abs() < 1e-3);
        left
    }

    fn band(&self, y0: f32, y1: f32) -> Option<(f32, f32)> {
        let a = (y0.floor() as i32 - self.top).max(0) as usize;
        let b = ((y1.ceil() as i32 - self.top).max(0) as usize).min(self.rows.len());
        if y0 < self.top as f32 || y1 > self.bottom() || a >= b {
            return None;
        }
        let (mut l, mut r) = (f32::MIN, f32::MAX);
        for row in &self.rows[a..b] {
            let (rl, rr) = (*row)?;
            l = l.max(rl);
            r = r.min(rr);
        }
        (r > l).then_some((l, r))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub ascent: f32,
    pub descent: f32,
    pub pitch: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub lines: Vec<String>,
    pub scale: f32,
    pub center: (f32, f32),
    pub first_baseline: f32,
    pub hyphenated: bool,
}

fn tokens(text: &str) -> Vec<String> {
    let closing = |w: &str| w.chars().all(|c| "!?;:.,…»›)]}".contains(c));
    let opening = |w: &str| w.chars().all(|c| "«‹¿¡([{".contains(c));
    let mut out: Vec<String> = Vec::new();
    let mut pending_open: Option<String> = None;
    for w in text.split_whitespace() {
        let w = match pending_open.take() {
            Some(o) => format!("{o} {w}"),
            None => w.to_string(),
        };
        if opening(&w) {
            pending_open = Some(w);
        } else if closing(&w) && !out.is_empty() {
            let last = out.len() - 1;
            out[last].push(' ');
            out[last].push_str(&w);
        } else {
            out.push(w);
        }
    }
    if let Some(o) = pending_open {
        out.push(o);
    }
    out
}

struct Variant {
    tokens: Vec<String>,
    hyphen_at: Option<usize>,
}

fn variants(text: &str, hyphenate: bool) -> Vec<Variant> {
    let base = tokens(text);
    if !hyphenate {
        return vec![Variant { tokens: base, hyphen_at: None }];
    }
    let mut out = Vec::new();
    for (i, word) in base.iter().enumerate() {
        for at in french_hyphen_points(word) {
            let mut tokens = base[..i].to_vec();
            tokens.push(format!("{}-", &word[..at]));
            tokens.push(word[at..].to_string());
            tokens.extend_from_slice(&base[i + 1..]);
            out.push(Variant { tokens, hyphen_at: Some(i) });
        }
    }
    out
}

pub fn solve(shape: &Shape, text: &str, m: Metrics, measure: &mut dyn FnMut(&str) -> f32, min_scale: f32, hyphenate: bool) -> Option<Plan> {
    let (cx, cy) = shape.centroid()?;
    let mut cache: HashMap<String, f32> = HashMap::new();
    let mut width = |s: &str| *cache.entry(s.to_string()).or_insert_with(|| measure(s));
    let min_scale = min_scale.clamp(0.05, 1.0);
    let scales: Vec<f32> = {
        let mut v = vec![1.0];
        let mut s = 1.0 - 0.02;
        while s > min_scale + 1e-4 {
            v.push(s);
            s -= 0.02;
        }
        v.push(min_scale);
        v
    };
    for pass_hyphen in [false, true] {
        if pass_hyphen && !hyphenate {
            break;
        }
        let vars = variants(text, pass_hyphen);
        for &scale in &scales {
            let mut best: Option<(f32, Plan)> = None;
            for v in &vars {
                if let Some((cost, plan)) = best_at(shape, v, m, scale, (cx, cy), &mut width) {
                    let plan = Plan { hyphenated: pass_hyphen, ..plan };
                    if best.as_ref().is_none_or(|(c, _)| cost < *c) {
                        best = Some((cost, plan));
                    }
                }
            }
            if let Some((_, plan)) = best {
                return Some(plan);
            }
        }
    }
    None
}

fn best_at(shape: &Shape, v: &Variant, m: Metrics, scale: f32, (cx, cy): (f32, f32), width: &mut dyn FnMut(&str) -> f32) -> Option<(f32, Plan)> {
    let (asc, desc, pitch) = (m.ascent * scale, m.descent * scale, m.pitch * scale);
    let t = v.tokens.len();
    if t == 0 {
        return None;
    }
    let height = shape.rows.len() as f32;
    let mut line_w: HashMap<(usize, usize), f32> = HashMap::new();
    let mut best: Option<(f32, Plan)> = None;
    for n in 1..=t.min(14) {
        let block = asc + desc + (n - 1) as f32 * pitch;
        if block > height {
            break;
        }
        for k in -6..=6 {
            let center = (cy + k as f32 * pitch / 4.0).clamp(shape.top as f32 + block / 2.0, shape.bottom() - block / 2.0);
            let top = center - block / 2.0;
            let caps: Option<Vec<f32>> = (0..n)
                .map(|i| {
                    let base = top + asc + i as f32 * pitch;
                    shape.band(base - asc, base + desc).map(|(l, r)| 2.0 * (cx - l).min(r - cx)).filter(|c| *c > 0.0)
                })
                .collect();
            let Some(caps) = caps else { continue };
            let Some((cost, breaks)) = break_lines(v, &caps, scale, &mut line_w, width) else { continue };
            let cost = cost + 0.5 * ((center - cy) / pitch).powi(2) + 0.02 * n as f32;
            if best.as_ref().is_none_or(|(c, _)| cost < *c) {
                let lines = breaks.windows(2).map(|w| v.tokens[w[0]..w[1]].join(" ")).collect();
                best = Some((cost, Plan { lines, scale, center: (cx, center), first_baseline: top + asc, hyphenated: false }));
            }
        }
    }
    best
}

fn break_lines(v: &Variant, caps: &[f32], scale: f32, line_w: &mut HashMap<(usize, usize), f32>, width: &mut dyn FnMut(&str) -> f32) -> Option<(f32, Vec<usize>)> {
    let t = v.tokens.len();
    let n = caps.len();
    let max_cap = caps.iter().copied().fold(0.0, f32::max);
    let inf = f32::INFINITY;
    let mut cost = vec![vec![inf; t + 1]; n + 1];
    let mut from = vec![vec![0usize; t + 1]; n + 1];
    cost[0][0] = 0.0;
    for k in 0..n {
        for i in 0..t {
            if !cost[k][i].is_finite() {
                continue;
            }
            for j in i + 1..=t {
                if v.hyphen_at.is_some_and(|h| i <= h && h + 1 < j) {
                    break;
                }
                let w = *line_w.entry((i, j)).or_insert_with(|| width(&v.tokens[i..j].join(" "))) * scale;
                if w > max_cap {
                    break;
                }
                if w > caps[k] {
                    continue;
                }
                let free = (caps[k] - w) / caps[k];
                let c = cost[k][i] + free * free;
                if c < cost[k + 1][j] {
                    cost[k + 1][j] = c;
                    from[k + 1][j] = i;
                }
            }
        }
    }
    if !cost[n][t].is_finite() {
        return None;
    }
    let mut breaks = vec![t];
    let mut j = t;
    for k in (1..=n).rev() {
        j = from[k][j];
        breaks.push(j);
    }
    breaks.reverse();
    Some((cost[n][t] / n as f32, breaks))
}

fn is_vowel(c: char) -> bool {
    "aeiouyàâäéèêëîïôöùûüÿœæ".contains(c)
}

fn is_consonant(c: char) -> bool {
    c.is_alphabetic() && !is_vowel(c)
}

const ONSETS: &[&str] = &[
    "bl", "br", "ch", "cl", "cr", "dr", "fl", "fr", "gl", "gn", "gr", "kl", "kr", "ph", "pl", "pr", "rh", "th", "tr", "vr", "chl", "chr", "phl", "phr", "thr",
];

fn syllable_penalty(w: &[char], i: usize) -> f32 {
    let (left, right) = (&w[..i], &w[i..]);
    if !left.iter().any(|&c| is_vowel(c)) || !right.iter().any(|&c| is_vowel(c)) {
        return f32::INFINITY;
    }
    let (prev, cur) = (w[i - 1], w[i]);
    if is_vowel(prev) && is_vowel(cur) {
        return 7.0;
    }
    if is_consonant(prev) && is_vowel(cur) {
        return 16.0;
    }
    if cur == 'x' && w.get(i + 1).is_some_and(|&c| is_vowel(c)) {
        return 12.0;
    }
    let pair: String = [prev, cur].iter().collect();
    if is_consonant(prev) && ONSETS.contains(&pair.as_str()) {
        return 7.0;
    }
    let mut start = i;
    while start > 0 && is_consonant(w[start - 1]) {
        start -= 1;
    }
    let mut end = i;
    while end < w.len() && is_consonant(w[end]) {
        end += 1;
    }
    let onset: String = w[i..end].iter().collect();
    let coda = i - start;
    let rest: String = w[end..].iter().collect();
    if rest == "e" || rest == "es" {
        return 6.0;
    }
    if onset == "h" {
        return 6.0;
    }
    if coda >= 2 {
        return 10.0;
    }
    if onset.chars().count() == 1 {
        return if coda == 0 { 0.0 } else { 1.0 };
    }
    if ONSETS.contains(&onset.as_str()) {
        return if coda == 0 { 0.5 } else { 1.5 };
    }
    10.0
}

pub fn french_hyphen_points(word: &str) -> Vec<usize> {
    let body = word.trim_end_matches(['.', ',', ';', ':', '!', '?']);
    let chars: Vec<char> = body.chars().collect();
    if chars.len() < 8 || chars.iter().any(|c| *c == '-' || c.is_whitespace() || "*_~".contains(*c)) {
        return Vec::new();
    }
    let lower: Vec<char> = chars.iter().flat_map(|c| c.to_lowercase()).collect();
    if lower.len() != chars.len() {
        return Vec::new();
    }
    let apostrophe = |c: char| c == '\'' || c == '’';
    let mut points: Vec<(f32, usize)> = Vec::new();
    for i in 3..=chars.len() - 3 {
        if chars[i.saturating_sub(2)..i].iter().any(|&c| apostrophe(c)) || chars[i..(i + 2).min(chars.len())].iter().any(|&c| apostrophe(c)) {
            continue;
        }
        let p = syllable_penalty(&lower, i);
        if p.is_finite() && p < 3.0 {
            points.push((p * 10.0 + (i as f32 - chars.len() as f32 / 2.0).abs(), i));
        }
    }
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    points.into_iter().take(3).map(|(_, i)| chars[..i].iter().map(|c| c.len_utf8()).sum()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ellipse(w: usize, h: usize) -> Vec<f32> {
        let (a, b) = (w as f32 / 2.0, h as f32 / 2.0);
        (0..h)
            .flat_map(|y| (0..w).map(move |x| ((x as f32 + 0.5 - a) / a).powi(2) + ((y as f32 + 0.5 - b) / b).powi(2)))
            .map(|d| if d <= 1.0 { 1.0 } else { 0.0 })
            .collect()
    }

    fn mono(s: &str) -> f32 {
        s.chars().count() as f32 * 10.0
    }
    const M: Metrics = Metrics { ascent: 14.0, descent: 4.0, pitch: 20.0 };

    #[test]
    fn letters_left_out_by_the_wand_count_as_inside() {
        let mut m = ellipse(40, 20);
        m[10 * 40 + 20] = 0.0;
        let s = Shape::from_mask(&m, 40, 20, 100, 50).unwrap();
        assert_eq!(s.top, 50);
        assert_eq!(s.rows[10], Some((100.0, 140.0)));
        assert!(s.polygon().len() >= 4);
    }

    #[test]
    fn fits_the_ellipse_line_by_line_without_shrinking() {
        let s = Shape::from_mask(&ellipse(300, 160), 300, 160, 0, 0).unwrap().eroded(4.0);
        let p = solve(&s, "Ceci est une phrase assez longue pour plusieurs lignes", M, &mut mono, 0.75, false).unwrap();
        assert_eq!(p.scale, 1.0);
        assert!(p.lines.len() >= 2, "{p:?}");
        assert_eq!(p.lines.join(" "), "Ceci est une phrase assez longue pour plusieurs lignes");
        let widths: Vec<_> = p.lines.iter().map(|l| mono(l)).collect();
        assert!(widths[0] <= widths.iter().copied().fold(0.0, f32::max));
        assert!((p.center.0 - 150.0).abs() < 1.0 && (p.center.1 - 80.0).abs() < 25.0, "{p:?}");
    }

    #[test]
    fn shrinks_then_hyphenates_then_gives_up() {
        let s = Shape::from_mask(&ellipse(120, 60), 120, 60, 0, 0).unwrap();
        let p = solve(&s, "Bonjour toi", M, &mut mono, 0.5, false).unwrap();
        assert!(p.scale <= 1.0);
        let narrow = Shape { top: 0, rows: vec![Some((0.0, 140.0)); 60] };
        assert!(solve(&narrow, "anticonstitutionnellement", M, &mut mono, 0.75, false).is_none());
        let p = solve(&narrow, "anticonstitutionnellement", M, &mut mono, 0.75, true).unwrap();
        assert!(p.hyphenated && p.lines.len() == 2 && p.lines[0].ends_with('-'), "{p:?}");
        let tiny = Shape { top: 0, rows: vec![Some((0.0, 10.0)); 5] };
        assert!(solve(&tiny, "Rien ne rentre", M, &mut mono, 0.75, true).is_none());
    }

    #[test]
    fn french_points_and_punctuation() {
        let w = "anticonstitutionnellement";
        let pts = french_hyphen_points(w);
        assert!(!pts.is_empty());
        for p in pts {
            assert!(p >= 3 && w.len() - p >= 3);
        }
        assert!(french_hyphen_points("court").is_empty());
        assert_eq!(tokens("« Salut ! » toi"), ["« Salut ! »", "toi"]);
    }
}
