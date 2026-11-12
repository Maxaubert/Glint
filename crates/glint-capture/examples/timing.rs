use std::time::Instant;
fn main() {
    glint_capture::make_dpi_aware();
    for _ in 0..3 {
        let t = Instant::now();
        let mons = glint_capture::monitors::enumerate();
        let t1 = t.elapsed();
        let vb = glint_core::geom::virtual_bounds(&mons);
        let _img = glint_capture::gdi::capture(&vb).unwrap();
        let t2 = t.elapsed();
        let outs = glint_capture::hdr::hdr_outputs().unwrap();
        let t3 = t.elapsed();
        let n = outs.len();
        for o in &outs {
            let w = glint_capture::hdr::sdr_white_level(&o.device);
            let _ = glint_capture::hdr::capture_output(o).map_err(|e| eprintln!("{e}"));
            eprintln!("hdr {} white {:?}", o.device, w);
        }
        let t4 = t.elapsed();
        println!("mons {:?} gdi {:?} hdr-list {:?} ({n}) hdr-cap {:?} | {:?}", t1, t2 - t1, t3 - t2, t4 - t3, mons);
    }
}
