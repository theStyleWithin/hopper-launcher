mod classify;
mod device;

use classify::{builds_for, is_handoff, Asset, Build};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
#[cfg_attr(target_os = "android", allow(unused_imports))]
use std::process::{Command, Stdio};

const BUNDLED_MANIFEST: &str = include_str!("../../apps.json");

#[derive(Deserialize, Serialize, Clone)]
struct AppEntry {
    id: String,
    name: String,
    tagline: String,
    /// GitHub "owner/name"
    repo: String,
    #[serde(default)]
    accent: Option<String>,
    /// Operating systems this app's builds are not yet tested on.
    #[serde(default)]
    untested_on: Vec<String>,
    /// The Internet Archive item that holds this app's files, kept as a backup.
    #[serde(default)]
    archive: Option<String>,
}

#[derive(Deserialize)]
struct Manifest {
    apps: Vec<AppEntry>,
}

#[derive(Deserialize, Serialize, Clone)]
struct InstalledApp {
    tag: String,
    /// The file the launcher starts.
    path: PathBuf,
    /// True when the app was unpacked into its own folder (portable zip builds).
    #[serde(default)]
    folder: bool,
    /// True when this is the user's own copy, found or chosen in place. The
    /// launcher runs it where it is and never moves, changes or deletes it.
    #[serde(default)]
    adopted: bool,
    /// Android only: the installed app's package name, read from its APK.
    #[serde(default)]
    package: Option<String>,
    /// A release the user chose to skip. The update prompt stays away until a newer one appears.
    #[serde(default)]
    ignored_version: Option<String>,
}

type State = HashMap<String, InstalledApp>;

#[derive(Serialize)]
struct AppInfo {
    #[serde(flatten)]
    entry: AppEntry,
    installed: bool,
    version: Option<String>,
    /// Installed as a whole folder, which may hold the app's own settings.
    folder: bool,
    /// The user's own copy, used in place.
    adopted: bool,
    path: Option<String>,
    /// The release the user skipped, if any.
    ignored_version: Option<String>,
}

#[derive(Serialize)]
struct ReleaseInfo {
    id: String,
    tag: Option<String>,
    builds: Vec<Build>,
    error: Option<String>,
}

#[derive(Serialize, Debug)]
struct InstallResult {
    tag: String,
    /// True when a published checksum matched the download.
    verified: bool,
    /// True when the file was handed to the system's own installer.
    handed_off: bool,
}

#[derive(Deserialize, Clone)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

// ---------- locations and manifest ----------

fn api_base() -> String {
    std::env::var("HOPPER_API_BASE").unwrap_or_else(|_| "https://api.github.com".into())
}

fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("hopper-launcher")
}

fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("HOPPER_DATA_DIR") {
        return PathBuf::from(d);
    }
    #[cfg(target_os = "android")]
    if let Some(app) = APP.get() {
        use tauri::Manager;
        if let Ok(dir) = app.path().app_data_dir() {
            return dir.join("hopper-launcher");
        }
    }
    dirs::data_dir().unwrap_or_default().join("hopper-launcher")
}

// ---------- Android bridge ----------

#[cfg(target_os = "android")]
static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// Calls a command of the native Android plugin (installing, launching, checking packages).
#[cfg(target_os = "android")]
fn android_call(command: &str, payload: serde_json::Value) -> Result<serde_json::Value, String> {
    let app = APP.get().ok_or("The launcher is still starting")?;
    tauri_plugin_hopper_android::call(app, command, payload)
}

/// Is an installed copy of this app actually there? On Android that means "is its package on the phone".
#[cfg(target_os = "android")]
fn is_present(s: &InstalledApp) -> bool {
    let Some(pkg) = s.package.as_deref() else { return false };
    android_call("isInstalled", serde_json::json!({ "packageName": pkg }))
        .map(|v| v["installed"].as_bool().unwrap_or(false))
        .unwrap_or(false)
}

#[cfg(not(target_os = "android"))]
fn is_present(s: &InstalledApp) -> bool {
    s.path.is_file()
}

fn state_path() -> PathBuf {
    data_dir().join("state.json")
}

/// A user-edited manifest in the config folder overrides the bundled one.
fn load_manifest() -> Result<Vec<AppEntry>, String> {
    let text = fs::read_to_string(config_dir().join("apps.json")).unwrap_or_else(|_| BUNDLED_MANIFEST.to_string());
    serde_json::from_str::<Manifest>(&text)
        .map(|m| m.apps)
        .map_err(|e| format!("apps.json is invalid: {e}"))
}

fn find_app(id: &str) -> Result<AppEntry, String> {
    load_manifest()?
        .into_iter()
        .find(|a| a.id == id)
        .ok_or_else(|| format!("unknown app: {id}"))
}

fn load_state() -> State {
    fs::read_to_string(state_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_state(state: &State) -> Result<(), String> {
    fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    fs::write(state_path(), text).map_err(|e| e.to_string())
}

// ---------- GitHub ----------

/// The last answer GitHub gave for each repo, with its ETag. Asking again with
/// "only if changed" costs GitHub almost nothing and does not use up the rate limit.
static RELEASE_CACHE: std::sync::Mutex<Option<HashMap<String, (String, Release)>>> = std::sync::Mutex::new(None);

fn cached_release(repo: &str) -> Option<(String, Release)> {
    RELEASE_CACHE.lock().ok()?.as_ref()?.get(repo).cloned()
}

fn latest_release(repo: &str) -> Result<Release, String> {
    let url = format!("{}/repos/{repo}/releases/latest", api_base());
    let cached = cached_release(repo);
    let mut req = ureq::get(&url)
        .set("User-Agent", "hopper-launcher")
        .set("Accept", "application/vnd.github+json");
    if let Some((etag, _)) = &cached {
        req = req.set("If-None-Match", etag);
    }
    let resp = req.call().map_err(|e| match e {
        ureq::Error::Status(404, _) => "No release published yet".to_string(),
        ureq::Error::Status(403 | 429, _) => "GitHub rate limit reached, try again later".to_string(),
        other => format!("Could not reach GitHub: {other}"),
    })?;
    if resp.status() == 304 {
        return cached.map(|(_, r)| r).ok_or_else(|| "GitHub said nothing changed, but there is nothing saved".to_string());
    }
    let etag = resp.header("etag").map(str::to_string);
    let release: Release = resp.into_json().map_err(|e| e.to_string())?;
    if let (Some(etag), Ok(mut cache)) = (etag, RELEASE_CACHE.lock()) {
        cache.get_or_insert_with(HashMap::new).insert(repo.to_string(), (etag, release.clone()));
    }
    Ok(release)
}

/// Writes through to a file while hashing everything that passes.
struct HashingWriter<W: Write> {
    inner: W,
    hasher: Sha256,
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn download(url: &str, dest: &Path) -> Result<String, String> {
    let resp = ureq::get(url)
        .set("User-Agent", "hopper-launcher")
        .call()
        .map_err(|e| format!("Download failed: {e}"))?;
    let file = fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut w = HashingWriter { inner: file, hasher: Sha256::new() };
    io::copy(&mut resp.into_reader(), &mut w).map_err(|e| format!("Download failed: {e}"))?;
    w.flush().map_err(|e| e.to_string())?;
    Ok(to_hex(&w.hasher.finalize()))
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Finds the expected SHA-256 for `file` in the text of a checksum file.
fn expected_hash(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        let name = name.rsplit('/').next().unwrap_or(name);
        (name.eq_ignore_ascii_case(file) && hash.len() == 64).then(|| hash.to_lowercase())
    })
}

/// Some(true) = matched, Some(false) = a published checksum did not match, None = none published.
fn verify(release: &Release, file: &str, actual: &str) -> Option<bool> {
    for a in release.assets.iter().filter(|a| a.name.to_lowercase().contains("sha256")) {
        let Ok(resp) = ureq::get(&a.browser_download_url).set("User-Agent", "hopper-launcher").call() else { continue };
        let mut text = String::new();
        if resp.into_reader().take(1 << 20).read_to_string(&mut text).is_err() {
            continue;
        }
        if let Some(want) = expected_hash(&text, file) {
            return Some(want == actual);
        }
    }
    None
}

// ---------- installing ----------

fn unzip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(fs::File::open(zip_path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        // enclosed_name rejects paths that would escape the destination folder.
        let Some(rel) = entry.enclosed_name() else { continue };
        let out = dest.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut f = fs::File::create(&out).map_err(|e| e.to_string())?;
            io::copy(&mut entry, &mut f).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// The shallowest .exe under `dir`.
fn find_exe(dir: &Path) -> Option<PathBuf> {
    let mut level = vec![dir.to_path_buf()];
    while !level.is_empty() {
        let mut next = Vec::new();
        let mut found: Vec<PathBuf> = Vec::new();
        for d in &level {
            let Ok(rd) = fs::read_dir(d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    next.push(p);
                } else if p.extension().map_or(false, |x| x.eq_ignore_ascii_case("exe")) {
                    found.push(p);
                }
            }
        }
        if let Some(first) = found.into_iter().min() {
            return Some(first);
        }
        level = next;
    }
    None
}

fn make_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// When the launcher runs as an AppImage, its start-up script points many environment
/// variables (PYTHONHOME, LD_LIBRARY_PATH, GTK_PATH, PATH ...) at the launcher's own
/// bundled files. An app started from here must not inherit those, or it looks for its
/// libraries in the wrong place and fails to start. This removes exactly what points
/// into the launcher's own AppImage and keeps everything that came from the system.
#[cfg_attr(target_os = "android", allow(dead_code))]
fn scrub_appimage_env(cmd: &mut Command) {
    let Some(appdir) = std::env::var("APPDIR").ok().map(|d| d.trim_end_matches('/').to_string()).filter(|d| !d.is_empty()) else {
        return; // not running as an AppImage: nothing was added
    };
    let inside = |p: &str| p == appdir || p.starts_with(&format!("{appdir}/"));
    for (key, value) in std::env::vars_os() {
        let Some(value) = value.to_str() else { continue };
        if !value.split(':').any(|p| inside(p)) {
            continue;
        }
        let kept: Vec<&str> = value.split(':').filter(|p| !p.is_empty() && !inside(p)).collect();
        if kept.is_empty() {
            cmd.env_remove(&key);
        } else {
            cmd.env(&key, kept.join(":"));
        }
    }
    // Set by the AppImage runtime itself, only meaningful for the launcher.
    for key in ["APPDIR", "APPIMAGE", "ARGV0", "OWD"] {
        cmd.env_remove(key);
    }
    // The GTK helper in the AppImage forces this when nothing else did.
    if std::env::var("GTK_THEME").map_or(false, |t| t == "Adwaita:light") {
        cmd.env_remove("GTK_THEME");
    }
}

fn open_with_system(target: &str) -> Result<(), String> {
    #[cfg(target_os = "android")]
    return android_call("openUrl", serde_json::json!({ "url": target })).map(|_| ());

    #[cfg(target_os = "linux")]
    let mut cmd = Command::new("xdg-open");
    #[cfg(target_os = "macos")]
    let mut cmd = Command::new("open");
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows", target_os = "android")))]
    return Err(format!("Opening {target} is not supported on this system"));

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        scrub_appimage_env(&mut cmd);
        cmd.arg(target)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("Could not open {target}: {e}"))
    }
}

/// Deletes release files (old versions of the program) directly inside `dir`, nothing else.
fn remove_old_builds(dir: &Path) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let is_build = e.file_name().to_str().map_or(false, |n| classify::classify(n).is_some());
        if p.is_file() && is_build {
            let _ = fs::remove_file(&p);
        }
    }
}

