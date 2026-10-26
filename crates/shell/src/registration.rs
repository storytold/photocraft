//! Per-user registry registration and the public on/off switch.
//!
//! Everything is written under `HKEY_CURRENT_USER\Software\Classes`, the per-user view of
//! `HKEY_CLASSES_ROOT`, so no elevation is needed.
#![allow(unsafe_code)]

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW, GetModuleHandleExW,
};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW,
    RegDeleteTreeW, RegGetValueW, RegSetValueExW,
};
use windows_sys::Win32::UI::Shell::{ASSOCSTR_PROGID, AssocQueryStringW, SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};

/// CLSID of the PhotoCraft PSD thumbnail provider.
pub const CLSID_STR: &str = "{8D6B2D5C-3F41-4B7A-9C0E-5A17E2B94D63}";

/// CLSID registry values are case-insensitive, even when returned as strings.
fn is_our_clsid(value: &str) -> bool {
    value.eq_ignore_ascii_case(CLSID_STR)
}
const EXTENSIONS: [&str; 3] = [".psd", ".psb", ".pcraft"];
const THUMBNAIL_IID: &str = "{E357FCCD-A995-4576-B01F-234630154E96}";
fn handler_key(ext: &str) -> String {
    format!(r"{ext}\ShellEx\{THUMBNAIL_IID}")
}
fn user_handler_key(ext: &str) -> String {
    format!(r"Software\Classes\{}", handler_key(ext))
}
// Keep the original PSD backup location, including when upgrading the PSD-only provider.
fn state_key(ext: &str) -> String {
    if ext == ".psd" { class_key() } else { format!(r"{}\Formats\{ext}", class_key()) }
}
const DLL_NAME: &str = "photocraft_shell.dll";
const FRIENDLY_NAME: &str = "PhotoCraft Document Thumbnail Provider";
/// Windows draws the file type's icon as a small badge on thumbnails unless the file type has an
/// empty `TypeOverlay` value.
const OVERLAY_VALUE: &str = "TypeOverlay";
/// Value of our class key that remembers which key we put `TypeOverlay` on, so turning the badge
/// back on removes exactly that and nothing else.
const OVERLAY_KEY_VALUE: &str = "TypeOverlayKey";
/// The value of `TypeOverlay` before PhotoCraft hid the badge, if one existed.
const PREVIOUS_OVERLAY_VALUE: &str = "PreviousTypeOverlay";
/// Whether a `TypeOverlay` value existed before PhotoCraft hid the badge.
const HAD_PREVIOUS_OVERLAY_VALUE: &str = "HadPreviousTypeOverlay";
/// Value of our class key that remembers the `.psd` thumbnail handler this user had before, which
/// turning the thumbnails off puts back.
const PREVIOUS_HANDLER_VALUE: &str = "PreviousHandler";

/// Turns PSD thumbnails in Windows Explorer on or off for the current user, with or without the
/// file type's logo badge on them. This is the explicit switch: enabling takes `.psd` over from
/// any other program's thumbnail handler.
///
/// Enabling expects `photocraft_shell.dll` next to the running executable. Disabling removes only
/// registrations made by PhotoCraft and leaves other programs' handlers alone.
pub fn enable_explorer_thumbnails(enable: bool, show_logo: bool) -> Result<(), String> {
    if enable {
        let dll = sibling_dll()?;
        register(&dll, show_logo)
    } else {
        unregister()
    }
}

/// The default-on behaviour at startup: registers the thumbnails unless another program already
/// provides the `.psd` thumbnails (Photoshop, for one), which is only replaced by an explicit
/// [`enable_explorer_thumbnails`]. A registration pointing at a moved copy of the DLL is repaired.
pub fn ensure_explorer_thumbnails(show_logo: bool) -> Result<(), String> {
    let dll = sibling_dll()?;
    for ext in EXTENSIONS {
        match current_handler(ext) {
            Some(other) if !is_our_clsid(&other) => {}
            _ if is_registered_to(&dll, ext) && overlay_ok(show_logo, ext) => {}
            _ => register_extension(&dll, show_logo, ext)?,
        }
    }
    Ok(())
}

/// A handler may be registered on the extension or on its associated ProgID. Automatic
/// registration must preserve both; explicit takeover only writes our per-user extension key.
fn current_handler(ext: &str) -> Option<String> {
    read_default(HKEY_CLASSES_ROOT, &handler_key(ext)).or_else(|| prog_id(ext).and_then(|id| read_default(HKEY_CLASSES_ROOT, &handler_key(&id))))
}

