//! Release checks through `craft-update`, the library every Craft App shares. Only public release
//! metadata is requested.

use photocraft_engine::updates_cmds::{CheckFn, CheckResult, Release};

pub fn service() -> CheckFn {
    Box::new(|| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("update check".into())
            .spawn(move || {
                let result = check();
                if let Err(e) = &result {
                    log::warn!("Update check: {e}");
                }
                let _ = tx.send(result);
            })
            .map_err(|e| format!("Could not start update check: {e}"))?;
        Ok(rx)
    })
}

fn check() -> CheckResult {
    let http = craft_update::Http::new("PhotoCraft/Update")?;
    let found = craft_update::check_for_update(&photocraft_ui_egui::UPDATE_APP, &http, photocraft_engine::build_info::VERSION)?;
    Ok(found.map(|release| Release { version: release.tag.trim_start_matches(['v', 'V']).to_string(), url: release.html_url }))
}

/// Why the previous self-update failed, once: the update helper records its error beside the app's
/// data before it starts the old copy again, and nothing else ever reads it, so without this a
/// failed update is a silent restart into the same prompt (#10477).
pub fn previous_failure() -> Option<String> {
    let paths = craft_update::Paths::for_app(&photocraft_ui_egui::UPDATE_APP).ok()?;
    failure_in(&paths)
}

fn failure_in(paths: &craft_update::Paths) -> Option<String> {
    let error = craft_update::self_update::take_failure(paths)?;
    log::warn!("The previous update failed: {}", error.trim());
    Some(error)
}

/// Whether the release tagged `tag` is the one this copy installed itself within the last day. A
/// copy that is offered that release again is still the old one: the install did not take effect.
pub fn installed_a_moment_ago(tag: &str) -> bool {
    craft_update::Paths::for_app(&photocraft_ui_egui::UPDATE_APP).is_ok_and(|paths| installed_in(&paths, tag))
}

