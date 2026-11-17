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

Changed on 2026-11-17 at the user's request: cheapedit now **bundles** FFmpeg and libmpv (it used to require them on PATH and never shipped them).

- **All three are required:** `ffmpeg.exe`, `ffprobe.exe` and `libmpv-2.dll`. If anything is missing everywhere, the app stays on the setup screen. There is no partial or export-only mode.
- **Bundled copies** ship in a `deps\` folder next to `cheapedit.exe`:
  - FFmpeg: Gyan's **full, shared** build (ffmpeg.exe, ffprobe.exe and their DLLs). It includes SVT-AV1, NVENC, AMF, QSV, x264, x265, VP9, Opus and zimg (zscale).
  - From v0.6, for tracked zoom zones (section 5b): ONNX Runtime (DirectML build), DirectML.dll and `models\vittrack.onnx`. These are only needed for tracking: if they're missing, Track explains it and everything else works. They're always the bundled copies (never from PATH).
  - libmpv: shinchiro's `mpv-dev` **x86_64** build (not `-v3`, which needs AVX2).
  - Licence texts in `deps\licenses\`, and `deps\VERSIONS.txt` naming the exact builds.
  - Nothing is added to PATH or installed system-wide. Everything stays in cheapedit's own folder and goes with the uninstaller.
- **Lookup order:** the bundled copy first, then PATH. A switch on the setup screen, **"Use my own FFmpeg and libmpv from PATH"** (saved in settings), puts PATH first. In either order, if the first copy is missing or broken (wrong version, 32-bit DLL, the mpv player instead of the library), the other is used. Dev builds and tests also look in the repo's `target\deps`.
- **Checks** apply to every copy: FFmpeg 6.0 or newer (git builds are assumed new enough), libmpv client API 2 or newer.
- **The setup screen** is skipped when all three work. **Setup** in the top bar still opens it:
  - each dependency with "Bundled with cheapedit" or "Your own copy, from PATH", its version and path, or what's wrong
  - the "use my own" switch. Changing it while the editor is open takes effect at the next start.
  - **Check again**
  - "Installing your own copies (optional)": the step-by-step guides, with copy buttons and the *why* behind each step. They open by themselves only when something is missing.
- **Install methods the guide teaches** (for people who want their own copies):
  - **FFmpeg:** `winget install Gyan.FFmpeg`, which adds itself to PATH and updates with `winget upgrade`.
  - **libmpv:** the official shinchiro `mpv-dev` build, extracted to a user-owned tools folder (e.g. `C:\Tools\libmpv\`), with that folder added to the **user** PATH, not the system PATH.
- **The app never edits PATH or changes system settings.** It only teaches and offers copy buttons.
- The same guide is also kept in `SETUP.md`.
- The log records which copy of each is used at startup.

## 3. Layout

Video preview on the left, tools on the right (see the approved mockup), and an optional **Clips drawer** on the far left (section 3a).

**Left side:**
- Original / Result toggle, plus a source info line (resolution, frame rate, codec)
- the player
- a timeline with draggable start and end handles, kept in sync with the trim boxes. Dragging on it scrubs: the playhead follows the cursor at once, the picture jumps to keyframes while moving, and an exact seek lands on release (v0.4.1).
- a **waveform** drawn faintly inside the timeline (added in v0.5.0, issue #7): the mix of the audio tracks that are on, at their volumes, so you can see the loud moments. A **Waveform** toggle under the timeline, next to the end time, shows or hides it (remembered, on by default). It's worked out in the background by FFmpeg (8 kHz mono, one peak per 1/100 s per track), drawn on a decibel scale, and hidden while the exported file is playing.
- **audio track lanes** under the timeline (section 8a), only for files with two or more audio tracks
- transport buttons (frame back, play, frame forward), plus Set start and Set end
- a **ghost line**: a thin, faint line on the timeline where playback last started (set whenever the player goes from paused to playing, on the source only; cleared when another video is opened). **Go to ghost** moves the playhead there, **Start at ghost** sets the trim start there, **Play at ghost** jumps back to it and plays (the line stays put), and a **Ghost** toggle shows or hides the line (remembered).
- a **Listen** volume slider (0–100 %, remembered), at the right of the shortcut strip, for the player only. It never changes the export; the clip's own volume is in Audio and video.
- a shortcut strip showing the current bindings

**Right side, top to bottom:**
1. Output folder
2. Output name
3. Trim
4. Zoom
5. Cut mode
6. Speed
7. Audio and video
8. Quality and codec: a **Preset** dropdown on top (section 10a), then the codec and quality controls, with a collapsible Advanced section
9. **Estimate** card (section 10a)
10. Export button, progress bar, time remaining and Cancel

**Also:**
- Open a file with the Open button, `Ctrl+O`, drag and drop, or "Edit with cheapedit" in Explorer.
- An in-app **Log viewer** panel.

## 3a. Clips drawer (added in v0.4.0)

A drawer on the far left with two file lists, one above the other.

- **Collapsible:** a **Clips** button with a three-line menu icon (drawn, since the ☰ glyph isn't in the built-in fonts) at the left of the top bar shows or hides it. It's about 250 px wide and resizable (190–480 px).
- **Split:** **Editing files** on top, **Finished files** below, with a divider that can be dragged (each half 15–85 %, starting at 50/50). Each half scrolls on its own. Open/closed, width, split and folders are remembered.
- **Editing files:** the clips folder the user chooses, with its subfolders (up to 6 levels, 20,000 files). The **output folder is left out**, so exports only appear under Finished.
- **Audio files** (`mp3`, `m4a`, `wav`, `flac`, `ogg`, `opus`) are listed in both halves alongside videos (v0.5.0, issue #5: MP3 exports were missing from Finished). Each half has its own **Audio** checkbox in its header to hide them; both are on by default and remembered.
  - No folder yet: a short explanation and a **Choose clips folder** button. **Change** switches folder later.
  - **Click** opens the clip (blocked during an export, like Open). No dragging. **Right-click:** Show in folder, Copy file.
  - A green tick (drawn with lines, since the built-in fonts have no check-mark glyph) marks a clip that has been exported. Hovering it shows "Exported 3 times, last 17-11-26 21-04" (see Export history below).
- **Finished files:** the files directly in the output folder (no subfolders), the same folder as Output folder in the tools.
  - No output folder yet: a short explanation and a **Choose output folder** button. **Change** sets the output folder.
  - **Drag** carries the real file into Discord, Explorer, a browser or any app, using Windows' own drag and drop, as if from Explorer. **Click** opens it in the editor. **Right-click:** Show in folder, Copy file (puts the file on the clipboard like Explorer's Copy, so Ctrl+V pastes it).
  - After an export, the new file is outlined in green for 5 seconds (fading), its game group is opened, and it's scrolled into view.
- **Scanning:** in the background. Both lists rescan when the drawer opens, on **Refresh**, when either folder changes, and after every export. A rescan asked for during a scan runs straight after it. There's no live file watcher.
- **Game**, in order:
  1. the folder the clip is in (ShadowPlay saves to `Videos\NVIDIA\<Game>\`), unless it's the listed folder itself or a generic name (Captures, Clips, Replays, Videos, NVIDIA, cheapedit, …)
  2. the text before the date in the file name: a recorder's `YYYY.MM.DD` / `YYYY-MM-DD` (`Valorant 2026-11-17 21-04-12.mp4`, `Ready Or Not 2026.07.21 - 23.22.26.07.DVR.mp4`), or cheapedit's own `dd-mm-yy hh-mm` (`Call of Duty Black Ops 6 17-11-26 21-04.mp4`)
  3. otherwise **Unsorted**

  Double spaces are squashed (ShadowPlay writes `Call of Duty  Black Ops 6` where it dropped a colon), and names that differ only in case are merged.
- **Order:** collapsible game groups. Inside each, newest first by **modified** date. Groups are ordered by their newest clip, with Unsorted last.
- **Rows:** the name, then a relative date ("2 h ago") and the size. No thumbnails. The file open in the editor is highlighted.

**Export history** (`%APPDATA%\cheapedit\exports.json`):
- Each successful export is recorded against its source's full path (ignoring case): a count and the time of the last one. A Discord re-encode counts once, and cancelled or failed exports don't count.
- A source that's renamed or moved loses its tick. The tick **stays** if the export is deleted: it records that the clip was dealt with.
- **One-time catch-up:** the first time history is kept (once the clips list has been scanned and an output folder is set), any file in the output folder named `<source name>_…` (the naming before v0.4.0) marks that source as exported, using the file's modified time.

## 3b. Montage mode (v0.6)

Edit several clips one after another and join them into one file.

- **Montage** toggle in the top bar. With no montage open, a **Montage** window asks for a **projects folder** (first time; remembered, changeable), then offers **New montage** (a name) or a list of the folder's montages (newest first) to open. Closing it turns Montage mode off.
- **Projects:** each montage is its own file, `<name>.cheapmontage` (JSON), in the projects folder. Every change is **saved automatically** (half a second after it stops changing, via a temporary file so a crash can't lose it).
- **Montage strip** under the timeline: one card per clip, in order, with its number, name and finished length, and the **total length and clip count** at the end. Drag a card to move it (a coral line shows where it lands). Right-click: Duplicate, Remove (and Find file… when missing).
- **Montage section** at the top of the tools: the project name, "Saved automatically", **Add to montage** (or `M`), what happened last, and **Size** for the finished file: "Same as the first clip" (default) or 1920x1080, 1280x720, 2560x1440, 1080x1920, 1080x1080 at the first clip's frame rate, 30 or 60. **Switch** goes back to the Montage window.
- **Adding:** stores the open clip's **recipe**: source, trim, speed zones, zoom zones, overlays, audio tracks, audio mode, volume and pitch setting. Nothing is rendered yet. Audio-only files can't be added.
- **Re-editing:** clicking a card opens that clip with its recipe; it shows "Editing", and changes save into the card by themselves. Opening another file stops editing that card.
- **Missing source:** the card turns red ("File missing"); clicking it (or Find file…) picks its new location. Finish is blocked until it's fixed or removed.
- **Finish montage** replaces Export (and the Estimate card) while Montage is on. One set of Quality / codec / Preset settings applies to every clip, always in **Precise** mode. A Discord preset plans for the whole montage's length.
  - **Size and frame rate:** every clip is fitted to the montage's size (black bars, never stretched) at its frame rate, cut to a whole number of frames, with 48 kHz stereo sound cut to exactly the same length (a clip without sound, or with "Remove audio", gets a silent track). So the clips join **without re-encoding** and the sound never drifts.
  - **Hard cuts** between clips. Each clip keeps its own audio; there is no montage-wide music.
  - Each clip is rendered into a work folder, `.cheapedit-montage`, inside the output folder, then joined with the concat demuxer (`-c copy`). Work files are deleted afterwards, also after a failure or Cancel. One progress bar covers all clips and the join. A clip whose zones or overlays have a problem stops the finish with "Clip N: … Click its card to fix it."
- **Name:** the montage name and the date, `Valorant highlights 17-11-26 21-04.mp4`. Every source clip gets the green exported tick.

## 4. Output

- **The output folder is required.** It's remembered between sessions, and Export is blocked until it's set, exists and is writable.
- **The original file is never modified.**
- **Name box:** if left blank, the name is generated automatically as **game, then the export's date and time** (`dd-mm-yy hh-mm`, local time), e.g. `Call of Duty Black Ops 6 17-11-26 21-04.mp4`. Windows doesn't allow `:` in names, so the time uses a dash.
  - The game is worked out as in the Clips drawer when the source is inside the clips folder. Elsewhere only the file name is trusted (a folder like Downloads isn't a game).
  - If the game can't be worked out, the source's own name is used instead: `firefox 17-11-26 21-04.mp4`.
  - No trim, speed or zoom tags. The "Saves as" hint shows the name with the current minute. A Discord re-encode reuses the first try's name.
- **Name collisions** add `(2)`, `(3)` and so on. Files are never overwritten.
- Metadata from the source (e.g. creation date) is copied to the output.

## 5. Trim (single segment in v1)

- **Start and End boxes** accept `25`, `1:25`, `0:01:25.500` and decimals. Each shows the value converted to `h:mm:ss.ms` below it.
- **"Until end of video"** checkbox.
- **Times refer to the original video.** Trim is applied first, then speed. "New clip length" shows the final duration after the speed change.
- Cuts are stored internally as a list of segments, so adding multiple segments later is straightforward.

## 5a. Zoom zones (added in v0.3.0)

Zoom into part of the picture for a stretch of time. Done entirely with FFmpeg (`crop` with time expressions, then `scale` back to the output size), so it needs re-encoding.

- **Panel:** under Trim. With no zones it shows a **"+ New zone"** button and a one-line explanation. Up to **20 zones**; at 20 the button greys out with a short reason.
- **Greyed out** in Fast cut mode and for "Audio only", with the reason, like speed.
- **Each zone is a card**, top to bottom:
  - the zone number, its zoom (e.g. `2.0x`) and a delete button
  - **Start box** and **End box** columns, each with **Draw box** (until drawn), **Select box** (shows it on the video to move or resize it) and **Redraw**. Each button jumps the playhead to the zone's start or end frame.
  - start and end times (typed like trim, with `h:mm:ss.ms` below and a "Use playhead" button)
  - under the times, side by side: **Smooth** under Start (on by default, glide 0.5 s) and **No box change** under End. No box change is on by default. It locks the end box to the start box, so there's no pan, and greys out the end box's buttons. Turning it off starts drawing the end box.
  - Clicking the zone number selects the zone and jumps to its start.
- **Times refer to the original video**, like trim. Speed is applied after the zoom.
- **Drawing:** "+ New zone" pauses and you draw the box on the Original view. Boxes are locked to the video's aspect ratio. Drag the middle to move, the corners to resize. The selected box is highlighted; other zones show faintly while the playhead is inside their time.
- **Pan:** with No box change off, the zone pans from its Start box to its End box (they may differ in size). The move is eased (slow-fast-slow).
- **Smooth:** the glide happens **inside** the zone: full frame at the zone start, fully zoomed one glide later, zooming out one glide before the end, full frame at the end. With Pan on, the pan runs between the two glides. With Smooth off the zoom cuts in and out, and the pan spans the whole zone.
- **Touching zones:** when the gap between two smooth zones is no longer than the glide, the view glides straight from the first zone's (end) box to the next zone's (start) box instead of zooming fully out.
- **Timeline:** each zone is a numbered, coloured band under the timeline, matching its box and card. Clicking a band selects it; dragging its edges changes its times. Ctrl/Shift+Click still only set the trim.
- **Limits:** a warning above 3× enlargement; a box can't be smaller than 1/10 of the frame.
- **Checks that block Export:**
  - a zone's box hasn't been drawn yet; a panning zone's end box hasn't been drawn (offers "No box change")
  - zones overlap (offers a fix)
  - a zone isn't inside the trim, e.g. after the trim was changed (offers "Shrink zone to fit" and "Delete zone")
  - a smooth zone is shorter than two glides; a zone with Smooth off is shorter than 0.1 s
- **Output:** the zoom never changes the output resolution. HDR is converted to SDR before the crop. (The automatic name has no zoom tag.)
- **Export filter:** `fps=<source fps>` (constant frame rate, so frames map 1:1) then `zoompan` with `d=1`, its `z`/`x`/`y` built as piecewise `if(lt(it,…))` expressions with smoothstep easing. (Tested: `crop` can't do this, because it fixes its size at start-up.)
- **Preview:** Result's live preview drives mpv's `video-zoom` / `video-pan-x/y` from the same segments the FFmpeg expressions are built from (unit-tested to agree). zoompan can't run inside mpv, because it renumbers timestamps, which breaks seeking. The letterbox bars are kept black. A note says to use static preview if playback stutters (the export is not affected). A **"Static zoom preview"** checkbox shows box outlines instead of the live zoom.
- **Shortcuts:** `Z` new zone, `Delete` delete the selected zone (both rebindable).
- Zones are **not saved** between sessions and are cleared when another video is opened.

## 5b. Tracked zoom zones (v0.6)

A zoom zone can follow something on screen by itself.

- **"Track: follow what's in the box"** checkbox on a zoom zone card (turns off panning). Draw the Start box around the thing to follow and click **Track**: it runs in the background with a progress bar and Cancel (one zone at a time), then the zoom follows it for the whole zone. **Track again** starts over from the start box.
- **Tracker:** the **VitTrack** model from OpenCV's model zoo (Apache 2.0), run with **ONNX Runtime 1.24.4 (DirectML build)**, both bundled in `deps\` (`onnxruntime.dll`, `DirectML.dll`, `modelsittrack.onnx`, about 36 MB together) with their licences, pinned in `tools\deps.toml`. Changed from NanoTrack on 2026-11-17: NanoTrack's models state no licence, so they can't be shipped; VitTrack is the same kind of small, fast tracker. 1.24.4 is the last ONNX Runtime with a DirectML build.
  - Pre- and post-processing follow OpenCV's TrackerVit: a 128x128 template from twice the box, a 256x256 search area four times the box, BGR pixels with ImageNet mean/std, a 16x16 score map weighted by a Hann window; below a 0.2 score the target counts as lost for that frame and the box holds still.
  - Frames come from FFmpeg as raw pictures, at most 640 wide and 30 per second.
- **CPU or GPU:** the CPU by default; **Features** can switch to the graphics card (DirectML). If the GPU fails, it uses the CPU by itself, logs why, and Features shows it ("Last run: GPU failed (…), so the CPU was used.").
- **The box keeps its size** (the zoom level doesn't pump in and out); only its position follows. A **Smoothness** slider (Gaussian smoothing, up to 0.8 s) makes the camera glide. Smooth glide in and out of the zone still applies.
- **Fixing mistakes:** pause where it went wrong and click **Fix at playhead**; a yellow box appears on the video, drag it onto the target and let go. That frame becomes a **fix** and tracking re-runs **forwards** from it, keeping everything before. Fixes are listed on the card with **Remove** (tracks again from the fix before it). Smoothing never blurs across a fix.
- **On the video:** after the zone's first frame, Original shows the tracked box ("Zone N - tracked") where the zoom will be.
- **Export:** the smoothed path is simplified to at most 40 keyframes per zone (straight lines between them, within 0.2% of the frame), built into the same zoompan expressions as other zones. **Preview** drives mpv's zoom/pan from the same path. If the tracked zones together would make the command line too long for Windows, Export is blocked with a hint to raise Smoothness or use fewer tracked zones. A zone set to Track that hasn't been tracked blocks Export too.
- Tracked paths are saved with a montage's clips.

## 5c. Overlays (v0.6)

Images, GIFs, videos and text on top of the clip.

- **Panel:** under Zoom. **"+ Image or GIF"**, **"+ Video"**, **"+ Text"**, and **"Search KLIPY"** (section 5d). Up to **20 overlays**. Greyed out in Fast cut mode and for "Audio only".
- **Assets:** an **Assets folder** the user chooses (like the clips folder, remembered) is listed inside the panel: images, GIFs, WebP and videos (newest first, with the file type), in a short scrolling list. Clicking one adds it as an overlay at the playhead. "+ Image or GIF" / "+ Video" can also pick any file.
- **Each overlay is a card**, like a zoom zone: type and number, name, ▲/▼ (Bring forward / Send back) and Delete; start and end times (original time, with "= in final video"); **Start position / End position** and **No position change** (on = stays put; off = eased glide from the start position to the end position).
- **Placing:** a new overlay appears centred (media a third of the frame wide). The selected overlay is shown at the position being edited while paused: drag it to move it, drag a corner to resize a picture or video (keeping its shape). The end position only moves it; its size stays the start's. Text is sized with its Size slider.
- **Controls on every overlay:** opacity (0–100 %), fade in and fade out (0–2 s), **Bring forward / Send back** (the card order is the layer order; later cards are on top).
- **GIF / animated WebP:** Loop (default) or Play once (then holds the last frame).
- **Video:** its sound is off by default; **Sound** turns it on, with a volume slider on the card (mixed into the clip's single audio track). **"Follow clip speed"** (off by default): when on, it speeds up and slows down with the speed zones.
- **Text:** the text (several lines allowed), a font from the fonts installed in Windows, size, colour, outline (colour, width), shadow, and a background box (colour, opacity). Built with FFmpeg's `drawtext`.
- **Speed:** GIFs, images and text always play at normal speed, even inside a speed zone.
- **Timeline:** each overlay is a band under the speed and zoom bands, in its card's colour.
- **Preview:** cheapedit draws the overlays over the player itself (in both views, while editing): pictures and animated GIF/WebP frames, and text with its font, outline, shadow, background, opacity, fades and movement. Video overlays show a still frame in the preview; the export plays them. Text may differ slightly from the export in spacing.
- **Layers:** the card order is the layer order, except that "Follow clip speed" videos always sit under the other overlays (they're added before the speed change).
- **Checks that block Export:** a box not drawn; an overlay not inside the trim; an overlay file missing; empty text.
- **Shortcut:** `T` new text overlay.
- Overlays are not saved between sessions (except inside a montage).

## 5d. KLIPY GIF search (v0.6)

Uses KLIPY's GIF API (`https://api.klipy.com/api/v1/{key}/gifs/search` and `/gifs/trending`, 24 per page, `content_filter=off`) through Windows' own `curl.exe`, like the update check.

