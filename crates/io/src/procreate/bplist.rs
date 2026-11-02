//! Binary property lists (`bplist00`), the format of Procreate's `Brush.archive`: an 8-byte
//! header, the objects, a table of object offsets and a 32-byte trailer (offset and reference
//! widths, object count, top object, table position), as Apple documents the format.
//!
//! Objects are decoded one at a time by index and containers hold their members' indices, so
//! an object referenced from many places is never expanded: a small hostile file can't grow
//! into a large tree, and a cycle costs nothing until something follows it.

/// One decoded object.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Obj<'a> {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Date(f64),
    Data(&'a [u8]),
    Str(String),
    Uid(u64),
    /// Arrays and sets: the members' object indices.
    Array(Vec<usize>),
    /// Key and value object indices.
    Dict(Vec<(usize, usize)>),
}

/// A parsed trailer over the file's bytes.
pub(crate) struct Plist<'a> {
    bytes: &'a [u8],
    table: usize,
    offset_size: usize,
    ref_size: usize,
    count: usize,
    top: usize,
}

/// Big-endian unsigned integer of up to 8 bytes.
fn be(b: &[u8]) -> u64 {
    b.iter().fold(0u64, |v, &x| (v << 8) | u64::from(x))
}

impl<'a> Plist<'a> {
    pub(crate) fn parse(bytes: &'a [u8]) -> Result<Self, String> {
        if !bytes.starts_with(b"bplist0") {
            return Err("not a binary property list".into());
        }
        let field = |at: usize, len: usize| bytes.len().checked_sub(32).and_then(|t| bytes.get(t + at..t + at + len)).ok_or("truncated property list");
        let offset_size = usize::from(field(6, 1)?.first().copied().unwrap_or(0));
        let ref_size = usize::from(field(7, 1)?.first().copied().unwrap_or(0));
        if !(1..=8).contains(&offset_size) || !(1..=8).contains(&ref_size) {
            return Err("property list has invalid integer widths".into());
        }
        let as_index = |v: u64| usize::try_from(v).map_err(|_| "property list is too large");
        let count = as_index(be(field(8, 8)?))?;
        let top = as_index(be(field(16, 8)?))?;
        let table = as_index(be(field(24, 8)?))?;
        // Every object takes at least a byte between the header and the offset table.
        let table_end = count.checked_mul(offset_size).and_then(|n| n.checked_add(table)).ok_or("property list is too large")?;
        if count == 0 || top >= count || table < 8 || count > table - 8 || table_end > bytes.len() - 32 {
            return Err("property list's offset table is out of range".into());
        }
        Ok(Plist { bytes, table, offset_size, ref_size, count, top })
    }

    /// The top object's index.
    pub(crate) fn top(&self) -> usize {
        self.top
    }

    fn slice(&self, at: usize, len: usize) -> Result<&'a [u8], String> {
        at.checked_add(len).and_then(|end| self.bytes.get(at..end)).ok_or_else(|| "property list object runs past its end".to_string())
    }

    /// A length from an object's marker: the low nibble, or the integer object after it.
    fn length(&self, low: u8, at: usize) -> Result<(usize, usize), String> {
        if low < 0x0F {
            return Ok((usize::from(low), at));
        }
        let marker = *self.slice(at, 1)?.first().unwrap_or(&0);
        if marker >> 4 != 1 || marker & 0x0F > 3 {
            return Err("property list has an invalid length".into());
        }
        let n = 1usize << (marker & 0x0F);
        let len = usize::try_from(be(self.slice(at + 1, n)?)).map_err(|_| "property list length is too large")?;
        Ok((len, at + 1 + n))
    }

    fn refs(&self, at: usize, n: usize) -> Result<Vec<usize>, String> {
        let raw = self.slice(at, n.checked_mul(self.ref_size).ok_or("property list container is too large")?)?;
        Ok(raw.chunks_exact(self.ref_size).map(|r| usize::try_from(be(r)).unwrap_or(usize::MAX)).collect())
    }

    /// Object `index`.
    pub(crate) fn object(&self, index: usize) -> Result<Obj<'a>, String> {
        if index >= self.count {
            return Err("property list refers to a missing object".into());
        }
        let entry = self.slice(self.table + index * self.offset_size, self.offset_size)?;
        let at = usize::try_from(be(entry)).map_err(|_| "property list offset is too large")?;
        if !(8..self.table).contains(&at) {
            return Err("property list object is out of range".into());
        }
        let marker = *self.slice(at, 1)?.first().unwrap_or(&0);
        let (kind, low) = (marker >> 4, marker & 0x0F);
        let at = at + 1;
        Ok(match kind {
            0x0 => match low {
                0x0 => Obj::Null,
                0x8 => Obj::Bool(false),
                0x9 => Obj::Bool(true),
                _ => return Err("property list has an unknown object".into()),
            },
            // 1, 2 and 4-byte integers are unsigned, 8-byte ones signed.
            0x1 if low <= 3 => {
                let n = 1usize << low;
                let v = be(self.slice(at, n)?);
                Obj::Int(if n == 8 { v as i64 } else { v as i64 & ((1i64 << (n * 8)) - 1) })
            }
            0x2 if low == 2 => Obj::Real(f64::from(f32::from_bits(be(self.slice(at, 4)?) as u32))),
            0x2 if low == 3 => Obj::Real(f64::from_bits(be(self.slice(at, 8)?))),
            0x3 if low == 3 => Obj::Date(f64::from_bits(be(self.slice(at, 8)?))),
            0x4 => {
                let (n, at) = self.length(low, at)?;
                Obj::Data(self.slice(at, n)?)
            }
            0x5 => {
                let (n, at) = self.length(low, at)?;
                Obj::Str(String::from_utf8_lossy(self.slice(at, n)?).into_owned())
            }
            0x6 => {
                let (n, at) = self.length(low, at)?;
                let raw = self.slice(at, n.checked_mul(2).ok_or("property list string is too large")?)?;
                let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
                Obj::Str(String::from_utf16_lossy(&units))
            }
            0x8 => Obj::Uid(be(self.slice(at, usize::from(low) + 1)?)),
            0xA | 0xC => {
                let (n, at) = self.length(low, at)?;
                Obj::Array(self.refs(at, n)?)
            }
            0xD => {
                let (n, at) = self.length(low, at)?;
                let keys = self.refs(at, n)?;
                let values = self.refs(at + n * self.ref_size, n)?;
                Obj::Dict(keys.into_iter().zip(values).collect())
            }
            _ => return Err("property list has an unknown object".into()),
        })
    }

    /// The value object of `key` in a dictionary's entries.
    pub(crate) fn lookup(&self, entries: &[(usize, usize)], key: &str) -> Option<usize> {
        entries.iter().find(|(k, _)| matches!(self.object(*k), Ok(Obj::Str(s)) if s == key)).map(|(_, v)| *v)
    }
}

