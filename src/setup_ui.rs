//! The setup screen: checks for FFmpeg, ffprobe and libmpv and teaches how to
//! install anything missing the right way. It never installs or changes anything itself.

use crate::deps::{Report, Status};
use crate::theme;
use eframe::egui::{self, RichText};

pub const LIBMPV_RELEASES: &str = "https://github.com/shinchiro/mpv-winbuild-cmake/releases";

// One line on purpose: multi-line pastes can run line-by-line in some terminals and break.
pub const ADD_TO_USER_PATH: &str = r#"$dir="$env:USERPROFILE\Tools\libmpv"; $p=[string][Environment]::GetEnvironmentVariable('Path','User'); if (($p -split ';') -notcontains $dir) { [Environment]::SetEnvironmentVariable('Path', ($p.TrimEnd(';') + ';' + $dir), 'User') }"#;

pub enum SetupAction {
    None,
    CheckAgain,
    Continue,
}

/// Draw the setup screen. `can_close` is true when opened from the editor.
pub fn show(ui: &mut egui::Ui, report: &Report, can_close: bool) -> SetupAction {
    let mut action = SetupAction::None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.set_max_width(820.0);
        ui.add_space(12.0);
        ui.heading(RichText::new("Let's get cheapedit ready").size(24.0));
        ui.add_space(4.0);
        ui.label(
            "cheapedit is a front end: FFmpeg does the editing and libmpv plays the video. \
             You install them once, system-wide, so every tool on your PC can use the same up-to-date copy.",
        );
        ui.add_space(12.0);

        status_row(ui, "FFmpeg", "Does all the cutting, converting and encoding.", &report.ffmpeg);
        status_row(ui, "ffprobe", "Reads a video's length, codecs and frame rate. Comes with FFmpeg.", &report.ffprobe);
        status_row(ui, "libmpv", "Plays the video in the preview.", &report.libmpv);

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(RichText::new("Check again").size(15.0)).clicked() {
                action = SetupAction::CheckAgain;
            }
            if report.all_ok() {
                let label = if can_close { "Back to editor" } else { "Continue" };
                if ui.add(theme::primary_button(label)).clicked() {
                    action = SetupAction::Continue;
                }
            } else {
                ui.label(RichText::new("All three are needed before cheapedit can open.").color(theme::MUTED));
            }
        });
        ui.label(
            RichText::new("Check again re-reads your PATH straight from Windows, so you don't need to restart cheapedit.")
                .small()
                .color(theme::MUTED),
        );

        ui.add_space(16.0);
        let ff_missing = !report.ffmpeg.is_ok() || !report.ffprobe.is_ok();
        egui::CollapsingHeader::new(RichText::new("How to install FFmpeg (and ffprobe)").size(17.0))
            .default_open(ff_missing)
            .show(ui, ffmpeg_guide);
        ui.add_space(6.0);
        egui::CollapsingHeader::new(RichText::new("How to install libmpv").size(17.0))
            .default_open(!report.libmpv.is_ok())
            .show(ui, libmpv_guide);
        ui.add_space(6.0);
        egui::CollapsingHeader::new(RichText::new("What is PATH, and why use it?").size(17.0))
            .default_open(false)
            .show(ui, path_explainer);
        ui.add_space(24.0);
    });
    action
}

fn status_row(ui: &mut egui::Ui, name: &str, what: &str, status: &Status) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let color = match status {
                Status::Ok { .. } => theme::OK,
                Status::Missing => theme::DANGER,
                Status::Problem { .. } => theme::WARN,
            };
            let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 22.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 6.0, color);
            ui.vertical(|ui| {
                ui.label(RichText::new(name).strong().size(15.0));
                ui.label(RichText::new(what).color(theme::MUTED));
                match status {
                    Status::Ok { path, version, note } => {
                        ui.label(format!("Found {version}"));
                        ui.label(RichText::new(path.display().to_string()).monospace().small().color(theme::MUTED));
                        if let Some(n) = note {
                            ui.label(RichText::new(n).small().color(theme::MUTED));
                        }
                    }
                    Status::Missing => {
                        ui.label(RichText::new("Not found on your PATH.").color(theme::DANGER));
                    }
                    Status::Problem { path, reason } => {
                        ui.label(RichText::new(reason).color(theme::WARN));
                        ui.label(RichText::new(path.display().to_string()).monospace().small().color(theme::MUTED));
                    }
                }
            });
        });
    });
    ui.add_space(4.0);
}

