use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use ureq::ResponseExt;
use winsafe::{self as w, co, prelude::*};

pub const RELEASE_PAGE: &str = photocraft_ui_egui::links::RELEASES;
const RELEASE_API: &str = "https://api.github.com/repos/storytold/photocraft/releases/latest";
const MAX_CHECKSUM_BYTES: u64 = 512 * 1024;
const MAX_MSI_BYTES: u64 = 1024 * 1024 * 1024;
const DOWNLOAD_CHUNK_BYTES: usize = 256 * 1024;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone)]
struct VerifiedInstaller {
    path: PathBuf,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Clone)]
struct AvailableRelease {
    version: semver::Version,
    installer: Asset,
    checksums: Asset,
}

pub fn services() -> (
    photocraft_ui_egui::CheckForUpdatesFn,
    photocraft_ui_egui::DownloadUpdateFn,
    photocraft_ui_egui::InstallUpdateFn,
) {
    let release = Arc::new(Mutex::new(None::<AvailableRelease>));
    let verified_installer = Arc::new(Mutex::new(None::<VerifiedInstaller>));

    let check_release = release.clone();
    let check_for_updates = Box::new(move || {
        let (sender, receiver) = mpsc::channel();
        let failed = sender.clone();
        let release = check_release.clone();
        if let Err(error) = std::thread::Builder::new().name("PhotoCraft update check".into()).spawn(move || {
            match latest_release() {
                Ok(Some(found)) => {
                    if let Ok(mut slot) = release.lock() {
                        *slot = Some(found.clone());
                    }
                    let _ = sender.send(photocraft_ui_egui::UpdateEvent::Available {
                        version: found.version.to_string(),
                        installable: is_msi_install(),
                    });
                }
                Ok(None) => {
                    if let Ok(mut slot) = release.lock() {
                        *slot = None;
                    }
                    let _ = sender.send(photocraft_ui_egui::UpdateEvent::Current);
                }
                Err(error) => {
                    let _ = sender.send(photocraft_ui_egui::UpdateEvent::Failed(format!("Couldn't check for updates: {error}")));
                }
            }
        }) {
            let _ = failed.send(photocraft_ui_egui::UpdateEvent::Failed(format!("Couldn't start the update check: {error}")));
        }
        receiver
    });

    let download_release = release.clone();
    let download_installer = verified_installer.clone();
    let download_update = Box::new(move || {
        let (sender, receiver) = mpsc::channel();
        let failed = sender.clone();
        let release = download_release.clone();
        let verified_installer = download_installer.clone();
        if let Err(error) = std::thread::Builder::new().name("PhotoCraft update download".into()).spawn(move || {
            let result = release
                .lock()
                .map_err(|_| "the release check is busy".to_string())
                .and_then(|release| release.clone().ok_or_else(|| "check for updates again before downloading".to_string()))
                .and_then(|release| download_verified_installer(&release, &sender));
            match result {
                Ok((version, installer, sha256)) => {
                    if let Ok(mut slot) = verified_installer.lock() {
                        *slot = Some(VerifiedInstaller { path: installer.clone(), sha256 });
                    }
                    let _ = sender.send(photocraft_ui_egui::UpdateEvent::Ready { version, installer });
                }
                Err(error) => {
                    let _ = sender.send(photocraft_ui_egui::UpdateEvent::Failed(format!("Couldn't download the update: {error}")));
                }
            }
        }) {
            let _ = failed.send(photocraft_ui_egui::UpdateEvent::Failed(format!("Couldn't start the update download: {error}")));
        }
        receiver
    });

    let install_verified = verified_installer;
    let parent_pid = std::process::id();
    let install_update = Box::new(move |installer: PathBuf| {
        let expected = install_verified.lock().map_err(|_| "the verified update is busy".to_string())?;
        let Some(verified) = expected.as_ref().filter(|verified| verified.path == installer) else {
            return Err("the installer is not the package verified by this update session".into());
        };
        if hash_file(&installer)? != verified.sha256 {
            return Err("the installer changed after its SHA256 verification".into());
        }
        let executable = std::env::current_exe().map_err(|error| format!("couldn't find PhotoCraft executable: {error}"))?;
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|error| error.to_string())?.as_nanos();
        let helper = std::env::temp_dir().join(format!("photocraft-update-helper-{parent_pid}-{nonce}.exe"));
        std::fs::copy(&executable, &helper).map_err(|error| format!("couldn't stage the update helper: {error}"))?;
        Command::new(&helper)
            .arg("--update-helper")
            .arg(&installer)
            .arg(&verified.sha256)
            .arg(parent_pid.to_string())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|error| {
                let _ = std::fs::remove_file(&helper);
                format!("couldn't start the update helper: {error}")
            })?;
        Ok(())
    });

    (check_for_updates, download_update, install_update)
}

