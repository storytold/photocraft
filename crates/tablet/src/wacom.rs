//! The Wacom driver's settings: the scope's "Use Windows Ink" checkbox.
//!
//! The Windows driver (6.4.x) keeps its settings in `%APPDATA%\WTablet\Wacom_Tablet.dat`, a plain
//! XML document keyed per application. [`windows_ink`] reads the scope's checkbox (`WinUseInk`)
//! that applies to this executable. The checkbox is what picks the app's pen path: off, the driver
//! serves Wintab ([`crate::wintab`]) and synthesizes the mouse itself; on, it serves Windows Ink
//! (`WM_POINTER`, which winit forwards).
//!
//! The file also rewrites itself constantly (the driver records usage statistics); callers read
//! it once at start-up, which is enough — the Ink checkbox needs a driver-settings visit anyway.
//! Everything here is plain file and string handling: no `unsafe`, no dependencies.

use std::fs;
use std::path::{Path, PathBuf};

/// The driver's settings file, `%APPDATA%\WTablet\Wacom_Tablet.dat`, if it exists.
pub fn settings_path() -> Option<PathBuf> {
    let mut path = PathBuf::from(std::env::var_os("APPDATA")?);
    path.push("WTablet");
    path.push("Wacom_Tablet.dat");
    path.is_file().then_some(path)
}

/// The scope's "Use Windows Ink" checkbox for `exe` - the running executable when `None`: the
/// application scope matching its full path, else one matching its file name alone, else the
/// "All" scope. `None` when there is no readable settings file (or no scope for it). The driver's
/// default when the file omits the line is `true`, the Windows Ink path.
pub fn windows_ink(exe: Option<&str>) -> Option<bool> {
    let text = fs::read_to_string(settings_path()?).ok()?;
    let exe = exe.map(str::to_owned).or_else(|| std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned()))?;
    let scope = application_scope(&text, &exe)?;
    scope_ink(&text, scope)
}

/// The application scope id the driver's settings pick for `exe` (lowercased on entry).
fn application_scope(text: &str, exe: &str) -> Option<u32> {
    let name = file_name(exe);
    let (mut by_name, mut all) = (None, None);
    let mut rest = text;
    while let Some(i) = rest.find("<AppID") {
        rest = &rest[i..];
        let Some(end) = rest.find("</AppID") else { break };
        let block = &rest[..end];
        rest = &rest[end..];
        let digits: String = block["<AppID".len()..].chars().take_while(char::is_ascii_digit).collect();
        let Ok(id) = digits.parse::<u32>() else { continue };
        let Some(path) = tagged(block, "ApplicationLongName") else { continue };
        let path = path.to_lowercase();
        if path == exe {
            return Some(id);
        }
        if by_name.is_none() && name.is_some() && name == file_name(&path) {
            by_name = Some(id);
        }
        if id == 0 {
            all = Some(id);
        }
    }
    by_name.or(all)
}

/// The scope `id`'s "Use Windows Ink" checkbox (the first tablet's, on a multi-tablet desk).
/// `None` when the file has no scope for `id`.
fn scope_ink(text: &str, id: u32) -> Option<bool> {
    let start = text.find("<TabletTransducerArray")?;
    let end = text[start..].find("</TabletTransducerArray>")?;
    let mut rest = &text[start..start + end];
    while let Some(i) = rest.find("<ArrayElement") {
        rest = &rest[i..];
        let Some(end) = rest.find("</ArrayElement>") else { break };
        let block = &rest[..end];
        rest = &rest[end..];
        let Some(associated) = tagged(block, "ApplicationAssociated").and_then(|v| v.parse::<u32>().ok()) else { continue };
        if associated != id {
            continue;
        }
        return Some(tagged(block, "WinUseInk").map(|v| v == "true").unwrap_or(true));
    }
    None
}

/// The text of the first `<Tag ...>value</Tag>` in `text`.
fn tagged<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag} ");
    let start = text.find(&open)? + open.len();
    let start = text[start..].find('>')? + start + 1;
    let end = text[start..].find('<')? + start;
    Some(text[start..end].trim())
}

/// The lowercased file name of a path (for matching an application scope by name alone).
fn file_name(path: &str) -> Option<String> {
    Path::new(path).file_name().map(|n| n.to_string_lossy().to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of a real `Wacom_Tablet.dat`, trimmed to what the reader looks at: two scopes
    /// ("All" and one per application) and one tablet's elements, each holding its scope's
    /// settings. Tags carry attributes, so `tagged`'s `<Tag ` form matches.
    const DAT: &str = r#"<WacomTablet version="...">
  <AppID0>
    <ApplicationLongName modifier="modified">C:\Apps\Others\SomeOther.exe</ApplicationLongName>
  </AppID0>
  <AppID7>
    <ApplicationLongName modifier="modified">C:\Apps\PhotoCraft\photocraft.exe</ApplicationLongName>
  </AppID7>
  <TabletTransducerArray>
    <ArrayElement>
      <ApplicationAssociated modifier="modified">0</ApplicationAssociated>
    </ArrayElement>
    <ArrayElement>
      <ApplicationAssociated modifier="modified">7</ApplicationAssociated>
      <WinUseInk modifier="modified">false</WinUseInk>
    </ArrayElement>
  </TabletTransducerArray>
</WacomTablet>"#;

    #[test]
    fn the_scope_matches_the_full_path_then_the_name_then_all() {
        assert_eq!(application_scope(DAT, r"c:\apps\photocraft\photocraft.exe"), Some(7));
        // A different directory, same file name: the name-only match.
        assert_eq!(application_scope(DAT, r"d:\portable\photocraft.exe"), Some(7));
        // Case-insensitive names: the name-only match again.
        assert_eq!(application_scope(DAT, r"d:\portable\PHOTOCRAFT.EXE"), Some(7));
        // An unknown application: the "All" scope.
        assert_eq!(application_scope(DAT, r"c:\apps\others\reader.exe"), Some(0));
    }

    #[test]
    fn windows_ink_reads_the_checkbox_or_defaults_to_ink_on() {
        assert_eq!(scope_ink(DAT, 7), Some(false), "WinUseInk false is the Wintab case");
        // The "All" scope has no WinUseInk line: the driver's default, Windows Ink on.
        assert_eq!(scope_ink(DAT, 0), Some(true));
        assert_eq!(scope_ink(DAT, 9), None, "no element is associated with an unknown scope");
        assert_eq!(scope_ink("<WacomTablet></WacomTablet>", 0), None, "no transducer array");
    }
}
