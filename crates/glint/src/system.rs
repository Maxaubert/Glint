//! Windows settings Glint touches: start with Windows, and the Print Screen key.

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::System::Registry::*;

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const KEYBOARD: PCWSTR = w!("Control Panel\\Keyboard");

fn open(path: PCWSTR, write: bool) -> Option<HKEY> {
    let mut key = HKEY::default();
    let access = if write { KEY_READ | KEY_WRITE } else { KEY_READ };
    unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path, None, access, &mut key) }.ok().ok()?;
    Some(key)
}

fn read_string(key: HKEY, name: PCWSTR) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    unsafe { RegQueryValueExW(key, name, None, None, Some(buf.as_mut_ptr() as *mut u8), Some(&mut size)) }.ok().ok()?;
    let n = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..n.min(buf.len())]))
}

fn read_dword(key: HKEY, name: PCWSTR) -> Option<u32> {
    let mut v = 0u32;
    let mut size = 4u32;
    unsafe { RegQueryValueExW(key, name, None, None, Some(&mut v as *mut u32 as *mut u8), Some(&mut size)) }.ok().ok()?;
    Some(v)
}

fn run_command() -> String {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    format!("\"{exe}\" --background")
}

/// True when the Run key points at this executable.
pub fn starts_with_windows() -> bool {
    let Some(key) = open(RUN, false) else { return false };
    let v = read_string(key, w!("Glint"));
    unsafe {
        let _ = RegCloseKey(key);
    }
    v.is_some_and(|v| v == run_command())
}

pub fn set_start_with_windows(on: bool) -> Result<(), String> {
    let key = open(RUN, true).ok_or("cannot open the Run key")?;
    let r = unsafe {
        if on {
            let v: Vec<u16> = run_command().encode_utf16().chain(Some(0)).collect();
            let bytes = std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2);
            RegSetValueExW(key, w!("Glint"), None, REG_SZ, Some(bytes))
        } else {
            RegDeleteValueW(key, w!("Glint"))
        }
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    r.ok().map_err(|e| e.to_string())
}

/// Whether Windows gives the Print Screen key to its own Snipping Tool (the default on Windows 11).
pub fn print_screen_opens_snipping_tool() -> bool {
    let Some(key) = open(KEYBOARD, false) else { return true };
    let v = read_dword(key, w!("PrintScreenKeyForSnippingEnabled"));
    unsafe {
        let _ = RegCloseKey(key);
    }
    v != Some(0)
}

/// Turns the Windows "Print screen key opens screen capture" setting on or off.
pub fn set_print_screen_opens_snipping_tool(on: bool) -> Result<(), String> {
    let key = open(KEYBOARD, true).ok_or("cannot open Control Panel\\Keyboard")?;
    let v: u32 = on.into();
    let r = unsafe { RegSetValueExW(key, w!("PrintScreenKeyForSnippingEnabled"), None, REG_DWORD, Some(&v.to_le_bytes())) };
    unsafe {
        let _ = RegCloseKey(key);
    }
    r.ok().map_err(|e| e.to_string())
}

/// Opens a file or folder with its default app.
pub fn shell_open(path: &std::path::Path) {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    unsafe {
        ShellExecuteW(None, w!("open"), &HSTRING::from(path.as_os_str()), None, None, SW_SHOWNORMAL);
    }
}
