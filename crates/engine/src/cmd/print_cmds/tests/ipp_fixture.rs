//! Test-only wire fixture helpers retained from the former photocraft-ipp crate.
//! Production protocol parsing and transport are exclusively craft_print::cups.

pub const OP_PRINT_JOB: u16 = 0x0002;
pub const OP_CUPS_GET_DEFAULT: u16 = 0x4001;
pub const OP_CUPS_GET_PRINTERS: u16 = 0x4002;
pub const OP_CUPS_GET_CLASSES: u16 = 0x4005;

pub const TAG_OPERATION_GROUP: u8 = 0x01;
pub const TAG_JOB_GROUP: u8 = 0x02;
pub const TAG_END: u8 = 0x03;
pub const TAG_PRINTER_GROUP: u8 = 0x04;
pub const TAG_INTEGER: u8 = 0x21;
pub const TAG_NAME: u8 = 0x42;
pub const TAG_CHARSET: u8 = 0x47;
pub const TAG_LANGUAGE: u8 = 0x48;

pub struct Request {
    buf: Vec<u8>,
}

impl Request {
    pub fn new(op: u16, id: u32) -> Self {
        // Shared CUPS requests and their replies use IPP 1.1.
        let mut buf = vec![1, 1];
        buf.extend_from_slice(&op.to_be_bytes());
        buf.extend_from_slice(&id.to_be_bytes());
        buf.push(TAG_OPERATION_GROUP);
        let mut r = Request { buf };
        r.attr(TAG_CHARSET, "attributes-charset", b"utf-8");
        r.attr(TAG_LANGUAGE, "attributes-natural-language", b"en");
        r
    }

    /// Appends an attribute to the current group.
    pub fn attr(&mut self, tag: u8, name: &str, value: &[u8]) {
        // IPP lengths are 16-bit; longer names and values are cut, both the length and the bytes.
        let name = name.as_bytes().get(..name.len().min(0xffff)).unwrap_or_default();
        let value = value.get(..value.len().min(0xffff)).unwrap_or_default();
        self.buf.push(tag);
        self.buf.extend_from_slice(&(name.len() as u16).to_be_bytes());
        self.buf.extend_from_slice(name);
        self.buf.extend_from_slice(&(value.len() as u16).to_be_bytes());
        self.buf.extend_from_slice(value);
    }

    pub fn integer(&mut self, name: &str, v: i32) {
        self.attr(TAG_INTEGER, name, &v.to_be_bytes());
    }

    /// Starts a new attribute group.
    pub fn group(&mut self, tag: u8) {
        self.buf.push(tag);
    }

    /// The message with its end tag.
    pub fn end(mut self) -> Vec<u8> {
        self.buf.push(TAG_END);
        self.buf
    }
}

/// One attribute of a reply: the group it sits in, its tag, name and values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub group: u8,
    pub tag: u8,
    pub name: String,
    pub values: Vec<Vec<u8>>,
}

impl Attribute {
    /// The first value as text.
    pub fn text(&self) -> String {
        self.values.first().map(|v| String::from_utf8_lossy(v).into_owned()).unwrap_or_default()
    }
    /// The first value as a big-endian integer.
    pub fn int(&self) -> Option<i32> {
        self.values.first().and_then(|v| <[u8; 4]>::try_from(v.as_slice()).ok()).map(i32::from_be_bytes)
    }
}

pub fn parse_request(bytes: &[u8]) -> Option<(u16, Vec<Attribute>, Vec<u8>)> {
    let operation = u16::from_be_bytes(bytes.get(2..4)?.try_into().ok()?);
    let mut pos = 8;
    let mut group = 0;
    let mut attrs: Vec<Attribute> = Vec::new();
    loop {
        let tag = *bytes.get(pos)?;
        pos += 1;
        if tag == TAG_END {
            return Some((operation, attrs, bytes.get(pos..)?.to_vec()));
        }
        if tag < 0x10 {
            group = tag;
            continue;
        }
        let name_len = usize::from(u16::from_be_bytes(bytes.get(pos..pos + 2)?.try_into().ok()?));
        pos += 2;
        let name = bytes.get(pos..pos + name_len)?;
        pos += name_len;
        let value_len = usize::from(u16::from_be_bytes(bytes.get(pos..pos + 2)?.try_into().ok()?));
        pos += 2;
        let value = bytes.get(pos..pos + value_len)?.to_vec();
        pos += value_len;
        if name.is_empty() {
            attrs.last_mut()?.values.push(value);
        } else {
            attrs.push(Attribute { group, tag, name: String::from_utf8_lossy(name).into_owned(), values: vec![value] });
        }
    }
}

#[test]
fn fake_reply_preserves_shared_ipp_version_and_request_id() {
    let reply = Request::new(0, 0x12345678).end();
    assert_eq!(&reply[..8], &[1, 1, 0, 0, 0x12, 0x34, 0x56, 0x78]);
}