/// `photocraft_shell.dll` in the folder of the running executable.
fn sibling_dll() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate the executable: {e}"))?;
    exe.parent().map(|dir| dir.join(DLL_NAME)).filter(|path| path.is_file()).ok_or_else(|| format!("{DLL_NAME} not found next to the executable"))
}

/// The registry path of this provider's COM class.
fn class_key() -> String {
    format!(r"Software\Classes\CLSID\{CLSID_STR}")
}

/// Is `.psd` already registered to this class, served from `dll`?
fn is_registered_to(dll: &Path, ext: &str) -> bool {
    read_default(HKEY_CURRENT_USER, &user_handler_key(ext)).as_deref().is_some_and(is_our_clsid)
        && read_default(HKEY_CURRENT_USER, &format!(r"{}\InprocServer32", class_key())).as_deref() == Some(&*dll.to_string_lossy())
}

/// Registers the COM class and associates it with `.psd`.
pub(crate) fn register(dll: &Path, show_logo: bool) -> Result<(), String> {
    for ext in EXTENSIONS {
        if !is_registered_to(dll, ext) || !overlay_ok(show_logo, ext) {
            register_extension(dll, show_logo, ext)?;
        }
    }
    Ok(())
}

fn register_extension(dll: &Path, show_logo: bool, ext: &str) -> Result<(), String> {
    let class_key = class_key();
    set_value(&class_key, None, FRIENDLY_NAME)?;
    let inproc = format!(r"{class_key}\InprocServer32");
    set_value(&inproc, None, &dll.to_string_lossy())?;
    set_value(&inproc, Some("ThreadingModel"), "Apartment")?;
    let class_key = state_key(ext);
    // Taking `.psd` over from a handler registered for this user: keep it, to give it back. When
    // there is none now (another program removed it), forget the one saved earlier.
    match read_default(HKEY_CURRENT_USER, &user_handler_key(ext)) {
        Some(handler) if is_our_clsid(&handler) => {}
        Some(previous) => set_value(&class_key, Some(PREVIOUS_HANDLER_VALUE), &previous)?,
        None => delete_value(&class_key, PREVIOUS_HANDLER_VALUE)?,
    }
    set_value(&user_handler_key(ext), None, CLSID_STR)?;
    apply_overlay(show_logo, ext)?;
    notify_shell();
    Ok(())
}

/// Removes the `.psd` association (only when it is ours), the badge setting and the COM class.
///
/// The association comes first: it is what makes Explorer load the provider, so a failure to put the
/// cosmetic badge setting back must not leave the provider enabled. When only the badge restore
/// fails, the class key (which remembers what to restore) is kept and the error is returned, so a
/// later call can finish the job.
pub(crate) fn unregister() -> Result<(), String> {
    for ext in EXTENSIONS {
        unregister_extension(ext)?;
    }
    delete_tree(&class_key())
}

fn unregister_extension(ext: &str) -> Result<(), String> {
    let class_key = state_key(ext);
    let ours = read_default(HKEY_CURRENT_USER, &user_handler_key(ext)).as_deref().is_some_and(is_our_clsid);
    let overlay_key = read_string(HKEY_CURRENT_USER, &class_key, Some(OVERLAY_KEY_VALUE));
    let registered = ours || overlay_key.is_some() || read_default(HKEY_CURRENT_USER, &format!(r"{class_key}\InprocServer32")).is_some();
    if ours {
        let removed = match read_string(HKEY_CURRENT_USER, &class_key, Some(PREVIOUS_HANDLER_VALUE)) {
            Some(previous) => set_value(&user_handler_key(ext), None, &previous),
            None => delete_tree(&user_handler_key(ext)),
        };
        removed?;
    }
    let overlay = match &overlay_key {
        Some(key) => restore_overlay(key, &class_key),
        None => Ok(()),
    };
    let result = overlay.and_then(|()| delete_value(&class_key, OVERLAY_KEY_VALUE));
    // Refreshing every icon is disruptive: only when something was actually removed.
    if registered {
        notify_shell();
    }
    result
}

/// The key that holds the badge setting: the file type (ProgID) Windows resolves for `.psd`, whose
/// icon is the badge, or the `.psd` key itself when there is none.
fn overlay_target(ext: &str) -> String {
    match prog_id(ext) {
        Some(id) => format!(r"Software\Classes\{id}"),
        None => format!(r"Software\Classes\{ext}"),
    }
}

