//! The tray icon handle, built from the shared icon pixels.

use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

/// A Windows icon handle of size `n`.
pub fn hicon(n: u32) -> Option<HICON> {
    let px = glint_core::icon::pixels(n);
    let mut bgra = px.clone();
    for p in bgra.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    unsafe {
        let color = CreateBitmap(n as i32, n as i32, 1, 32, Some(bgra.as_ptr() as _));
        let mask_bits = vec![0u8; (n.div_ceil(16) * 2 * n) as usize];
        let mask = CreateBitmap(n as i32, n as i32, 1, 1, Some(mask_bits.as_ptr() as _));
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}
