# Hopper Launcher: project notes and roadmap

## Status
Design phase started (see `DESIGN.md` and `design/mockup.html`). Previously on hold. The owner is finishing HopCast, Alias Maker and HopTunes first.
The Tauri scaffold on this branch has never been compiled; start with a build check.

## Context
- GitHub account is `gamehopper`. The old name `theStyleWithin` is a dead name: never use it.
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
- Platform plans: HopTunes has no Windows build and none is planned (Android only, planned).
  Alias Maker stays Linux-only; an Android fake-terminal simulator for learners is planned
  but not ready, so it is not listed in `apps.json` yet. HopCast has Windows and Android planned.
