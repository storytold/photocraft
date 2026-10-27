//! Reads PE headers and named exports as data; never loads the library.
use std::{io, path::Path};
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Architecture {
    X86,
    X64,
}
#[derive(Clone, Debug)]
pub struct Metadata {
    pub architecture: Architecture,
    pub entry: String,
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn bytes_at(bytes: &[u8], offset: usize, len: usize) -> io::Result<&[u8]> {
    bytes.get(offset..offset.checked_add(len).ok_or_else(|| invalid("PE offset overflow"))?).ok_or_else(|| invalid("truncated PE file"))
}
fn u16_at(bytes: &[u8], offset: usize) -> io::Result<u16> {
    Ok(u16::from_le_bytes(bytes_at(bytes, offset, 2)?.try_into().map_err(|_| invalid("invalid PE integer"))?))
}
fn u32_at(bytes: &[u8], offset: usize) -> io::Result<u32> {
    Ok(u32::from_le_bytes(bytes_at(bytes, offset, 4)?.try_into().map_err(|_| invalid("invalid PE integer"))?))
}
fn rva_offset(bytes: &[u8], table: usize, sections: usize, rva: u32) -> io::Result<usize> {
    for i in 0..sections {
        let section = table.checked_add(i.checked_mul(40).ok_or_else(|| invalid("PE section overflow"))?).ok_or_else(|| invalid("PE section overflow"))?;
        let va = u32_at(bytes, section + 12)?;
        let raw_size = u32_at(bytes, section + 16)?;
        if let Some(delta) = rva.checked_sub(va).filter(|&n| n < raw_size) {
            let raw = u32_at(bytes, section + 20)?;
            let offset = raw.checked_add(delta).ok_or_else(|| invalid("PE RVA overflow"))? as usize;
            bytes_at(bytes, offset, 1)?;
            return Ok(offset);
        }
    }
    Err(invalid("PE RVA is outside mapped sections"))
}

fn rva_range_offset(bytes: &[u8], table: usize, sections: usize, rva: u32, len: usize) -> io::Result<usize> {
    if len == 0 {
        return Err(invalid("empty PE RVA range"));
    }
    for i in 0..sections {
        let section = table.checked_add(i.checked_mul(40).ok_or_else(|| invalid("PE section overflow"))?).ok_or_else(|| invalid("PE section overflow"))?;
        let va = u32_at(bytes, section + 12)?;
        let raw_size = u32_at(bytes, section + 16)? as usize;
        if let Some(delta) = rva.checked_sub(va).map(|n| n as usize).filter(|&n| n.checked_add(len).is_some_and(|end| end <= raw_size)) {
            let raw = u32_at(bytes, section + 20)? as usize;
            let offset = raw.checked_add(delta).ok_or_else(|| invalid("PE RVA overflow"))?;
            bytes_at(bytes, offset, len)?;
            return Ok(offset);
        }
    }
    Err(invalid("PE RVA range is outside mapped sections"))
}

fn resource_bytes(bytes: &[u8], table: usize, sections: usize, resource_rva: u32, resource_size: usize, relative: usize, len: usize) -> io::Result<&[u8]> {
    let end = relative.checked_add(len).ok_or_else(|| invalid("PE resource offset overflow"))?;
    if end > resource_size {
        return Err(invalid("PE resource is outside resource directory"));
    }
    let rva = resource_rva
        .checked_add(u32::try_from(relative).map_err(|_| invalid("PE resource RVA overflow"))?)
        .ok_or_else(|| invalid("PE resource RVA overflow"))?;
    let offset = rva_range_offset(bytes, table, sections, rva, len)?;
    bytes_at(bytes, offset, len)
}

fn resource_directory_entries(
    bytes: &[u8],
    table: usize,
    sections: usize,
    resource_rva: u32,
    resource_size: usize,
    relative: usize,
) -> io::Result<Vec<(u32, u32)>> {
    let header = resource_bytes(bytes, table, sections, resource_rva, resource_size, relative, 16)?;
    let count = usize::from(u16::from_le_bytes([header[12], header[13]]))
        .checked_add(usize::from(u16::from_le_bytes([header[14], header[15]])))
        .ok_or_else(|| invalid("PE resource entry count overflow"))?;
    if count > 4096 {
        return Err(invalid("too many PE resource directory entries"));
    }
    let start = relative.checked_add(16).ok_or_else(|| invalid("PE resource offset overflow"))?;
    let entries = resource_bytes(
        bytes,
        table,
        sections,
        resource_rva,
        resource_size,
        start,
        count.checked_mul(8).ok_or_else(|| invalid("PE resource entry count overflow"))?,
    )?;
    let mut result = Vec::with_capacity(count);
    for entry in entries.as_chunks::<8>().0 {
        result.push((
            u32::from_le_bytes(entry[0..4].try_into().map_err(|_| invalid("invalid PE resource entry"))?),
            u32::from_le_bytes(entry[4..8].try_into().map_err(|_| invalid("invalid PE resource entry"))?),
        ));
    }
    Ok(result)
}

fn resource_name_is_pipl(bytes: &[u8], table: usize, sections: usize, resource_rva: u32, resource_size: usize, name: u32) -> io::Result<bool> {
    if name & 0x8000_0000 == 0 {
        return Ok(false);
    }
    let relative = (name & 0x7fff_ffff) as usize;
    let name_header = resource_bytes(bytes, table, sections, resource_rva, resource_size, relative, 2)?;
    let length = usize::from(u16::from_le_bytes([name_header[0], name_header[1]]));
    if length > 256 {
        return Err(invalid("PE resource name is too long"));
    }
    let string_start = relative.checked_add(2).ok_or_else(|| invalid("PE resource name overflow"))?;
    let raw = resource_bytes(
        bytes,
        table,
        sections,
        resource_rva,
        resource_size,
        string_start,
        length.checked_mul(2).ok_or_else(|| invalid("PE resource name overflow"))?,
    )?;
    let mut units = Vec::with_capacity(length);
    for pair in raw.as_chunks::<2>().0 {
        units.push(u16::from_le_bytes([pair[0], pair[1]]));
    }
    let decoded = String::from_utf16(&units).map_err(|_| invalid("invalid UTF-16 PE resource name"))?;
    Ok(decoded.eq_ignore_ascii_case("PiPL"))
}

#[derive(Clone, Copy)]
struct ResourceLayout<'a> {
    bytes: &'a [u8],
    table: usize,
    sections: usize,
    resource_rva: u32,
    resource_size: usize,
}