fn install_blocking(app: &AppEntry, file: &str) -> Result<InstallResult, String> {
    let dev = device::detect();
    let release = latest_release(&app.repo)?;
    // Only files that really are builds for this device can be installed.
    let build = builds_for(&release.assets, &dev.os, &dev.arch)
        .into_iter()
        .find(|b| b.file == file)
        .ok_or_else(|| format!("{file} is not a build for this device"))?;
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == file)
        .ok_or_else(|| format!("{file} is missing from the release"))?;

    let app_dir = data_dir().join("apps").join(&app.id);
    let part_dir = app_dir.join(".part");
    let _ = fs::remove_dir_all(&part_dir);
    fs::create_dir_all(&part_dir).map_err(|e| e.to_string())?;
    let part_file = part_dir.join(file);

    let fail = |msg: String| -> String {
        let _ = fs::remove_dir_all(&part_dir);
        msg
    };

    let actual = download(&asset.browser_download_url, &part_file).map_err(&fail)?;
    let verified = match verify(&release, file, &actual) {
        Some(true) => true,
        Some(false) => return Err(fail("The download did not match its published checksum, so it was discarded.".into())),
        None => false,
    };

    // The new version is in hand and checked. Replace the old program files only:
    // anything else in the folder (settings a portable build keeps beside itself,
    // an AppImage's .home/.config folders) belongs to the app and stays untouched.
    remove_old_builds(&app_dir);

    let handed_off = is_handoff(&build.kind);
    let launch_path = if build.kind == "zip" {
        let target = app_dir.join("app");
        fs::create_dir_all(&target).map_err(|e| fail(e.to_string()))?;
        unzip(&part_file, &target).map_err(&fail)?;
        find_exe(&target).ok_or_else(|| fail("The zip does not contain a program (.exe)".into()))?
    } else {
        let dest = app_dir.join(file);
        fs::rename(&part_file, &dest).map_err(|e| fail(e.to_string()))?;
        make_executable(&dest).map_err(&fail)?;
        dest
    };
    let _ = fs::remove_dir_all(&part_dir);

    let mut state = load_state();
    #[cfg(target_os = "android")]
    if handed_off {
        // Read the package name from the APK, remember it, then open Android's installer.
        // Whether the app is installed is checked live afterwards, so a cancelled install is fine.
        let path_text = launch_path.to_string_lossy().into_owned();
        let pkg = android_call("apkPackageName", serde_json::json!({ "path": path_text }))?["packageName"]
            .as_str()
            .map(str::to_string)
            .ok_or("Could not read the app's package name")?;
        state.insert(
            app.id.clone(),
            InstalledApp { tag: release.tag_name.clone(), path: launch_path.clone(), folder: false, adopted: false, package: Some(pkg), ignored_version: None },
        );
        save_state(&state)?;
        android_call("installApk", serde_json::json!({ "path": path_text }))?;
        return Ok(InstallResult { tag: release.tag_name, verified, handed_off: true });
    }
    if handed_off {
        // The system's installer owns this app from here on.
        state.remove(&app.id);
        save_state(&state)?;
        open_with_system(&launch_path.to_string_lossy())?;
    } else {
        state.insert(
            app.id.clone(),
            InstalledApp { tag: release.tag_name.clone(), path: launch_path, folder: build.kind == "zip", adopted: false, package: None, ignored_version: None },
        );
        save_state(&state)?;
    }
    Ok(InstallResult { tag: release.tag_name, verified, handed_off })
}

// ---------- settings ----------

fn default_mode() -> String {
    "auto".into()
}

fn default_true() -> bool {
    true
}

/// How the launcher treats apps the user installed before the launcher existed.
///   "auto"     search the usual folders and ask before using each find
///   "manual"   never search; the user points at each app in Settings
///   "in_place" use existing apps where they are and never download a copy of them
#[derive(Deserialize, Serialize)]
struct Settings {
    #[serde(default = "default_mode")]
    existing_mode: String,
    /// Only used by "in_place": also search the usual folders.
    #[serde(default = "default_true")]
    search_folders: bool,
    /// Extra folders the user wants searched for existing installs.
    #[serde(default)]
    extra_folders: Vec<PathBuf>,
    #[serde(default)]
    first_run_done: bool,
    /// The order the user dragged the apps into. Empty means alphabetical.
    #[serde(default)]
    app_order: Vec<String>,
    /// Show the Internet Archive backup. Off until the user turns it on.
    #[serde(default)]
    archive_enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { existing_mode: default_mode(), search_folders: true, extra_folders: vec![], first_run_done: false, app_order: vec![], archive_enabled: false }
    }
}

fn scanning_enabled(s: &Settings) -> bool {
    if cfg!(target_os = "android") {
        return false; // phones have no folders of loose app files to search
    }
    match s.existing_mode.as_str() {
        "auto" => true,
        "in_place" => s.search_folders,
        _ => false,
    }
}

/// In "in_place" mode the launcher never replaces a copy the user brought with its own download.
fn check_update_allowed(id: &str) -> Result<(), String> {
    let adopted = load_state().get(id).map_or(false, |s| s.adopted);
    if adopted && load_settings().existing_mode == "in_place" {
        return Err("Your setting keeps this app where it is. Update it yourself, or change the setting.".into());
    }
    Ok(())
}

fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

fn load_settings() -> Settings {
    fs::read_to_string(settings_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_settings(settings: &Settings) -> Result<(), String> {
    fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(settings_path(), text).map_err(|e| e.to_string())
}

/// The usual places people keep downloaded apps.
fn default_scan_folders() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        // Downloads and Desktop are listed explicitly because the system's own lookup
        // returns nothing on setups without an XDG user-dirs file.
        for sub in ["Applications", "AppImages", "bin", ".local/bin", "Downloads", "Desktop"] {
            v.push(home.join(sub));
        }
    }
    // The system's own locations too, for translated or moved folders.
    v.extend(dirs::download_dir());
    v.extend(dirs::desktop_dir());
    let mut seen = std::collections::HashSet::new();
    v.retain(|p| seen.insert(p.clone()));
    v
}

// ---------- Internet Archive backup ----------
//
// A safety net, not a second store: when it is switched on the user can browse what has been
// preserved on the Internet Archive and download a file by hand. Nothing here runs unless the
// user turned it on, and nothing is ever chosen or replaced automatically.

fn archive_base() -> String {
    std::env::var("HOPPER_ARCHIVE_BASE").unwrap_or_else(|_| "https://archive.org".into())
}

