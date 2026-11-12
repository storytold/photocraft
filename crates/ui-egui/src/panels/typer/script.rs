#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub style: Option<usize>,
    pub page: Option<u32>,
    pub ignore: bool,
}

const IGNORE_PREFIXES: &[&str] = &["##"];

pub fn parse(script: &str, prefixes: &[(String, usize)]) -> Vec<Line> {
    let mut previous = None;
    script
        .lines()
        .map(|raw| {
            let page = page_marker(raw);
            let note = IGNORE_PREFIXES.iter().any(|p| raw.starts_with(p));
            let (prefix, mut style) = if let Some(rest) = raw.strip_prefix("//") {
                (raw.len() - rest.strip_prefix(':').unwrap_or(rest).len(), previous)
            } else {
                prefixes
                    .iter()
                    .filter(|(p, _)| !p.is_empty() && raw.starts_with(p.as_str()))
                    .max_by_key(|(p, _)| p.len())
                    .map_or((0, None), |(p, i)| (p.len(), Some(*i)))
            };
            let text = raw[prefix..].trim().to_string();
            let ignore = note || page.is_some() || text.is_empty();
            if ignore {
                style = None;
            } else if style.is_some() {
                previous = style;
            }
            Line { text, style, page, ignore }
        })
        .collect()
}

fn strip_emphasis(s: &str) -> &str {
    let s = s.trim_start_matches(['*', '_', '~']).trim_start();
    s.trim_end_matches(['*', '_', '~']).trim_end()
}

pub fn page_marker(line: &str) -> Option<u32> {
    let mut h = line.trim();
    let hashes = h.len() - h.trim_start_matches('#').len();
    if (1..=6).contains(&hashes) && h[hashes..].starts_with(char::is_whitespace) {
        h = h[hashes..].trim_start();
    }
    h = h.trim_end_matches('#').trim_end();
    h = strip_emphasis(h);
    if let Some(inner) = h.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
        h = strip_emphasis(inner.trim());
    }
    let word = h.get(..4)?;
    if !word.eq_ignore_ascii_case("page") {
        return None;
    }
    let rest = &h[4..];
    let digits_from = rest.len() - rest.trim_start().len();
    if digits_from == 0 {
        return None;
    }
    let rest = &rest[digits_from..];
    let n_len = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let n: u32 = rest[..n_len].parse().ok()?;
    let tail = rest[n_len..].trim_start_matches(['*', '_', '~']).trim_start();
    let ok = tail.is_empty() || tail == "." || tail.starts_with([':', '-', '–', '—']);
    (ok && n > 0).then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_headings() {
        for (s, n) in [("Page 1", Some(1)), ("Page 12 :", Some(12)), ("Page 12 - titre", Some(12)), ("[Page 3]", Some(3)), ("## Page 4", Some(4)), ("**Page 5:** x", Some(5)), ("page 6.", Some(6))] {
            assert_eq!(page_marker(s), n, "{s}");
        }
        for s in ["Page", "Pages 2", "On page 3", "Page 2 is nice", "Page 0"] {
            assert_eq!(page_marker(s), None, "{s}");
        }
    }

    #[test]
    fn lines_styles_and_skips() {
        let p = vec![("REG:".to_string(), 0), ("SFX:".to_string(), 1), ("SFX:BIG:".to_string(), 2)];
        let lines = parse("Page 1\nREG: Salut.\n// Suite\n## note\n\nSFX:BIG: BOUM\nSans préfixe", &p);
        let usable: Vec<_> = lines.iter().filter(|l| !l.ignore).map(|l| (l.text.as_str(), l.style)).collect();
        assert_eq!(usable, [("Salut.", Some(0)), ("Suite", Some(0)), ("BOUM", Some(2)), ("Sans préfixe", None)]);
        assert_eq!(lines[0].page, Some(1));
    }
}