fn collect_resource_payloads<'a>(
    layout: &ResourceLayout<'a>,
    relative: usize,
    depth: usize,
    payloads: &mut Vec<&'a [u8]>,
    budget: &mut usize,
) -> io::Result<()> {
    let ResourceLayout { bytes, table, sections, resource_rva, resource_size } = *layout;
    if depth > 3 {
        return Err(invalid("PE resource directory is nested too deeply"));
    }
    for (_, target) in resource_directory_entries(bytes, table, sections, resource_rva, resource_size, relative)? {
        *budget = budget.checked_sub(1).ok_or_else(|| invalid("too many PE resource entries"))?;
        let target_offset = (target & 0x7fff_ffff) as usize;
        if target & 0x8000_0000 != 0 {
            collect_resource_payloads(layout, target_offset, depth + 1, payloads, budget)?;
        } else {
            let data_entry = resource_bytes(bytes, table, sections, resource_rva, resource_size, target_offset, 16)?;
            let data_rva = u32::from_le_bytes(data_entry[0..4].try_into().map_err(|_| invalid("invalid PE resource data entry"))?);
            let size = u32::from_le_bytes(data_entry[4..8].try_into().map_err(|_| invalid("invalid PE resource data entry"))?) as usize;
            if size > 1024 * 1024 {
                return Err(invalid("PiPL resource exceeds 1 MiB limit"));
            }
            let data_offset = rva_range_offset(bytes, table, sections, data_rva, size.max(1))?;
            payloads.push(bytes_at(bytes, data_offset, size)?);
        }
    }
    Ok(())
}

