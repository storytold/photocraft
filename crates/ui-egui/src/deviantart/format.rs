//! Pure DeviantArt request and response formatting helpers.

use serde_json::Value;

pub use photocraft_engine::deviantart_cmds::clean_tags;

/// Sta.sh shows one private item at `https://www.deviantart.com/stash/0` plus the id in base 36.
pub fn stash_url(item_id: i64) -> Option<String> {
    if item_id <= 0 {
        return None;
    }
    let mut n = item_id as u64;
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut code = Vec::new();
    while n > 0 {
        let index = (n % 36) as usize;
        code.push(*digits.get(index)?);
        n /= 36;
    }
    code.reverse();
    let code = String::from_utf8(code).ok()?;
    Some(format!("https://www.deviantart.com/stash/0{code}"))
}

pub fn item_id(body: &str) -> Option<i64> {
    let value: Value = serde_json::from_str(body).ok()?;
    value.get("itemid")?.as_i64()
}

pub fn multipart(title: &str, comments: &str, tags: &[String], filename: &str, bytes: &[u8]) -> (String, Vec<u8>) {
    let boundary = boundary_for(std::iter::once(bytes).chain([title.as_bytes(), comments.as_bytes()]).chain(tags.iter().map(|tag| tag.as_bytes())));
    let filename: String = filename.chars().map(|c| if matches!(c, '"' | '\r' | '\n' | '\\') { '_' } else { c }).collect();
    let mut body = Vec::new();
    part(&mut body, &boundary, "title", title);
    part(&mut body, &boundary, "artist_comments", comments);
    for (index, tag) in tags.iter().enumerate() {
        part(&mut body, &boundary, &format!("tags[{index}]"), tag);
    }
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

fn part(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
}

// Scan delimiter lines once, then choose the first unused numeric suffix. This also
// handles crafted input without repeatedly rescanning a large upload for collisions.
fn boundary_for<'a>(parts: impl Iterator<Item = &'a [u8]>) -> String {
    let mut used = std::collections::BTreeSet::new();
    for part in parts {
        for line in part.split(|byte| *byte == b'\n') {
            let Some(tail) = line.strip_prefix(b"--PcDeviantArt") else { continue };
            let tail = tail.trim_ascii_end();
            let tail = tail.strip_suffix(b"--").unwrap_or(tail);
            if let Some(number) = std::str::from_utf8(tail).ok().and_then(|text| text.parse::<usize>().ok()) {
                used.insert(number);
            }
        }
    }
    let suffix = (0..=used.len()).find(|number| !used.contains(number)).unwrap_or(used.len());
    format!("PcDeviantArt{suffix}")
}
