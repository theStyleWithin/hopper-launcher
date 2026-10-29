//! The native Android half of Hopper Launcher: opening Android's installer for an
//! APK, starting and removing installed apps, and checking what is installed.
//! On other systems this plugin does nothing.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::Runtime;

#[cfg(target_os = "android")]
use tauri::{plugin::PluginHandle, AppHandle, Manager};

#[cfg(target_os = "android")]
pub struct Hopper<R: Runtime>(PluginHandle<R>);

/// Must match the package of `HopperPlugin.kt`.
#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "com.gamehopper.launcher.hopper";

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("hopper-android")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            {
                let handle = _api.register_android_plugin(PLUGIN_IDENTIFIER, "HopperPlugin")?;
                _app.manage(Hopper(handle));
            }
            Ok(())
        })
        .build()
}

/// Runs one of the Kotlin plugin's commands and returns its JSON answer.
#[cfg(target_os = "android")]
pub fn call<R: Runtime>(app: &AppHandle<R>, command: &str, payload: serde_json::Value) -> Result<serde_json::Value, String> {
    app.state::<Hopper<R>>()
        .0
        .run_mobile_plugin(command, payload)
        .map_err(|e| e.to_string())
}
