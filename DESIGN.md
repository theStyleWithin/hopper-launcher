# Hopper Launcher design

Open `design/mockup.html` in a browser to see it. The theme picker and the
"simulate device" picker are live; hardware detection and downloads are faked.

## What it is
One launcher for Windows, Linux and Android that shows the GameHopper apps and
recommends the right build for the device it is running on.

## Which apps appear
Only the apps listed in `apps.json`. The launcher never scans the GitHub
account, so forks of other people's projects can never show up. Adding an app
is a deliberate edit to that file.

Current list: Alias Maker, Anti-Social, HopCast, HopTunes, sudo make art, sudo make music.

## Manifest (schema 2)
Each app has a `builds` list. A build is `os` (`linux`, `windows`, `android`),
`arch` (`x86_64`, `aarch64`, or `any`), `kind` (`appimage`, `installer`,
`portable`, `zip`, `apk`), `asset` (text the release file name must contain),
and optional `untested: true` or `planned: true` (the build does not exist yet; shown as "Coming soon" with no Install button). An app with no build for the current device
stays visible, greyed out, as "Not available for <OS> yet".

## Hardware detector
Reads, locally and without sending anything anywhere: OS, CPU architecture,
RAM, and whether a discrete GPU exists. The launcher then
1. keeps only builds matching OS and architecture,
2. prefers the easiest format (installer or AppImage over zip),
3. labels the pick "Recommended", or "Build available (untested)" when flagged.
Optional per-build requirements such as `min_ram_gb` can be added to an app
once the owner knows real numbers; they are not guessed here.
The recommendation is advice only. The user can always pick another build.

## Look and feel (built for people who like order)
- One 8px spacing grid. Every tile is the same size.
- Apps are always in alphabetical order. The layout never shifts or reorders
  itself, and there is no auto-playing motion, glitch or flicker.
- Text is never clipped inconsistently: taglines are one line, status is one line.
- Same structure in every tile: name, tagline, status label, one button.
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
1. Release file names for HopCast, HopTunes and Alias Maker on Windows/Android (none yet?).
2. Should the Android launcher hand APKs to the system installer, or just open the download?
3. Is the built-in theme list enough, or should users also get an accent-colour picker?
4. `sudo-make-art`'s README calls it "inspired by" a well-known design tool;
   consider rewording it, like the media player description.
