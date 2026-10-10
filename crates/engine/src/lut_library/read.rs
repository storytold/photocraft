//! Reading an installed LUT by the id `lut.library` lists, for callers that may not name files.

use std::fs;

use super::{LutLibrary, MAX_LUT_BYTES, hashes};

impl LutLibrary {
    /// The file name and bytes of the installed LUT `id` (`Pack/path.cube`, as
    /// [`list`](Self::list) spells it), or `None` when no installed LUT has exactly that id.
    ///
    /// The id is only ever compared with the listing, never joined into a path, so `..`, absolute
    /// paths, drive letters, backslashes and empty segments are simply ids nothing has. A listed
    /// file that is no longer a plain file (replaced by a symbolic link since the listing), or is
    /// over [`MAX_LUT_BYTES`], is an error rather than something read.
    pub fn read_by_id(&self, id: &str) -> Result<Option<(String, Vec<u8>)>, String> {
        let packs = self.list()?;
        let found = packs.iter().find_map(|pack| {
            let rest = id.strip_prefix(pack.name.as_str())?.strip_prefix('/')?;
            pack.luts.iter().find(|l| l.file == rest).map(|l| (pack.name.as_str(), l.file.as_str()))
        });
        let Some((pack, file)) = found else { return Ok(None) };
        let path = self.path_of(pack, file);
        let meta = fs::symlink_metadata(&path).map_err(|e| format!("`{id}`: {e}"))?;
        if !meta.is_file() {
            return Err(format!("`{id}` is not a plain file"));
        }
        if meta.len() > MAX_LUT_BYTES {
            return Err(format!("`{id}` is larger than {} MiB, the most a LUT may be", MAX_LUT_BYTES >> 20));
        }
        let name = file.rsplit('/').next().unwrap_or(file).to_string();
        let bytes = hashes::read_bounded(&path).ok_or_else(|| format!("`{id}` could not be read"))?;
        Ok(Some((name, bytes)))
    }
}