- **Features** (a top-bar button, a small window) has a **KLIPY API key** box (hidden like a password, with Show) and "How to get a free key": make an account at partner.klipy.com, create a platform under API Keys, paste the key. It explains why it's your own key, the Testing-mode limit (100 searches an hour until KLIPY grants Production access), and where the key is kept (cheapedit's settings file). Without a key, "Search KLIPY" opens Features instead.
- **KLIPY's terms:** they say apps shouldn't store KLIPY GIFs (only search thumbnails). The user chose (2026-11-17) to download picked GIFs into Assets anyway; the guide says so plainly and points to developers@klipy.com.
- **Search window:** a box with the placeholder "Search KLIPY" (required by KLIPY), results after a short pause in typing (or Enter), Trending when the box is empty, a grid of animated previews in KLIPY's order (nothing re-ordered or filtered), "Powered by KLIPY", and **More** for the next page. Ads or other non-GIF items in the results are skipped.
- **Clicking a result** downloads it (HD, or the next size down) to the Assets folder as **WebP when offered, otherwise GIF** (both keep transparency), named after the search plus its id (`gg-ez_8421.webp`; reused if already there), and adds it as an overlay at the playhead. Downloads go through a `.part` file, so a failed one leaves nothing behind. Without an Assets folder it asks for one first.
- Thumbnails are cached in the temp folder while the app runs.
- Animated WebP overlays loop with FFmpeg's `loop` filter (`-stream_loop` hangs on animated WebP); GIFs use `-ignore_loop 0`.
- Network errors, a rejected key and a used-up limit are shown in plain English. Only the search text and the key are sent to KLIPY; no user id.
- **Shortcut:** `G` opens the search.