fn pipl_payloads(bytes: &[u8], table: usize, sections: usize, opt: usize, directory: usize, optional_len: usize) -> io::Result<Option<Vec<&[u8]>>> {
    if optional_len < directory + 24 {
        return Ok(None);
    }
    let resource_rva = u32_at(bytes, opt + directory + 16)?;
    let resource_size = u32_at(bytes, opt + directory + 20)? as usize;
    if resource_rva == 0 && resource_size == 0 {
        return Ok(None);
    }
    if resource_rva == 0 || resource_size < 16 {
        return Err(invalid("invalid PE resource directory"));
    }

    let layout = ResourceLayout { bytes, table, sections, resource_rva, resource_size };
    let mut payloads = Vec::new();
    let mut budget = 8192usize;
    for (name, target) in resource_directory_entries(bytes, table, sections, resource_rva, resource_size, 0)? {
        if !resource_name_is_pipl(bytes, table, sections, resource_rva, resource_size, name)? {
            continue;
        }
        if target & 0x8000_0000 != 0 {
            collect_resource_payloads(&layout, (target & 0x7fff_ffff) as usize, 1, &mut payloads, &mut budget)?;
        } else {
            let data_entry = resource_bytes(bytes, table, sections, resource_rva, resource_size, (target & 0x7fff_ffff) as usize, 16)?;
            let data_rva = u32::from_le_bytes(data_entry[0..4].try_into().map_err(|_| invalid("invalid PE resource data entry"))?);
            let size = u32::from_le_bytes(data_entry[4..8].try_into().map_err(|_| invalid("invalid PE resource data entry"))?) as usize;
            if size > 1024 * 1024 {
                return Err(invalid("PiPL resource exceeds 1 MiB limit"));
            }
            let data_offset = rva_range_offset(bytes, table, sections, data_rva, size.max(1))?;
            payloads.push(bytes_at(bytes, data_offset, size)?);
        }
    }
    if payloads.is_empty() { Ok(None) } else { Ok(Some(payloads)) }
}

// Windows stores the PiPL four-character codes as little-endian integers.
const PIPL_PHOTOSHOP_VENDOR: &[u8; 4] = b"MIB8";
const PIPL_KIND_KEY: &[u8; 4] = b"dnik";
const PIPL_FILTER_KIND: &[u8; 4] = b"MFB8";
const PIPL_X86_CODE_KEY: &[u8; 4] = b"68xw";
const PIPL_X64_CODE_KEY: &[u8; 4] = b"4668";

fn pipl_filter_entry(payload: &[u8], architecture: Architecture) -> io::Result<Option<String>> {
    if payload.len() < 10 {
        return Err(invalid("truncated PiPL resource header"));
    }
    // The signature is a 16-bit word; version and count follow at byte offset 2.
    let version = u32_at(payload, 2)?;
    if version != 0 {
        return Err(invalid("unsupported PiPL resource version"));
    }
    let count = usize::try_from(u32_at(payload, 6)?).map_err(|_| invalid("invalid PiPL property count"))?;
    if count == 0 || count > 4096 {
        return Err(invalid("invalid PiPL property count"));
    }
    let code_key = match architecture {
        Architecture::X86 => PIPL_X86_CODE_KEY,
        Architecture::X64 => PIPL_X64_CODE_KEY,
    };
    let mut offset = 10usize;
    let mut kind: Option<[u8; 4]> = None;
    let mut code_data: Option<&[u8]> = None;
    for _ in 0..count {
        let header = bytes_at(payload, offset, 16)?;
        let vendor = &header[..4];
        let key = &header[4..8];
        let length = usize::try_from(u32::from_le_bytes(header[12..16].try_into().map_err(|_| invalid("invalid PiPL property length"))?))
            .map_err(|_| invalid("invalid PiPL property length"))?;
        let data_start = offset.checked_add(16).ok_or_else(|| invalid("PiPL property offset overflow"))?;
        let data = bytes_at(payload, data_start, length)?;
        if vendor == PIPL_PHOTOSHOP_VENDOR && key == PIPL_KIND_KEY {
            if kind.is_some() || length != 4 {
                return Err(invalid("ambiguous or invalid PiPL Kind property"));
            }
            kind = Some(data.try_into().map_err(|_| invalid("invalid PiPL Kind property"))?);
        } else if vendor == PIPL_PHOTOSHOP_VENDOR && key == code_key {
            if code_data.is_some() {
                return Err(invalid("ambiguous PiPL code entry"));
            }
            code_data = Some(data);
        }
        let padded = length.checked_add(3).ok_or_else(|| invalid("PiPL property length overflow"))? & !3;
        offset = data_start.checked_add(padded).ok_or_else(|| invalid("PiPL property offset overflow"))?;
        if offset > payload.len() {
            return Err(invalid("truncated PiPL property padding"));
        }
    }
    if kind != Some(*PIPL_FILTER_KIND) {
        return Ok(None);
    }
    let data = code_data.ok_or_else(|| invalid("filter PiPL is missing a supported architecture entry"))?;
    let end = data.iter().position(|&byte| byte == 0).ok_or_else(|| invalid("unterminated PiPL code entry"))?;
    if end == 0 || data[end + 1..].iter().any(|&byte| byte != 0) {
        return Err(invalid("invalid PiPL code entry"));
    }
    let name = std::str::from_utf8(&data[..end]).map_err(|_| invalid("PiPL code entry is not UTF-8"))?;
    if name.len() > 255 || name.bytes().any(|b| b.is_ascii_control()) {
        return Err(invalid("invalid PiPL filter entry name"));
    }
    Ok(Some(name.to_owned()))
}

