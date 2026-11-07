# cheapedit — v1 specification

A lightweight Windows UI for FFmpeg. The app is only a front end: **FFmpeg does all of the editing**. Playback uses libmpv.

## 1. Platform and stack

- **Windows only.** Code stays portable: Windows-specific calls live in one small module.
- **Rust + egui (eframe)** for the UI.
- **libmpv** for playback, embedded through mpv's render API.
- **`ffmpeg.exe` / `ffprobe.exe` run as separate processes.** Progress is read from `-progress pipe:1`.
- **Dark theme only.** Light mode will never be added.
- Minimum window size: 1000×600.

## 2. Dependencies and setup screen

- `ffmpeg`, `ffprobe` and `libmpv-2.dll` are found **on PATH**.
- **All three are required.** If anything is missing, the app stays on the setup screen. There is no partial or export-only mode.
- **The setup screen** shows at first launch and whenever something is missing:
  - shows each dependency as found or missing, with its version and path
  - requires FFmpeg 6.0 or newer
  - walks through installing anything missing, step by step, with copy buttons for commands and a "Check again" button
  - explains *why* each step is done that way, so the user learns good practice and not only what the app needs
  - accepts any working install, so an existing Chocolatey or scoop FFmpeg is fine
- **Install methods the guide teaches:**
  - **FFmpeg:** `winget install Gyan.FFmpeg`, which adds itself to PATH and updates with `winget upgrade`.
  - **libmpv:** the official shinchiro `mpv-dev` build, extracted to a user-owned tools folder (e.g. `C:\Tools\libmpv\`), with that folder added to the **user** PATH, not the system PATH.
- **The app never installs anything, edits PATH, or changes system settings.** It only teaches and offers copy buttons.
- The same guide is also kept in `SETUP.md`.

## 3. Layout

Video preview on the left, tools on the right (see the approved mockup).

**Left side:**
- Original / Result toggle, plus a source info line (resolution, frame rate, codec)
- the player
- a timeline with draggable start and end handles, kept in sync with the trim boxes
- transport buttons (frame back, play, frame forward), plus Set start and Set end
- a shortcut strip showing the current bindings

**Right side, top to bottom:**
1. Output folder
2. Output name
3. Trim
4. Cut mode
5. Speed
6. Audio and video
7. Quality and codec, with a collapsible Advanced section
8. Export button, progress bar, time remaining and Cancel

**Also:**
- Open a file with the Open button, `Ctrl+O`, drag and drop, or "Edit with cheapedit" in Explorer.
- An in-app **Log viewer** panel.

## 4. Output

- **The output folder is required.** It's remembered between sessions, and Export is blocked until it's set, exists and is writable.
- **The original file is never modified.**
- **Name box:** if left blank, the name is generated automatically, e.g. `holiday_clip_25-45_1.5x.mp4`.
- **Name collisions** add `(2)`, `(3)` and so on. Files are never overwritten.
- Metadata from the source (e.g. creation date) is copied to the output.

## 5. Trim (single segment in v1)

- **Start and End boxes** accept `25`, `1:25`, `0:01:25.500` and decimals. Each shows the value converted to `h:mm:ss.ms` below it.
- **"Until end of video"** checkbox.
- **Times refer to the original video.** Trim is applied first, then speed. "New clip length" shows the final duration after the speed change.
- Cuts are stored internally as a list of segments, so adding multiple segments later is straightforward.

## 6. Cut modes

| Mode | What it does | Plain-English explanation in the app |
|---|---|---|
| **Precise** (default) | Re-encodes the clip | "Cuts are exact to the frame. Slower, and quality depends on your settings." |
| **Fast** | Stream copy (`-c copy`) | "Done in about a second with no quality loss, but the start snaps to the nearest earlier keyframe, so your clip may begin slightly early." |

- In Fast mode, speed, codec, quality, frame rate, resolution and volume are **greyed out**, with the note "Switch to Precise to change this."
- Remove audio, mute, and changing the container all still work in Fast mode, as long as the new format can hold the source's codecs.

## 7. Speed

- Slider from 0.25× to 4× in 0.25 steps, plus a box for typed values.
- **Pitch is kept normal by default**, using FFmpeg's `atempo` filter (chained for extreme values).
- "Change pitch with speed" checkbox (tape-style, using `asetrate` + `aresample`).

## 8. Audio and video modes

1. **Keep video and audio**
2. **Mute**: the video keeps a silent audio track
3. **Remove audio track**: no audio track at all
4. **Audio only (MP3)**: no video. Bitrate choices are 128, 192 (default) and 320 kbps.

A volume slider (0–200%) appears in every mode that has audio.

## 9. Formats and codecs

- **Every combination that works is available.** Choices that would break the export are filtered out.
- **Choices that work but have downsides** show the pitfall in plain English.
- If changing the format makes the current codec invalid, the app switches to a sensible default and says so.

**Candidate combinations** (each one is verified with a test encode on the real FFmpeg before it ships):

| Format | Video | Audio | Hint |
|---|---|---|---|
| **MP4** (default) | H.264, H.265, AV1, VP9 | AAC, MP3, Opus, FLAC | "Plays almost everywhere. Best choice if unsure." Opus/FLAC in MP4: "Works, but many players and phones won't play this audio." |
| **MKV** | H.264, H.265, AV1, VP9 | AAC, MP3, Opus, FLAC | "Flexible, great for archiving. Some phones and editing apps won't open it." |
| **WebM** | VP9, AV1 | Opus, Vorbis | "For websites and embeds." |
| **MOV** | H.264, H.265, ProRes | AAC, PCM | "Apple and editing-software friendly." ProRes: "Huge files, built for editing, not sharing." |
| **MP3** | none | MP3 | "Audio only." |

**Codec hints:**
- H.264: "Plays everywhere."
- H.265: "Smaller files, but older devices may not play it."
- AV1: "Smallest files, slow to encode on the CPU, and older devices may not play it."
- VP9: "Good for the web, slow to encode."

**Encoders:**
- Only encoders that FFmpeg reports with `ffmpeg -encoders` are offered.
- The GPU encoder (NVENC) is used automatically when available, with the CPU as fallback. There's a GPU on/off switch.
- Anything unavailable is greyed out with the reason shown.

## 10. Quality

| Preset | CPU (x264/x265) | NVENC |
|---|---|---|
| High | CRF 18 | CQ 19 |
| Medium (default) | CRF 23 | CQ 24 |
| Small file | CRF 28 | CQ 30 |
| Custom bitrate | typed kbps, single pass | typed kbps |

- Equivalent values are used for AV1 and VP9.
- **Frame rate:** Same as source / 60 / 30 / 24 (and other common rates).
- **Advanced section:**
  - resolution: Same / 2160 / 1440 / 1080 / 720 / 480
  - encoder speed preset
  - audio codec and bitrate
  - GPU switch
- **Not in v1:** keyframe interval, 2-pass encoding.

## 11. Preview

- **Original** plays the source as it is.
- **Result, before exporting:** a live preview in mpv, with the trim range looping and speed, volume and mute applied.
- **Result, after exporting:** plays the exported file, with an "Open in folder" button.
- Changing any setting after an export switches Result back to the live preview.
- A label always shows which one is playing: "Live preview" or "Exported file".
- Hardware decoding is on (`hwdec=auto`).
- **No test render.**

## 12. Export

- **One export at a time**, with a progress bar, time remaining and Cancel.
- Cancel deletes the partial file.
- **Checks before export.** Each blocks Export with a plain-English message:
  - start is at or after end
  - end is past the video's length (offers a "Use end of video" button)
  - Fast mode with a format that can't hold the source's codec
  - the output folder is missing or read-only
  - not enough free disk space for the estimated output size
- **HDR sources are always converted to standard colour (SDR)**, with a note. HDR output is not supported.

## 13. Errors and logs

- Errors are shown in plain English. Raw FFmpeg output is never shown as the error message.
- **Log viewer:** an in-app panel showing the exact FFmpeg command and its full output, with a copy button.
- **Log files** are saved to `%APPDATA%\cheapedit\logs\`.

## 14. Keybindings

**Defaults:**

| Key | Action |
|---|---|
| `Space` | Play / pause |
| `←` / `→` | Move one frame |
| `Shift+←` / `Shift+→` | Move 1 second |
| `I` / `O` | Set start / end |
| `Ctrl+E` | Export |
| `Ctrl+O` | Open file |

**Settings → Keybindings page:**
- a list of every action and its key: click to rebind
- a conflict warning that offers to swap the two keys
- a Reset button on each action, plus Reset all

**On-screen keyboard map:**
- keys that have an action are highlighted and labelled with it
- unused keys are dimmed
- hovering a key shows what it does
- clicking an unused key lets you assign an action to it

**Also:**
- The shortcut strip under the player reflects the current bindings.
- A `?` button opens the full list of shortcuts.
- Bindings apply **inside the app only**. They are never global hotkeys and change no system settings.

## 15. Settings

- Stored in `%APPDATA%\cheapedit\settings.toml`.
- Holds the output folder, last-used options and keybindings.

## 16. Installer and branding

- **The official name is `cheapedit`.**
- **Installer:** Inno Setup, producing `cheapedit-Setup.exe`.
  - Per-user install by default, to `%LOCALAPPDATA%\Programs\cheapedit`, with no admin needed. "Install for all users" is an option.
  - Start menu shortcut.
  - Desktop shortcut: a checkbox, off by default.
  - Listed in Apps with an uninstaller.
  - **"Edit with cheapedit"** on the right-click menu for video files: a checkbox, on by default. This is registered per-user, under HKCU.
  - On uninstall, asks "Also remove your settings and logs?", defaulting to no.
  - At the end, a note that the app will guide FFmpeg and libmpv setup on first launch. The installer doesn't install them.
- **Unsigned for now.** The README explains the SmartScreen warning.
- **Auto-update from GitHub releases** (public repo `JWK95/cheapedit`). Changed from "no update checks" on 2026-11-07 at the user's request.
  - One check on startup against the GitHub "latest release" API, done with Windows' built-in `curl.exe`. It can be switched off in **About**, and there's a **Check now** button there too. This is the only network traffic the app ever makes, and it sends nothing about the user.
  - A newer version shows a banner with **Update now / What's new / Later**. Nothing installs without a click, and updating is blocked while an export is running.
  - The installer is downloaded, its SHA-256 is checked against the `.sha256` file published with the release (a release without one is refused), and it then runs with `/SILENT /RELAUNCH=1`. It updates in place, keeps settings, and reopens cheapedit.
- **Dev and release builds.**
  - Dev: `tools\dev.ps1` (`cargo run`) gives `target\debug\cheapedit.exe`, which needs no install. It shows a DEV badge, keeps its settings and logs in `%APPDATA%\cheapedit-dev`, and never checks for updates.
  - Release: `tools\release.ps1 -Version X.Y.Z -Notes "..."` sets the version, runs the tests, builds the exe, the installer `cheapedit-Setup-X.Y.Z.exe` and its `.sha256`, then commits, tags, pushes and creates the GitHub release.
- **Icon:** a film strip with a play triangle, and coral scissors cutting through the lower right, on a dark purple rounded square (approved draft). Shipped as a multi-size `.ico` (16–256 px), simplified at the small sizes.

## 17. Out of scope for v1

- multiple segments and joining them
- an export queue
- test renders
- HDR output
- 2-pass encoding
- macOS and Linux
- code signing
- light mode (permanently out of scope)
