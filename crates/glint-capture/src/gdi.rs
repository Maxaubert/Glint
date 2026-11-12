//! GDI capture of a rectangle of the virtual desktop.

use glint_core::{Image, Rect};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};

/// Copies `r` (virtual-desktop pixels) from the screen.
pub fn capture(r: &Rect) -> Result<Image, String> {
    if r.is_empty() {
        return Err("empty capture rectangle".into());
    }
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("GetDC(screen) failed".into());
        }
        let mem = CreateCompatibleDC(Some(screen));
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: r.w,
                biHeight: -r.h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let result = match CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bmp) => {
                let old = SelectObject(mem, bmp.into());
                let ok = BitBlt(mem, 0, 0, r.w, r.h, Some(screen), r.x, r.y, SRCCOPY);
                SelectObject(mem, old);
                let out = match ok {
                    Ok(()) => {
                        let len = r.w as usize * r.h as usize * 4;
                        let src = std::slice::from_raw_parts(bits as *const u8, len);
                        Ok(Image::from_bgra(r.w as u32, r.h as u32, src, r.w as usize * 4, true))
                    }
                    Err(e) => Err(format!("BitBlt failed: {e}")),
                };
                let _ = DeleteObject(bmp.into());
                out
            }
            Err(e) => Err(format!("CreateDIBSection failed: {e}")),
        };
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        result
    }
}
