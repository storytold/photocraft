use crate::error::Error;

/// The 4-byte ASCII signature of Paint.NET documents.
pub const MAGIC: &[u8; 4] = b"PDN3";

/// Returns true if `bytes` begins with the PDN3 magic signature.
pub fn is_pdn(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// Metadata extracted from the XML document header.
#[derive(Debug, Clone, Default)]
pub struct HeaderInfo {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub layers: Option<usize>,
    pub version: Option<String>,
    pub thumbnail_png: Option<Vec<u8>>,
}

/// Parses the PDN3 header, verifying magic, header size, XML content, and trailing indicator bytes.
/// Returns the parsed header info and the remaining slice containing the NRBF and deferred streams.
pub fn parse_header(bytes: &[u8], max_header_size: usize) -> Result<(HeaderInfo, &[u8]), Error> {
    if bytes.len() < 9 {
        return Err(Error::Malformed("file too small for PDN header"));
    }
    if !is_pdn(bytes) {
        return Err(Error::Malformed("missing PDN3 magic signature"));
    }

    let b4 = *bytes.get(4).ok_or(Error::Malformed("unexpected EOF in header length"))? as usize;
    let b5 = *bytes.get(5).ok_or(Error::Malformed("unexpected EOF in header length"))? as usize;
    let b6 = *bytes.get(6).ok_or(Error::Malformed("unexpected EOF in header length"))? as usize;
    let header_size = b4 | (b5 << 8) | (b6 << 16);

    if header_size > max_header_size {
        return Err(Error::Limit("PDN XML header exceeds maximum allowed size"));
    }

    let header_end = 7usize
        .checked_add(header_size)
        .ok_or(Error::Malformed("header size overflow"))?;
    let indicator_end = header_end
        .checked_add(2)
        .ok_or(Error::Malformed("indicator offset overflow"))?;

    if bytes.len() < indicator_end {
        return Err(Error::Malformed("file truncated in PDN header"));
    }

    let xml_bytes = bytes.get(7..header_end).ok_or(Error::Malformed("invalid header slice"))?;
    let xml_str = std::str::from_utf8(xml_bytes).map_err(|_| Error::Malformed("invalid UTF-8 in XML header"))?;

    let indicator = bytes.get(header_end..indicator_end).ok_or(Error::Malformed("invalid indicator slice"))?;
    if indicator != [0x00, 0x01] {
        return Err(Error::Malformed("invalid PDN stream indicator bytes (expected 0x00, 0x01)"));
    }

    let info = parse_xml_info(xml_str);
    let remaining = bytes.get(indicator_end..).unwrap_or(&[]);
    Ok((info, remaining))
}

fn parse_xml_info(xml: &str) -> HeaderInfo {
    let width = find_attr(xml, "width").and_then(|v| v.parse::<u32>().ok());
    let height = find_attr(xml, "height").and_then(|v| v.parse::<u32>().ok());
    let layers = find_attr(xml, "layers").and_then(|v| v.parse::<usize>().ok());
    let version = find_attr(xml, "savedWithVersion").map(ToOwned::to_owned);
    let thumbnail_png = find_attr(xml, "png").and_then(decode_base64);

    HeaderInfo {
        width,
        height,
        layers,
        version,
        thumbnail_png,
    }
}

fn find_attr<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    for quote in ['"', '\''] {
        let prefix = format!("{name}={quote}");
        if let Some(idx) = xml.find(&prefix) {
            let start = idx.checked_add(prefix.len())?;
            if let Some(sub) = xml.get(start..)
                && let Some(end) = sub.find(quote)
            {
                return sub.get(..end);
            }
        }
    }
    None
}

fn decode_base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity((s.len().checked_mul(3)?).checked_div(4)?);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &b in s.as_bytes() {
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b' ' | b'\r' | b'\n' | b'\t' => continue,
            _ => return None,
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}
