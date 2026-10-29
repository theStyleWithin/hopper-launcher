# Android version: plan

Status: not started. This file records what is known so the work can be done in
the right order. Nothing here has been built or tested.

## Why it is a separate project
The Linux and Windows launcher run as desktop programs. Android works differently
in four ways, and each needs new code:

1. **Installing apps.** The launcher downloads an `.apk`, then must hand it to
   Android's own installer. That needs a small native (Kotlin) plugin that
   opens the file through a FileProvider, plus the `REQUEST_INSTALL_PACKAGES`
   permission. Android also asks the user, once, to allow installs from the launcher.
2. **Launching apps.** On a phone, "Launch" means starting an installed app by its
   package name (for example `com.example.hopcast`), not running a file. Each app
   in `apps.json` would need an optional `android_package` field, and the plugin
   needs a "start this package" call and an "is it installed" check.
3. **Storage.** The `dirs` crate has no data folder on Android. The launcher
   would use the folder Tauri gives each app instead, for downloads and settings.
4. **"Apps you already have".** The auto-detect and "use an existing file" ideas
   do not apply. On Android, "already installed" means "this package is on the
   phone". Settings would offer a simpler choice there.

What already works unchanged: reading releases, picking the `.apk` (classification
treats `.apk` as Android), checksum verification, themes and the notice panel.

## Build
- Tauri 2 supports Android. It needs the Android SDK, NDK, JDK 17+ and the Rust
  targets `aarch64-linux-android` (plus optionally `armv7`, `x86_64`).
- The session used to build the other versions cannot do this (no SDK, and Google's
  download host is blocked), but GitHub's `ubuntu` build machines include the SDK
  and NDK. So the Android build belongs in the release workflow as a third job.
- A phone (or emulator) is needed to test. None of the Android code can be
  tested in the Linux sandbox.

## Things only the owner can do
- **Create a signing key** (a keystore file) for the APK and store it, and its
  password, as GitHub secrets. Android only installs updates signed with the same
  key, so it must be kept safe and never lost or committed to the repo.
- Decide the app's package name (for example `com.gamehopper.launcher`; the
  current setting is `com.gamehopper.launcher`).
- Test on a real phone and report what happens.

## Suggested order
1. Add `android_package` to `apps.json` and the plugin for install/launch/is-installed.
2. Switch storage to Tauri's per-app folders on Android.
3. Add the Android job to the release workflow (unsigned debug build first).
4. Test on a phone, fix problems, then add signing and publish.
