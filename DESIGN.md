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

## Starting apps
The launcher starts every app as a separate program (see above). When the launcher
itself runs as an AppImage, its start-up script points many environment variables
(PYTHONHOME, PYTHONPATH, LD_LIBRARY_PATH, GTK_PATH, PATH and others) at the launcher's own
bundled files. Passing those on made apps that ship their own Python or libraries
crash on start (sudo make music and Alias Maker did exactly this). So the launcher removes
only what points into its own AppImage, keeps everything that came from the system, and
does the same before opening a web link. When the launcher is not an AppImage, nothing is changed.

## Custom background
Settings, "Background": the user picks their own picture; the aim is to use the empty space around the tiles
without hurting the calm, orderly look.
- **Accepted pictures.** PNG, JPEG, GIF or WebP, judged by what is really inside the file (its first bytes), not its
  name, so an SVG (which can carry script), or a text file renamed `.png` is refused with a plain message.
  Under 12 MB. GIFs are accepted and animate; SVG is not.
- **A copy is kept** in the launcher's folder (`background/`), one picture at a time, so the background survives
  the original moving. The name is checked against the three allowed names before it is ever used as a path, so
  a tampered settings file cannot make the launcher read some other file. The picture is sent to the page as raw
  bytes, so even a large one stays quick. Nothing is uploaded or contacted.
- **Readability.** The picture sits behind everything. Tiles and panels stay solid. A "veil" of the *theme's own
  background colour* is laid over the picture, so dimming works on dark and light themes alike. Dim runs 0 to
  90% (default 55%) and Blur 0 to 20 px (default 0); both move live and are saved when the slider is released.
  Remove goes back to the plain theme.
- **Not on Android yet:** the file picker there returns a content address rather than a file path.

## Download counts
A neon-green chip on the top edge of each tile ("▼ 3,071"), so no tile changes size. It is one exact total per
app: the sum of GitHub's public `download_count` over every program file (AppImage, installer, zip, APK, console
builds ...) of every release, all systems together. Checksum files, debug builds, source tarballs and packaging
kits are not counted (the launcher fetches the checksum file on every install, so counting it would inflate
the figure). GitHub's pages are followed, so a long history is counted in full. Nothing is sent and nobody is
identified. Limits of the number: it is GitHub's figure, so it includes bots and the developer's own tests, and a
file that is deleted and re-uploaded starts again from zero. The last reading is saved (`counts.json`) and shown
when offline, with the time it was read in the hover note. Neon green on dark themes, a deeper green on the light
Paper theme for contrast. A switch in Settings turns it off, and then nothing extra is requested.
Checked against GitHub's real data: the launcher's own four releases add up to the same total as an independent sum.

## Offline, status and apps that can no longer be delivered
**Nothing needs the internet.** Installed apps launch, settings and ordering work, and the interface never waits
on a network call. Every web request gives up after 10 seconds, so a connection that goes nowhere can't freeze
anything. Being offline only changes wording ("You're offline") and turns the corner indicator grey.

**Status in the corner** (a switch in Settings, on by default): two small pills at the top right, "Internet" and
"GitHub". GitHub's state comes from its public status page feed (`githubstatus.com`, indicator none / minor /
major / critical / maintenance); clicking the pill opens that page. If the feed can't be read, a bare connection to
a public DNS server (nothing sent) tells "offline" from "online but the status page is unreachable". The same check
is used, only when GitHub failed to answer, to tell the user's own internet from GitHub trouble.

**When an app's source disappears.** Each check is classified: working, no release yet (the repo exists), offline,
trouble, or gone. "Not found" is split into "no release yet" and "no such repository" by asking GitHub about the
repository itself. An app is **gone** only after patient evidence: the repository reported missing (404 or 451)
twice at least a day apart, or GitHub unreachable for 7 days in a row *while the user's internet works*. Time spent
offline never counts, a rate limit (real ones are recognised by GitHub's headers) changes nothing, and any success
clears the marks. What the user sees:
- An app that is **not installed** and gone is replaced, in the same slot, by a dashed tile: "I'm sorry, Dave. I can't help you."
- An app that **is installed** is never hidden: it keeps its tile and Launch button and shows "v1.2.3 · can't update".
- The count line says "Offline · what you already have still works" when offline.
- The Internet Archive list is unaffected, so it can still offer older versions if the user turned it on.
The marks live in `health.json` in the launcher's data folder.

## Internet Archive backup
A manual safety net for the day a repository is taken down, or GitHub is unavailable for a while or for good.
Older versions of each app are preserved on the Internet Archive (one item per app, named in `apps.json`
under `archive`), and the launcher can list them.

- **Off by default.** A switch in Settings turns it on. While it is off nothing is shown and archive.org is never
  contacted (checked in the real app: zero requests off, six on). It never switches to the archive by itself, and
  it is not wired into the install buttons. The user opens it on purpose and picks a file.
- **A list beside the tiles,** not part of them. The window widens when it is switched on so the list can sit on
  the right; in a narrow window or on a phone it sits below. Rows are a fixed height, grouped by app, newest first.
  Filters: app, "my system" or "all systems", a search box, and an "also show debug, source and packaging files" tick.
- **What is listed.** The launcher reads each item's file list and works out what each file is from its name, the
  same way as for GitHub. The archive's own housekeeping files (`*_files.xml`, `*_meta.xml`, `*_meta.sqlite`) and
  checksum files are never listed. It reads the archive's lower-cased extensions (`.appimage`) and odd version
  numbers (`0.1.15.0`, `Sudo.Make.Art.0.1.2.exe`). Other systems (console builds) and extras (debug, source,
  packaging) stay out of the way until asked for.
- **Download.** A file is saved to the user's Downloads folder (never over an existing file; it gets " (1)"), and
  AppImages are made runnable. It is checked against the archive's own SHA-1 for the file and, when the item has
  a `SHA256SUMS` listing it (upper/lower case ignored), against that too. A mismatch deletes the download. Progress
  is shown on the button. On Android an APK is handed to Android's installer.
- **Source of the backups.** The sidebar links to the developer's page, `archive.org/details/@gameh0pper`, and says so.
- **Not included:** the launcher itself (a manual download). The upload side is outside the launcher: files are
  put into the items by hand, or by a workflow later.
- **Privacy.** archive.org sees a connection only while the backup is switched on, like any website would.

## New releases and updates
The launcher finds out about new releases by asking GitHub itself. It does not
phone home to a server of ours, so nothing identifies the user and no server has
to be kept running. It checks when it opens, when "Check for updates" is pressed,
and every 3 hours while it stays open. It asks GitHub "only if changed" (an ETag
request), which costs GitHub almost nothing and does not use up the rate limit
when nothing is new. A failed check keeps the last good answer on screen.

When a newer release exists than the installed one:
- The main button changes from **Launch** to **Update**, with **Launch anyway**
  beside it. Launch anyway runs the installed version and changes nothing.
- The dropdown offers **Skip this version (stop asking)**. The skip is remembered
  for that exact release, so a newer release brings the prompt back. A skipped
  app shows **Launch** again, and its dropdown can still say "Update to ...".
- Installing an update clears any skip.

## Order of the apps
Apps start in alphabetical order and never move on their own. A handle on each
tile lets the user drag it elsewhere (mouse or finger), or press the arrow keys
while the handle has focus. The order is saved, and Settings has a button to go
back to alphabetical. Apps added later appear after the user's own order.

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