pub fn run_update_helper(args: &[std::ffi::OsString]) -> Result<bool, String> {
    if args.first().and_then(|arg| arg.to_str()) != Some("--update-helper") {
        return Ok(false);
    }
    let helper = std::env::current_exe().map_err(|error| format!("update helper: can't find itself: {error}"))?;
    let helper_name = helper.file_name().and_then(|name| name.to_str()).unwrap_or_default();
    let temp_dir = std::env::temp_dir();
    if !same_windows_path(helper.parent().unwrap_or_else(|| Path::new("")), &temp_dir)
        || !helper_name.starts_with("photocraft-update-helper-")
    {
        return Err("update helper: executable isn't a staged PhotoCraft helper".into());
    }
    let mut cleanup = UpdateHelperCleanup { helper, installer: None };
    let installer = args.get(1).map(PathBuf::from).ok_or("update helper: missing installer path")?;
    let expected_sha256 = args.get(2).and_then(|arg| arg.to_str()).ok_or("update helper: missing installer checksum")?;
    let parent_pid = args
        .get(3)
        .and_then(|arg| arg.to_str())
        .and_then(|arg| arg.parse::<u32>().ok())
        .ok_or("update helper: invalid PhotoCraft process id")?;
    if !installer.is_absolute() || !installer.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("msi")) {
        return Err("update helper: expected an absolute MSI path".into());
    }
    let installer_name = installer.file_name().and_then(|name| name.to_str()).unwrap_or_default();
    if !same_windows_path(installer.parent().unwrap_or_else(|| Path::new("")), &temp_dir)
        || !installer_name.starts_with(&format!("photocraft-update-{parent_pid}-"))
    {
        return Err("update helper: MSI isn't a staged PhotoCraft update".into());
    }
    cleanup.installer = Some(installer.clone());
    let metadata = std::fs::metadata(&installer).map_err(|error| format!("update helper: can't read installer: {error}"))?;
    if metadata.len() == 0 || metadata.len() > MAX_MSI_BYTES {
        return Err("update helper: MSI size is outside the accepted range".into());
    }
    match w::HPROCESS::OpenProcess(co::PROCESS::SYNCHRONIZE, false, parent_pid) {
        Ok(process) => {
            let waited = process.WaitForSingleObject(None).map_err(|error| format!("update helper: waiting for PhotoCraft failed: {error}"))?;
            if waited != co::WAIT::OBJECT_0 {
                return Err("update helper: PhotoCraft process wait returned an unexpected result".into());
            }
        }
        Err(error) if error != co::ERROR::INVALID_PARAMETER => {
            return Err(format!("update helper: can't wait for PhotoCraft to exit: {error}"));
        }
        // The main process can exit before this helper has opened its process handle.
        Err(_) => {}
    }
    if hash_file(&installer)? != expected_sha256 {
        return Err("update helper: installer checksum changed before installation".into());
    }
    let status = Command::new("msiexec.exe")
        .arg("/i")
        .arg(&installer)
        .status()
        .map_err(|error| format!("update helper: can't start Windows Installer: {error}"))?;
    if status.code() == Some(1602) {
        return Ok(true);
    }
    if !status.success() {
        return Err(format!("update helper: Windows Installer exited with {status}"));
    }
    Ok(true)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_body(Some(Duration::from_secs(30)))
        .timeout_global(Some(Duration::from_secs(300)))
        .max_redirects(5)
        .save_redirect_history(true)
        .https_only(true)
        .user_agent(concat!("PhotoCraft/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn latest_release() -> Result<Option<AvailableRelease>, String> {
    let agent = agent();
    let mut response = agent.get(RELEASE_API).header("Accept", "application/vnd.github+json").call().map_err(|error| error.to_string())?;
    validate_response_urls(&response, &["api.github.com"])?;
    let found = response.body_mut().read_json::<GitHubRelease>().map_err(|error| error.to_string())?;
    if found.draft || found.prerelease {
        return Ok(None);
    }
    let version = parse_release_version(&found.tag_name)?;
    let installed = semver::Version::parse(env!("CARGO_PKG_VERSION")).map_err(|error| format!("invalid installed version: {error}"))?;
    if version <= installed {
        return Ok(None);
    }
    let msi_name = installer_name(&version, std::env::consts::ARCH).ok_or_else(|| format!("unsupported Windows architecture: {}", std::env::consts::ARCH))?;
    let installer = found.assets.iter().find(|asset| asset.name == msi_name).cloned().ok_or_else(|| format!("release {} has no {msi_name}", version))?;
    let checksums = found.assets.iter().find(|asset| asset.name == "SHA256SUMS.txt").cloned().ok_or_else(|| "the release has no SHA256SUMS.txt".to_string())?;
    validate_release_asset_url(&installer.browser_download_url)?;
    validate_release_asset_url(&checksums.browser_download_url)?;
    Ok(Some(AvailableRelease { version, installer, checksums }))
}

fn download_verified_installer(
    release: &AvailableRelease,
    sender: &mpsc::Sender<photocraft_ui_egui::UpdateEvent>,
) -> Result<(String, PathBuf), String> {
    let agent = agent();
    let manifest = get_limited(&agent, &release.checksums.browser_download_url, MAX_CHECKSUM_BYTES)?;
    let manifest = std::str::from_utf8(&manifest).map_err(|error| format!("checksum file is not UTF-8: {error}"))?;
    let expected = checksum_for_file(manifest, &release.installer.name)?;
    let installer = temp_installer_path(&release.installer.name);
    let result = download_installer(&agent, &release.installer, &installer, sender).and_then(|actual| {
        if actual.eq_ignore_ascii_case(&expected) {
            Ok(())
        } else {
            Err("the downloaded installer does not match its published SHA256 checksum".into())
        }
    });
    if let Err(error) = result {
        let _ = std::fs::remove_file(&installer);
        return Err(error);
    }
    Ok((release.version.to_string(), installer))
}

fn get_limited(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut response = agent.get(url).call().map_err(|error| error.to_string())?;
    validate_asset_response_urls(&response)?;
    response.body_mut().with_config().limit(limit).read_to_vec().map_err(|error| error.to_string())
}

fn download_installer(
    agent: &ureq::Agent,
    asset: &Asset,
    destination: &Path,
    sender: &mpsc::Sender<photocraft_ui_egui::UpdateEvent>,
) -> Result<String, String> {
    let mut response = agent.get(&asset.browser_download_url).call().map_err(|error| error.to_string())?;
    validate_asset_response_urls(&response)?;
    let total = response.headers().get("content-length").and_then(|value| value.to_str().ok()).and_then(|value| value.parse::<u64>().ok());
    if total.is_some_and(|size| size > MAX_MSI_BYTES) {
        return Err("the release installer exceeds the 1 GiB download limit".into());
    }
    let mut body = response.body_mut().with_config().limit(MAX_MSI_BYTES).reader();
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(destination).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut downloaded = 0u64;
    let mut buffer = vec![0; DOWNLOAD_CHUNK_BYTES];
    loop {
        let read = body.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read]).map_err(|error| error.to_string())?;
        digest.update(&buffer[..read]);
        downloaded = downloaded.saturating_add(read as u64);
        sender
            .send(photocraft_ui_egui::UpdateEvent::Progress { downloaded, total })
            .map_err(|_| "the About window was closed during the download".to_string())?;
    }
    file.flush().map_err(|error| error.to_string())?;
    Ok(format!("{:x}", digest.finalize()))
}

