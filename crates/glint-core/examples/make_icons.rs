//! Writes assets/glint.ico (16 to 256 px) and assets/glint-256.png from the icon drawn in code.
//! Run: cargo run -p glint-core --example make_icons
fn main() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    std::fs::create_dir_all(&dir).unwrap();
    let mut dirx = ico::IconDir::new(ico::ResourceType::Icon);
    for n in [16, 20, 24, 32, 40, 48, 64, 128, 256] {
        let img = ico::IconImage::from_rgba_data(n, n, glint_core::icon::pixels(n));
        dirx.add_entry(ico::IconDirEntry::encode(&img).unwrap());
    }
    dirx.write(std::fs::File::create(dir.join("glint.ico")).unwrap())
        .unwrap();
    let img = glint_core::Image {
        width: 256,
        height: 256,
        data: glint_core::icon::pixels(256),
    };
    std::fs::write(
        dir.join("glint-256.png"),
        glint_core::encode::encode_png_best(&img),
    )
    .unwrap();
}
