# Hopper Launcher

The GameHopper launchpad: a small, lightweight desktop launcher for Linux that
gathers the GameHopper apps in one place. It launches the apps you have
installed, downloads them from their GitHub releases when you don't, and tells
you when an update is available.

## Apps

| App | What it is |
|---|---|
| HopCast | Bloat-free, easy livestreaming |
| Alias Maker | Create your own custom terminal commands, with safety rails |
| HopTunes | A sleek, minimalist media player |

All three are free to use. The launcher itself collects no data and needs no account.

## How it works

- The app list lives in `apps.json`. Adding an app means adding one entry.
- Installing downloads the latest `.AppImage` from that app's GitHub release.
- The launcher only contacts GitHub when it opens or when you press
  "Check for updates". It does nothing in the background.
- Installed apps are stored in `~/.local/share/hopper-launcher/`.
- To use your own list, place an `apps.json` in `~/.config/hopper-launcher/`.

## Building

Requires Rust and the Tauri 2 system dependencies (WebKitGTK 4.1, GTK 3).

```sh
cd src-tauri
cargo build --release
```

## Disclaimer

This project is independent and is not affiliated with, endorsed by, or
sponsored by any other company or product. All product names, logos and
trademarks mentioned anywhere are the property of their respective owners.
The software is provided "as is", without warranty of any kind.

## License

No license has been chosen yet. Until one is added, all rights are reserved by
the author.