fn export_names(bytes: &[u8], table: usize, sections: usize, export: usize) -> io::Result<Vec<String>> {
    let names = u32_at(bytes, export + 24)? as usize;
    if names == 0 || names > 4096 {
        return Err(invalid("invalid PE named export count"));
    }
    let names_table = rva_offset(bytes, table, sections, u32_at(bytes, export + 32)?)?;
    bytes_at(bytes, names_table, names.checked_mul(4).ok_or_else(|| invalid("PE export table overflow"))?)?;
    let mut result = Vec::new();
    for i in 0..names {
        let offset = rva_offset(bytes, table, sections, u32_at(bytes, names_table + i * 4)?)?;
        let tail = bytes.get(offset..).ok_or_else(|| invalid("invalid PE export offset"))?;
        let len = tail.iter().take(256).position(|&c| c == 0).ok_or_else(|| invalid("unterminated PE export name"))?;
        let name =
            std::str::from_utf8(tail.get(..len).ok_or_else(|| invalid("invalid PE export name"))?).map_err(|_| invalid("PE export name is not UTF-8"))?;
        result.push(name.to_owned());
    }
    Ok(result)
}

fn select_export(names: &[String], selected: Option<&str>) -> io::Result<String> {
    if selected.is_some_and(|name| name.ends_with("@16")) {
        return Err(invalid("stdcall x86 PluginMain entry is not supported"));
    }
    let supported = ["PluginMain", "_PluginMain", "ENTRYPOINT"];
    if let Some(selected) = selected {
        let matches = names.iter().filter(|name| name.as_str() == selected).count();
        if matches != 1 {
            return Err(invalid("PiPL filter entry is missing or ambiguous in PE exports"));
        }
        return Ok(selected.to_owned());
    }
    if names.iter().any(|name| name.ends_with("@16")) {
        return Err(invalid("stdcall x86 PluginMain entry is not supported"));
    }
    let mut candidates = Vec::new();
    for candidate in supported {
        let count = names.iter().filter(|name| name.as_str() == candidate).count();
        if count > 1 {
            return Err(invalid("ambiguous supported filter entry exports"));
        }
        if count == 1 {
            candidates.push(candidate);
        }
    }
    if candidates.len() != 1 {
        return Err(invalid("missing or ambiguous supported filter entry export"));
    }
    Ok(candidates[0].to_owned())
}