/// The file type (ProgID) Windows resolves for `.psd`, if any.
fn prog_id(extension: &str) -> Option<String> {
    let ext = wide(extension);
    let mut buf = [0u16; 256];
    let mut len = buf.len() as u32;
    // SAFETY: `ext` is NUL-terminated and `len` is the exact capacity of `buf` in characters.
    let hr = unsafe { AssocQueryStringW(0, ASSOCSTR_PROGID, ext.as_ptr(), null(), buf.as_mut_ptr(), &mut len) };
    if hr != 0 {
        return None;
    }
    let text = buf.get(..(len as usize).min(buf.len()))?;
    let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
    String::from_utf16(text.get(..end)?).ok().filter(|id| !id.is_empty())
}

/// Hides the file type's logo badge on `.psd` thumbnails, or shows it again.
fn apply_overlay(show_logo: bool, ext: &str) -> Result<(), String> {
    let class_key = state_key(ext);
    let stored = read_string(HKEY_CURRENT_USER, &class_key, Some(OVERLAY_KEY_VALUE));
    if show_logo {
        if let Some(key) = stored {
            restore_overlay(&key, &class_key)?;
            delete_value(&class_key, OVERLAY_KEY_VALUE)?;
        }
        return Ok(());
    }
    let target = overlay_target(ext);
    if let Some(old) = stored.filter(|old| *old != target) {
        restore_overlay(&old, &class_key)?;
    }
    // Save what is there unless it is our own empty value; another program may have changed it
    // since we last hid the badge, and that is what turning this off must put back.
    let current = read_string(HKEY_CURRENT_USER, &target, Some(OVERLAY_VALUE));
    if read_string(HKEY_CURRENT_USER, &class_key, Some(OVERLAY_KEY_VALUE)).as_deref() != Some(&*target) || current.as_deref() != Some("") {
        match current {
            Some(previous) => {
                set_value(&class_key, Some(PREVIOUS_OVERLAY_VALUE), &previous)?;
                set_value(&class_key, Some(HAD_PREVIOUS_OVERLAY_VALUE), "1")?;
            }
            None => {
                delete_value(&class_key, PREVIOUS_OVERLAY_VALUE)?;
                set_value(&class_key, Some(HAD_PREVIOUS_OVERLAY_VALUE), "0")?;
            }
        }
    }
    // Record the key before changing it, so a failure in between never leaves an empty value
    // that nothing remembers to undo.
    set_value(&class_key, Some(OVERLAY_KEY_VALUE), &target)?;
    set_value(&target, Some(OVERLAY_VALUE), "")
}

/// Restores the previous `TypeOverlay` value, removing only the empty value PhotoCraft applied.
fn restore_overlay(key: &str, class_key: &str) -> Result<(), String> {
    if read_string(HKEY_CURRENT_USER, key, Some(OVERLAY_VALUE)).as_deref() == Some("") {
        match read_string(HKEY_CURRENT_USER, class_key, Some(HAD_PREVIOUS_OVERLAY_VALUE)).as_deref() {
            Some("1") => {
                if let Some(previous) = read_string(HKEY_CURRENT_USER, class_key, Some(PREVIOUS_OVERLAY_VALUE)) {
                    set_value(key, Some(OVERLAY_VALUE), &previous)?;
                }
            }
            Some("0") | None => delete_value(key, OVERLAY_VALUE)?,
            Some(_) => {}
        }
    }
    delete_value(class_key, PREVIOUS_OVERLAY_VALUE)?;
    delete_value(class_key, HAD_PREVIOUS_OVERLAY_VALUE)?;
    Ok(())
}

/// Does the badge setting in the registry match `show_logo`?
fn overlay_ok(show_logo: bool, ext: &str) -> bool {
    let stored = read_string(HKEY_CURRENT_USER, &state_key(ext), Some(OVERLAY_KEY_VALUE));
    if show_logo {
        return stored.is_none();
    }
    let target = overlay_target(ext);
    stored.as_deref() == Some(&*target) && read_string(HKEY_CURRENT_USER, &target, Some(OVERLAY_VALUE)).as_deref() == Some("")
}

/// Anchor whose address lies inside this DLL, used to look the module up.
static MODULE_ANCHOR: u8 = 0;

/// Path of the DLL containing this code (used by `DllRegisterServer`).
pub(crate) fn current_module_path() -> Option<PathBuf> {
    let mut module = null_mut();
    let mut buf = [0u16; 1024];
    let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    // SAFETY: the address belongs to this module and `buf`'s length is passed along.
    let len = unsafe {
        if GetModuleHandleExW(flags, std::ptr::addr_of!(MODULE_ANCHOR).cast(), &mut module) == 0 {
            return None;
        }
        GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) as usize
    };
    if len == 0 || len >= buf.len() {
        return None;
    }
    Some(PathBuf::from(OsString::from_wide(buf.get(..len)?)))
}

