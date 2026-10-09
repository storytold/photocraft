use std::collections::HashMap;

use crate::error::Error;

/// A parsed value from the NRBF serialization stream.
#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Boolean(bool),
    Byte(u8),
    Char(char),
    Int16(i16),
    Int32(i32),
    Int64(i64),
    SByte(i8),
    Single(f32),
    Double(f64),
    UInt16(u16),
    UInt32(u32),
    UInt64(u64),
    String(String),
    Reference(i32),
    ClassInstance(ClassInstance),
    Array(Vec<Value>),
    PrimitiveArray(Vec<Value>),
}

impl Value {
    pub fn as_i32(&self) -> Option<i32> {
        match self {
            Self::Int32(v) => Some(*v),
            Self::Byte(v) => Some(*v as i32),
            Self::Int16(v) => Some(*v as i32),
            Self::Int64(v) => i32::try_from(*v).ok(),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int64(v) => Some(*v),
            Self::Int32(v) => Some(*v as i64),
            Self::Byte(v) => Some(*v as i64),
            Self::Int16(v) => Some(*v as i64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_class(&self) -> Option<&ClassInstance> {
        match self {
            Self::ClassInstance(c) => Some(c),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(a) | Self::PrimitiveArray(a) => Some(a.as_slice()),
            _ => None,
        }
    }
}

/// An instance of a .NET class with named fields.
#[derive(Debug, Clone)]
pub struct ClassInstance {
    pub object_id: i32,
    pub class_name: String,
    pub members: Vec<(String, Value)>,
}

impl ClassInstance {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.members.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    pub fn find(&self, suffix: &str) -> Option<&Value> {
        if let Some(v) = self.get(suffix) {
            return Some(v);
        }
        let norm_suffix = suffix.replace('+', "_");
        self.members
            .iter()
            .find(|(k, _)| {
                let norm_k = k.replace('+', "_");
                norm_k.eq_ignore_ascii_case(&norm_suffix) || norm_k.ends_with(&norm_suffix)
            })
            .map(|(_, v)| v)
    }
}

#[derive(Debug, Clone)]
pub struct ClassDefinition {
    pub class_name: String,
    pub member_names: Vec<String>,
    pub member_types: Option<Vec<(u8, Option<AdditionalInfo>)>>,
}

#[derive(Debug, Clone)]
pub enum AdditionalInfo {
    Primitive(u8),
    Class(String, i32),
    SystemClass(String),
    PrimitiveArray(u8),
}

/// Parsing context for the NRBF stream.
pub struct NrbfContext {
    pub root_id: i32,
    pub header_id: i32,
    pub objects: HashMap<i32, Value>,
    pub classes: HashMap<i32, ClassDefinition>,
    pub libraries: HashMap<i32, String>,
}

impl NrbfContext {
    pub fn new() -> Self {
        Self {
            root_id: 0,
            header_id: 0,
            objects: HashMap::new(),
            classes: HashMap::new(),
            libraries: HashMap::new(),
        }
    }
}

impl Default for NrbfContext {
    fn default() -> Self {
        Self::new()
    }
}

impl NrbfContext {

    /// Recursively dereferences a `Value::Reference` up to a bounded depth.
    pub fn resolve<'a>(&'a self, val: &'a Value) -> Option<&'a Value> {
        let mut curr = val;
        let mut depth = 0usize;
        while let Value::Reference(id) = curr {
            if depth > 32 {
                return None;
            }
            depth += 1;
            curr = self.objects.get(id)?;
        }
        Some(curr)
    }

    pub fn get_object(&self, id: i32) -> Option<&Value> {
        self.resolve(self.objects.get(&id)?)
    }
}

enum RecordResult {
    Value(Value),
    BinaryLibrary,
    NullMultiple(usize),
    MessageEnd,
}

pub fn parse_nrbf(mut input: &[u8], max_objects: usize) -> Result<(NrbfContext, &[u8]), Error> {
    let mut ctx = NrbfContext::new();

    // The first record must be SerializedStreamHeader (record 0)
    let first_rec_type = read_u8(&mut input)?;
    if first_rec_type != 0 {
        return Err(Error::Malformed("NRBF stream does not start with SerializedStreamHeader"));
    }
    ctx.root_id = read_i32(&mut input)?;
    ctx.header_id = read_i32(&mut input)?;
    let major = read_i32(&mut input)?;
    let minor = read_i32(&mut input)?;
    if major != 1 || minor != 0 {
        return Err(Error::Malformed("unsupported NRBF major/minor version"));
    }
    if ctx.root_id == 0 {
        return Err(Error::Unsupported("NRBF BinaryMethodCall is unsupported".into()));
    }

    // Read records until MessageEnd (11)
    loop {
        if ctx.objects.len() > max_objects {
            return Err(Error::Limit("exceeded maximum NRBF object count"));
        }
        let rec = read_record(&mut input, &mut ctx, 0)?;
        if matches!(rec, RecordResult::MessageEnd) {
            break;
        }
    }

    Ok((ctx, input))
}

fn read_record(input: &mut &[u8], ctx: &mut NrbfContext, depth: usize) -> Result<RecordResult, Error> {
    if depth > 64 {
        return Err(Error::Limit("exceeded maximum NRBF recursion depth"));
    }

    let rec_type = read_u8(input)?;
    match rec_type {
        0 => Err(Error::Malformed("unexpected SerializedStreamHeader in mid-stream")),
        1 => {
            // ClassWithId
            let object_id = read_i32(input)?;
            let metadata_id = read_i32(input)?;
            let def = ctx
                .classes
                .get(&metadata_id)
                .cloned()
                .ok_or(Error::Malformed("missing metadata class for ClassWithId"))?;
            let instance = read_class_members(input, object_id, &def, ctx, depth)?;
            let val = Value::ClassInstance(instance);
            ctx.objects.insert(object_id, val);
            Ok(RecordResult::Value(Value::Reference(object_id)))
        }
        2 => {
            // SystemClassWithMembers
            let object_id = read_i32(input)?;
            let class_name = read_identifier(input)?;
            let member_count = read_i32(input)?;
            let member_count = validate_count(member_count, 10_000)?;
            let mut member_names = Vec::with_capacity(member_count);
            for _ in 0..member_count {
                member_names.push(read_identifier(input)?);
            }
            let def = ClassDefinition {
                class_name,
                member_names,
                member_types: None,
            };
            ctx.classes.insert(object_id, def.clone());
            let instance = read_class_members(input, object_id, &def, ctx, depth)?;
            let val = Value::ClassInstance(instance);
            ctx.objects.insert(object_id, val);
            Ok(RecordResult::Value(Value::Reference(object_id)))
        }
        3 => {
            // ClassWithMembers
            let object_id = read_i32(input)?;
            let class_name = read_identifier(input)?;
            let member_count = read_i32(input)?;
            let member_count = validate_count(member_count, 10_000)?;
            let mut member_names = Vec::with_capacity(member_count);
            for _ in 0..member_count {
                member_names.push(read_identifier(input)?);
            }
            let _library_id = read_i32(input)?;
            let def = ClassDefinition {
                class_name,
                member_names,
                member_types: None,
            };
            ctx.classes.insert(object_id, def.clone());
            let instance = read_class_members(input, object_id, &def, ctx, depth)?;
            let val = Value::ClassInstance(instance);
            ctx.objects.insert(object_id, val);
            Ok(RecordResult::Value(Value::Reference(object_id)))
        }
        4 => {
            // SystemClassWithMembersAndTypes
            let object_id = read_i32(input)?;
            let class_name = read_identifier(input)?;
            let member_count = read_i32(input)?;
            let member_count = validate_count(member_count, 10_000)?;
            let mut member_names = Vec::with_capacity(member_count);
            for _ in 0..member_count {
                member_names.push(read_identifier(input)?);
            }
            let member_types = read_member_type_info(input, member_count)?;
            let def = ClassDefinition {
                class_name,
                member_names,
                member_types: Some(member_types),
            };
            ctx.classes.insert(object_id, def.clone());
            let instance = read_class_members(input, object_id, &def, ctx, depth)?;
            let val = Value::ClassInstance(instance);
            ctx.objects.insert(object_id, val);
            Ok(RecordResult::Value(Value::Reference(object_id)))
        }
        5 => {
            // ClassWithMembersAndTypes
            let object_id = read_i32(input)?;
            let class_name = read_identifier(input)?;
            let member_count = read_i32(input)?;
            let member_count = validate_count(member_count, 10_000)?;
            let mut member_names = Vec::with_capacity(member_count);
            for _ in 0..member_count {
                member_names.push(read_identifier(input)?);
            }
            let member_types = read_member_type_info(input, member_count)?;
            let _library_id = read_i32(input)?;
            let def = ClassDefinition {
                class_name,
                member_names,
                member_types: Some(member_types),
            };
            ctx.classes.insert(object_id, def.clone());
            let instance = read_class_members(input, object_id, &def, ctx, depth)?;
            let val = Value::ClassInstance(instance);
            ctx.objects.insert(object_id, val);
            Ok(RecordResult::Value(Value::Reference(object_id)))
        }
        6 => {
            // BinaryObjectString
            let object_id = read_i32(input)?;
            let str_val = read_length_prefixed_string(input)?;
            ctx.objects.insert(object_id, Value::String(str_val));
            Ok(RecordResult::Value(Value::Reference(object_id)))
        }
        7 => {
            // BinaryArray
            let object_id = read_i32(input)?;
            let array_type = read_u8(input)?;
            let rank = read_i32(input)?;
            let rank = validate_count(rank, 8)?;
            let mut lengths = Vec::with_capacity(rank);
            let mut total_len: usize = 1;
            for _ in 0..rank {
                let l = read_i32(input)?;
                let l = validate_count(l, 100_000)?;
                total_len = total_len
                    .checked_mul(l)
                    .ok_or(Error::Malformed("array length overflow"))?;
                lengths.push(l);
            }
            if (3..=5).contains(&array_type) {
                for _ in 0..rank {
                    let _ = read_i32(input)?; // lower bound
                }
            }
            let binary_type = read_u8(input)?;
            let add_info = read_additional_info(input, binary_type)?;

            if binary_type == 0 /* Primitive */ {
                let pt = match add_info {
                    Some(AdditionalInfo::Primitive(p)) => p,
                    _ => return Err(Error::Malformed("missing primitive type for binary array")),
                };
                let mut items = Vec::with_capacity(total_len);
                for _ in 0..total_len {
                    items.push(read_primitive(input, pt)?);
                }
                let val = Value::PrimitiveArray(items);
                ctx.objects.insert(object_id, val);
            } else {
                let items = read_object_array_elements(input, total_len, ctx, depth)?;
                let val = Value::Array(items);
                ctx.objects.insert(object_id, val);
            }
            Ok(RecordResult::Value(Value::Reference(object_id)))
        }
        8 => {
            // MemberPrimitiveTyped
            let primitive_type = read_u8(input)?;
            let val = read_primitive(input, primitive_type)?;
            Ok(RecordResult::Value(val))
        }
        9 => {
            // MemberReference
            let id_ref = read_i32(input)?;
            Ok(RecordResult::Value(Value::Reference(id_ref)))
        }
        10 => Ok(RecordResult::Value(Value::Null)),
        11 => Ok(RecordResult::MessageEnd),
        12 => {
            // BinaryLibrary
            let library_id = read_i32(input)?;
            let library_name = read_length_prefixed_string(input)?;
            ctx.libraries.insert(library_id, library_name);
            Ok(RecordResult::BinaryLibrary)
        }
        13 => {
            // ObjectNullMultiple256
            let count = read_u8(input)? as usize;
            Ok(RecordResult::NullMultiple(count))
        }
        14 => {
            // ObjectNullMultiple
            let count = read_i32(input)?;
            let count = validate_count(count, 100_000)?;
            Ok(RecordResult::NullMultiple(count))
        }
        15 => {
            // ArraySinglePrimitive
            let object_id = read_i32(input)?;
            let length = read_i32(input)?;
            let length = validate_count(length, 100_000)?;
            let primitive_type = read_u8(input)?;
            let mut items = Vec::with_capacity(length);
            for _ in 0..length {
                items.push(read_primitive(input, primitive_type)?);
            }
            ctx.objects.insert(object_id, Value::PrimitiveArray(items));
            Ok(RecordResult::Value(Value::Reference(object_id)))
        }
        16 => {
            // ArraySingleObject
            let object_id = read_i32(input)?;
            let length = read_i32(input)?;
            let length = validate_count(length, 100_000)?;
            let items = read_object_array_elements(input, length, ctx, depth)?;
            ctx.objects.insert(object_id, Value::Array(items));
            Ok(RecordResult::Value(Value::Reference(object_id)))
        }
        17 => {
            // ArraySingleString
            let object_id = read_i32(input)?;
            let length = read_i32(input)?;
            let length = validate_count(length, 100_000)?;
            let items = read_object_array_elements(input, length, ctx, depth)?;
            ctx.objects.insert(object_id, Value::Array(items));
            Ok(RecordResult::Value(Value::Reference(object_id)))
        }
        other => Err(Error::Unsupported(format!("unsupported NRBF record type {other}"))),
    }
}

fn read_class_members(
    input: &mut &[u8],
    object_id: i32,
    def: &ClassDefinition,
    ctx: &mut NrbfContext,
    depth: usize,
) -> Result<ClassInstance, Error> {
    let member_count = def.member_names.len();
    let mut members = Vec::with_capacity(member_count);
    let mut idx = 0usize;

    while idx < member_count {
        if let Some(types) = &def.member_types
            && let Some((btype, add_info)) = types.get(idx)
            && *btype == 0 /* Primitive */
        {
            let pt = match add_info {
                Some(AdditionalInfo::Primitive(p)) => *p,
                _ => return Err(Error::Malformed("missing primitive type info for member")),
            };
            let val = read_primitive(input, pt)?;
            let name = def.member_names.get(idx).cloned().unwrap_or_default();
            members.push((name, val));
            idx += 1;
            continue;
        }

        // Non-primitive member
        let rec = read_record(input, ctx, depth + 1)?;
        match rec {
            RecordResult::BinaryLibrary => continue,
            RecordResult::NullMultiple(count) => {
                for _ in 0..count {
                    if idx < member_count {
                        let name = def.member_names.get(idx).cloned().unwrap_or_default();
                        members.push((name, Value::Null));
                        idx += 1;
                    }
                }
            }
            RecordResult::Value(val) => {
                let name = def.member_names.get(idx).cloned().unwrap_or_default();
                members.push((name, val));
                idx += 1;
            }
            RecordResult::MessageEnd => {
                return Err(Error::Malformed("unexpected MessageEnd in class member stream"));
            }
        }
    }

    Ok(ClassInstance {
        object_id,
        class_name: def.class_name.clone(),
        members,
    })
}

fn read_object_array_elements(
    input: &mut &[u8],
    length: usize,
    ctx: &mut NrbfContext,
    depth: usize,
) -> Result<Vec<Value>, Error> {
    let mut items = Vec::with_capacity(length);
    while items.len() < length {
        let rec = read_record(input, ctx, depth + 1)?;
        match rec {
            RecordResult::BinaryLibrary => continue,
            RecordResult::NullMultiple(count) => {
                for _ in 0..count {
                    if items.len() < length {
                        items.push(Value::Null);
                    }
                }
            }
            RecordResult::Value(val) => {
                items.push(val);
            }
            RecordResult::MessageEnd => {
                return Err(Error::Malformed("unexpected MessageEnd in object array stream"));
            }
        }
    }
    Ok(items)
}

fn read_member_type_info(input: &mut &[u8], count: usize) -> Result<Vec<(u8, Option<AdditionalInfo>)>, Error> {
    let mut btypes = Vec::with_capacity(count);
    for _ in 0..count {
        btypes.push(read_u8(input)?);
    }
    let mut result = Vec::with_capacity(count);
    for bt in btypes {
        let add = read_additional_info(input, bt)?;
        result.push((bt, add));
    }
    Ok(result)
}

fn read_additional_info(input: &mut &[u8], binary_type: u8) -> Result<Option<AdditionalInfo>, Error> {
    match binary_type {
        0 => Ok(Some(AdditionalInfo::Primitive(read_u8(input)?))),
        7 => Ok(Some(AdditionalInfo::PrimitiveArray(read_u8(input)?))),
        3 => {
            let name = read_identifier(input)?;
            Ok(Some(AdditionalInfo::SystemClass(name)))
        }
        4 => {
            let name = read_identifier(input)?;
            let lib_id = read_i32(input)?;
            Ok(Some(AdditionalInfo::Class(name, lib_id)))
        }
        _ => Ok(None),
    }
}

pub fn read_primitive(input: &mut &[u8], primitive_type: u8) -> Result<Value, Error> {
    match primitive_type {
        1 => Ok(Value::Boolean(read_u8(input)? != 0)),
        2 => Ok(Value::Byte(read_u8(input)?)),
        3 => {
            let b = read_u8(input)?;
            Ok(Value::Char(b as char))
        }
        5 => {
            let s = read_length_prefixed_string(input)?;
            Ok(Value::String(s))
        }
        6 => Ok(Value::Double(f64::from_le_bytes(read_fixed(input)?))),
        7 => Ok(Value::Int16(i16::from_le_bytes(read_fixed(input)?))),
        8 => Ok(Value::Int32(read_i32(input)?)),
        9 => Ok(Value::Int64(read_i64(input)?)),
        10 => Ok(Value::SByte(read_u8(input)? as i8)),
        11 => Ok(Value::Single(f32::from_le_bytes(read_fixed(input)?))),
        12 => Ok(Value::Int64(read_i64(input)?)), // TimeSpan as 64-bit ticks
        13 => Ok(Value::UInt64(read_u64(input)?)), // DateTime as 64-bit int
        14 => Ok(Value::UInt16(u16::from_le_bytes(read_fixed(input)?))),
        15 => Ok(Value::UInt32(read_u32(input)?)),
        16 => Ok(Value::UInt64(read_u64(input)?)),
        17 => Ok(Value::Null),
        18 => Ok(Value::String(read_length_prefixed_string(input)?)),
        other => Err(Error::Unsupported(format!("unsupported primitive type {other}"))),
    }
}

fn validate_count(count: i32, max: usize) -> Result<usize, Error> {
    if count < 0 {
        return Err(Error::Malformed("negative count in NRBF record"));
    }
    let c = count as usize;
    if c > max {
        return Err(Error::Limit("count in NRBF record exceeds safety limit"));
    }
    Ok(c)
}

fn read_u8(input: &mut &[u8]) -> Result<u8, Error> {
    let (&first, rest) = input.split_first().ok_or(Error::Malformed("unexpected EOF in NRBF stream"))?;
    *input = rest;
    Ok(first)
}

fn read_slice<'a>(input: &mut &'a [u8], len: usize) -> Result<&'a [u8], Error> {
    if input.len() < len {
        return Err(Error::Malformed("unexpected EOF reading slice in NRBF"));
    }
    let (head, tail) = input.split_at(len);
    *input = tail;
    Ok(head)
}

fn read_fixed<const N: usize>(input: &mut &[u8]) -> Result<[u8; N], Error> {
    let slice = read_slice(input, N)?;
    let mut arr = [0u8; N];
    arr.copy_from_slice(slice);
    Ok(arr)
}

fn read_i32(input: &mut &[u8]) -> Result<i32, Error> {
    Ok(i32::from_le_bytes(read_fixed(input)?))
}

fn read_i64(input: &mut &[u8]) -> Result<i64, Error> {
    Ok(i64::from_le_bytes(read_fixed(input)?))
}

fn read_u32(input: &mut &[u8]) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(read_fixed(input)?))
}

fn read_u64(input: &mut &[u8]) -> Result<u64, Error> {
    Ok(u64::from_le_bytes(read_fixed(input)?))
}

fn read_7bit_int(input: &mut &[u8]) -> Result<usize, Error> {
    let mut count: usize = 0;
    let mut shift: u32 = 0;
    while shift < 35 {
        let b = read_u8(input)?;
        count |= ((b & 0x7f) as usize) << shift;
        if (b & 0x80) == 0 {
            return Ok(count);
        }
        shift += 7;
    }
    Err(Error::Malformed("invalid 7-bit encoded integer"))
}

pub fn read_length_prefixed_string(input: &mut &[u8]) -> Result<String, Error> {
    let len = read_7bit_int(input)?;
    let slice = read_slice(input, len)?;
    Ok(String::from_utf8_lossy(slice).into_owned())
}

pub fn sanitize_identifier(s: String) -> String {
    s.replace('+', "_")
}

pub fn read_identifier(input: &mut &[u8]) -> Result<String, Error> {
    let s = read_length_prefixed_string(input)?;
    Ok(sanitize_identifier(s))
}