/// Item names come from apps.json; refuse anything that could change the address.
fn valid_item(id: &str) -> bool {
    !id.is_empty() && id.len() <= 100 && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[derive(Serialize, Clone, Debug, PartialEq)]
struct ArchiveFile {
    name: String,
    size: u64,
    /// When the file was last changed, as Unix seconds.
    mtime: Option<i64>,
    version: Option<String>,
    /// "linux", "windows", "android", "console" or "any".
    os: String,
    arch: String,
    /// What it is, for showing: "AppImage", "Installer", "Source code" ...
    kind: String,
    /// "program", "source", "packaging" or "other".
    group: String,
    debug: bool,
}

#[derive(Serialize, Debug)]
struct ArchiveItem {
    app_id: String,
    app_name: String,
    item: Option<String>,
    files: Vec<ArchiveFile>,
    error: Option<String>,
}

#[derive(Serialize, Debug)]
struct ArchiveDownload {
    path: String,
    size: u64,
    /// Which checks passed, for example "Internet Archive checksum".
    verified: Vec<String>,
    /// True when the file was handed to the system's installer (Android APKs).
    handed_off: bool,
}

struct ArchiveClass {
    os: &'static str,
    arch: &'static str,
    kind: String,
    group: &'static str,
    debug: bool,
}

/// Says what a file in an archive item is. None means "not something to list": the archive's own
/// housekeeping files, checksum files and update helpers.
fn describe_archive_file(name: &str) -> Option<ArchiveClass> {
    let f = name.to_lowercase();
    let housekeeping = ["_files.xml", "_meta.xml", "_meta.sqlite", "__ia_thumb.jpg", ".torrent"];
    if housekeeping.iter().any(|h| f.ends_with(h)) || f.contains("sha256") || [".sig", ".asc", ".zsync"].iter().any(|e| f.ends_with(e)) {
        return None;
    }
    let debug = f.contains("debug");
    let arch = if f.contains("aarch64") || f.contains("arm64") {
        "aarch64"
    } else if ["x86_64", "x64", "amd64", "win64"].iter().any(|t| f.contains(t)) {
        "x86_64"
    } else {
        "any"
    };
    let program = |os: &'static str, kind: &str| ArchiveClass { os, arch, kind: kind.into(), group: "program", debug };
    let other = |group: &'static str, kind: &str| ArchiveClass { os: "any", arch: "any", kind: kind.into(), group, debug };
    Some(if f.ends_with(".appimage") {
        program("linux", "AppImage")
    } else if f.ends_with(".deb") {
        program("linux", ".deb")
    } else if f.ends_with(".rpm") {
        program("linux", ".rpm")
    } else if f.ends_with(".flatpak") {
        program("linux", "Flatpak")
    } else if f.ends_with(".pkg.tar.zst") {
        program("linux", "Arch package")
    } else if f.ends_with(".run") {
        program("linux", "Installer (.run)")
    } else if f.ends_with(".apk") {
        program("android", "APK")
    } else if f.ends_with(".msi") {
        program("windows", "MSI")
    } else if f.ends_with(".exe") {
        if f.contains("setup") || f.contains("install") {
            program("windows", "Installer")
        } else if f.contains("portable") {
            program("windows", "Portable")
        } else {
            program("windows", "Program (.exe)")
        }
    } else if f.ends_with(".zip") && f.contains("win") {
        program("windows", "Zip")
    } else if f.ends_with(".3dsx") || f.ends_with(".cia") {
        program("console", "3DS")
    } else if f.ends_with(".nro") {
        program("console", "Switch")
    } else if f.ends_with(".rpx") || f.ends_with(".wuhb") {
        program("console", "Wii U")
    } else if [".tar.gz", ".tar.xz", ".tar.zst", ".zip"].iter().any(|e| f.ends_with(e)) {
        if ["aur", "flathub", "fdroid", "flatpak", "kit", "handoff"].iter().any(|t| f.contains(t)) {
            other("packaging", "Packaging files")
        } else {
            other("source", "Source code")
        }
    } else {
        other("other", "Other file")
    })
}

/// Sort key for versions like "0.1.15.0": compared number by number, newest first once reversed.
fn version_key(v: &Option<String>) -> Vec<u64> {
    v.as_deref().map(|s| s.split('.').filter_map(|p| p.parse().ok()).collect()).unwrap_or_default()
}

fn json_number(v: &serde_json::Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn archive_files_from(meta: &serde_json::Value) -> Vec<ArchiveFile> {
    let Some(files) = meta.get("files").and_then(|f| f.as_array()) else { return vec![] };
    let mut out: Vec<ArchiveFile> = files
        .iter()
        .filter_map(|f| {
            let name = f.get("name")?.as_str()?;
            let base = name.rsplit('/').next().unwrap_or(name);
            let class = describe_archive_file(base)?;
            Some(ArchiveFile {
                name: name.to_string(),
                size: f.get("size").and_then(json_number).unwrap_or(0).max(0) as u64,
                mtime: f.get("mtime").and_then(json_number),
                version: version_from_name(name),
                os: class.os.into(),
                arch: class.arch.into(),
                kind: class.kind,
                group: class.group.into(),
                debug: class.debug,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        version_key(&b.version).cmp(&version_key(&a.version)).then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

fn fetch_archive_metadata(item: &str) -> Result<serde_json::Value, String> {
    if !valid_item(item) {
        return Err("That item name is not valid".into());
    }
    let url = format!("{}/metadata/{item}", archive_base());
    let resp = ureq::get(&url).set("User-Agent", "hopper-launcher").call().map_err(|e| match e {
        ureq::Error::Status(404, _) => "Not found on the Internet Archive".to_string(),
        ureq::Error::Status(403 | 429, _) => "The Internet Archive asked us to slow down. Try again later.".to_string(),
        other => format!("Could not reach the Internet Archive: {other}"),
    })?;
    let meta: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
    if meta.get("files").is_none() {
        return Err("This item does not exist yet, or is empty".into()); // a missing item answers with {}
    }
    Ok(meta)
}

/// Every app's archived files. Only runs when the backup is switched on.
fn archive_overview() -> Result<Vec<ArchiveItem>, String> {
    if !load_settings().archive_enabled {
        return Err("The Internet Archive backup is switched off.".into());
    }
    let mut apps = load_manifest()?;
    apps.sort_by_key(|a| a.name.to_lowercase());
    Ok(std::thread::scope(|s| {
        let handles: Vec<_> = apps
            .iter()
            .map(|app| {
                s.spawn(move || {
                    let mut item = ArchiveItem {
                        app_id: app.id.clone(),
                        app_name: app.name.clone(),
                        item: app.archive.clone(),
                        files: vec![],
                        error: None,
                    };
                    match &app.archive {
                        None => item.error = Some("Not set up yet".into()),
                        Some(id) => match fetch_archive_metadata(id) {
                            Ok(meta) => item.files = archive_files_from(&meta),
                            Err(e) => item.error = Some(e),
                        },
                    }
                    item
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    }))
}

/// Where hand-downloaded files go: the user's Downloads folder.
fn save_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("HOPPER_DOWNLOAD_DIR") {
        return PathBuf::from(d);
    }
    #[cfg(target_os = "android")]
    return data_dir().join("downloads");
    #[cfg(not(target_os = "android"))]
    dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .unwrap_or_else(|| data_dir().join("downloads"))
}

/// "name.ext" that does not exist yet: adds " (1)", " (2)" ... before the extension when needed.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("an unused name exists")
}

/// Writes through to a file while hashing with both algorithms and reporting progress.
struct ArchiveWriter<W: Write, F: FnMut(u64)> {
    inner: W,
    sha256: Sha256,
    sha1: sha1::Sha1,
    done: u64,
    report: F,
}

impl<W: Write, F: FnMut(u64)> Write for ArchiveWriter<W, F> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.sha256.update(&buf[..n]);
        self.sha1.update(&buf[..n]);
        self.done += n as u64;
        (self.report)(self.done);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Downloads one file of an item by hand, checks it, and saves it in the Downloads folder.
fn archive_download_blocking(app: &AppEntry, file: &str, mut progress: impl FnMut(u64, u64)) -> Result<ArchiveDownload, String> {
    let item = app.archive.as_deref().ok_or_else(|| format!("{} has no Internet Archive item yet", app.name))?;
    let meta = fetch_archive_metadata(item)?;
    let entries = meta["files"].as_array().cloned().unwrap_or_default();
    // Only a file the item really lists, and only a real file (not the archive's housekeeping).
    let entry = entries
        .iter()
        .find(|f| f["name"].as_str() == Some(file))
        .ok_or_else(|| format!("{file} is not in the item"))?;
    let base = file.rsplit('/').next().unwrap_or(file).to_string();
    describe_archive_file(&base).ok_or_else(|| "That file cannot be downloaded from here".to_string())?;
    let total = entry.get("size").and_then(json_number).unwrap_or(0).max(0) as u64;
    let want_sha1 = entry["sha1"].as_str().map(str::to_lowercase);

    let dir = save_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let dest = unique_path(&dir, &base);
    let part = dest.with_file_name(format!("{}.part", dest.file_name().and_then(|n| n.to_str()).unwrap_or("download")));

    let url = format!("{}/download/{item}/{}", archive_base(), percent_encode(file));
    let resp = ureq::get(&url).set("User-Agent", "hopper-launcher").call().map_err(|e| format!("Download failed: {e}"))?;
    let (sha256, sha1) = {
        let mut w = ArchiveWriter {
            inner: fs::File::create(&part).map_err(|e| e.to_string())?,
            sha256: Sha256::new(),
            sha1: <sha1::Sha1 as Digest>::new(),
            done: 0,
            report: |done| progress(done, total.max(done)),
        };
        let copied = io::copy(&mut resp.into_reader(), &mut w);
        let flushed = w.flush();
        if let Err(e) = copied.and(flushed) {
            let _ = fs::remove_file(&part);
            return Err(format!("Download failed: {e}"));
        }
        (to_hex(&w.sha256.finalize()), to_hex(&w.sha1.finalize()))
    };

    let discard = |why: &str| -> String {
        let _ = fs::remove_file(&part);
        why.to_string()
    };
    let mut verified: Vec<String> = Vec::new();
    if let Some(want) = want_sha1 {
        if want != sha1 {
            return Err(discard("The download did not match the Internet Archive's checksum, so it was discarded."));
        }
        verified.push("Internet Archive checksum".into());
    }
    // A checksum file the developer uploaded alongside, when it lists this file.
    for e in entries.iter().filter(|f| f["name"].as_str().map_or(false, |n| n.to_lowercase().contains("sha256")) && f.get("size").and_then(json_number).unwrap_or(0) < (1 << 20)) {
        let Some(sums_name) = e["name"].as_str() else { continue };
        let sums_url = format!("{}/download/{item}/{}", archive_base(), percent_encode(sums_name));
        let Ok(r) = ureq::get(&sums_url).set("User-Agent", "hopper-launcher").call() else { continue };
        let mut text = String::new();
        if r.into_reader().take(1 << 20).read_to_string(&mut text).is_err() {
            continue;
        }
        if let Some(want) = expected_hash(&text, &base) {
            if want != sha256 {
                return Err(discard("The download did not match the published SHA256SUMS, so it was discarded."));
            }
            verified.push("SHA256SUMS".into());
            break;
        }
    }

    fs::rename(&part, &dest).map_err(|e| e.to_string())?;
    if base.to_lowercase().ends_with(".appimage") {
        make_executable(&dest)?;
    }
    #[allow(unused_mut)]
    let mut handed_off = false;
    #[cfg(target_os = "android")]
    if base.to_lowercase().ends_with(".apk") {
        android_call("installApk", serde_json::json!({ "path": dest.to_string_lossy() }))?;
        handed_off = true;
    }
    let size = fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    Ok(ArchiveDownload { path: dest.to_string_lossy().into_owned(), size, verified, handed_off })
}

// ---------- existing installs ----------

#[derive(Serialize, Debug, PartialEq)]
struct Candidate {
    app_id: String,
    app_name: String,
    path: String,
    version: Option<String>,
}

/// Lowercase letters and digits only, so "Hop-Tunes_1.0" and "hoptunes" compare equal.
fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

/// The first dotted version number in a file name, like "0.1.14".
fn version_from_name(name: &str) -> Option<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_ascii_digit()) {
            let mut j = i;
            let mut dots = 0;
            loop {
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                if j + 1 < chars.len() && chars[j] == '.' && chars[j + 1].is_ascii_digit() {
                    dots += 1;
                    j += 1;
                } else {
                    break;
                }
            }
            if dots > 0 {
                return Some(chars[i..j].iter().collect());
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

fn is_program_file(name: &str) -> bool {
    let n = name.to_lowercase();
    if n.contains("debug") {
        return false;
    }
    if cfg!(windows) {
        n.ends_with(".exe") && !["setup", "install", "unins"].iter().any(|w| n.contains(w))
    } else {
        n.ends_with(".appimage")
    }
}

/// Looks one level deep in each folder for program files named after one of the apps.
fn scan_folders(apps: &[AppEntry], folders: &[PathBuf], skip: &Path) -> Vec<Candidate> {
    let mut out = Vec::new();
    for folder in folders {
        let Ok(rd) = fs::read_dir(folder) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else { continue };
            if !path.is_file() || path.starts_with(skip) || !is_program_file(&name) {
                continue;
            }
            let stem = squash(&name);
            for app in apps {
                if stem.contains(&squash(&app.id)) || stem.contains(&squash(&app.name)) {
                    out.push(Candidate {
                        app_id: app.id.clone(),
                        app_name: app.name.clone(),
                        path: path.to_string_lossy().into_owned(),
                        version: version_from_name(&name),
                    });
                    break;
                }
            }
        }
    }
    out.sort_by(|a, b| a.app_name.to_lowercase().cmp(&b.app_name.to_lowercase()).then(a.path.cmp(&b.path)));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

/// Starts using a copy the user already has, exactly where it is.
fn adopt_entry(app: &AppEntry, path: &Path) -> Result<(), String> {
    if cfg!(target_os = "android") {
        return Err("Android manages installed apps itself, so this is not needed there.".into());
    }
    let mut state = load_state();
    if let Some(existing) = state.get(&app.id) {
        if !existing.adopted && is_present(existing) {
            return Err(format!("{} is already installed by the launcher. Uninstall that copy first to use your own.", app.name));
        }
    }
    let meta = fs::metadata(path).map_err(|e| format!("Could not read that file: {e}"))?;
    if !meta.is_file() {
        return Err("That is not a file.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err("That file is not marked as runnable. Allow it to run as a program in its Properties, then try again.".into());
        }
    }
    let file = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    state.insert(
        app.id.clone(),
        InstalledApp {
            tag: version_from_name(file).unwrap_or_default(),
            path: path.to_path_buf(),
            folder: false,
            adopted: true,
            package: None,
            ignored_version: None,
        },
    );
    save_state(&state)
}

// ---------- commands ----------

#[tauri::command]
fn get_device() -> device::Device {
    device::detect()
}

fn list_apps_from(entries: Vec<AppEntry>) -> Vec<AppInfo> {
    let state = load_state();
    let mut apps: Vec<AppInfo> = entries
        .into_iter()
        .map(|entry| {
            let managed = state.get(&entry.id).filter(|s| is_present(s));
            AppInfo {
                installed: managed.is_some(),
                version: managed.map(|s| s.tag.clone()),
                folder: managed.map_or(false, |s| s.folder),
                adopted: managed.map_or(false, |s| s.adopted),
                ignored_version: managed.and_then(|s| s.ignored_version.clone()),
                path: managed.map(|s| s.path.to_string_lossy().into_owned()),
                entry,
            }
        })
        .collect();
    apps.sort_by_key(|a| a.entry.name.to_lowercase());
    apply_order(&mut apps, &load_settings().app_order);
    apps
}

/// Puts the apps in the user's own order. Apps not in that list (for example a newly
/// added one) follow, still in alphabetical order, because the sort is stable.
fn apply_order(apps: &mut [AppInfo], order: &[String]) {
    apps.sort_by_key(|a| order.iter().position(|id| *id == a.entry.id).unwrap_or(usize::MAX));
}

/// Keeps only apps that exist, each once, in the order given.
fn clean_order(ids: Vec<String>, known: &[AppEntry]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        if known.iter().any(|a| a.id == id) && !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

#[tauri::command]
fn list_apps() -> Result<Vec<AppInfo>, String> {
    Ok(list_apps_from(load_manifest()?))
}

/// Reads every app's latest release. Network only happens when this is called
/// (startup and the refresh button), never in the background.
#[tauri::command]
async fn check_releases() -> Result<Vec<ReleaseInfo>, String> {
    let apps = load_manifest()?;
    let dev = device::detect();
    tauri::async_runtime::spawn_blocking(move || {
        std::thread::scope(|s| {
            let handles: Vec<_> = apps
                .iter()
                .map(|app| {
                    let dev = &dev;
                    s.spawn(move || match latest_release(&app.repo) {
                        Ok(r) => ReleaseInfo {
                            id: app.id.clone(),
                            builds: builds_for(&r.assets, &dev.os, &dev.arch),
                            tag: Some(r.tag_name),
                            error: None,
                        },
                        Err(e) => ReleaseInfo { id: app.id.clone(), tag: None, builds: vec![], error: Some(e) },
                    })
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        })
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn install_app(id: String, file: String) -> Result<InstallResult, String> {
    check_update_allowed(&id)?;
    let app = find_app(&id)?;
    tauri::async_runtime::spawn_blocking(move || install_blocking(&app, &file))
        .await
        .map_err(|e| e.to_string())?
}

fn launch_entry(app: &AppEntry) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        let pkg = load_state()
            .get(&app.id)
            .and_then(|s| s.package.clone())
            .ok_or_else(|| format!("{} is not installed", app.name))?;
        return android_call("launchPackage", serde_json::json!({ "packageName": pkg })).map(|_| ());
    }

    #[cfg(not(target_os = "android"))]
    {
        launch_file(app)
    }
}

#[cfg(not(target_os = "android"))]
fn launch_file(app: &AppEntry) -> Result<(), String> {
    let target = load_state()
        .get(&app.id)
        .filter(|s| is_present(s))
        .map(|s| s.path.clone())
        .ok_or_else(|| format!("{} is not installed", app.name))?;

    let mut cmd = Command::new(target);
    scrub_appimage_env(&mut cmd);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // Put the app in its own process group (Windows: detached) so closing the
    // launcher, or the terminal it was started from, never takes the app with it.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    let mut child = cmd.spawn().map_err(|e| format!("Could not start {}: {e}", app.name))?;
    // Reap the child when it exits so it never lingers as a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Remembers that the user does not want to be asked about this release.
fn skip_entry(app: &AppEntry, tag: &str) -> Result<(), String> {
    let mut state = load_state();
    let entry = state.get_mut(&app.id).ok_or_else(|| format!("{} is not installed", app.name))?;
    entry.ignored_version = Some(tag.to_string());
    save_state(&state)
}

/// Lists what is preserved on the Internet Archive. Only works while the backup is switched on.
#[tauri::command]
async fn archive_list() -> Result<Vec<ArchiveItem>, String> {
    tauri::async_runtime::spawn_blocking(archive_overview).await.map_err(|e| e.to_string())?
}

/// Downloads one archived file by hand, reporting progress as it goes.
#[tauri::command]
async fn archive_download(app: tauri::AppHandle, id: String, file: String) -> Result<ArchiveDownload, String> {
    use tauri::Emitter;
    if !load_settings().archive_enabled {
        return Err("The Internet Archive backup is switched off.".into());
    }
    let entry = find_app(&id)?;
    let name = file.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut last = 0u64;
        archive_download_blocking(&entry, &file, |done, total| {
            if done >= total || done.saturating_sub(last) >= 512 * 1024 {
                last = done;
                let _ = app.emit("archive-progress", serde_json::json!({ "file": name, "done": done, "total": total }));
            }
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The developer's page on the Internet Archive, where the backups are kept. The address is fixed
/// here, so the page can never ask for a different one.
const ARCHIVE_PROFILE_URL: &str = "https://archive.org/details/@gameh0pper";

#[tauri::command]
fn open_archive_profile() -> Result<(), String> {
    open_with_system(ARCHIVE_PROFILE_URL)
}

#[tauri::command]
fn set_archive_enabled(enabled: bool) -> Result<SettingsView, String> {
    let mut settings = load_settings();
    settings.archive_enabled = enabled;
    save_settings(&settings)?;
    Ok(settings_view())
}

#[tauri::command]
fn skip_update(id: String, tag: String) -> Result<(), String> {
    skip_entry(&find_app(&id)?, &tag)
}

#[tauri::command]
fn launch_app(id: String) -> Result<(), String> {
    launch_entry(&find_app(&id)?)
}

/// Removes the program. A single-file install leaves any settings folders the app
/// made next to it (the folder is removed only if nothing else is left in it).
/// A portable folder install is removed whole, so the UI warns about that first.
#[cfg_attr(target_os = "android", allow(unreachable_code))]
fn uninstall_entry(app: &AppEntry) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        // Android asks the user to confirm. The app disappears from the list once it is really gone.
        let pkg = load_state().get(&app.id).and_then(|s| s.package.clone());
        return match pkg {
            Some(pkg) => android_call("uninstallPackage", serde_json::json!({ "packageName": pkg })).map(|_| ()),
            None => Ok(()),
        };
    }

    let mut state = load_state();
    let entry = state.remove(&app.id);
    save_state(&state)?;
    if entry.as_ref().map_or(false, |e| e.adopted) {
        return Ok(()); // the user's own file: just stop tracking it
    }
    let dir = data_dir().join("apps").join(&app.id);
    if entry.map_or(false, |e| e.folder) {
        let _ = fs::remove_dir_all(&dir);
    } else {
        remove_old_builds(&dir);
        let _ = fs::remove_dir(&dir); // only succeeds when empty
    }
    Ok(())
}

#[tauri::command]
fn uninstall_app(id: String) -> Result<(), String> {
    uninstall_entry(&find_app(&id)?)
}

#[derive(Serialize)]
struct FolderInfo {
    path: String,
    custom: bool,
    exists: bool,
}

#[derive(Serialize)]
struct SettingsView {
    /// True until the first-run existing-install check has been shown once.
    first_run: bool,
    /// True when the user has dragged the apps into their own order.
    custom_order: bool,
    /// Whether the Internet Archive backup is switched on.
    archive_enabled: bool,
    existing_mode: String,
    search_folders: bool,
    /// Whether the launcher will search folders at all under the current choice.
    scanning: bool,
    folders: Vec<FolderInfo>,
}

fn settings_view() -> SettingsView {
    let settings = load_settings();
    let mut folders: Vec<FolderInfo> = default_scan_folders()
        .into_iter()
        .map(|p| FolderInfo { exists: p.is_dir(), path: p.to_string_lossy().into_owned(), custom: false })
        .collect();
    folders.extend(
        settings
            .extra_folders
            .iter()
            .map(|p| FolderInfo { exists: p.is_dir(), path: p.to_string_lossy().into_owned(), custom: true }),
    );
    SettingsView {
        first_run: !settings.first_run_done,
        custom_order: !settings.app_order.is_empty(),
        archive_enabled: settings.archive_enabled,
        scanning: scanning_enabled(&settings),
        existing_mode: settings.existing_mode,
        search_folders: settings.search_folders,
        folders,
    }
}

#[tauri::command]
fn get_settings() -> SettingsView {
    settings_view()
}

#[tauri::command]
fn set_existing_mode(mode: String, search_folders: bool) -> Result<SettingsView, String> {
    if !["auto", "manual", "in_place"].contains(&mode.as_str()) {
        return Err(format!("unknown choice: {mode}"));
    }
    let mut settings = load_settings();
    settings.existing_mode = mode;
    settings.search_folders = search_folders;
    save_settings(&settings)?;
    Ok(settings_view())
}

#[tauri::command]
fn set_app_order(order: Vec<String>) -> Result<(), String> {
    let known = load_manifest()?;
    let mut settings = load_settings();
    settings.app_order = clean_order(order, &known);
    save_settings(&settings)
}

#[tauri::command]
fn reset_app_order() -> Result<(), String> {
    let mut settings = load_settings();
    settings.app_order.clear();
    save_settings(&settings)
}

#[tauri::command]
fn finish_first_run() -> Result<(), String> {
    let mut settings = load_settings();
    settings.first_run_done = true;
    save_settings(&settings)
}

#[tauri::command]
fn add_scan_folder(path: String) -> Result<SettingsView, String> {
    let p = PathBuf::from(&path);
    if !p.is_dir() {
        return Err("That is not a folder.".into());
    }
    let mut settings = load_settings();
    if !settings.extra_folders.contains(&p) && !default_scan_folders().contains(&p) {
        settings.extra_folders.push(p);
        save_settings(&settings)?;
    }
    Ok(settings_view())
}

#[tauri::command]
fn remove_scan_folder(path: String) -> Result<SettingsView, String> {
    let mut settings = load_settings();
    settings.extra_folders.retain(|p| p.to_string_lossy() != path);
    save_settings(&settings)?;
    Ok(settings_view())
}

/// Existing copies of your apps found in the searched folders (apps already set up are skipped).
#[tauri::command]
async fn scan_existing() -> Result<Vec<Candidate>, String> {
    let apps = load_manifest()?;
    let settings = load_settings();
    if !scanning_enabled(&settings) {
        return Ok(vec![]);
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state = load_state();
        let mut folders = default_scan_folders();
        folders.extend(settings.extra_folders);
        let todo: Vec<AppEntry> = apps
            .into_iter()
            .filter(|a| !state.get(&a.id).map_or(false, is_present))
            .collect();
        scan_folders(&todo, &folders, &data_dir())
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn adopt_app(id: String, path: String) -> Result<(), String> {
    adopt_entry(&find_app(&id)?, Path::new(&path))
}

/// Each app's own issues page, for reporting problems with that app.
#[tauri::command]
fn open_issues(id: String) -> Result<(), String> {
    let app = find_app(&id)?;
    open_with_system(&format!("https://github.com/{}/issues", app.repo))
}

#[tauri::command]
fn open_repo(id: String) -> Result<(), String> {
    let app = find_app(&id)?;
    open_with_system(&format!("https://github.com/{}/releases", app.repo))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_hopper_android::init())
        .setup(|_app| {
            #[cfg(target_os = "android")]
            let _ = APP.set(_app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_existing_mode,
            set_app_order,
            reset_app_order,
            finish_first_run,
            add_scan_folder,
            remove_scan_folder,
            scan_existing,
            adopt_app,
            open_issues,
            get_device,
            list_apps,
            check_releases,
            install_app,
            launch_app,
            uninstall_app,
            skip_update,
            archive_list,
            archive_download,
            set_archive_enabled,
            open_archive_profile,
            open_repo
        ])
        .run(tauri::generate_context!())
        .expect("error while running Hopper Launcher");
}

#[cfg(test)]
#[allow(dead_code, unused_imports)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    type Routes = Arc<Mutex<HashMap<String, Vec<u8>>>>;

    /// How many full (not "unchanged") release answers the fake server has sent.
    static FULL_RELEASE_ANSWERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    /// Tests that set process-wide environment variables take turns.
    static ENV: Mutex<()> = Mutex::new(());

    fn entry(id: &str, name: &str) -> AppEntry {
        AppEntry { id: id.into(), name: name.into(), tagline: String::new(), repo: "me/x".into(), accent: None, untested_on: vec![], archive: None }
    }

    fn script(path: &Path, body: &str, mode: u32) {
        fs::write(path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
        }
        let _ = mode;
    }

    /// The environment a child would get: the process's own, changed by what the Command sets or removes.
    fn child_env(cmd: &Command) -> HashMap<String, String> {
        let mut env: HashMap<String, String> =
            std::env::vars().collect();
        for (k, v) in cmd.get_envs() {
            let k = k.to_string_lossy().into_owned();
            match v {
                Some(v) => env.insert(k, v.to_string_lossy().into_owned()),
                None => env.remove(&k),
            };
        }
        env
    }

    #[test]
    fn apps_do_not_inherit_the_launchers_appimage_settings() {
        let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let mount = "/tmp/.mount_HopperAbc123";
        let saved: Vec<(String, Option<String>)> = ["APPDIR", "APPIMAGE", "ARGV0", "OWD", "PYTHONHOME", "PYTHONPATH", "LD_LIBRARY_PATH", "PATH", "GTK_PATH", "XDG_DATA_DIRS", "GTK_THEME", "MY_OWN_SETTING"]
            .iter()
            .map(|k| (k.to_string(), std::env::var(k).ok()))
            .collect();

        std::env::set_var("APPDIR", mount);
        std::env::set_var("APPIMAGE", "/home/me/Hopper-Launcher.AppImage");
        std::env::set_var("ARGV0", "Hopper-Launcher.AppImage");
        std::env::set_var("OWD", "/home/me");
        std::env::set_var("PYTHONHOME", format!("{mount}/usr/")); // entirely the launcher's
        std::env::set_var("PYTHONPATH", format!("{mount}/usr/share/pyshared/:"));
        std::env::set_var("LD_LIBRARY_PATH", format!("{mount}/usr/lib/:/opt/cuda/lib:{mount}//usr/lib/i386-linux-gnu")); // mixed
        std::env::set_var("PATH", format!("{mount}/usr/bin/:/usr/local/bin:/usr/bin:{mount}/usr/sbin/"));
        std::env::set_var("GTK_PATH", format!("{mount}//usr/lib/gtk-3.0"));
        std::env::set_var("XDG_DATA_DIRS", format!("{mount}/usr/share/:/usr/share:"));
        std::env::set_var("GTK_THEME", "Adwaita:light");
        std::env::set_var("MY_OWN_SETTING", format!("{mount}-lookalike/keep")); // only looks similar

        let mut cmd = Command::new("true");
        scrub_appimage_env(&mut cmd);
        let env = child_env(&cmd);

        for gone in ["APPDIR", "APPIMAGE", "ARGV0", "OWD", "PYTHONHOME", "PYTHONPATH", "GTK_PATH", "GTK_THEME"] {
            assert!(!env.contains_key(gone), "{gone} leaked into the app");
        }
        assert_eq!(env["LD_LIBRARY_PATH"], "/opt/cuda/lib", "the system's own part must stay");
        assert_eq!(env["PATH"], "/usr/local/bin:/usr/bin");
        assert_eq!(env["XDG_DATA_DIRS"], "/usr/share");
        assert_eq!(env["MY_OWN_SETTING"], format!("{mount}-lookalike/keep"), "a similar-looking path was removed");

        // Without an AppImage nothing is touched at all.
        std::env::remove_var("APPDIR");
        let mut cmd = Command::new("true");
        scrub_appimage_env(&mut cmd);
        assert_eq!(cmd.get_envs().count(), 0);

        for (k, v) in saved {
            match v {
                Some(v) => std::env::set_var(&k, v),
                None => std::env::remove_var(&k),
            }
        }
    }

    /// Not a normal test: reads an environment from HOPPER_ENV_IN, runs the real clean-up on it,
    /// and writes the result to HOPPER_ENV_OUT, so real apps can be started with exactly that.
    #[test]
    #[ignore]
    fn dump_scrubbed_env() {
        let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let (input, output) = (std::env::var("HOPPER_ENV_IN").unwrap(), std::env::var("HOPPER_ENV_OUT").unwrap());
        for (k, _) in std::env::vars_os().collect::<Vec<_>>() {
            std::env::remove_var(k);
        }
        for line in fs::read_to_string(&input).unwrap().lines() {
            if let Some((k, v)) = line.split_once('=') {
                std::env::set_var(k, v);
            }
        }
        let mut cmd = Command::new("true");
        scrub_appimage_env(&mut cmd);
        let mut out = String::new();
        for (k, v) in child_env(&cmd) {
            out.push_str(&format!("{k}={v}\n"));
        }
        fs::write(output, out).unwrap();
    }

    #[test]
    fn user_order_is_applied_and_cleaned() {
        let mk = |id: &str| AppInfo {
            entry: entry(id, id),
            installed: false,
            version: None,
            folder: false,
            adopted: false,
            path: None,
            ignored_version: None,
        };
        let ids = |apps: &[AppInfo]| apps.iter().map(|a| a.entry.id.clone()).collect::<Vec<_>>();
        let mut apps = vec![mk("a"), mk("b"), mk("c"), mk("d")]; // already alphabetical
        // The user dragged c first, then a. b and d were never placed, so they follow alphabetically.
        apply_order(&mut apps, &["c".to_string(), "a".to_string()]);
        assert_eq!(ids(&apps), ["c", "a", "b", "d"]);
        // No custom order leaves alphabetical order alone.
        let mut apps = vec![mk("a"), mk("b"), mk("c")];
        apply_order(&mut apps, &[]);
        assert_eq!(ids(&apps), ["a", "b", "c"]);
        // Unknown and repeated ids from the page are dropped.
        let known = vec![entry("a", "a"), entry("b", "b")];
        let cleaned = clean_order(vec!["b".into(), "zzz".into(), "a".into(), "b".into()], &known);
        assert_eq!(cleaned, ["b", "a"]);
    }

    // ---------- Internet Archive backup ----------

    /// Metadata in the shape the archive really returns, with real file names from the items.
    fn item_json(files: &[(&str, &str, &str)]) -> Vec<u8> {
        let list: Vec<_> = files
            .iter()
            .map(|(name, size, sha1)| serde_json::json!({"name": name, "size": size, "sha1": sha1, "source": "original", "mtime": "1793300000"}))
            .collect();
        serde_json::to_vec(&serde_json::json!({"metadata": {"title": "x"}, "files": list})).unwrap()
    }

    #[test]
    fn archive_files_are_told_apart_from_real_item_listings() {
        let class = |n: &str| describe_archive_file(n).map(|c| (c.os, c.kind, c.group, c.debug));
        // the archive's own housekeeping and checksum files are never listed
        for hidden in ["hop-tunes_files.xml", "hop-tunes_meta.xml", "hop-tunes_meta.sqlite", "SHA256SUMS", "SHA256SUMS-windows", "SHA256SUMS.txt", "KIT-SHA256SUMS", "Hop-tunes-1.4.0-x86_64.AppImage.zsync", "__ia_thumb.jpg"] {
            assert!(class(hidden).is_none(), "{hidden} should be hidden");
        }
        // programs, with the lower-case extensions the archive stores
        assert_eq!(class("Hop-tunes-1.4.0-x86_64.appimage"), Some(("linux", "AppImage".into(), "program", false)));
        assert_eq!(class("HopCast-release-0.1.15.0-x86_64.appimage"), Some(("linux", "AppImage".into(), "program", false)));
        assert_eq!(class("HopCast-debug-0.1.15.0-x86_64.appimage"), Some(("linux", "AppImage".into(), "program", true)));
        assert_eq!(class("Alias-Maker-Debug-x86_64.appimage").unwrap().3, true);
        assert_eq!(class("HopCast-windows-x64-0.1.15.0.zip"), Some(("windows", "Zip".into(), "program", false)));
        assert_eq!(class("HopCast-android-0.1.15.0.apk"), Some(("android", "APK".into(), "program", false)));
        assert_eq!(class("Anti-Social-Setup-0.23.0-win-x64.exe"), Some(("windows", "Installer".into(), "program", false)));
        assert_eq!(class("Anti-Social-0.23.0-win-x64-portable.exe"), Some(("windows", "Portable".into(), "program", false)));
        assert_eq!(class("Sudo.Make.Art.0.1.2.exe"), Some(("windows", "Program (.exe)".into(), "program", false)));
        assert_eq!(class("Anti-Social-0.23.0-linux-amd64.deb").unwrap().1, ".deb");
        assert_eq!(class("alias-maker-installer.run").unwrap().1, "Installer (.run)");
        assert_eq!(class("alias-maker-1.5.1-1-any.pkg.tar.zst").unwrap().1, "Arch package");
        // other systems and extras are kept but sorted into their own groups
        assert_eq!(class("Anti-Social-0.23.0-3ds.3dsx").unwrap().0, "console");
        assert_eq!(class("Anti-Social-0.23.0-switch.nro").unwrap().1, "Switch");
        assert_eq!(class("Anti-Social-0.23.0-wiiu.wuhb").unwrap().1, "Wii U");
        assert_eq!(class("hop-tunes-1.4.0.tar.gz").unwrap().2, "source");
        assert_eq!(class("hopcast-bin-aur-0.1.15.0.tar.gz").unwrap().2, "packaging");
        assert_eq!(class("hop-tunes-1.4.0-flathub-kit.tar.gz").unwrap().2, "packaging");
        assert_eq!(class("Hop-tunes-x86_64.flatpak").unwrap().1, "Flatpak");
    }

    #[test]
    fn archive_listing_is_sorted_newest_first_and_reads_odd_versions() {
        let meta: serde_json::Value = serde_json::from_slice(&item_json(&[
            ("HopTunes-1.3.2-x86_64.appimage", "10", "a"),
            ("HopTunes-1.10.0-x86_64.appimage", "10", "a"),
            ("HopTunes-1.4.0-x86_64.appimage", "10", "a"),
            ("Sudo.Make.Art.0.1.2.exe", "5", "a"),
            ("app-debug.apk", "5", "a"),
            ("hop-tunes_meta.xml", "5", "a"),
        ]))
        .unwrap();
        let files = archive_files_from(&meta);
        let names: Vec<_> = files.iter().map(|f| f.name.as_str()).collect();
        // 1.10.0 is newer than 1.4.0 (compared number by number, not as text); unversioned files go last
        assert_eq!(names, ["HopTunes-1.10.0-x86_64.appimage", "HopTunes-1.4.0-x86_64.appimage", "HopTunes-1.3.2-x86_64.appimage", "Sudo.Make.Art.0.1.2.exe", "app-debug.apk"]);
        assert_eq!(files[3].version.as_deref(), Some("0.1.2"));
        assert_eq!(files[0].size, 10);
        assert_eq!(files[0].mtime, Some(1793300000));
        assert_eq!(version_from_name("HopCast-release-0.1.15.0-x86_64.appimage"), Some("0.1.15.0".into()));
    }

    #[test]
    fn archive_addresses_are_safe() {
        assert!(valid_item("anti-social-") && valid_item("sudo.-make.-art") && valid_item("hop-tunes"));
        assert!(!valid_item("") && !valid_item("a/b") && !valid_item("a b") && !valid_item("a?x=1") && !valid_item("../x"));
        assert_eq!(percent_encode("My App 1.0 (x).appimage"), "My%20App%201.0%20%28x%29.appimage");
        assert_eq!(percent_encode("v1/a-b_c.d~e"), "v1/a-b_c.d~e");
    }

    #[test]
    fn archive_downloads_are_checked_and_saved_safely() {
        let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("hopper-archive-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        std::env::set_var("HOPPER_DATA_DIR", root.join("data"));
        std::env::set_var("HOPPER_DOWNLOAD_DIR", root.join("downloads"));
        let (port, routes) = serve();
        std::env::set_var("HOPPER_ARCHIVE_BASE", format!("http://127.0.0.1:{port}"));

        let body: &[u8] = b"#!/bin/sh\nexit 0\n";
        let good_sha1 = to_hex(&<sha1::Sha1 as Digest>::digest(body));
        let good_sha256 = sha(body);
        let app = AppEntry { archive: Some("test-item".into()), ..entry("tool", "Tool") };
        let publish_item = |sha1: &str, sums: &str| {
            let mut r = routes.lock().unwrap();
            r.clear();
            r.insert(
                "/metadata/test-item".into(),
                item_json(&[("Tool-1.2.0-x86_64.appimage", "17", sha1), ("SHA256SUMS", "100", "x"), ("test-item_meta.xml", "5", "x")]),
            );
            r.insert("/download/test-item/Tool-1.2.0-x86_64.appimage".into(), body.to_vec());
            // the developer's checksum file names the file with the GitHub spelling (capital letters)
            r.insert("/download/test-item/SHA256SUMS".into(), sums.as_bytes().to_vec());
        };

        // A good file is saved, made runnable, and checked two ways, with upper/lower case ignored.
        publish_item(&good_sha1, &format!("{good_sha256}  Tool-1.2.0-x86_64.AppImage\n"));
        let got = archive_download_blocking(&app, "Tool-1.2.0-x86_64.appimage", |_, _| {}).unwrap();
        assert_eq!(got.verified, ["Internet Archive checksum", "SHA256SUMS"]);
        assert_eq!(fs::read(&got.path).unwrap(), body);
        assert!(got.path.ends_with("Tool-1.2.0-x86_64.appimage"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(fs::metadata(&got.path).unwrap().permissions().mode() & 0o111, 0, "an AppImage must be runnable");
        }

        // A second download never overwrites the first.
        let again = archive_download_blocking(&app, "Tool-1.2.0-x86_64.appimage", |_, _| {}).unwrap();
        assert!(again.path.ends_with("Tool-1.2.0-x86_64 (1).appimage"), "{}", again.path);
        assert!(Path::new(&got.path).exists());

        // A file that does not match the archive's checksum is thrown away, with no leftovers.
        let before = fs::read_dir(root.join("downloads")).unwrap().count();
        publish_item("0000000000000000000000000000000000000000", &format!("{good_sha256}  Tool-1.2.0-x86_64.appimage\n"));
        let err = archive_download_blocking(&app, "Tool-1.2.0-x86_64.appimage", |_, _| {}).unwrap_err();
        assert!(err.contains("Internet Archive's checksum"), "{err}");
        assert_eq!(fs::read_dir(root.join("downloads")).unwrap().count(), before, "a bad download left a file behind");

        // So does one that fails the developer's own SHA256SUMS.
        publish_item(&good_sha1, &format!("{}  Tool-1.2.0-x86_64.AppImage\n", "f".repeat(64)));
        let err = archive_download_blocking(&app, "Tool-1.2.0-x86_64.appimage", |_, _| {}).unwrap_err();
        assert!(err.contains("SHA256SUMS"), "{err}");
        assert_eq!(fs::read_dir(root.join("downloads")).unwrap().count(), before);

        // Only files the item really lists, and real files, can be fetched.
        assert!(archive_download_blocking(&app, "../../etc/passwd", |_, _| {}).is_err());
        assert!(archive_download_blocking(&app, "test-item_meta.xml", |_, _| {}).is_err());
        assert!(archive_download_blocking(&entry("none", "None"), "x.appimage", |_, _| {}).unwrap_err().contains("no Internet Archive item"));

        // A missing item says so plainly.
        routes.lock().unwrap().insert("/metadata/test-item".into(), b"{}".to_vec());
        assert!(fetch_archive_metadata("test-item").unwrap_err().contains("does not exist"));

        std::env::remove_var("HOPPER_ARCHIVE_BASE");
        std::env::remove_var("HOPPER_DOWNLOAD_DIR");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn archive_backup_is_off_by_default_and_never_calls_out_while_off() {
        let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        assert!(!Settings::default().archive_enabled);
        let root = std::env::temp_dir().join(format!("hopper-archive-off-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        std::env::set_var("HOPPER_DATA_DIR", &root);
        // Nothing is listening here: if the code tried to call out it would fail differently.
        std::env::set_var("HOPPER_ARCHIVE_BASE", "http://127.0.0.1:1");
        assert!(archive_overview().unwrap_err().contains("switched off"));
        assert!(!settings_view().archive_enabled);
        set_archive_enabled(true).unwrap();
        assert!(settings_view().archive_enabled);
        // On: apps without an item say so, and an unreachable archive is an error for that app only.
        let items = archive_overview().unwrap();
        assert!(items.iter().all(|i| i.error.is_some()));
        set_archive_enabled(false).unwrap();
        assert!(archive_overview().is_err());
        std::env::remove_var("HOPPER_ARCHIVE_BASE");
        let _ = fs::remove_dir_all(&root);
    }

    /// Needs the internet: reads your real archive items. Run with `cargo test real_archive -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_archive_items_are_readable() {
        for app in load_manifest().unwrap() {
            let item = app.archive.clone().unwrap_or_else(|| panic!("{} has no archive item", app.name));
            // HOPPER_REAL_META_DIR: item listings saved earlier (for machines that cannot reach archive.org directly).
            let meta = match std::env::var_os("HOPPER_REAL_META_DIR") {
                Some(dir) => serde_json::from_slice(&fs::read(Path::new(&dir).join(format!("{item}.json"))).unwrap()).unwrap(),
                None => fetch_archive_metadata(&item).unwrap_or_else(|e| panic!("{item}: {e}")),
            };
            let files = archive_files_from(&meta);
            let programs: Vec<_> = files.iter().filter(|f| f.group == "program" && !f.debug).collect();
            println!("{:<16} {:<16} {:>2} files shown, {:>2} programs: {}", app.name, item, files.len(), programs.len(), programs.iter().map(|f| f.kind.as_str()).collect::<Vec<_>>().join(", "));
            assert!(!programs.is_empty(), "{item} lists no programs");
        }
    }

    #[test]
    fn reads_versions_from_file_names() {
        assert_eq!(version_from_name("HopCast-release-0.1.14-x86_64.AppImage"), Some("0.1.14".into()));
        assert_eq!(version_from_name("Anti-Social-0.22.0.AppImage"), Some("0.22.0".into()));
        assert_eq!(version_from_name("HopTunes-1.0-x86_64.AppImage"), Some("1.0".into()));
        assert_eq!(version_from_name("HopTunes.AppImage"), None);
        assert_eq!(version_from_name("x86_64"), None);
    }

    #[test]
    #[cfg(unix)]
    fn scan_finds_only_your_apps_and_skips_the_rest() {
        let d = std::env::temp_dir().join(format!("hopper-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("Downloads")).unwrap();
        fs::create_dir_all(d.join("launcher/apps/hopcast")).unwrap();
        for f in ["HopCast-release-0.1.14-x86_64.AppImage", "HopCast-debug-0.1.14-x86_64.AppImage", "Hop-Tunes_1.0.AppImage", "random.AppImage", "notes.txt"] {
            script(&d.join("Downloads").join(f), "#!/bin/sh\n", 0o755);
        }
        // The launcher's own copy must never be offered back to the user.
        script(&d.join("launcher/apps/hopcast/HopCast-0.1.15.AppImage"), "#!/bin/sh\n", 0o755);

        let apps = vec![entry("hopcast", "HopCast"), entry("hop-tunes", "HopTunes"), entry("sudo-make-art", "sudo make art")];
        let folders = vec![d.join("Downloads"), d.join("launcher/apps"), d.join("does-not-exist")];
        let found = scan_folders(&apps, &folders, &d.join("launcher"));
        let summary: Vec<_> = found.iter().map(|c| (c.app_id.as_str(), c.version.clone())).collect();
        assert_eq!(summary, vec![("hopcast", Some("0.1.14".into())), ("hop-tunes", Some("1.0".into()))]);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn existing_installs_stay_in_place_and_settings_are_respected() {
        let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let d = std::env::temp_dir().join(format!("hopper-adopt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("mine")).unwrap();
        std::env::set_var("HOPPER_DATA_DIR", d.join("data"));
        let mark = d.join("mark");
        std::env::set_var("HOPPER_TEST_MARK", &mark);
        let app = entry("hopcast", "HopCast");
        let theirs = d.join("mine/HopCast-release-0.1.14-x86_64.AppImage");
        script(&theirs, "#!/bin/sh\necho ran > \"$HOPPER_TEST_MARK\"\n", 0o755);
        let status = || {
            let l = list_apps_from(vec![app.clone()]);
            (l[0].installed, l[0].adopted, l[0].version.clone(), l[0].path.clone())
        };

        // Bad choices are refused with a clear reason.
        assert!(adopt_entry(&app, &d.join("mine")).unwrap_err().contains("not a file"));
        assert!(adopt_entry(&app, &d.join("mine/missing")).is_err());
        let plain = d.join("mine/plain.AppImage");
        script(&plain, "x", 0o644);
        #[cfg(unix)]
        assert!(adopt_entry(&app, &plain).unwrap_err().contains("runnable"));
        assert_eq!(status().0, false);

        // Adopting records the copy where it is, with its version from the file name.
        adopt_entry(&app, &theirs).unwrap();
        let (installed, adopted, version, path) = status();
        assert!(installed && adopted);
        assert_eq!(version, Some("0.1.14".into()));
        assert_eq!(path, Some(theirs.to_string_lossy().into_owned()));

        // Launching runs the user's own file in place.
        launch_entry(&app).unwrap();
        for _ in 0..50 {
            if mark.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(mark.exists(), "the adopted app never ran");

        // "Keep in place" refuses to replace it with a launcher download; the other choices allow it.
        save_settings(&Settings { existing_mode: "in_place".into(), ..Settings::default() }).unwrap();
        assert!(check_update_allowed("hopcast").is_err());
        save_settings(&Settings { existing_mode: "auto".into(), ..Settings::default() }).unwrap();
        assert!(check_update_allowed("hopcast").is_ok());
        save_settings(&Settings { existing_mode: "manual".into(), ..Settings::default() }).unwrap();
        assert!(check_update_allowed("hopcast").is_ok());
        assert!(!scanning_enabled(&load_settings()), "manual mode must not scan");
        save_settings(&Settings { existing_mode: "in_place".into(), search_folders: false, ..Settings::default() }).unwrap();
        assert!(!scanning_enabled(&load_settings()));
        save_settings(&Settings { existing_mode: "in_place".into(), search_folders: true, ..Settings::default() }).unwrap();
        assert!(scanning_enabled(&load_settings()));

        // Uninstalling an adopted app only forgets it. The file is never touched.
        uninstall_entry(&app).unwrap();
        assert_eq!(status().0, false);
        assert!(theirs.is_file(), "the user's own file was deleted");

        // A copy installed by the launcher itself must be removed before adopting another.
        let managed = d.join("data/apps/hopcast/HopCast-0.1.15.AppImage");
        fs::create_dir_all(managed.parent().unwrap()).unwrap();
        script(&managed, "#!/bin/sh\n", 0o755);
        let mut st = load_state();
        st.insert("hopcast".into(), InstalledApp { tag: "v0.1.15".into(), path: managed, folder: false, adopted: false, package: None, ignored_version: None });
        save_state(&st).unwrap();
        assert!(adopt_entry(&app, &theirs).unwrap_err().contains("already installed"));

        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn reads_checksum_lines() {
        let h = "a".repeat(64);
        let sums = format!("{h}  other.zip\n{h}  ./dist/App.AppImage\n{} *Star.apk\n", "b".repeat(64));
        assert_eq!(expected_hash(&sums, "App.AppImage"), Some(h));
        assert_eq!(expected_hash(&sums, "Star.apk"), Some("b".repeat(64)));
        assert_eq!(expected_hash(&sums, "missing"), None);
        assert_eq!(expected_hash("nothex  App.AppImage", "App.AppImage"), None);
    }

    #[test]
    fn finds_shallowest_exe() {
        let d = std::env::temp_dir().join(format!("hopper-exe-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("a/b")).unwrap();
        fs::write(d.join("a/b/deep.exe"), b"x").unwrap();
        fs::write(d.join("a/App.EXE"), b"x").unwrap();
        assert_eq!(find_exe(&d), Some(d.join("a/App.EXE")));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn unzip_extracts_and_refuses_paths_that_escape() {
        let d = std::env::temp_dir().join(format!("hopper-zip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let zip_path = d.join("t.zip");
        {
            let mut w = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            w.start_file("App/Prog.exe", o).unwrap();
            w.write_all(b"exe").unwrap();
            w.start_file("../evil.txt", o).unwrap();
            w.write_all(b"evil").unwrap();
            w.finish().unwrap();
        }
        let out = d.join("out");
        fs::create_dir_all(&out).unwrap();
        unzip(&zip_path, &out).unwrap();
        assert_eq!(find_exe(&out), Some(out.join("App/Prog.exe")));
        assert!(!d.join("evil.txt").exists(), "zip entry escaped the destination");
        fs::remove_dir_all(&d).unwrap();
    }

    /// A tiny local HTTP server whose responses can be swapped while it runs.
    fn serve() -> (u16, Routes) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes: Routes = Arc::new(Mutex::new(HashMap::new()));
        let shared = routes.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let routes = shared.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let mut if_none_match = None;
                    loop {
                        let mut h = String::new();
                        if reader.read_line(&mut h).unwrap_or(0) <= 2 {
                            break;
                        }
                        if let Some(v) = h.to_lowercase().strip_prefix("if-none-match:") {
                            if_none_match = Some(v.trim().to_string());
                        }
                    }
                    let mut s = stream;
                    match routes.lock().unwrap().get(&path) {
                        Some(body) => {
                            let etag = format!("\"{}\"", &to_hex(&Sha256::digest(body))[..16]);
                            if if_none_match.as_deref() == Some(etag.to_lowercase().as_str()) {
                                let _ = write!(s, "HTTP/1.1 304 Not Modified\r\nETag: {etag}\r\nConnection: close\r\n\r\n");
                            } else {
                                if path.ends_with("/releases/latest") {
                                    FULL_RELEASE_ANSWERS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                }
                                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: {etag}\r\nConnection: close\r\n\r\n", body.len());
                                let _ = s.write_all(body);
                            }
                        }
                        None => {
                            let _ = write!(s, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        }
                    }
                });
            }
        });
        (port, routes)
    }

    /// Publishes a fake release `tag` whose AppImage has contents `body` and a checksum line `hash`.
    fn publish(port: u16, routes: &Routes, tag: &str, body: &[u8], hash: &str) {
        let files = ["App-x86_64.AppImage", "App-debug-x86_64.AppImage", "SHA256SUMS", "App-win.zip"];
        let assets: Vec<_> = files
            .iter()
            .map(|f| serde_json::json!({"name": f, "size": 4, "browser_download_url": format!("http://127.0.0.1:{port}/dl/{f}")}))
            .collect();
        let mut r = routes.lock().unwrap();
        r.clear();
        r.insert(
            "/repos/me/app/releases/latest".into(),
            serde_json::to_vec(&serde_json::json!({"tag_name": tag, "assets": assets})).unwrap(),
        );
        r.insert("/dl/App-x86_64.AppImage".into(), body.to_vec());
        r.insert("/dl/SHA256SUMS".into(), format!("{hash}  App-x86_64.AppImage\n").into_bytes());
    }

    fn sha(bytes: &[u8]) -> String {
        to_hex(&Sha256::digest(bytes))
    }

    /// One test drives the whole flow because it sets process-wide environment variables.
    #[test]
    #[cfg(target_os = "linux")]
    fn install_launch_update_uninstall_against_fake_github() {
        let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let data = std::env::temp_dir().join(format!("hopper-data-{}", std::process::id()));
        let _ = fs::remove_dir_all(&data);
        std::env::set_var("HOPPER_DATA_DIR", &data);
        let mark = data.join("mark");
        std::env::set_var("HOPPER_TEST_MARK", &mark);

        let (port, routes) = serve();
        std::env::set_var("HOPPER_API_BASE", format!("http://127.0.0.1:{port}"));
        let app = AppEntry {
            id: "app".into(),
            name: "App".into(),
            tagline: String::new(),
            repo: "me/app".into(),
            accent: None,
            untested_on: vec![],
            archive: None,
        };
        let status = || {
            let l = list_apps_from(vec![app.clone()]);
            (l[0].installed, l[0].version.clone())
        };

        // v1 is a tiny shell script posing as an AppImage, with a correct checksum.
        let v1: &[u8] = b"#!/bin/sh\necho started > \"$HOPPER_TEST_MARK\"\n";
        publish(port, &routes, "v1", v1, &sha(v1));

        // Only the real build is offered (no debug build, checksum file or Windows zip).
        let rel = latest_release("me/app").unwrap();
        let offered: Vec<_> = builds_for(&rel.assets, "linux", "x86_64").into_iter().map(|b| b.file).collect();
        assert_eq!(offered, vec!["App-x86_64.AppImage"]);

        // Asking again when nothing changed is answered "unchanged" (cheap), and a new release gets a full answer.
        let answers = || FULL_RELEASE_ANSWERS.load(std::sync::atomic::Ordering::SeqCst);
        let before = answers();
        assert_eq!(latest_release("me/app").unwrap().tag_name, "v1");
        assert_eq!(answers(), before, "an unchanged release was downloaded again");

        // Installing a file that is not a build for this device is refused.
        assert!(install_blocking(&app, "App-debug-x86_64.AppImage").is_err());
        assert_eq!(status(), (false, None));

        // A good download installs, verifies and becomes launchable.
        let r = install_blocking(&app, "App-x86_64.AppImage").unwrap();
        assert_eq!((r.tag.as_str(), r.verified, r.handed_off), ("v1", true, false));
        assert_eq!(status(), (true, Some("v1".into())));

        // The app makes its own settings folder beside the program.
        let settings = data.join("apps/app/App-x86_64.AppImage.home/settings.json");
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        fs::write(&settings, b"mine").unwrap();

        // Launching starts the app as its own program.
        launch_entry(&app).unwrap();
        for _ in 0..50 {
            if mark.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(mark.exists(), "the launched app never ran");

        // Skipping a release is remembered.
        skip_entry(&app, "v2").unwrap();
        assert_eq!(list_apps_from(vec![app.clone()])[0].ignored_version.as_deref(), Some("v2"));

        // A tampered update is rejected and the working install is left alone.
        publish(port, &routes, "v2", b"#!/bin/sh\necho evil\n", &sha(v1));
        let before = answers();
        assert_eq!(latest_release("me/app").unwrap().tag_name, "v2");
        assert_eq!(answers(), before + 1, "a changed release was not downloaded");
        let err = install_blocking(&app, "App-x86_64.AppImage").unwrap_err();
        assert!(err.contains("checksum"), "{err}");
        assert_eq!(status(), (true, Some("v1".into())));

        // A good update replaces the old version and leaves only one file behind.
        let v2: &[u8] = b"#!/bin/sh\nexit 0\n";
        publish(port, &routes, "v2", v2, &sha(v2));
        assert_eq!(install_blocking(&app, "App-x86_64.AppImage").unwrap().tag, "v2");
        assert_eq!(status(), (true, Some("v2".into())));
        assert_eq!(list_apps_from(vec![app.clone()])[0].ignored_version, None, "installing should clear a skipped version");
        let left: Vec<_> = fs::read_dir(data.join("apps/app")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left.len(), 2, "expected the new program and the settings folder, got {left:?}");
        assert_eq!(fs::read_to_string(&settings).unwrap(), "mine", "an update touched the app's settings");

        // Uninstalling removes the program but keeps the app's own settings.
        uninstall_entry(&app).unwrap();
        assert_eq!(status(), (false, None));
        assert!(!data.join("apps/app/App-x86_64.AppImage").exists());
        assert_eq!(fs::read_to_string(&settings).unwrap(), "mine", "uninstall deleted the app's settings");

        let _ = fs::remove_dir_all(&data);
    }
}