/// `s` as a NUL-terminated UTF-16 string for Win32 calls.
fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// `Ok` for `ERROR_SUCCESS`, otherwise an error naming the failed call and its Win32 code.
fn check(code: u32, what: &str) -> Result<(), String> {
    if code == ERROR_SUCCESS { Ok(()) } else { Err(format!("{what} failed (Win32 error {code})")) }
}

/// Creates `path` under HKCU if needed and sets its value `name` (the default value for `None`).
fn set_value(path: &str, name: Option<&str>, value: &str) -> Result<(), String> {
    let path_w = wide(path);
    let name_w = name.map(wide);
    let value_w = wide(value);
    let bytes = u32::try_from(value_w.len() * 2).map_err(|_| "registry value too long".to_owned())?;
    let name_ptr = name_w.as_ref().map_or(null(), |n| n.as_ptr());
    let mut key: HKEY = null_mut();
    // SAFETY: every pointer references a live, NUL-terminated buffer owned by this function, and
    // the key is closed on every path after it was opened.
    unsafe {
        check(
            RegCreateKeyExW(HKEY_CURRENT_USER, path_w.as_ptr(), 0, null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, null(), &mut key, null_mut()),
            "RegCreateKeyExW",
        )?;
        let rc = RegSetValueExW(key, name_ptr, 0, REG_SZ, value_w.as_ptr().cast(), bytes);
        RegCloseKey(key);
        check(rc, "RegSetValueExW")
    }
}

/// Reads a key's default string value under `root`; `None` when missing or not a string.
fn read_default(root: HKEY, path: &str) -> Option<String> {
    read_string(root, path, None)
}

/// Reads the string value `name` (the default value for `None`) of `path` under `root`; `None`
/// when missing or not a string.
fn read_string(root: HKEY, path: &str, name: Option<&str>) -> Option<String> {
    let path_w = wide(path);
    let name_w = name.map(wide);
    let name_ptr = name_w.as_ref().map_or(null(), |n| n.as_ptr());
    // MAX_PATH-sized paths of the DLL fit with room to spare.
    let mut buf = [0u16; 1024];
    let mut size = std::mem::size_of_val(&buf) as u32;
    // SAFETY: `size` is the exact byte capacity of `buf`; the name pointers are NUL-terminated.
    let rc = unsafe { RegGetValueW(root, path_w.as_ptr(), name_ptr, RRF_RT_REG_SZ, null_mut(), buf.as_mut_ptr().cast(), &mut size) };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let text = buf.get(..(size as usize / 2).min(buf.len()))?;
    let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
    String::from_utf16(text.get(..end)?).ok()
}

/// Deletes one value of an HKCU key; a value that is already gone is fine.
fn delete_value(path: &str, name: &str) -> Result<(), String> {
    let (path_w, name_w) = (wide(path), wide(name));
    // SAFETY: both strings are NUL-terminated and outlive the call.
    let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, path_w.as_ptr(), name_w.as_ptr()) };
    if matches!(rc, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) { Ok(()) } else { check(rc, "RegDeleteKeyValueW") }
}

/// Deletes an HKCU key and everything under it; a key that is already gone is fine.
fn delete_tree(path: &str) -> Result<(), String> {
    let path_w = wide(path);
    // SAFETY: `path_w` is NUL-terminated and outlives the call.
    let rc = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, path_w.as_ptr()) };
    if matches!(rc, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) { Ok(()) } else { check(rc, "RegDeleteTreeW") }
}

/// Tells Explorer that file associations changed, so it refreshes its thumbnails and icons.
fn notify_shell() {
    // SAFETY: null item pointers are valid for `SHCNE_ASSOCCHANGED`.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, null(), null()) };
}

#[cfg(test)]
mod tests {
    use super::{CLSID_STR, is_our_clsid};

    #[test]
    fn every_extension_has_its_own_handler_and_backup() {
        use super::{EXTENSIONS, handler_key, state_key, user_handler_key};
        let mut handlers = std::collections::HashSet::new();
        let mut backups = std::collections::HashSet::new();
        for ext in EXTENSIONS {
            assert!(handlers.insert(handler_key(ext)));
            assert!(backups.insert(state_key(ext)));
            assert!(user_handler_key(ext).starts_with(r"Software\Classes\"));
        }
        assert_eq!(handlers.len(), 3);
        assert_eq!(state_key(".psd"), super::class_key(), "retain PSD-only version's backup");
    }

    #[test]
    fn recognizes_case_variants_of_our_clsid() {
        assert!(is_our_clsid(CLSID_STR));
        assert!(is_our_clsid(&CLSID_STR.to_ascii_lowercase()));
        assert!(!is_our_clsid("{11111111-2222-3333-4444-555555555555}"));
    }
}
