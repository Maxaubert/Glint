//! Monitor layout in physical pixels.

use glint_core::{Monitor, Rect};
use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

pub(crate) fn rect(r: &RECT) -> Rect {
    Rect::from_ltrb(r.left, r.top, r.right, r.bottom)
}

pub(crate) fn wide_to_string(w: &[u16]) -> String {
    let n = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..n])
}

unsafe extern "system" fn collect(hmon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let out = &mut *(data.0 as *mut Vec<Monitor>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(hmon, &mut info as *mut _ as *mut MONITORINFO).as_bool() {
        let (mut dx, mut dy) = (96, 96);
        let _ = GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        out.push(Monitor {
            rect: rect(&info.monitorInfo.rcMonitor),
            work: rect(&info.monitorInfo.rcWork),
            dpi: dx,
            primary: info.monitorInfo.dwFlags & 1 != 0,
            device: wide_to_string(&info.szDevice),
        });
    }
    BOOL(1)
}

/// All monitors, primary first.
pub fn enumerate() -> Vec<Monitor> {
    let mut out: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut out as *mut _ as isize));
    }
    out.sort_by_key(|m| !m.primary);
    out
}
