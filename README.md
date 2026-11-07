# cheapedit

A lightweight Windows front end for FFmpeg. You can:

- trim a clip between two timestamps
- change its speed
- keep, mute or remove the audio, or export the audio alone as an MP3
- pick the format, codec and quality

**FFmpeg does all the editing. cheapedit just gives it a friendly face.**

## Requirements

cheapedit needs **FFmpeg 6+**, **ffprobe** and **libmpv** on your PATH. The first time you open the app, it checks for them and walks you through any setup that's needed. The same steps are in [SETUP.md](SETUP.md).

## Installing

Download `cheapedit-Setup-<version>.exe` from [Releases](https://github.com/JWK95/cheapedit/releases) and run it. By default it installs just for you, so no admin rights are needed.

After that, cheapedit keeps itself up to date. When a new version is out, a bar at the top offers **Update now**. It downloads the installer, checks it isn't damaged, updates, and reopens with your settings intact.

### "Windows protected your PC"

cheapedit isn't code-signed yet. A signing certificate costs money every year, so on first run Windows SmartScreen will show this warning. Click **More info → Run anyway**. The warning appears because the app is unsigned and new, not because anything is wrong with it.

## Privacy

The only time cheapedit goes online is one update check when it starts. It asks github.com for the latest version number and sends nothing about you or your videos. You can turn it off under **About**. cheapedit doesn't install anything else on your system and doesn't change any system settings.

## Developing

**Dev build, no install needed:**

```powershell
.\tools\dev.ps1                  # build and run target\debug\cheapedit.exe
.\tools\dev.ps1 "D:\clip.mp4"    # ...with a video already open
```

The dev build shows a **DEV** badge, keeps its own settings and logs in `%APPDATA%\cheapedit-dev`, and never checks for updates, so it can't interfere with the installed copy.

**Release:**

```powershell
.\tools\release.ps1 -Version 0.2.0 -Notes "What changed"
```

This one command:

1. Runs the tests.
2. Builds the exe, the installer and its SHA-256 checksum.
3. Commits, tags and pushes.
4. Publishes a GitHub release.

Installed copies pick the release up on their next start.

**Other commands:**

```powershell
cargo test                                   # unit tests
cargo test -- --ignored --nocapture          # real FFmpeg exports of every format/codec combo
cargo run --example make_icon                # regenerate icons from assets/icon.svg
```

`tools/verify_combos.ps1` re-checks which format and codec combinations your FFmpeg build accepts.
