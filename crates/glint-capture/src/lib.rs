//! Screen capture for Glint.
//!
//! The whole virtual desktop is grabbed once, at hotkey time, with GDI `BitBlt` (fast, no yellow
//! border, no GPU state kept). Measured 2026-11-12 on Windows 11 26300 with an HDR 4K screen: GDI
//! already gives correct SDR colours for desktop content there (same pixels as the tone-mapped
//! DXGI copy), in ~55 ms. Real HDR content (games, video) clips in GDI, so `hdr` re-grabs HDR
//! monitors with DXGI Desktop Duplication in FP16 and tone-maps them. That costs ~200 ms (mostly
//! creating the D3D device), so it is opt-in.

pub mod gdi;
pub mod hdr;
pub mod monitors;
pub mod windows_list;

use glint_core::geom::virtual_bounds;
use glint_core::{Image, Monitor, Rect};

/// A frozen copy of the desktop.
#[derive(Debug)]
pub struct Frame {
    /// The whole virtual desktop. Pixel (0,0) is virtual point `origin`.
    pub image: Image,
    /// Virtual-desktop coordinates of the image's top-left pixel.
    pub origin: (i32, i32),
    pub monitors: Vec<Monitor>,
    /// Things that went wrong but did not stop the capture (for the log).
    pub notes: Vec<String>,
}

impl Frame {
    /// The virtual-desktop rectangle the image covers.
    pub fn bounds(&self) -> Rect {
        Rect::new(self.origin.0, self.origin.1, self.image.width as i32, self.image.height as i32)
    }

    /// Crops a rectangle given in virtual-desktop coordinates.
    pub fn crop_virtual(&self, r: &Rect) -> Image {
        self.image.crop(&r.translate(-self.origin.0, -self.origin.1))
    }
}

/// Grabs every monitor. Needs a per-monitor-DPI-aware process.
/// `hdr`: re-grab HDR monitors through DXGI and tone-map them (slower, better for HDR content).
pub fn capture_desktop(hdr: bool) -> Result<Frame, String> {
    let monitors = monitors::enumerate();
    if monitors.is_empty() {
        return Err("no monitors found".into());
    }
    let vb = virtual_bounds(&monitors);
    let mut image = gdi::capture(&vb)?;
    let mut notes = Vec::new();
    match if hdr { hdr::hdr_outputs() } else { Ok(Vec::new()) } {
        Ok(outs) => {
            for out in outs {
                match hdr::capture_output(&out) {
                    Ok(img) => image.blit(&img, out.rect.x - vb.x, out.rect.y - vb.y),
                    Err(e) => notes.push(format!("HDR capture of {} failed, kept GDI copy: {e}", out.device)),
                }
            }
        }
        Err(e) => notes.push(format!("HDR check failed: {e}")),
    }
    Ok(Frame { image, origin: (vb.x, vb.y), monitors, notes })
}

/// A made-up desktop (a colour gradient) with the real monitor layout. End-to-end tests use it,
/// because a hidden test desktop has no screen to copy from.
pub fn synthetic_desktop() -> Frame {
    let monitors = monitors::enumerate();
    let vb = virtual_bounds(&monitors);
    let mut image = Image::new(vb.w.max(1) as u32, vb.h.max(1) as u32);
    let (w, h) = (image.width as usize, image.height as usize);
    for (i, px) in image.data.chunks_exact_mut(4).enumerate() {
        let (x, y) = (i % w, i / w);
        px.copy_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, 128, 255]);
    }
    Frame { image, origin: (vb.x, vb.y), monitors, notes: vec!["synthetic desktop".into()] }
}

/// Turns on per-monitor DPI awareness v2 for this process (tests and tools; the apps use a manifest).
pub fn make_dpi_aware() {
    use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the real screen; harmless and quick, so it runs by default.
    #[test]
    fn captures_the_desktop() {
        make_dpi_aware();
        let t = std::time::Instant::now();
        let f = capture_desktop(false).expect("capture");
        let ms = t.elapsed().as_millis();
        eprintln!("captured {:?} at {:?} in {ms} ms, notes: {:?}", f.image, f.origin, f.notes);
        assert_eq!(f.bounds(), virtual_bounds(&f.monitors));
        let m = &f.monitors[0];
        let crop = f.crop_virtual(&Rect::new(m.rect.x, m.rect.y, 10, 10));
        assert_eq!((crop.width, crop.height), (10, 10));
        assert!(crop.data.chunks_exact(4).all(|p| p[3] == 255), "GDI capture must be opaque");
    }
}
