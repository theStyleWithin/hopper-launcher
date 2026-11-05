# Android version

Status: the release workflow now builds a **signed APK** (`Hopper-Launcher-<version>-android-arm64.apk`) and
attaches it to each release, once the four signing secrets below are set. The Kotlin plugin and the Gradle
build compile on GitHub, but **no build has ever been run on a phone**, so treat it as a first test version.

What exists:
- The app is a library plus a thin `main`, which Android needs.
- A native plugin (`src-tauri/plugins/hopper-android`) that reads an APK's package name, opens Android's
  installer, checks what is installed, launches and removes apps, and opens links. Apps are recognised by the
  package name read from their APK, so no extra field in `apps.json` is needed.
- Launcher changes for Android: per-app storage folder, installed state checked live, no folder-searching
  options, and the list refreshes when you come back from Android's installer.
- A manual `Android test build` workflow (debug APK) and an `android` job in the Release workflow (signed
  release APK, 64-bit ARM phones and tablets, Android 7 and newer).
- A copy of the Android release APK's SHA-256 in `SHA256SUMS-android`.

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
1. **Add four repository secrets** (repository → Settings → Secrets and variables → Actions → New repository
   secret): `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` and
   `ANDROID_KEY_PASSWORD`. They come from the signing key made for the launcher.
2. **Keep the signing key (the `.jks` file) and its password safe, and never commit them.** Android only
   installs updates signed with the same key. If it is lost, an Android launcher can't be updated in place
   (people would have to uninstall first).
3. Test on a real phone and report what happens.

If the secrets are missing when a release is published, the Linux and Windows files are still built and
attached as usual; the Android job just stops with a clear message. To add the APK to an existing release
later, run the Release workflow by hand (Actions → Release → Run workflow) with that tag.

## Next steps
1. Add the secrets, publish a release, install the APK on a phone: install an app, launch it, remove it, and
   check the "allow installs" prompt appears the first time.
2. Fix whatever the phone shows.
