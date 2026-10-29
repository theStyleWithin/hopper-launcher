# Hopper Launcher design

Open `design/mockup.html` in a browser to see it. The theme picker and the
"simulate device" picker are live. The file lists in the mockup are samples
(HopCast's come from release v0.1.15), and downloads are simulated.

## What it is
One launcher for Windows, Linux and Android that shows the GameHopper apps and
recommends the right build for the device it is running on.

## Which apps appear
Only the apps listed in `apps.json`. The launcher never scans the GitHub
account, so forks of other people's projects can never show up. Adding an app
is a deliberate edit to that file.

Current list: Alias Maker, Anti-Social, HopCast, HopTunes, sudo make art, sudo make music.

## Manifest (schema 3)
Each app is just: `id`, `name`, `tagline`, `repo`, `accent`, and an optional
`untested_on` list (for example `["windows", "android"]`). There is no list of
builds to maintain. The launcher reads each repo's latest GitHub release and
works out the builds from the file names.

## How builds are found
For the user's OS and CPU architecture, the launcher looks at every file in the
repo's latest release and keeps only the ones that are clearly a build:

| File | Counts as |
|---|---|
| `.AppImage`, `.deb`, `.rpm` | Linux |
| `.exe` (installer if the name has "setup"/"install", portable if it has "portable"), `.msi`, `.zip` with "win" in the name | Windows |
| `.apk` | Android |

Ignored: checksums (`SHA256SUMS`), signatures, `.tar.gz` (source and packaging
files), anything marked `debug`, and any file it can't classify. Files with
an `aarch64`/`arm64` or `x86_64`/`x64`/`amd64` tag are matched to the CPU;
untagged Linux and Windows files are assumed to be x86_64.

Debug builds are left out on purpose. They exist for people reporting bugs,
who can find them on the repo's GitHub releases page without the launcher.

Builds for other systems never appear. A Linux user never sees Windows files,
and a Windows user never sees Linux files.

## The dropdown
Each tile has one dropdown listing the matching builds, with the best one
preselected and labelled "Recommended". Order: on Linux AppImage, then .deb,
then .rpm; on Windows installer, MSI, portable, plain .exe, then zip.
Zip builds are unpacked by the launcher into its own folder. If an app has no
build for the device, the tile stays in place with a disabled dropdown reading
"No <OS> build yet", so the grid never changes shape. Owner decision: such apps stay part of the lineup but nothing on the tile is clickable.

## Hardware detector
Reads, locally and without sending anything anywhere: OS, CPU architecture,
RAM, and whether a discrete GPU exists. OS and architecture decide which files
are shown. RAM and GPU can later be used for warnings on heavy apps such as
`sudo make music`; real numbers are needed first, so none are guessed.
The recommendation is advice only. The user can pick any listed build.

## Safety
Releases that include `SHA256SUMS` files (HopCast does) let the launcher verify
every download before installing it. Planned.

## Look and feel (built for people who like order)
- One 8px spacing grid. Every tile is the same size.
- Apps are always in alphabetical order. The layout never shifts or reorders
  itself, and there is no auto-playing motion, glitch or flicker.
- Taglines are one line and every tile has the same structure: name, tagline,
  dropdown, one button.
- Visible keyboard focus; all colours chosen for readable contrast.
- Respects the system "reduce motion" setting.

## Themes
All colours come from a handful of tokens, so a theme is a small file.
Shipped: **Cyberpunk** (default: dark navy, cyan accent, one thin neon line,
no glow), **Midnight** (calm blue), **Paper** (light). Users can add their own
theme file with the same tokens.

## Platform notes
- **Windows:** unsigned installers trigger a SmartScreen warning; explain it in the UI.
- **Android:** APKs are installed outside the Play Store, so Android will ask
  the user to allow installs from this app. Needs a clear one-time explanation.
- **Linux:** AppImages are downloaded and made executable, as in the scaffold.

## Open questions for the owner
1. On Android, should the launcher hand APKs to the system installer, or just open the download?
2. Is the built-in theme list enough, or should users also get an accent-colour picker?
3. `sudo-make-art`'s README calls it "inspired by" a well-known design tool;
   consider rewording it, like the media player description.
