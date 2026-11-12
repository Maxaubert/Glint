//! Glint's icon, loaded from this exe's own resources (assets/glint.ico, resource id 1) at the
//! exact size asked for. Windows picks the closest of the ICO's 12 sizes (16 to 256 px) and scales
//! that one, so the tray icon is sharp at every display scale. Build the ICO with
//! `python tools/icons/make_icons.py`.

use windows::core::PCWSTR;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{LoadImageW, HICON, IMAGE_ICON, LR_DEFAULTCOLOR};

/// A Windows icon handle of size `n`.
pub fn hicon(n: u32) -> Option<HICON> {
    unsafe {
        let inst = GetModuleHandleW(None).ok()?;
        let h = LoadImageW(
            Some(inst.into()),
            // MAKEINTRESOURCE(1): the app icon id winresource gives assets/glint.ico.
            #[allow(clippy::manual_dangling_ptr)]
            PCWSTR(1 as *const u16),
            IMAGE_ICON,
            n as i32,
            n as i32,
            LR_DEFAULTCOLOR,
        )
        .ok()?;
        Some(HICON(h.0))
    }
}
