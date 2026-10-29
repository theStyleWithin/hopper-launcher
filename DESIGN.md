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

## Installed apps and launching
Once an app is installed its tile shows a **Launch** button in the same spot
as Install, and the dropdown shows what is installed. The launcher starts the
app as a fully separate program. Closing the launcher, even right after
launching, never closes the app, so a user can pick a tool and put the launcher
away. (An optional "close the launcher after launching" setting is a possible
later addition.) Updates appear on the tile when a newer release exists, and
install files live in the launcher's own folder (see the install notes in `ROADMAP.md`).

## Apps you already have
For people who installed an app before the launcher, Settings has one exclusive
choice, **Apps you already have**:
- **Auto-detect** (default): looks one level deep in the usual folders
  (`~/Applications`, `~/AppImages`, `~/bin`, `~/.local/bin`, `~/Downloads`,
  `~/Desktop`, plus any folder the user adds) for files named after the apps,
  and asks before using each one. Debug builds and the launcher's own copies are ignored.
- **Choose myself**: never searches. Each app has a "Use an existing install…" button in Settings.
- **Keep in place**: existing apps are used where they are and the launcher never
  downloads its own copy of them. An available update shows as a notice with a
  link to the releases page. A switch decides whether the usual folders are searched.

Rules that always apply: a copy the user points the launcher at is never moved,
changed or deleted. "Forget" only makes the launcher stop tracking it. In the
other two modes, updating an adopted app asks first, then downloads the new
version into the launcher's folder and leaves the old file alone. A copy
installed by the launcher itself must be uninstalled before adopting another.
On first run a short welcome screen asks this question.

## Android
On Android the launcher shows only `.apk` builds (nothing else), with the
recommended one first. Tapping Install downloads the APK and hands it to
Android's own installer, which asks the user to confirm. Android will also ask,
once, to allow installs from the launcher, so the app explains that first.
Apps with no APK stay in the lineup but are not clickable, as on every platform.

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

**Accent picker:** a colour picker plus Reset in the header. The chosen accent
applies on top of any theme and is remembered between sessions. The text on
accent-coloured buttons switches between dark and white automatically so it
stays readable, and a note appears if the colour is too faint against the
theme's background.

## Platform notes
- **Windows:** unsigned installers trigger a SmartScreen warning; explain it in the UI.
- **Android:** see the Android section above.
- **Linux:** AppImages are downloaded and made executable, as in the scaffold.

## Open questions for the owner
1. `sudo-make-art`'s README calls it "inspired by" a well-known design tool;
   consider rewording it, like the media player description.
