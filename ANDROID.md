# Android version: plan

Status: first version written, **not yet built into an APK and never run on a phone**.

Done (on the `android-support` branch):
- The app is now a library plus a thin `main`, which Android needs. Linux and
  Windows still build and pass their tests.
- A native plugin (`src-tauri/plugins/hopper-android`) that reads an APK's
  package name, opens Android's installer, checks what is installed, launches
  and removes apps, and opens links. Apps are recognised by the package name read
  from their APK, so no extra field in `apps.json` is needed.
- Launcher changes for Android: per-app storage folder, installed state checked
  live, no folder-searching options, and the list refreshes when you come back
  from Android's installer.
- A manual workflow, `Android test build`, that builds a debug APK on GitHub and
  attaches it to the run.
- The Rust code was compile-checked for the Android target with a stand-in
  compiler. That proves it type-checks, not that it works.

Not done: building the APK on GitHub (the plugin's Kotlin and Gradle files have
never been compiled), testing on a phone, signing, and publishing.

## Why it is a separate project
The Linux and Windows launcher run as desktop programs. Android works differently
in four ways, and each needs new code:

1. **Installing apps.** The launcher downloads an `.apk`, then must hand it to
   Android's own installer. That needs a small native (Kotlin) plugin that
   opens the file through a FileProvider, plus the `REQUEST_INSTALL_PACKAGES`
   permission. Android also asks the user, once, to allow installs from the launcher.
2. **Launching apps.** On a phone, "Launch" means starting an installed app by its
   package name (for example `com.example.hopcast`), not running a file. The
   plugin reads the package name from the downloaded APK and offers "start this
   package" and "is it installed" calls.
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

## Next steps
1. Run the `Android test build` workflow (Actions tab, Run workflow). Fix whatever
   the Kotlin or Gradle build reports.
2. Download the APK from the run and try it on a phone: install an app, launch
   it, remove it, and check the "allow installs" prompt appears the first time.
3. Add signing and an Android job to the release workflow, then publish.