## 6. Cut modes

| Mode | What it does | Plain-English explanation in the app |
|---|---|---|
| **Precise** (default) | Re-encodes the clip | "Cuts are exact to the frame. Slower, and quality depends on your settings." |
| **Fast** | Stream copy (`-c copy`) | "Done in about a second with no quality loss, but the start snaps to the nearest earlier keyframe, so your clip may begin slightly early." |

- In Fast mode, speed, codec, quality, frame rate, resolution and volume are **greyed out**, with the note "Switch to Precise to change this."
- Remove audio, mute, and changing the container all still work in Fast mode, as long as the new format can hold the source's codecs.

## 7. Speed zones (replaces the speed slider in v0.6)

Change the speed of parts of the clip: slow motion or fast forward between a start and an end time.

- **Panel:** where the old Speed section was. With no zones it shows **"+ New zone"**, a **"Whole clip"** button (one zone covering the trim, which does what the old slider did) and a one-line explanation. Up to **20 zones**.
- **Greyed out** in Fast cut mode, with the reason, like zoom. They also work for "Audio only" (the MP3 is sped up or slowed down).
- **Each zone is a card:** its number, its speed (e.g. `0.5x`) and a delete button; start and end times (typed like trim, with "Use playhead"); a **speed slider** from 0.25× to 4× in 0.25 steps plus a box for typed values; and:
  - **Smooth slow-mo** (only below 1×, off by default): FFmpeg's `minterpolate` makes the in-between frames. A note says it makes the export much slower.
  - **Audio:** **Keep (pitch-corrected)** (default, `atempo`, chained for extreme values) or **Mute**.
  - "Change pitch with speed" (tape-style, `asetrate` + `aresample`) stays as one checkbox for all zones.