fn validate_response_urls(response: &ureq::Response<ureq::Body>, hosts: &[&str]) -> Result<(), String> {
    if let Some(history) = response.get_redirect_history() {
        for uri in history {
            validate_https_host(&uri.to_string(), hosts)?;
        }
    }
    validate_https_host(response.get_uri(), hosts)
}

fn validate_asset_response_urls(response: &ureq::Response<ureq::Body>) -> Result<(), String> {
    validate_response_urls(response, &["github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com"])
}

fn validate_release_asset_url(url: &str) -> Result<(), String> {
    validate_https_host(url, &["github.com"])?;
    let uri = url.parse::<ureq::http::Uri>().map_err(|error| format!("invalid release asset URL: {error}"))?;
    if !uri.path().starts_with("/storytold/photocraft/releases/download/") {
        return Err("the release asset URL is outside the official PhotoCraft repository".into());
    }
    Ok(())
}

fn validate_https_host(url: &str, allowed_hosts: &[&str]) -> Result<(), String> {
    let uri = url.parse::<ureq::http::Uri>().map_err(|error| format!("invalid HTTPS URL: {error}"))?;
    let host = uri.host().unwrap_or_default();
    if uri.scheme_str() != Some("https") || !allowed_hosts.iter().any(|allowed| host.eq_ignore_ascii_case(allowed)) {
        return Err(format!("release response used an unexpected host: {host}"));
    }
    Ok(())
}