fn installed_in(paths: &craft_update::Paths, tag: &str) -> bool {
    craft_update::self_update::just_updated_to(paths, &craft_update::Release { tag: tag.to_owned(), ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_update::{Arch, Asset, Os, PackageKind, Platform, Release, pick_asset};

    /// The names `packaging/*` give the 0.18.0 release files (`gh release view v0.18.0`), with the
    /// look-alikes the updater must not pick: the symbols archive, the CLI and the web build.
    const ASSETS: &[&str] = &[
        "photocraft-0.18.0-linux-aarch64.AppImage",
        "photocraft-0.18.0-linux-aarch64.deb",
        "photocraft-0.18.0-linux-x86_64.AppImage",
        "photocraft-0.18.0-linux-x86_64.deb",
        "photocraft-0.18.0-linux-x86_64.tar.gz",
        "photocraft-0.18.0-macos-universal.dmg",
        "photocraft-0.18.0-windows-arm64-portable.zip",
        "photocraft-0.18.0-windows-arm64-symbols.zip",
        "photocraft-0.18.0-windows-arm64.msi",
        "photocraft-0.18.0-windows-x64-portable.zip",
        "photocraft-0.18.0-windows-x64-symbols.zip",
        "photocraft-0.18.0-windows-x64.msi",
        "photocraft-0.18.0-windows-x86-portable.zip",
        "photocraft-0.18.0-windows-x86-symbols.zip",
        "photocraft-0.18.0-windows-x86.msi",
        "photocraft-cli-0.18.0-macos-universal.zip",
        "photocraft-web-0.18.0.zip",
        "SHA256SUMS.txt",
    ];

    fn release() -> Release {
        let assets = ASSETS
            .iter()
            .map(|name| Asset {
                name: (*name).to_owned(),
                size: 1,
                url: format!("https://github.com/storytold/photocraft/releases/download/v0.18.0/{name}"),
                sha256: None,
            })
            .collect();
        Release { tag: "v0.18.0".into(), version: craft_update::parse_version("0.18.0"), assets, ..Default::default() }
    }

    fn picked(os: Os, arch: Arch, kind: PackageKind) -> Option<String> {
        pick_asset(&release(), &[photocraft_ui_egui::UPDATE_APP.id], Platform { os, arch }, &[kind]).map(|(asset, _)| asset.name.clone())
    }

    /// An installed copy updates itself with the installer of its own kind and architecture:
    /// the MSI for an MSI install, the portable zip for a portable or loose folder, never the
    /// symbols archive or another architecture's file.
    #[test]
    fn every_windows_install_finds_its_own_file_in_the_release() {
        for (arch, name) in [(Arch::X64, "x64"), (Arch::X86, "x86"), (Arch::Arm64, "arm64")] {
            assert_eq!(picked(Os::Windows, arch, PackageKind::Msi), Some(format!("photocraft-0.18.0-windows-{name}.msi")));
            assert_eq!(picked(Os::Windows, arch, PackageKind::PortableZip), Some(format!("photocraft-0.18.0-windows-{name}-portable.zip")));
        }
    }

    #[test]
    fn the_other_platforms_find_theirs_and_unshipped_kinds_find_nothing() {
        assert_eq!(picked(Os::MacOs, Arch::Arm64, PackageKind::Dmg).as_deref(), Some("photocraft-0.18.0-macos-universal.dmg"));
        assert_eq!(picked(Os::Linux, Arch::X64, PackageKind::AppImage).as_deref(), Some("photocraft-0.18.0-linux-x86_64.AppImage"));
        assert_eq!(picked(Os::Linux, Arch::Arm64, PackageKind::Deb).as_deref(), Some("photocraft-0.18.0-linux-aarch64.deb"));
        // No Windows build for RISC-V, and no Windows MSI for a Linux machine.
        assert_eq!(picked(Os::Windows, Arch::Riscv64, PackageKind::Msi), None);
        assert_eq!(picked(Os::Linux, Arch::X64, PackageKind::Msi), None);
    }

    /// The version the update check compares is the bare release version, never the "About" text
    /// (`0.18.0 (8a87a457a, 2026-10-31)`), which does not parse as a version.
    #[test]
    fn the_running_version_is_a_plain_version_and_orders_against_releases() {
        use craft_update::version::offers_update;
        let running = craft_update::parse_version(photocraft_engine::build_info::VERSION).expect("the build's version parses");
        assert!(running.build.is_empty());
        assert!(craft_update::parse_version("0.18.0 (8a87a457a, 2026-10-31)").is_none());
        let v = |s: &str| craft_update::parse_version(s).unwrap();
        assert!(offers_update(&v("0.18.0"), &v("0.17.0")));
        assert!(!offers_update(&v("0.18.0"), &v("0.18.0")));
        assert!(!offers_update(&v("0.17.0"), &v("0.18.0")));
        assert!(offers_update(&v("0.18.0"), &v("0.18.0-rc.1")));
    }

    fn paths() -> (tempfile::TempDir, craft_update::Paths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = craft_update::Paths::under(dir.path());
        (dir, paths)
    }

    #[test]
    fn a_failure_the_update_helper_recorded_is_read_once() {
        let (_dir, paths) = paths();
        assert_eq!(failure_in(&paths), None, "nothing recorded");
        let file = paths.logs().join("self-update-error.txt");
        std::fs::create_dir_all(paths.logs()).unwrap();
        std::fs::write(&file, "the Windows Installer stopped with error 1603").unwrap();
        assert_eq!(failure_in(&paths).as_deref(), Some("the Windows Installer stopped with error 1603"));
        assert_eq!(failure_in(&paths), None, "it is told once, then removed");
        assert!(!file.exists());
    }

    #[test]
    fn a_release_installed_a_moment_ago_is_recognised_only_for_that_tag() {
        let (_dir, paths) = paths();
        assert!(!installed_in(&paths, "v0.18.0"), "no record");
        let marker = paths.data.join("self-update.txt");
        std::fs::create_dir_all(&paths.data).unwrap();
        let now = craft_update::now();
        std::fs::write(&marker, format!("v0.18.0\n{now}\n")).unwrap();
        assert!(installed_in(&paths, "v0.18.0"));
        assert!(!installed_in(&paths, "v0.19.0"), "another release is still on offer");
        std::fs::write(&marker, format!("v0.18.0\n{}\n", now - 2 * 24 * 3600)).unwrap();
        assert!(!installed_in(&paths, "v0.18.0"), "a day later the record no longer blocks an install");
        std::fs::write(&marker, "garbage").unwrap();
        assert!(!installed_in(&paths, "v0.18.0"));
    }
}