- **Times refer to the original video**, like trim and zoom. Each time box shows `h:mm:ss.ms` and, below it, "= 0:14.2 in final video".
- **The timeline shows finished-video time in both views.** A 0.5× zone takes twice its width, a 2× zone half. Zones are coloured bands under the timeline, like zoom zones (dragging edges changes their times). The timeline re-flows **when the mouse is released**; while dragging, a label under the timeline shows the change ("New length 0:42 to 0:58"; drawn glyphs only, since arrows and triangles aren't in the built-in fonts). The end-time label and "New clip length" show the finished length.
- **Original view** plays at normal speed (the playhead moves slower through a stretched slow zone). **Result view** plays the zones at their speeds.
- **Checks that block Export** (same wording style as zoom): zones overlap (offers a fix); a zone isn't inside the trim (offers "Shrink zone to fit" / "Delete zone"); a zone is shorter than 0.1 s.
- **Export:** the trimmed clip is split at zone edges with `trim`/`atrim`, each piece gets its own `setpts` (and `minterpolate` / `atempo` / silence), then `concat` joins them. Zoom (and video overlays with "Follow clip speed") is applied **before** speed. Other overlays are applied **after** speed, with their times converted to finished-video time, so GIFs and text play at normal speed.
- **Shortcut:** `S` new speed zone. `Delete` deletes the selected zone or overlay.
- Zones are not saved between sessions (except inside a montage, section 3b).

## 8. Audio and video modes

1. **Keep video and audio**
2. **Mute**: the video keeps a silent audio track
3. **Remove audio track**: no audio track at all
4. **Audio only (MP3)**: no video. Bitrate choices are 128, 192 (default) and 320 kbps.

A volume slider (0–200%) appears in every mode that has audio.

## 8a. Several audio tracks (added in v0.5.0, issue #6)

Recorders like ShadowPlay can put game sound and the microphone on separate tracks. Files with **two or more** audio tracks get one **lane** per track under the timeline; files with one track look exactly as before.

- **Lane:** "Track N", plus the track's name from the file when it has a real one (MKV `title`, MP4 `name`; boilerplate like "SoundHandler" is ignored). Under it, the track's own waveform with the trim range and playhead; clicking or dragging it seeks like the timeline.
- **On/off** (checkbox): the track is in the export and the preview. If every track is off, the export has no audio and a warning says so.
- **Volume** (0–200 %, with Reset): the track's level in the mix.
- **Solo:** hear only the soloed track(s) while editing. Preview only; it never changes the export.
- Track settings belong to the open file and reset when another file is opened. They're not saved.
- **Export:** the tracks that are on are mixed into **one** audio track (`amix` with `normalize=0`, so each keeps its loudness), because Discord, browsers and phones only play the first track of a file. The Audio and video volume, Mute and Remove audio apply on top as a master control. A single track that's on at 100 % is taken as it is (so Fast mode can still copy it); any real mix is re-encoded, also in Fast mode.
- **Preview:** mpv mixes the same tracks with a `lavfi-complex` graph. It's only updated when no mouse button is held, so dragging a volume slider doesn't restart the audio on every frame.

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

## 10a. Discord presets and the Estimate card (added in v0.4.0)

- **Preset dropdown** at the top of Quality and codec: **Custom**, **Discord 10**, **Discord 20**, **Discord 50**. The number is the size cap in MB (10 = the free Discord limit, 50 = Nitro Basic). 1 MB counts as 1,000,000 bytes, the smaller reading.
- The choice is remembered with the last-used options. A preset never overwrites the user's Custom settings. It only overrides them while it's chosen, and Custom brings them back.
- **What a Discord preset fixes:** Precise mode, MP4, H.264, AAC (128 kbps, or 96 when the budget is tight), and its own resolution, frame rate and video bitrate. Those controls are greyed out showing the preset's values, with "Set by Discord N". Trim, speed, zoom, volume, Mute and Remove audio still work. **Audio only** is greyed out ("Not available with Discord presets"). Remove audio and Mute give the audio's share of the budget to the picture. A file with no picture ignores the preset.
- **Budget:** (cap × 92 %) ÷ output length gives audio + video. The 8 % headroom covers the MP4 container and encoder overshoot. The video peak (`-maxrate`) is held at the target, not 1.5×.
- **Step-down:** the highest of 1080p60 → 1080p30 → 720p30 → 480p30 whose video bitrate is still decent (about 4,500 / 3,000 / 1,500 / 250 kbps). It never upscales and never raises the frame rate above the source.
- **Too long:** if even 480p would get under 250 kbps of video, Export is blocked: "Too long for Discord 10 (10 MB) even at 480p. Trim it to under m:ss." The time is in original-video time, so it allows for speed.
- **Guarantee:** after the export, if the file is over the cap, it's deleted and re-encoded **once** with the video bitrate scaled by (cap × 92 % ÷ actual size × 95 %). If it's somehow still over, the file is kept and a warning shows under "Saved …" (and in the log). (Tested on noisy synthetic footage and real game clips, with x264 and NVENC: the first pass lands at 88–97 % of the cap.)
- **Estimate card**, above Export, always shown once a video is open: size, resolution, frame rate, video and audio bitrate.
  - Custom: a rough size marked "≈" (bitrate marked "≈" unless it's a typed custom bitrate), with "A rough guess…".
  - Discord: the exact planned values and "fits under N MB" in green.

## 11. Preview

- **Original** plays the source as it is.
- **Result, before exporting:** a live preview in mpv, with the trim range looping and speed, volume and mute applied.
- **Result, after exporting:** the view switches to Result by itself and plays the exported file, with an "Open in folder" button. (Dragging the file out is done from Finished files in the Clips drawer.)
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
| `Z` | New zoom zone |
| `Delete` | Delete the selected zone or overlay |
| `S` | New speed zone |
| `T` | New text overlay |
| `G` | Open the KLIPY search |
| `M` | Add the current clip to the montage |
| `Ctrl+Click` on the timeline | Set start there (drag to keep moving it) |
| `Shift+Click` on the timeline | Set end there (drag to keep moving it) |

The timeline-click bindings are rebound by choosing a modifier: Ctrl, Shift, Alt, or a pair of them. A plain click always scrubs. While the modifier is held, a coral line and label show where the point will land, and a hint under the timeline lists both combos.

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
- Holds the output folder, last-used options (including the preset), keybindings, the Clips drawer's folder, open/closed state, width, split and Audio checkboxes, and the Ghost and Waveform toggles. Export history is kept separately in `exports.json`.
- From v0.6 it also holds the Assets folder, the montage folder, the KLIPY key and the tracker's CPU/GPU choice.
- **Features window** (v0.6, top-bar button): the KLIPY key with its guide, and the tracker's **CPU / Graphics card (GPU)** choice with what the last run used.

## 16. Installer and branding

- **The official name is `cheapedit`.**
- **Installer:** Inno Setup, producing `cheapedit-Setup.exe`.
  - Per-user install by default, to `%LOCALAPPDATA%\Programs\cheapedit`, with no admin needed. "Install for all users" is an option.
  - Start menu shortcut.
  - Desktop shortcut: a checkbox, off by default.
  - Listed in Apps with an uninstaller.
  - **"Edit with cheapedit"** on the right-click menu for video files: a checkbox, on by default. This is registered per-user, under HKCU.
  - On uninstall, asks "Also remove your settings and logs?", defaulting to no.
  - Shows cheapedit's licence (GPL-3.0) as a wizard page.
  - Bundles FFmpeg and libmpv into `{app}\deps` (section 2). `deps\` is deleted and rewritten on every install or update, so an older FFmpeg's DLLs never linger. The installer is about 100 MB.
  - At the end: "It comes with its own FFmpeg and libmpv, so there's nothing else to set up."
- **Licence:** cheapedit is **GPL-3.0-or-later**, Copyright (C) 2026 JWK95 (`LICENSE`, and `license` in Cargo.toml). Bundling GPL FFmpeg and loading GPL libmpv in-process need a GPL-compatible licence.
  - **About** has an **Includes** section: the bundled FFmpeg and libmpv versions, a note that they're GPL with their exact source attached to every release, and **Open licences folder**.
- **Pinned dependencies:** `tools\deps.toml` pins each build's download URL, version and SHA-256, and the same for the **exact source** it was built from (FFmpeg: the commit Gyan names in the release notes; mpv: the commit in shinchiro's file name).
  - `tools\fetch-deps.ps1` downloads into `target\deps-cache`, refuses any file whose hash doesn't match (and deletes it), unpacks with Windows' own `tar` (7-Zip as a fallback), and lays out `target\deps\`. `-Pin` writes new hashes after a version change.
  - `tools\dev.ps1` runs it once if `target\deps` is missing, so dev builds use the same files the installer ships.
- **Unsigned for now.** The README explains the SmartScreen warning.
- **Auto-update from GitHub releases** (public repo `JWK95/cheapedit`). Changed from "no update checks" on 2026-11-07 at the user's request.
  - One check on startup against the GitHub "latest release" API, done with Windows' built-in `curl.exe`. It can be switched off in **About**, and there's a **Check now** button there too. Apart from the KLIPY search (section 5d, only when the user has added their own key and searches), this is the only network traffic the app ever makes, and it sends nothing about the user. (Downloading FFmpeg and libmpv happens only on the developer's machine, in `fetch-deps.ps1`, never in the app.)
  - A newer version shows a banner with **Update now / What's new / Later**. Nothing installs without a click, and updating is blocked while an export is running.
  - The installer is downloaded, its SHA-256 is checked against the `.sha256` file published with the release (a release without one is refused), and it then runs with `/SILENT /RELAUNCH=1`. It updates in place, keeps settings, and reopens cheapedit.
- **Dev and release builds.**
  - Dev: `tools\dev.ps1` (`cargo run`) gives `target\debug\cheapedit.exe`, which needs no install. It shows a DEV badge, keeps its settings and logs in `%APPDATA%\cheapedit-dev`, and never checks for updates.
  - Release: `tools\release.ps1 -Version X.Y.Z -Notes "..."` fetches and checks the pinned FFmpeg and libmpv, sets the version, runs the tests, builds the exe, the installer `cheapedit-Setup-X.Y.Z.exe` and its `.sha256`, then commits, tags, pushes and creates the GitHub release. The release notes get an "Includes" list, and the FFmpeg and mpv **source archives** are attached to the release (the GPL's source requirement).
  - Each update downloads the full installer, about 100 MB, even when FFmpeg and libmpv haven't changed (chosen for simplicity).
- **Icon:** a film strip with a play triangle, and coral scissors cutting through the lower right, on a dark purple rounded square (approved draft). Shipped as a multi-size `.ico` (16–256 px), simplified at the small sizes.

## 17. Out of scope for v1

- multiple trim segments inside one clip (joining whole clips is Montage mode, section 3b)
- transitions between montage clips, montage-wide music, overlay rotation (not planned for v0.6)
- an export queue
- test renders
- HDR output
- 2-pass encoding
- macOS and Linux
- code signing
- light mode (permanently out of scope)