fn installer_name(version: &semver::Version, arch: &str) -> Option<String> {
    let arch = match arch {
        "x86_64" => "x64",
        "x86" => "x86",
        "aarch64" => "arm64",
        _ => return None,
    };
    Some(format!("photocraft-{version}-windows-{arch}.msi"))
}

fn parse_release_version(tag: &str) -> Result<semver::Version, String> {
    semver::Version::parse(tag.strip_prefix('v').unwrap_or(tag)).map_err(|error| format!("invalid release tag {tag:?}: {error}"))
}

fn checksum_for_file(manifest: &str, expected_name: &str) -> Result<String, String> {
    let mut found = None;
    for line in manifest.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let digest = fields.next().ok_or("checksum line is missing its digest")?;
        let filename = fields.next().ok_or("checksum line is missing its filename")?.trim_start_matches('*');
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("checksum for {filename:?} is not a SHA256 digest"));
        }
        if fields.next().is_some() {
            return Err(format!("checksum line for {filename:?} has extra fields"));
        }
        if filename == expected_name {
            if found.replace(digest.to_ascii_lowercase()).is_some() {
                return Err(format!("checksum file contains duplicate entries for {expected_name}"));
            }
        }
    }
    found.ok_or_else(|| format!("checksum file has no entry for {expected_name}"))
}

fn temp_installer_path(asset_name: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("photocraft-update-{}-{nonce}-{asset_name}", std::process::id()))
}

fn hash_file(path: &Path) -> Result<String, String> {
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() == 0 || metadata.len() > MAX_MSI_BYTES {
        return Err("installer size is outside the accepted range".into());
    }
    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0; DOWNLOAD_CHUNK_BYTES];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

struct UpdateHelperCleanup {
    helper: PathBuf,
    installer: Option<PathBuf>,
}

impl Drop for UpdateHelperCleanup {
    fn drop(&mut self) {
        if let Some(installer) = &self.installer {
            let _ = std::fs::remove_file(installer);
        }
        if let Some(helper) = self.helper.to_str() {
            let _ = w::MoveFileEx(helper, None, co::MOVEFILE::DELAY_UNTIL_REBOOT);
        }
    }
}

