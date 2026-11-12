//! Hotkeys that Windows keeps for itself (such as Win+Shift+S for the Snipping Tool) cannot be
//! claimed with RegisterHotKey. A low-level keyboard hook sees them first: Glint takes the key
//! press, tells its own window, and stops Windows from acting on it.
//!
//! The hook is installed only when such a hotkey is configured, and the callback does no more than
//! compare a key code, so it adds as little as possible to the system's input path.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy)]
struct Entry {
    id: i32,
    mods: u32,
    vk: u32,
}

static ENTRIES: Mutex<Vec<Entry>> = Mutex::new(Vec::new());
static TARGET: AtomicIsize = AtomicIsize::new(0);
static HOOK: AtomicIsize = AtomicIsize::new(0);
static SWALLOW_UP: AtomicBool = AtomicBool::new(false);
/// True while the snip overlay is open: Esc anywhere cancels it (see `guard_escape`).
static ESC_GUARD: AtomicBool = AtomicBool::new(false);

/// The WM_HOTKEY id the hook posts for Esc while the overlay is open.
pub const ESC_ID: i32 = 0x7E5C;

/// Hotkey modifiers currently held, as RegisterHotKey bits (Alt 1, Ctrl 2, Shift 4, Win 8).
fn held() -> u32 {
    let down = |vk: VIRTUAL_KEY| unsafe { GetAsyncKeyState(vk.0 as i32) } < 0;
    let mut m = 0;
    if down(VK_MENU) {
        m |= 1;
    }
    if down(VK_CONTROL) {
        m |= 2;
    }
    if down(VK_SHIFT) {
        m |= 4;
    }
    if down(VK_LWIN) || down(VK_RWIN) {
        m |= 8;
    }
    m
}

/// Presses and releases an unassigned key, so releasing Win afterwards does not open Start.
fn mask_win_key() {
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0xE8),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

unsafe extern "system" fn proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let kb = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        let injected = kb.flags.0 & LLKHF_INJECTED.0 != 0;
        if !injected {
            let down = wp.0 as u32 == WM_KEYDOWN || wp.0 as u32 == WM_SYSKEYDOWN;
            if ESC_GUARD.load(Ordering::Relaxed) && kb.vkCode == VK_ESCAPE.0 as u32 {
                if down {
                    let hwnd = HWND(TARGET.load(Ordering::Relaxed) as *mut _);
                    let _ = PostMessageW(Some(hwnd), WM_HOTKEY, WPARAM(ESC_ID as usize), LPARAM(0));
                }
                return LRESULT(1);
            }
            let hit = ENTRIES
                .lock()
                .ok()
                .and_then(|e| e.iter().find(|e| e.vk == kb.vkCode).copied());
            if let Some(e) = hit {
                if down && held() == e.mods {
                    let hwnd = HWND(TARGET.load(Ordering::Relaxed) as *mut _);
                    let _ = PostMessageW(Some(hwnd), WM_HOTKEY, WPARAM(e.id as usize), LPARAM(0));
                    SWALLOW_UP.store(true, Ordering::Relaxed);
                    if e.mods & 8 != 0 {
                        mask_win_key();
                    }
                    return LRESULT(1);
                }
                if !down && SWALLOW_UP.swap(false, Ordering::Relaxed) {
                    return LRESULT(1);
                }
            }
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

/// Sets the hotkeys the hook handles (`(id, mods, vk)`); installs or removes the hook as needed.
pub fn set(target: HWND, list: &[(i32, u32, u32)]) -> Result<(), String> {
    TARGET.store(target.0 as isize, Ordering::Relaxed);
    if let Ok(mut e) = ENTRIES.lock() {
        *e = list
            .iter()
            .map(|&(id, mods, vk)| Entry { id, mods, vk })
            .collect();
    }
    let installed = HOOK.load(Ordering::Relaxed) != 0;
    if list.is_empty() {
        if installed {
            let h = HHOOK(HOOK.swap(0, Ordering::Relaxed) as *mut _);
            unsafe {
                let _ = UnhookWindowsHookEx(h);
            }
        }
        return Ok(());
    }
    if !installed {
        let hinst = unsafe { GetModuleHandleW(None) }.map_err(|e| e.to_string())?;
        let h = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(proc), Some(hinst.into()), 0) }
            .map_err(|e| format!("keyboard hook: {e}"))?;
        HOOK.store(h.0 as isize, Ordering::Relaxed);
    }
    Ok(())
}

/// While the snip overlay is open, Esc cancels it even when the overlay does not have the keyboard
/// (with the Start menu open, Windows keeps the focus elsewhere and the overlay could not be left).
/// Installs the hook for that time if no Win+ hotkey needs it.
pub fn guard_escape(target: HWND, on: bool) {
    ESC_GUARD.store(on, Ordering::Relaxed);
    if on {
        if TARGET.load(Ordering::Relaxed) == 0 {
            TARGET.store(target.0 as isize, Ordering::Relaxed);
        }
        if HOOK.load(Ordering::Relaxed) == 0 {
            if let Ok(hinst) = unsafe { GetModuleHandleW(None) } {
                if let Ok(h) =
                    unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(proc), Some(hinst.into()), 0) }
                {
                    HOOK.store(h.0 as isize, Ordering::Relaxed);
                }
            }
        }
    } else {
        let wanted = ENTRIES.lock().map(|e| !e.is_empty()).unwrap_or(true);
        if !wanted {
            let h = HOOK.swap(0, Ordering::Relaxed);
            if h != 0 {
                unsafe {
                    let _ = UnhookWindowsHookEx(HHOOK(h as *mut _));
                }
            }
        }
    }
}

/// Re-installs the hook if one is wanted. Windows silently drops a low-level hook whose callback
/// is ever slower than LowLevelHooksTimeout (for example while a game is loading), with no error and
/// no notice, so the daemon calls this every 30 seconds to heal it (Wind issue #156 is the same bug).
pub fn refresh() {
    let old = HOOK.load(Ordering::Relaxed);
    if old == 0 {
        return;
    }
    let Ok(hinst) = (unsafe { GetModuleHandleW(None) }) else {
        return;
    };
    if let Ok(h) = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(proc), Some(hinst.into()), 0) } {
        HOOK.store(h.0 as isize, Ordering::Relaxed);
        unsafe {
            let _ = UnhookWindowsHookEx(HHOOK(old as *mut _));
        }
    }
}
