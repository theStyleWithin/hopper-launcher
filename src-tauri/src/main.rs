#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod classify;
mod device;

use classify::{builds_for, is_handoff, Asset, Build};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
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

#[derive(Deserialize)]
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
    match std::env::var_os("HOPPER_DATA_DIR") {
        Some(d) => PathBuf::from(d),
        None => dirs::data_dir().unwrap_or_default().join("hopper-launcher"),
    }
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

fn latest_release(repo: &str) -> Result<Release, String> {
    let url = format!("{}/repos/{repo}/releases/latest", api_base());
    ureq::get(&url)
        .set("User-Agent", "hopper-launcher")
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => "No release published yet".to_string(),
            ureq::Error::Status(403 | 429, _) => "GitHub rate limit reached, try again later".to_string(),
            other => format!("Could not reach GitHub: {other}"),
        })?
        .into_json::<Release>()
        .map_err(|e| e.to_string())
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
        (name == file && hash.len() == 64).then(|| hash.to_lowercase())
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

fn open_with_system(target: &str) -> Result<(), String> {
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
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    return Err(format!("Opening {target} is not supported on this system"));

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
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
    if handed_off {
        // The system's installer owns this app from here on.
        state.remove(&app.id);
        save_state(&state)?;
        open_with_system(&launch_path.to_string_lossy())?;
    } else {
        state.insert(
            app.id.clone(),
            InstalledApp { tag: release.tag_name.clone(), path: launch_path, folder: build.kind == "zip", adopted: false },
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
}

impl Default for Settings {
    fn default() -> Self {
        Settings { existing_mode: default_mode(), search_folders: true, extra_folders: vec![], first_run_done: false }
    }
}

fn scanning_enabled(s: &Settings) -> bool {
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
    let mut state = load_state();
    if let Some(existing) = state.get(&app.id) {
        if !existing.adopted && existing.path.is_file() {
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
            let managed = state.get(&entry.id).filter(|s| s.path.is_file());
            AppInfo {
                installed: managed.is_some(),
                version: managed.map(|s| s.tag.clone()),
                folder: managed.map_or(false, |s| s.folder),
                adopted: managed.map_or(false, |s| s.adopted),
                path: managed.map(|s| s.path.to_string_lossy().into_owned()),
                entry,
            }
        })
        .collect();
    apps.sort_by_key(|a| a.entry.name.to_lowercase());
    apps
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
    let target = load_state()
        .get(&app.id)
        .map(|s| s.path.clone())
        .filter(|p| p.is_file())
        .ok_or_else(|| format!("{} is not installed", app.name))?;

    let mut cmd = Command::new(target);
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

#[tauri::command]
fn launch_app(id: String) -> Result<(), String> {
    launch_entry(&find_app(&id)?)
}

/// Removes the program. A single-file install leaves any settings folders the app
/// made next to it (the folder is removed only if nothing else is left in it).
/// A portable folder install is removed whole, so the UI warns about that first.
fn uninstall_entry(app: &AppEntry) -> Result<(), String> {
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
            .filter(|a| !state.get(&a.id).map_or(false, |s| s.path.is_file()))
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

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_existing_mode,
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
            open_repo
        ])
        .run(tauri::generate_context!())
        .expect("error while running Hopper Launcher");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    type Routes = Arc<Mutex<HashMap<String, Vec<u8>>>>;

    /// Tests that set process-wide environment variables take turns.
    static ENV: Mutex<()> = Mutex::new(());

    fn entry(id: &str, name: &str) -> AppEntry {
        AppEntry { id: id.into(), name: name.into(), tagline: String::new(), repo: "me/x".into(), accent: None, untested_on: vec![] }
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

    #[test]
    fn reads_versions_from_file_names() {
        assert_eq!(version_from_name("HopCast-release-0.1.14-x86_64.AppImage"), Some("0.1.14".into()));
        assert_eq!(version_from_name("Anti-Social-0.22.0.AppImage"), Some("0.22.0".into()));
        assert_eq!(version_from_name("HopTunes-1.0-x86_64.AppImage"), Some("1.0".into()));
        assert_eq!(version_from_name("HopTunes.AppImage"), None);
        assert_eq!(version_from_name("x86_64"), None);
    }

    #[test]
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
        st.insert("hopcast".into(), InstalledApp { tag: "v0.1.15".into(), path: managed, folder: false, adopted: false });
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
                    loop {
                        let mut h = String::new();
                        if reader.read_line(&mut h).unwrap_or(0) <= 2 {
                            break;
                        }
                    }
                    let mut s = stream;
                    match routes.lock().unwrap().get(&path) {
                        Some(body) => {
                            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                            let _ = s.write_all(body);
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

        // A tampered update is rejected and the working install is left alone.
        publish(port, &routes, "v2", b"#!/bin/sh\necho evil\n", &sha(v1));
        let err = install_blocking(&app, "App-x86_64.AppImage").unwrap_err();
        assert!(err.contains("checksum"), "{err}");
        assert_eq!(status(), (true, Some("v1".into())));

        // A good update replaces the old version and leaves only one file behind.
        let v2: &[u8] = b"#!/bin/sh\nexit 0\n";
        publish(port, &routes, "v2", v2, &sha(v2));
        assert_eq!(install_blocking(&app, "App-x86_64.AppImage").unwrap().tag, "v2");
        assert_eq!(status(), (true, Some("v2".into())));
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
