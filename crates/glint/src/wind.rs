//! Cooperation with Wind, the owner's fullscreen magnifier.
//!
//! While Wind's transform engine zooms the screen, every capture API (GDI and DXGI, measured
//! 2026-11-12) returns the zoomed picture, so a snip would freeze the magnified view and zooming out
//! inside the overlay showed nothing more. Glint therefore asks Wind for a "capture blink": Wind
//! writes a 1x transform, waits until that frame is on screen, and pauses its own writes. Glint
//! copies the screen and ends the blink; Wind puts the same zoom and position back. Wind magnifies
//! Glint's overlay like any window, so the view looks unchanged and zooming out now shows the real
//! desktop. Cost: the unzoomed screen shows for about the length of one capture (~60 ms).
//!
//! Protocol (Wind side: WndProc in Wind's src/core/main.cpp): registered message
//! `Wind.CaptureBlink.v1` sent to the window of class `WindMagnifierWnd`. wParam 1 = start (reply 1
//! when Wind is now at 1x, 0 when it was not zoomed), wParam 0 = end. Wind ends the blink by itself
//! after 400 ms if Glint never does.

use windows::core::w;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, RegisterWindowMessageW, SendMessageTimeoutW, SMTO_ABORTIFHUNG,
};

fn send(on: bool) -> bool {
    unsafe {
        let Ok(hwnd) = FindWindowW(w!("WindMagnifierWnd"), w!("Wind")) else {
            return false;
        };
        let msg = RegisterWindowMessageW(w!("Wind.CaptureBlink.v1"));
        if msg == 0 {
            return false;
        }
        let mut result = 0usize;
        let ok = SendMessageTimeoutW(
            hwnd,
            msg,
            WPARAM(on as usize),
            LPARAM(0),
            SMTO_ABORTIFHUNG,
            250,
            Some(&mut result),
        );
        ok.0 != 0 && result == 1
    }
}

/// Asks Wind to show the unzoomed desktop. True when Wind did (it was zoomed); then call `end`.
pub fn start() -> bool {
    send(true)
}

/// Lets Wind put its zoom back.
pub fn end() {
    send(false);
}
