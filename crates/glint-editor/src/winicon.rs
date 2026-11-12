//! Sharp window and taskbar icons. Slint hands Windows one 256 px picture for every icon size and
//! Windows shrinks it. Instead, this loads the exe's own ICO (assets/glint.ico, resource id 1) at
//! the exact sizes Windows wants for the window's monitor scale, so Windows picks the closest of
//! the 12 hand-made sizes. Call after the window is shown.

use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumThreadWindows, IsWindowVisible, LoadImageW, SendMessageW, ICON_BIG, ICON_SMALL, IMAGE_ICON,
    LR_DEFAULTCOLOR, SM_CXICON, SM_CXSMICON, WM_SETICON,
};

fn load(n: i32) -> Option<isize> {
    unsafe {
        let inst = GetModuleHandleW(None).ok()?;
        let h = LoadImageW(
            Some(inst.into()),
            // MAKEINTRESOURCE(1): the app icon id winresource gives assets/glint.ico.
            #[allow(clippy::manual_dangling_ptr)]
            PCWSTR(1 as *const u16),
            IMAGE_ICON,
            n,
            n,
            LR_DEFAULTCOLOR,
        )
        .ok()?;
        Some(h.0 as isize)
    }
}

unsafe extern "system" fn each(hwnd: HWND, _: LPARAM) -> BOOL {
    if IsWindowVisible(hwnd).as_bool() {
        let dpi = GetDpiForWindow(hwnd).max(96);
        let big = GetSystemMetricsForDpi(SM_CXICON, dpi);
        let small = GetSystemMetricsForDpi(SM_CXSMICON, dpi);
        if let Some(h) = load(big) {
            SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_BIG as usize)),
                Some(LPARAM(h)),
            );
        }
        if let Some(h) = load(small) {
            SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_SMALL as usize)),
                Some(LPARAM(h)),
            );
        }
    }
    true.into()
}

/// Sets sharp icons on this thread's visible windows.
pub fn apply() {
    unsafe {
        let _ = EnumThreadWindows(GetCurrentThreadId(), Some(each), LPARAM(0));
    }
}

/// Applies the icons now and once more shortly after (the window may still be appearing).
pub fn apply_soon() {
    slint::Timer::single_shot(std::time::Duration::from_millis(0), apply);
    slint::Timer::single_shot(std::time::Duration::from_millis(400), apply);
}
