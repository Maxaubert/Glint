//! Top-level windows under the cursor, for window mode.

use crate::monitors::rect;
use glint_core::Rect;
use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowLongW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    GWL_EXSTYLE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};

#[derive(Clone, Debug)]
pub struct WindowInfo {
    pub hwnd: isize,
    /// Visible frame, without the invisible resize border.
    pub rect: Rect,
    pub title: String,
    pub pid: u32,
}

struct Ctx {
    own_pid: u32,
    out: Vec<WindowInfo>,
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> BOOL {
    let ctx = &mut *(data.0 as *mut Ctx);
    if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
        return BOOL(1);
    }
    let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if ex & WS_EX_TRANSPARENT.0 != 0 {
        return BOOL(1);
    }
    let mut cloaked = 0u32;
    let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
    if cloaked != 0 {
        return BOOL(1);
    }
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == ctx.own_pid {
        return BOOL(1);
    }
    let class = class_name(hwnd);
    if matches!(class.as_str(), "Progman" | "WorkerW") {
        return BOOL(1);
    }
    let mut r = RECT::default();
    if DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32)
        .is_err()
    {
        return BOOL(1);
    }
    let rc = rect(&r);
    if rc.w < 8 || rc.h < 8 {
        return BOOL(1);
    }
    let mut buf = [0u16; 256];
    let n = GetWindowTextW(hwnd, &mut buf) as usize;
    let title = String::from_utf16_lossy(&buf[..n]);
    // Untitled tool windows are mostly invisible helpers (tray flyouts, IME): skip them.
    if title.is_empty() && ex & WS_EX_TOOLWINDOW.0 != 0 {
        return BOOL(1);
    }
    ctx.out.push(WindowInfo { hwnd: hwnd.0 as isize, rect: rc, title, pid });
    BOOL(1)
}

/// Visible top-level windows of other processes, topmost first.
pub fn list() -> Vec<WindowInfo> {
    let mut ctx = Ctx { own_pid: std::process::id(), out: Vec::new() };
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut ctx as *mut _ as isize));
    }
    ctx.out
}

/// The topmost window containing the point.
pub fn at(windows: &[WindowInfo], x: i32, y: i32) -> Option<&WindowInfo> {
    windows.iter().find(|w| w.rect.contains(x, y))
}

/// The executable name of a process, for example `chrome.exe`.
pub fn process_name(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(h);
        ok.ok()?;
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().map(str::to_string)
    }
}

/// The process name of the foreground window (used to name region captures).
pub fn foreground_app() -> Option<String> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return None;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    process_name(pid)
}

#[cfg(test)]
mod tests {
    #[test]
    fn lists_windows_without_panicking() {
        let all = super::list();
        eprintln!("{} windows", all.len());
        for w in all.iter().take(5) {
            eprintln!("{:?} {:?} {:?}", w.title, w.rect, super::process_name(w.pid));
        }
    }
}
