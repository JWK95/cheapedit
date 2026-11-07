// Embeds the icon and version info into cheapedit.exe (shown in Explorer and the taskbar).
fn main() {
    println!("cargo:rerun-if-changed=assets/cheapedit.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/cheapedit.ico");
        res.set("ProductName", "cheapedit");
        res.set("FileDescription", "cheapedit — a simple front end for FFmpeg");
        res.compile().expect("embedding the icon needs the Windows SDK's rc.exe (installed with Visual Studio Build Tools)");
    }
}
