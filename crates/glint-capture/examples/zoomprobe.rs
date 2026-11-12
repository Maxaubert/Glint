//! Which capture API sees the screen BEFORE a fullscreen magnifier (Wind) zooms it?
//! Zoom in with Wind, run this, keep still for 5 seconds. Writes gdi.png and dxgi.png (half size)
//! to the folder given as the first argument.
fn main() {
    glint_capture::make_dpi_aware();
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    std::fs::create_dir_all(&dir).unwrap();
    println!("Zoom in with Wind now. Capturing in 5 seconds...");
    std::thread::sleep(std::time::Duration::from_secs(5));
    let mons = glint_capture::monitors::enumerate();
    let r = mons[0].rect;
    let save = |name: &str, img: &glint_core::Image| {
        let t = img.thumbnail(img.width / 2, img.height / 2);
        std::fs::write(dir.join(name), glint_core::encode::encode_png_fast(&t)).unwrap();
        println!("wrote {}", dir.join(name).display());
    };
    match glint_capture::gdi::capture(&r) {
        Ok(img) => save("gdi.png", &img),
        Err(e) => println!("gdi failed: {e}"),
    }
    match glint_capture::hdr::hdr_outputs() {
        Ok(outs) if !outs.is_empty() => match glint_capture::hdr::capture_output(&outs[0]) {
            Ok(img) => save("dxgi.png", &img),
            Err(e) => println!("dxgi failed: {e}"),
        },
        Ok(_) => println!("no HDR output, so no DXGI probe on this setup"),
        Err(e) => println!("dxgi list failed: {e}"),
    }
    println!("Done. You can zoom out.");
}