fn is_msi_install() -> bool {
    let access = co::KEY::READ
        | if cfg!(target_pointer_width = "64") { co::KEY::WOW64_64KEY } else { co::KEY::WOW64_32KEY };
    let Ok(key) = w::HKEY::LOCAL_MACHINE.RegOpenKeyEx(Some("Software\\PhotoCraft"), co::REG_OPTION::default(), access) else { return false };
    let Ok(value) = key.RegQueryValueEx(Some("InstallDir")) else { return false };
    let install_dir = match value {
        w::RegistryValue::Sz(value) | w::RegistryValue::ExpandSz(value) => value,
        _ => return false,
    };
    let Ok(executable) = std::env::current_exe() else { return false };
    same_windows_path(&PathBuf::from(install_dir).join("photocraft.exe"), &executable)
}

fn same_windows_path(expected: &Path, actual: &Path) -> bool {
    let normalized = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().replace('/', "\\");
    normalized(expected).eq_ignore_ascii_case(&normalized(actual))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_asset_name_matches_each_supported_windows_architecture() {
        let version = semver::Version::new(1, 2, 3);
        assert_eq!(installer_name(&version, "x86_64").as_deref(), Some("photocraft-1.2.3-windows-x64.msi"));
        assert_eq!(installer_name(&version, "x86").as_deref(), Some("photocraft-1.2.3-windows-x86.msi"));
        assert_eq!(installer_name(&version, "aarch64").as_deref(), Some("photocraft-1.2.3-windows-arm64.msi"));
        assert_eq!(installer_name(&version, "riscv64"), None);
    }

    #[test]
    fn release_tags_accept_v_prefix_and_semver_prereleases() {
        assert_eq!(parse_release_version("v1.2.3").ok(), Some(semver::Version::new(1, 2, 3)));
        assert_eq!(parse_release_version("1.2.3-rc.1").ok().map(|version| version.to_string()).as_deref(), Some("1.2.3-rc.1"));
        assert!(parse_release_version("not-a-version").is_err());
    }

    #[test]
    fn checksum_parser_selects_exact_asset_and_rejects_duplicates_or_bad_hashes() {
        let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let manifest = format!("{digest}  other.msi\n{digest} *photocraft.msi\n");
        assert_eq!(checksum_for_file(&manifest, "photocraft.msi").as_deref(), Ok(digest));
        assert!(checksum_for_file(&manifest, "missing.msi").is_err());
        assert!(checksum_for_file(&format!("{digest}  photocraft.msi\n{digest}  photocraft.msi\n"), "photocraft.msi").is_err());
        assert!(checksum_for_file("not-a-hash  photocraft.msi\n", "photocraft.msi").is_err());
    }

    #[test]
    fn asset_urls_must_use_the_official_https_release_path() {
        assert!(validate_release_asset_url("https://github.com/storytold/photocraft/releases/download/v1.2.3/photocraft.msi").is_ok());
        assert!(validate_release_asset_url("http://github.com/storytold/photocraft/releases/download/v1.2.3/photocraft.msi").is_err());
        assert!(validate_release_asset_url("https://github.com/other/repo/releases/download/v1.2.3/photocraft.msi").is_err());
        assert!(validate_https_host("https://github.com.evil.example/file", &["github.com"]).is_err());
    }

    #[test]
    fn msi_detection_compares_normalized_windows_paths_without_case_sensitivity() {
        let root = std::env::temp_dir().join(format!("photocraft-updater-path-{}", std::process::id()));
        let expected = root.join("PhotoCraft").join("photocraft.exe");
        let actual = root.join("photocraft").join("PHOTOCRAFT.EXE");
        assert!(same_windows_path(&expected, &actual));
    }

    #[test]
    fn update_helper_rejects_missing_or_unexpected_arguments_before_launching_installer() {
        assert_eq!(run_update_helper(&[]).ok(), Some(false));
        assert!(run_update_helper(&["--update-helper".into()]).is_err());
        assert!(run_update_helper(&["--update-helper".into(), "relative.exe".into(), "0".into(), "1".into()]).is_err());
    }
}