/// A value for [`write`]: what Procreate's archives hold.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Real(f64),
    Str(String),
    Uid(u64),
    Array(Vec<Value>),
    Dict(Vec<(String, Value)>),
}

/// Write `top` as a binary property list (every value its own object). Exposed for tests.
pub fn write(top: &Value) -> Vec<u8> {
    fn add(objects: &mut Vec<Vec<u8>>, v: &Value) -> usize {
        let index = objects.len();
        objects.push(Vec::new());
        let mut out = Vec::new();
        let head = |out: &mut Vec<u8>, kind: u8, n: usize| {
            if n < 15 {
                out.push(kind << 4 | n as u8);
            } else {
                out.push(kind << 4 | 0x0F);
                out.push(0x13);
                out.extend_from_slice(&(n as u64).to_be_bytes());
            }
        };
        match v {
            Value::Bool(b) => out.push(if *b { 0x09 } else { 0x08 }),
            Value::Int(i) => {
                out.push(0x13);
                out.extend_from_slice(&i.to_be_bytes());
            }
            Value::Real(r) => {
                out.push(0x23);
                out.extend_from_slice(&r.to_bits().to_be_bytes());
            }
            Value::Str(s) if s.is_ascii() => {
                head(&mut out, 0x5, s.len());
                out.extend_from_slice(s.as_bytes());
            }
            Value::Str(s) => {
                let units: Vec<u16> = s.encode_utf16().collect();
                head(&mut out, 0x6, units.len());
                out.extend(units.iter().flat_map(|u| u.to_be_bytes()));
            }
            Value::Uid(u) => {
                out.push(0x87);
                out.extend_from_slice(&u.to_be_bytes());
            }
            Value::Array(items) => {
                let refs: Vec<usize> = items.iter().map(|x| add(objects, x)).collect();
                head(&mut out, 0xA, refs.len());
                out.extend(refs.iter().flat_map(|r| (*r as u32).to_be_bytes()));
            }
            Value::Dict(entries) => {
                let keys: Vec<usize> = entries.iter().map(|(k, _)| add(objects, &Value::Str(k.clone()))).collect();
                let values: Vec<usize> = entries.iter().map(|(_, x)| add(objects, x)).collect();
                head(&mut out, 0xD, keys.len());
                out.extend(keys.iter().chain(&values).flat_map(|r| (*r as u32).to_be_bytes()));
            }
        }
        if let Some(slot) = objects.get_mut(index) {
            *slot = out;
        }
        index
    }
    let mut objects = Vec::new();
    add(&mut objects, top);
    let mut out = b"bplist00".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for o in &objects {
        offsets.push(out.len() as u64);
        out.extend_from_slice(o);
    }
    let table = out.len() as u64;
    out.extend(offsets.iter().flat_map(|o| o.to_be_bytes()));
    out.extend_from_slice(&[0; 6]);
    out.extend_from_slice(&[8, 4]);
    out.extend_from_slice(&(objects.len() as u64).to_be_bytes());
    out.extend_from_slice(&0u64.to_be_bytes());
    out.extend_from_slice(&table.to_be_bytes());
    out
}
