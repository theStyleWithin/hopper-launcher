# Hopper Launcher

The GameHopper launchpad: a small, lightweight desktop launcher for Linux and Windows that
gathers the GameHopper apps in one place. It launches the apps you have
installed, downloads them from their GitHub releases when you don't, and tells
you when an update is available.

## What it looks like

![The launcher's main window: six app tiles, each with a version dropdown and a button. HopCast has an update waiting, so its button reads Update with Launch anyway beside it.](docs/images/launcher-main.png)

*The main window. Each app has a tile with a dropdown of the builds that suit your device and one button. Here HopCast has a newer release, so its button reads **Update**, with **Launch anyway** beside it. Anti-Social is up to date, and sudo make art has an update the user chose to skip. Drag the dots on a tile to put the apps in your own order.*

![The first-run screen asking how to treat apps you already have: auto-detect, choose myself, or keep in place, with one existing copy found.](docs/images/first-run.png)

*On first run the launcher asks whether you already have some of the apps. Your own files are never moved or deleted.*

![The Before you install panel with four short notes.](docs/images/before-you-install.png)

*Below the tiles, a short "Before you install" panel.*

![The launcher in the Midnight and Paper (light) themes.](docs/images/themes.png)

*Three themes (Cyberpunk is the default, shown above), plus an accent colour picker. Midnight and Paper are shown here.*

These pictures use sample data. The versions and installs shown are made up for the screenshots.

## Apps

| App | What it is |
|---|---|
| Alias Maker | Create your own custom terminal commands, with safety rails |
| Anti-Social | Social media where nobody sees your posts |
| HopCast | Bloat-free, easy livestreaming |
| HopTunes | A sleek, minimalist media player |
| sudo make art | Offline, local-only design tool |
| sudo make music | Offline AI music generation |

All of them are free to use. The launcher itself collects no data and needs no account.

## Before you install

- Install anything from gamehopper at your own risk. If you are unsure, try it in a virtual machine first.
- If you have a problem, post it in the repository of the app that is giving you trouble.
- The developer uses Linux and tests the Linux and Android versions. Windows versions are untested and may have bugs. Windows users are wanted, and your reports help.
- The apps are free. If you enjoy them, a tip of any amount is appreciated (Cash App `$Normand64`), but there is no shame in freeloading.

## How it works

- `apps.json` lists the apps (name, tagline and GitHub repo). It never scans an
  account, so only apps listed there can appear.
- The launcher reads each app's latest GitHub release and shows only the builds
  that suit your device in a dropdown, best one first. Debug builds are ignored.
- Downloads are checked against the release's `SHA256SUMS` file when one is published.
- Launched apps run on their own, so closing the launcher never closes them.
- It only contacts GitHub when it opens or when you press "Check for updates".
  Nothing runs in the background.
- Apps it installs live in `~/.local/share/hopper-launcher/`. Updating or
  uninstalling never touches an app's own settings or data.
- Already have some of the apps? Settings, "Apps you already have", lets you
  choose: auto-detect, choose myself, or keep in place. Your own files are never
  moved or deleted.
- It checks GitHub for new releases when it opens and every 3 hours while open. When one
  appears, the button becomes Update, with Launch anyway beside it and a choice to skip that version.
- Drag the handle on a tile (or use the arrow keys) to put the apps in your own order.
- Themes: Cyberpunk, Midnight and Paper, plus an accent colour picker.
- To use your own list, place an `apps.json` in `~/.config/hopper-launcher/`.

## Building

Requires Rust and the Tauri 2 system dependencies (WebKitGTK 4.1, GTK 3).

```sh
cd src-tauri
cargo test            # run the tests
cargo build --release
```

Status: the Linux desktop version is working. Windows and Android versions are planned (see ROADMAP.md).

## Disclaimer

This project is independent and is not affiliated with, endorsed by, or
sponsored by any other company or product. All product names, logos and
trademarks mentioned anywhere are the property of their respective owners.
The software is provided "as is", without warranty of any kind.

## License

No license has been chosen yet. Until one is added, all rights are reserved by
the author.
