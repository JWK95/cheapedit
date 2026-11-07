//! Renders assets/icon.svg into assets/cheapedit.ico (16–256 px) and
//! assets/icon-64.rgba (the window icon). Run: `cargo run --example make_icon`

fn render(svg: &str, size: u32) -> Vec<u8> {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).expect("valid svg");
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).unwrap();
    let scale = size as f32 / tree.size().width();
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    // tiny-skia stores premultiplied alpha; icons want straight alpha.
    pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

fn main() {
    let full = std::fs::read_to_string("assets/icon.svg").unwrap();
    let small = std::fs::read_to_string("assets/icon-small.svg").unwrap();
    let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
    for size in [16, 24, 32, 48, 64, 128, 256] {
        let svg = if size <= 32 { &small } else { &full };
        let img = ico::IconImage::from_rgba_data(size, size, render(svg, size));
        dir.add_entry(ico::IconDirEntry::encode(&img).unwrap());
    }
    dir.write(std::fs::File::create("assets/cheapedit.ico").unwrap()).unwrap();
    std::fs::write("assets/icon-64.rgba", render(&full, 64)).unwrap();
    // A PNG for the README / installer wizard.
    let png = ico::IconImage::from_rgba_data(256, 256, render(&full, 256));
    png.write_png(std::fs::File::create("assets/icon-256.png").unwrap()).unwrap();
    println!("wrote assets/cheapedit.ico, assets/icon-64.rgba, assets/icon-256.png");
}