fn step(ui: &mut egui::Ui, n: u32, title: &str, body: &str, why: Option<&str>) {
    ui.add_space(6.0);
    ui.label(RichText::new(format!("{n}.  {title}")).strong());
    ui.label(body);
    if let Some(why) = why {
        ui.label(RichText::new(format!("Why: {why}")).italics().color(theme::MUTED));
    }
}

fn command_block(ui: &mut egui::Ui, cmd: &str) {
    theme::code_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(RichText::new(cmd).monospace()).wrap());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(cmd.to_string());
                }
            });
        });
    });
}

fn ffmpeg_guide(ui: &mut egui::Ui) {
    ui.label("Recommended: install with winget, Microsoft's package manager that's built into Windows 11.");
    step(ui, 1, "Open Terminal", "Right-click the Start button and choose Terminal. Admin isn't needed.", None);
    step(
        ui,
        2,
        "Install FFmpeg",
        "Paste this and press Enter:",
        Some("winget downloads the trusted gyan.dev build, adds it to your PATH for you, and keeps it updatable."),
    );
    command_block(ui, "winget install --id Gyan.FFmpeg -e");
    step(
        ui,
        3,
        "Keep it up to date (later on)",
        "Every so often, update everything winget installed:",
        Some("FFmpeg gets regular fixes and faster encoders. One command updates all your winget apps."),
    );
    command_block(ui, "winget upgrade --all");
    step(
        ui,
        4,
        "Click Check again",
        "ffprobe comes in the same package, so both should turn green.",
        Some("Programs read PATH when they start, so terminals you already had open won't see FFmpeg until you reopen them. cheapedit reads it fresh from Windows."),
    );
    ui.add_space(6.0);
    ui.label(
        RichText::new("Already use Chocolatey or Scoop? 'choco install ffmpeg' or 'scoop install ffmpeg' work just as well — cheapedit accepts any working install.")
            .color(theme::MUTED),
    );
}

fn libmpv_guide(ui: &mut egui::Ui) {
    ui.label("libmpv isn't in winget, so you'll put it in a tools folder of your own and add that folder to your PATH.");
    step(
        ui,
        1,
        "Download the library build",
        "Open the releases page and download the newest file named mpv-dev-x86_64-….7z (if your PC is from 2015 or later, mpv-dev-x86_64-v3-….7z is a little faster).",
        Some("The 'mpv-dev' files are the library for apps like cheapedit. The plain 'mpv-x86_64' files are the mpv player app, which doesn't include libmpv-2.dll."),
    );
    ui.hyperlink_to(format!("Open {LIBMPV_RELEASES}"), LIBMPV_RELEASES);
    step(
        ui,
        2,
        "Make a tools folder you own",
        "This creates C:\\Users\\<you>\\Tools\\libmpv:",
        Some("A folder inside your user profile needs no admin rights, can't break other accounts, and is easy to find when you want to update it."),
    );
    command_block(ui, r#"New-Item -ItemType Directory -Force "$env:USERPROFILE\Tools\libmpv""#);
    step(
        ui,
        3,
        "Copy in libmpv-2.dll",
        "Double-click the .7z (Windows 11 opens it directly; otherwise use 7-Zip) and copy just libmpv-2.dll into that folder.",
        Some("The other files in the download (include folder, .dll.a) are only for programmers building against libmpv."),
    );
    step(
        ui,
        4,
        "Add the folder to your user PATH",
        "Paste this into Terminal (PowerShell). It adds the folder once and leaves everything else in your PATH alone:",
        Some("Your user PATH only affects your account and doesn't need admin. Avoid 'setx PATH …' — it can cut off long PATHs and copy the system PATH into yours."),
    );
    command_block(ui, ADD_TO_USER_PATH);
    ui.label(
        RichText::new("Prefer clicking? Start → search 'Edit environment variables for your account' → Path → Edit → New → paste the folder path → OK.")
            .color(theme::MUTED),
    );
    step(
        ui,
        5,
        "Click Check again",
        "To update libmpv later, download a newer mpv-dev file and replace libmpv-2.dll in the same folder.",
        None,
    );
}

fn path_explainer(ui: &mut egui::Ui) {
    ui.label(
        "PATH is a list of folders Windows searches when a program asks for a tool by name, like 'ffmpeg'. \
         Putting FFmpeg and libmpv on PATH means one copy serves every app and terminal on your PC, and updating \
         that one copy updates them all — instead of each app hiding its own old copy.",
    );
    ui.add_space(4.0);
    ui.label(
        "There are two PATHs: the system PATH (all users, needs admin) and your user PATH (just you, no admin). \
         Windows combines them. For your own tools, the user PATH is the safer choice.",
    );
}
