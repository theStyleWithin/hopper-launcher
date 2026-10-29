# Hopper Launcher: project notes and roadmap

## Status
Desktop (Linux) launcher built on branch `design-clean`: release-driven installs, checksum
verification, existing-install handling and Settings. 16 Rust tests pass and the real app was run on a
virtual display. Not yet tested: the native file/folder pickers, a real GitHub release, Windows and
Android builds. See `DESIGN.md` and `design/mockup.html`. The owner is finishing HopCast, Alias Maker and HopTunes first.
The Tauri scaffold on this branch has never been compiled; start with a build check.

## Context
- The developer name is **gamehopper**. The GitHub repos still live at `theStyleWithin/<repo>`, and that path
  stays as it is (set in `apps.json`). Show the name gamehopper in the app and docs; the old name only
  appears inside those technical repo paths.
- Everything stays in this one repo. No new repos or workspaces can be created.
- Goals: sleek UI, minimal resource use (no background polling, no constant animation).
- Apps are free. Avoid third-party brand names in any descriptions or branding.

## Planned: Windows version
The launcher should not steer people toward Linux, nor gatekeep it. A Windows
build will find and point users to the apps that have dedicated Windows builds,
out of kindness to people who haven't switched.

Work needed when this resumes:
1. Per-platform assets in `apps.json` (Linux `.AppImage`, Windows `.exe`/`.msi`/`.zip`).
2. Detect the OS and pick the matching build; show "Linux only for now" (or
   "Windows only") instead of hiding apps without a build.
3. Replace Linux-only code: `xdg-open`, executable permissions, launch logic.
4. Decide per app how Windows installs work: run the installer, or unpack a portable zip.
5. Windows builds via GitHub Actions; note the SmartScreen warning for unsigned
   `.exe` files in the README.

## Other open items
- HopCast repo has been renamed to `hopcast`; `apps.json` already points at it.
- Choose a license (README currently says none chosen, all rights reserved).
- Optional: auto-discover apps from a repo topic like `gamehopper-app`.
- Optional: GitHub Actions workflow so each app publishes an AppImage release.
- Design decision: `apps.json` lists only repos. Builds are discovered from each repo's
  latest release by file name (see `DESIGN.md`), so no per-platform build lists to maintain.
- Platform notes: HopTunes has no Windows build. Alias Maker is Linux-only; an Android
  fake-terminal simulator for learners is planned but not ready.
- Install location: apps live in the launcher's own folder (Linux `~/.local/share/hopper-launcher/apps/<id>/`,
  Windows `%LOCALAPPDATA%\hopper-launcher\apps\`), with a small state file tracking versions.
  Planned: add a desktop menu entry for each installed Linux app.
- Android: see `ANDROID.md` for the plan (not started).