pub fn inspect_bytes(bytes: &[u8]) -> io::Result<Metadata> {
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(invalid("8BF library exceeds 64 MiB limit"));
    }
    if bytes_at(bytes, 0, 2)? != b"MZ" {
        return Err(invalid("8BF library is not a Windows PE file"));
    }
    let pe = u32_at(bytes, 0x3c)? as usize;
    if bytes_at(bytes, pe, 4)? != b"PE\0\0" {
        return Err(invalid("invalid PE signature"));
    }
    bytes_at(bytes, pe, 24)?;
    let architecture = match u16_at(bytes, pe + 4)? {
        0x14c => Architecture::X86,
        0x8664 => Architecture::X64,
        _ => return Err(invalid("8BF library must be Windows x86 or x64")),
    };
    let count = usize::from(u16_at(bytes, pe + 6)?);
    if count == 0 || count > 96 {
        return Err(invalid("invalid PE section count"));
    }
    if u16_at(bytes, pe + 22)? & 0x2000 == 0 {
        return Err(invalid("8BF PE file is not a DLL"));
    }
    let opt = pe + 24;
    let opt_len = usize::from(u16_at(bytes, pe + 20)?);
    bytes_at(bytes, opt, opt_len)?;
    let dir = match (architecture, u16_at(bytes, opt)?) {
        (Architecture::X86, 0x10b) => 96,
        (Architecture::X64, 0x20b) => 112,
        _ => return Err(invalid("PE machine and optional header disagree")),
    };
    if opt_len < dir + 8 {
        return Err(invalid("PE export directory is missing"));
    }
    let table = opt + opt_len;
    bytes_at(bytes, table, count * 40)?;
    let export = rva_offset(bytes, table, count, u32_at(bytes, opt + dir)?)?;
    bytes_at(bytes, export, 40)?;
    let exports = export_names(bytes, table, count, export)?;
    let resources = pipl_payloads(bytes, table, count, opt, dir, opt_len)?;
    let mut filter_entries = Vec::new();
    if let Some(resources) = resources.as_ref() {
        for payload in resources {
            if let Some(entry) = pipl_filter_entry(payload, architecture)? {
                filter_entries.push(entry);
            }
        }
    }
    // Localized PiPL leaves may describe the same exported filter entry more than once.
    filter_entries.sort_unstable();
    filter_entries.dedup();
    let selected = match filter_entries.as_slice() {
        [] if resources.is_none() => None,
        [] => return Err(invalid("PE has no supported filter PiPL entry")),
        [entry] => Some(entry.as_str()),
        _ => return Err(invalid("PE has ambiguous filter PiPL resources")),
    };
    let entry = select_export(&exports, selected)?;
    Ok(Metadata { architecture, entry })
}
pub fn inspect_file(path: &Path) -> io::Result<Metadata> {
    use std::io::Read;
    // Reject FIFOs/devices before opening: a read-only FIFO open can block the UI.
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
        return Err(invalid("8BF path must be a regular file no larger than 64 MiB"));
    }
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid("8BF path must be a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(64 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    inspect_bytes(&bytes)
}
#[cfg(test)]
pub(crate) fn fixture(architecture: Architecture) -> Vec<u8> {
    let mut b = vec![0u8; 1024];
    b[..2].copy_from_slice(b"MZ");
    b[0x3c..0x40].copy_from_slice(&128u32.to_le_bytes());
    b[128..132].copy_from_slice(b"PE\0\0");
    let (machine, magic, size, dir) = match architecture {
        Architecture::X86 => (0x14cu16, 0x10bu16, 224u16, 96),
        Architecture::X64 => (0x8664u16, 0x20bu16, 240u16, 112),
    };
    b[132..134].copy_from_slice(&machine.to_le_bytes());
    b[134..136].copy_from_slice(&1u16.to_le_bytes());
    b[148..150].copy_from_slice(&size.to_le_bytes());
    b[150..152].copy_from_slice(&0x2000u16.to_le_bytes());
    b[152..154].copy_from_slice(&magic.to_le_bytes());
    b[152 + dir..156 + dir].copy_from_slice(&0x1000u32.to_le_bytes());
    b[156 + dir..160 + dir].copy_from_slice(&40u32.to_le_bytes());
    let section = 152 + usize::from(size);
    b[section + 8..section + 12].copy_from_slice(&512u32.to_le_bytes());
    b[section + 12..section + 16].copy_from_slice(&0x1000u32.to_le_bytes());
    b[section + 16..section + 20].copy_from_slice(&512u32.to_le_bytes());
    b[section + 20..section + 24].copy_from_slice(&512u32.to_le_bytes());
    b[536..540].copy_from_slice(&1u32.to_le_bytes());
    b[544..548].copy_from_slice(&0x1040u32.to_le_bytes());
    b[576..580].copy_from_slice(&0x1050u32.to_le_bytes());
    b[592..603].copy_from_slice(b"PluginMain\0");
    b
}
#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn rejects_fifo_before_opening_it() {
        let path = std::env::temp_dir().join(format!("pc-8bf-fifo-{}.8bf", std::process::id()));
        assert!(std::process::Command::new("mkfifo").arg(&path).status().unwrap().success());
        let input = path.clone();
        let (send, receive) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let _ = send.send(inspect_file(&input));
        });
        let result = receive.recv_timeout(std::time::Duration::from_secs(1));
        if result.is_err() {
            // Linux O_RDWR opens a FIFO without waiting; unblock a regressed reader before joining.
            drop(std::fs::OpenOptions::new().read(true).write(true).open(&path).unwrap());
        }
        thread.join().unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(result.unwrap().is_err());
    }

    fn fourcc_bytes(code: &str) -> [u8; 4] {
        let mut bytes: [u8; 4] = code.as_bytes().try_into().unwrap();
        bytes.reverse();
        bytes
    }

    fn pipl(properties: &[(&str, &[u8])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(properties.len() as u32).to_le_bytes());
        for (key, value) in properties {
            bytes.extend_from_slice(&fourcc_bytes("8BIM"));
            bytes.extend_from_slice(&fourcc_bytes(key));
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
            bytes.extend_from_slice(value);
            bytes.resize(bytes.len() + ((4 - value.len() % 4) % 4), 0);
        }
        bytes
    }

    fn with_pipl(architecture: Architecture, payload: &[u8]) -> Vec<u8> {
        let mut bytes = fixture(architecture);
        bytes.resize(2048, 0);
        let pe = 128;
        let optional = pe + 24;
        let directory = match architecture {
            Architecture::X86 => 96,
            Architecture::X64 => 112,
        };
        let resource_rva = 0x1100u32;
        let resource_size = 0x400u32;
        bytes[optional + directory + 16..optional + directory + 20].copy_from_slice(&resource_rva.to_le_bytes());
        bytes[optional + directory + 20..optional + directory + 24].copy_from_slice(&resource_size.to_le_bytes());
        let section = optional + usize::from(u16_at(&bytes, pe + 20).unwrap());
        bytes[section + 8..section + 12].copy_from_slice(&0x600u32.to_le_bytes());
        bytes[section + 16..section + 20].copy_from_slice(&0x600u32.to_le_bytes());

        let resource = 768;
        bytes[resource + 12..resource + 14].copy_from_slice(&1u16.to_le_bytes());
        bytes[resource + 16..resource + 20].copy_from_slice(&(0x8000_0100u32).to_le_bytes());
        bytes[resource + 20..resource + 24].copy_from_slice(&(0x8000_0018u32).to_le_bytes());
        bytes[resource + 24 + 14..resource + 24 + 16].copy_from_slice(&1u16.to_le_bytes());
        bytes[resource + 40..resource + 44].copy_from_slice(&16000u32.to_le_bytes());
        bytes[resource + 44..resource + 48].copy_from_slice(&(0x8000_0030u32).to_le_bytes());
        bytes[resource + 48 + 14..resource + 48 + 16].copy_from_slice(&1u16.to_le_bytes());
        bytes[resource + 64..resource + 68].copy_from_slice(&1033u32.to_le_bytes());
        bytes[resource + 68..resource + 72].copy_from_slice(&72u32.to_le_bytes());
        bytes[resource + 72..resource + 76].copy_from_slice(&0x1300u32.to_le_bytes());
        bytes[resource + 76..resource + 80].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes[resource + 0x100..resource + 0x102].copy_from_slice(&4u16.to_le_bytes());
        for (i, code_unit) in "PiPL".encode_utf16().enumerate() {
            let offset = resource + 0x102 + i * 2;
            bytes[offset..offset + 2].copy_from_slice(&code_unit.to_le_bytes());
        }
        bytes[1280..1280 + payload.len()].copy_from_slice(payload);
        bytes
    }

    fn replace_export(bytes: &mut [u8], name: &[u8]) {
        let offset = 640;
        bytes[offset..offset + name.len() + 1].fill(0);
        bytes[offset..offset + name.len()].copy_from_slice(name);
        bytes[576..580].copy_from_slice(&0x1080u32.to_le_bytes());
    }

    #[test]
    fn recognizes_both_windows_dll_architectures_without_loading() {
        for arch in [Architecture::X86, Architecture::X64] {
            let bytes = fixture(arch);
            let got = inspect_bytes(&bytes).unwrap();
            assert_eq!(got.architecture, arch);
            assert_eq!(got.entry, "PluginMain");
            for len in [0, 2, 64, 128, 153, 400, 590] {
                assert!(inspect_bytes(&bytes[..len]).is_err());
            }
        }
    }
    #[test]
    fn hostile_headers_and_unknown_entry_are_errors() {
        let original = fixture(Architecture::X86);
        for (offset, value) in [(0x3c, u32::MAX), (134, 65535), (544, u32::MAX), (576, u32::MAX)] {
            let mut bytes = original.clone();
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(inspect_bytes(&bytes).is_err());
        }
        let mut bytes = original;
        bytes[592..603].copy_from_slice(b"NotAFilter\0");
        assert!(inspect_bytes(&bytes).is_err());
    }

    #[test]
    fn selects_the_code_entry_for_the_pe_architecture() {
        for (architecture, code_key) in [(Architecture::X86, "wx86"), (Architecture::X64, "8664")] {
            let code = b"PluginMain\0";
            let resource = pipl(&[("kind", &fourcc_bytes("8BFM")), (code_key, code)]);
            let bytes = with_pipl(architecture, &resource);
            let metadata = inspect_bytes(&bytes).unwrap();
            assert_eq!(metadata.architecture, architecture);
            assert_eq!(metadata.entry, "PluginMain");
        }
    }

    #[test]
    fn rejects_pipl_without_a_code_entry_for_the_pe_architecture() {
        let resource = pipl(&[("kind", &fourcc_bytes("8BFM")), ("8664", b"PluginMain\0")]);
        assert!(inspect_bytes(&with_pipl(Architecture::X86, &resource)).is_err());
        let resource = pipl(&[("kind", &fourcc_bytes("8BFM")), ("wx86", b"PluginMain\0")]);
        assert!(inspect_bytes(&with_pipl(Architecture::X64, &resource)).is_err());
    }

    #[test]
    fn rejects_non_filter_pipl_and_missing_kind() {
        let non_filter = pipl(&[("kind", &fourcc_bytes("8BEM")), ("wx86", b"PluginMain\0")]);
        assert!(inspect_bytes(&with_pipl(Architecture::X86, &non_filter)).is_err());
        let missing_kind = pipl(&[("wx86", b"PluginMain\0")]);
        assert!(inspect_bytes(&with_pipl(Architecture::X86, &missing_kind)).is_err());
    }

    #[test]
    fn localized_pipl_leaves_share_one_entry_but_distinct_entries_are_rejected() {
        for (arch, key) in [(Architecture::X86, "wx86"), (Architecture::X64, "8664")] {
            let payload = pipl(&[("kind", &fourcc_bytes("8BFM")), (key, b"PluginMain\0")]);
            let mut bytes = with_pipl(arch, &payload);
            let base = 768;
            bytes[base + 62..base + 64].copy_from_slice(&2u16.to_le_bytes());
            bytes[base + 68..base + 72].copy_from_slice(&88u32.to_le_bytes());
            bytes[base + 72..base + 76].copy_from_slice(&1031u32.to_le_bytes());
            bytes[base + 76..base + 80].copy_from_slice(&104u32.to_le_bytes());
            bytes[base + 88..base + 92].copy_from_slice(&0x1300u32.to_le_bytes());
            bytes[base + 92..base + 96].copy_from_slice(&(payload.len() as u32).to_le_bytes());
            bytes[base + 104..base + 108].copy_from_slice(&0x1380u32.to_le_bytes());
            bytes[base + 108..base + 112].copy_from_slice(&(payload.len() as u32).to_le_bytes());
            bytes[1408..1408 + payload.len()].copy_from_slice(&payload);
            assert_eq!(inspect_bytes(&bytes).unwrap().entry, "PluginMain");
            let different = pipl(&[("kind", &fourcc_bytes("8BFM")), (key, b"OtherEntry\0")]);
            bytes[1408..1408 + different.len()].copy_from_slice(&different);
            assert!(inspect_bytes(&bytes).unwrap_err().to_string().contains("ambiguous"));
        }
    }

    #[test]
    fn rejects_duplicate_or_unexported_pipl_entries() {
        let duplicate = pipl(&[("kind", &fourcc_bytes("8BFM")), ("wx86", b"PluginMain\0"), ("wx86", b"ENTRYPOINT\0")]);
        assert!(inspect_bytes(&with_pipl(Architecture::X86, &duplicate)).is_err());

        let unexported = pipl(&[("kind", &fourcc_bytes("8BFM")), ("wx86", b"ENTRYPOINT\0")]);
        assert!(inspect_bytes(&with_pipl(Architecture::X86, &unexported)).is_err());
    }

    #[test]
    fn resolves_custom_export_named_by_filter_metadata() {
        for (arch, key) in [(Architecture::X86, "wx86"), (Architecture::X64, "8664")] {
            let resource = pipl(&[("kind", &fourcc_bytes("8BFM")), (key, b"CustomFilterEntry\0")]);
            let mut bytes = with_pipl(arch, &resource);
            replace_export(&mut bytes, b"CustomFilterEntry");
            assert_eq!(inspect_bytes(&bytes).unwrap().entry, "CustomFilterEntry");
        }
    }

    #[test]
    fn rejects_x86_stdcall_entry_until_its_abi_is_supported() {
        let resource = pipl(&[("kind", &fourcc_bytes("8BFM")), ("wx86", b"_PluginMain@16\0")]);
        let mut bytes = with_pipl(Architecture::X86, &resource);
        replace_export(&mut bytes, b"_PluginMain@16");
        assert!(inspect_bytes(&bytes).is_err());
    }

    #[test]
    fn ignores_non_pipl_resources_for_legacy_pluginmain_fallback() {
        let resource = pipl(&[("kind", &fourcc_bytes("8BFM")), ("wx86", b"PluginMain\0")]);
        let mut bytes = with_pipl(Architecture::X86, &resource);
        bytes[1024..1032].copy_from_slice(&[4, 0, b'D', 0, b'A', 0, b'T', 0]);
        let metadata = inspect_bytes(&bytes).unwrap();
        assert_eq!(metadata.entry, "PluginMain");
    }

    #[test]
    fn rejects_ambiguous_legacy_entry_exports() {
        let mut bytes = fixture(Architecture::X86);
        bytes[536..540].copy_from_slice(&2u32.to_le_bytes());
        bytes[580..584].copy_from_slice(&0x1050u32.to_le_bytes());
        assert!(inspect_bytes(&bytes).is_err());
    }

    #[test]
    fn rejects_corrupt_resource_offsets_and_pipl_property_bounds() {
        let resource = pipl(&[("kind", &fourcc_bytes("8BFM")), ("wx86", b"PluginMain\0")]);
        let original = with_pipl(Architecture::X86, &resource);

        let mut bytes = original.clone();
        bytes[768 + 72..768 + 76].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(inspect_bytes(&bytes).is_err());

        let mut bytes = original.clone();
        bytes[1280 + 6..1280 + 10].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(inspect_bytes(&bytes).is_err());

        let mut bytes = original;
        bytes[1280 + 10 + 12..1280 + 10 + 16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(inspect_bytes(&bytes).is_err());
    }
}
