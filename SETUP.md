# Setting up cheapedit's tools

cheapedit is a front end. **FFmpeg** does all the editing and **libmpv** plays the video. You install them once, system-wide, so every tool on your PC shares the same up-to-date copy. cheapedit checks for them every time it starts and shows these same steps on its setup screen.

All three are required:

- **FFmpeg** (version 6.0 or newer)
- **ffprobe**, which comes with FFmpeg
- **libmpv-2.dll**

cheapedit never installs anything or changes your PATH itself.

---

## FFmpeg (and ffprobe)

The recommended way is **winget**, Microsoft's package manager. It's built into Windows 11.

1. Right-click Start and choose **Terminal**. You don't need admin.
2. Run:
   ```powershell
   winget install --id Gyan.FFmpeg -e
   ```
   *Why:* winget downloads the trusted gyan.dev build, adds it to your PATH, and keeps it updatable.
3. Later on, update it (along with everything else winget installed) with:
   ```powershell
   winget upgrade --all
   ```
4. Click **Check again** in cheapedit.

Already use Chocolatey (`choco install ffmpeg`) or Scoop (`scoop install ffmpeg`)? That works too. cheapedit accepts any working install.

---

## libmpv

libmpv isn't available in winget. You keep it in a tools folder you own and add that folder to your **user** PATH.

1. **Download.** Go to <https://github.com/shinchiro/mpv-winbuild-cmake/releases> and download the newest `mpv-dev-x86_64-….7z`. If your PC is from 2015 or later, `mpv-dev-x86_64-v3-….7z` is slightly faster.
   *Why:* the `mpv-dev` files are the library that apps like cheapedit use. The plain `mpv-x86_64` files are the mpv player app.
2. **Make a tools folder.**
   ```powershell
   New-Item -ItemType Directory -Force "$env:USERPROFILE\Tools\libmpv"
   ```
   *Why:* a folder in your user profile needs no admin rights and can't affect other accounts.
3. **Copy `libmpv-2.dll`** from the `.7z` into that folder. Windows 11 opens `.7z` files directly; otherwise use 7-Zip. The other files in the download are only for programmers.
4. **Add the folder to your user PATH.** This adds it once and leaves the rest of your PATH alone:
   ```powershell
   $dir="$env:USERPROFILE\Tools\libmpv"; $p=[string][Environment]::GetEnvironmentVariable('Path','User'); if (($p -split ';') -notcontains $dir) { [Environment]::SetEnvironmentVariable('Path', ($p.TrimEnd(';') + ';' + $dir), 'User') }
   ```
   It prints nothing when it works. To check, run `[Environment]::GetEnvironmentVariable('Path','User') -split ';'` — the last line should end in `\Tools\libmpv`.
   If you'd rather click: Start → *Edit environment variables for your account* → **Path** → **Edit** → **New** → paste the folder → **OK**.
   **Avoid `setx PATH …`.** It can cut long PATHs short and copy the system PATH into yours.
5. Click **Check again** in cheapedit.

To update libmpv later, download a newer `mpv-dev` file and replace `libmpv-2.dll` in the same folder.

---

## What is PATH?

PATH is the list of folders Windows searches when a program asks for a tool by name, such as `ffmpeg`. With your tools on PATH, one copy serves every app, and updating that copy updates them all.

There are two PATHs, and Windows combines them:

- **System PATH:** applies to all users and needs admin.
- **User PATH:** applies only to you and needs no admin. This is the safer choice for your own tools.

Programs read PATH when they start, so reopen any terminal after changing it. cheapedit's **Check again** button reads the current PATH directly from Windows, so you don't need to restart cheapedit.
