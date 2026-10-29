#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
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
    /// Substring the release asset name must contain (default ".AppImage")
    #[serde(default)]
    asset: Option<String>,
    /// Optional command already on PATH (for apps installed system-wide)
    #[serde(default)]
    command: Option<String>,
}

#[derive(Deserialize)]
struct Manifest {
    apps: Vec<AppEntry>,
}

#[derive(Deserialize, Serialize, Clone)]
struct InstalledApp {
    tag: String,
    path: PathBuf,
}

type State = HashMap<String, InstalledApp>;

#[derive(Serialize)]
struct AppInfo {
    #[serde(flatten)]
    entry: AppEntry,
    installed: bool,
    version: Option<String>,
}

fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("hopper-launcher")
}

fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_default().join("hopper-launcher")
}

fn state_path() -> PathBuf {
    data_dir().join("state.json")
}

/// A user-edited manifest in ~/.config/hopper-launcher/apps.json overrides the bundled one.
fn load_manifest() -> Result<Vec<AppEntry>, String> {
    let override_path = config_dir().join("apps.json");
    let text = match fs::read_to_string(&override_path) {
        Ok(t) => t,
        Err(_) => BUNDLED_MANIFEST.to_string(),
    };
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

fn on_path(cmd: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|p| p.join(cmd))
        .find(|p| p.is_file())
}

#[tauri::command]
fn list_apps() -> Result<Vec<AppInfo>, String> {
    let state = load_state();
    Ok(load_manifest()?
        .into_iter()
        .map(|entry| {
            let managed = state.get(&entry.id).filter(|s| s.path.is_file());
            let system = entry.command.as_deref().and_then(on_path).is_some();
            AppInfo {
                installed: managed.is_some() || system,
                version: managed.map(|s| s.tag.clone()),
                entry,
            }
        })
        .collect())
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

fn latest_release(repo: &str) -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    ureq::get(&url)
        .set("User-Agent", "hopper-launcher")
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => format!("{repo} has no published release yet"),
            other => format!("could not reach GitHub: {other}"),
        })?
        .into_json::<Release>()
        .map_err(|e| e.to_string())
}

/// Latest release tag for every app. Network only happens when this is called
/// (startup and the refresh button), never in the background.
#[tauri::command]
async fn check_updates() -> Result<HashMap<String, String>, String> {
    let apps = load_manifest()?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut out = HashMap::new();
        for app in apps {
            if let Ok(rel) = latest_release(&app.repo) {
                out.insert(app.id, rel.tag_name);
            }
        }
        out
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn install_app(id: String) -> Result<String, String> {
    let app = find_app(&id)?;
    tauri::async_runtime::spawn_blocking(move || install_blocking(&app))
        .await
        .map_err(|e| e.to_string())?
}

fn install_blocking(app: &AppEntry) -> Result<String, String> {
    let rel = latest_release(&app.repo)?;
    let needle = app.asset.clone().unwrap_or_else(|| ".AppImage".into()).to_lowercase();
    let arch = std::env::consts::ARCH;
    let matches: Vec<&ReleaseAsset> = rel
        .assets
        .iter()
        .filter(|a| a.name.to_lowercase().contains(&needle))
        .collect();
    let asset = matches
        .iter()
        .find(|a| a.name.contains(arch))
        .or_else(|| matches.first())
        .ok_or_else(|| format!("release {} has no asset matching \"{needle}\"", rel.tag_name))?;

    let dir = data_dir().join("apps").join(&app.id);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let final_path = dir.join(&asset.name);
    let part_path = dir.join(format!("{}.part", asset.name));

    let resp = ureq::get(&asset.browser_download_url)
        .set("User-Agent", "hopper-launcher")
        .call()
        .map_err(|e| format!("download failed: {e}"))?;
    {
        let mut file = fs::File::create(&part_path).map_err(|e| e.to_string())?;
        io::copy(&mut resp.into_reader(), &mut file).map_err(|e| format!("download failed: {e}"))?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&part_path, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    fs::rename(&part_path, &final_path).map_err(|e| e.to_string())?;

    let mut state = load_state();
    // Remove the previous version's file now that the new one is in place.
    if let Some(old) = state.get(&app.id) {
        if old.path != final_path {
            let _ = fs::remove_file(&old.path);
        }
    }
    state.insert(
        app.id.clone(),
        InstalledApp { tag: rel.tag_name.clone(), path: final_path },
    );
    save_state(&state)?;
    Ok(rel.tag_name)
}

#[tauri::command]
fn launch_app(id: String) -> Result<(), String> {
    let app = find_app(&id)?;
    let state = load_state();
    let target = state
        .get(&app.id)
        .map(|s| s.path.clone())
        .filter(|p| p.is_file())
        .or_else(|| app.command.as_deref().and_then(on_path))
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
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", app.name))?;
    // Reap the child when it exits so it never lingers as a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[tauri::command]
fn uninstall_app(id: String) -> Result<(), String> {
    let app = find_app(&id)?;
    let mut state = load_state();
    if let Some(entry) = state.remove(&app.id) {
        let _ = fs::remove_file(entry.path);
        save_state(&state)?;
    }
    Ok(())
}

#[tauri::command]
fn open_repo(id: String) -> Result<(), String> {
    let app = find_app(&id)?;
    Command::new("xdg-open")
        .arg(format!("https://github.com/{}", app.repo))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open browser: {e}"))
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            list_apps,
            check_updates,
            install_app,
            launch_app,
            uninstall_app,
            open_repo
        ])
        .run(tauri::generate_context!())
        .expect("error while running Hopper Launcher");
}
