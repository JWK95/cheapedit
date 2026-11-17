# cheapedit

A lightweight Windows front end for FFmpeg. You can:

- trim a clip between two timestamps
- zoom into part of the picture for a stretch of time, with smooth glides and pans (up to 20 zones)
- change its speed
- keep, mute or remove the audio, or export the audio alone as an MP3
- pick the format, codec and quality

**FFmpeg does all the editing. cheapedit just gives it a friendly face.**

## Requirements

Just Windows 10 or 11 (64-bit). cheapedit comes with its own **FFmpeg** and **libmpv**, so there's nothing else to install.

Prefer to use your own copies, to update them yourself? Turn on **Use my own FFmpeg and libmpv from PATH** on the **Setup** screen. [SETUP.md](SETUP.md) shows how to install them properly.

## Installing

Download `cheapedit-Setup-<version>.exe` from [Releases](https://github.com/JWK95/cheapedit/releases) and run it. By default it installs just for you, so no admin rights are needed.

After that, cheapedit keeps itself up to date. When a new version is out, a bar at the top offers **Update now**. It downloads the installer, checks it isn't damaged, updates, and reopens with your settings intact.

### "Windows protected your PC"

cheapedit isn't code-signed yet. A signing certificate costs money every year, so on first run Windows SmartScreen will show this warning. Click **More info → Run anyway**. The warning appears because the app is unsigned and new, not because anything is wrong with it.

## Privacy

The only time cheapedit goes online is one update check when it starts. It asks github.com for the latest version number and sends nothing about you or your videos. You can turn it off under **About**. cheapedit doesn't install anything else on your system and doesn't change any system settings. Its bundled FFmpeg and libmpv stay inside its own folder and are never added to PATH.

## Developing

**Dev build, no install needed:**

```powershell
.\tools\dev.ps1                  # build and run target\debug\cheapedit.exe
.\tools\dev.ps1 "D:\clip.mp4"    # ...with a video already open
```

The dev build shows a **DEV** badge, keeps its own settings and logs in `%APPDATA%\cheapedit-dev`, and never checks for updates, so it can't interfere with the installed copy.

**Bundled FFmpeg and libmpv:** `tools\deps.toml` pins the exact builds (and their source) by SHA-256. `.\tools\fetch-deps.ps1` downloads and checks them into `target\deps`; `dev.ps1` runs it for you the first time. To upgrade, edit the versions and URLs in `deps.toml`, run `.\tools\fetch-deps.ps1 -Pin`, then run the tests (the `--ignored` ones export with the bundled FFmpeg).

**Release:**

```powershell
.\tools\release.ps1 -Version 0.2.0 -Notes "What changed"
```

This one command:

1. Fetches and checks the pinned FFmpeg and libmpv.
2. Runs the tests.
3. Builds the exe, the installer (about 100 MB, with FFmpeg and libmpv inside) and its SHA-256 checksum.
4. Commits, tags and pushes.
5. Publishes a GitHub release, with the FFmpeg and mpv source archives attached.

Installed copies pick the release up on their next start.

**Other commands:**

```powershell
cargo test                                   # unit tests
cargo test -- --ignored --nocapture          # real FFmpeg exports of every format/codec combo
cargo run --example make_icon                # regenerate icons from assets/icon.svg
```

`tools/verify_combos.ps1` re-checks which format and codec combinations your FFmpeg build accepts.

## Licence

cheapedit is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version. See [LICENSE](LICENSE).

Copyright (C) 2026 JWK95

cheapedit includes [FFmpeg](https://ffmpeg.org) (Gyan's builds) and [libmpv](https://mpv.io) (shinchiro's builds), both under the GNU GPL. Their licences are in the `deps\licenses` folder of an install, and the exact source code for the bundled builds is attached to every [release](https://github.com/JWK95/cheapedit/releases).
