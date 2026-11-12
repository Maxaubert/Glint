//! Cooperation with Wind, the owner's fullscreen magnifier.
//!
//! While Wind's transform engine zooms the screen, every capture API (GDI and DXGI, measured
//! 2026-11-12 with the `zoomprobe` example) returns the zoomed picture, so a snip froze the
//! magnified view and zooming out inside the overlay showed nothing more.
//!
//! So Glint asks Wind for the real desktop. Glint creates a section `Local\Glint.CaptureFrame.<pid>`
//! and sends the registered message `Wind.CaptureBlink.v1` (wParam 2, lParam = pid) to the window of
//! class `WindMagnifierWnd`. Wind writes 1x for one composition, grabs that frame with Desktop
//! Duplication, writes the zoom straight back (the visible blink is about one frame) and copies the
//! pixels into the section. Reply 1 = the frame is there. Layout: a 32-byte header (magic 'GLBF',
//! version 1, width, height, stride, format 1 = BGRA8 or 2 = RGBA16F scRGB, monitor x, y), then rows.
//! Wind side: `src/transform/blink_capture.*` and WndProc in `src/core/main.cpp` (Wind #565).

use glint_core::Image;
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, LPARAM, WPARAM};
use windows::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS,
    PAGE_READWRITE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, RegisterWindowMessageW, SendMessageTimeoutW, SMTO_ABORTIFHUNG,
};

const MAGIC: u32 = 0x4642_4C47;
const HEADER: usize = 32;

/// The real (unzoomed) picture of the monitor Wind magnifies, and that monitor's origin.
pub struct WindFrame {
    pub image: Image,
    pub x: i32,
    pub y: i32,
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}

/// True when Wind is running (its magnifier window exists).
pub fn running() -> bool {
    unsafe { FindWindowW(w!("WindMagnifierWnd"), w!("Wind")).is_ok_and(|h| !h.is_invalid()) }
}

/// Asks Wind for one unzoomed frame. None when Wind is not running, not zoomed, or too old.
/// `max_w`, `max_h`: the largest monitor, to size the section.
pub fn grab(max_w: u32, max_h: u32) -> Option<WindFrame> {
    unsafe {
        let hwnd = FindWindowW(w!("WindMagnifierWnd"), w!("Wind")).ok()?;
        let msg = RegisterWindowMessageW(w!("Wind.CaptureBlink.v1"));
        if msg == 0 {
            return None;
        }
        let pid = std::process::id();
        let size = HEADER as u64 + max_w as u64 * max_h as u64 * 8;
        let name = HSTRING::from(format!("Local\\Glint.CaptureFrame.{pid}"));
        let section = CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            None,
            PAGE_READWRITE,
            (size >> 32) as u32,
            size as u32,
            &name,
        )
        .ok()?;
        let mut result = 0usize;
        // Wind's first blink creates its D3D device (~150 ms); later ones are fast.
        let sent = SendMessageTimeoutW(
            hwnd,
            msg,
            WPARAM(2),
            LPARAM(pid as isize),
            SMTO_ABORTIFHUNG,
            1500,
            Some(&mut result),
        );
        let frame = if sent.0 != 0 && result == 1 {
            read(section, size as usize)
        } else {
            None
        };
        let _ = CloseHandle(section);
        frame
    }
}

unsafe fn read(section: windows::Win32::Foundation::HANDLE, size: usize) -> Option<WindFrame> {
    let view: MEMORY_MAPPED_VIEW_ADDRESS = MapViewOfFile(section, FILE_MAP_READ, 0, 0, size);
    if view.Value.is_null() {
        return None;
    }
    let b = std::slice::from_raw_parts(view.Value as *const u8, size);
    let out = (|| {
        if u32_at(b, 0) != MAGIC || u32_at(b, 4) != 1 {
            return None;
        }
        let (w, h, stride, format) = (
            u32_at(b, 8),
            u32_at(b, 12),
            u32_at(b, 16) as usize,
            u32_at(b, 20),
        );
        let x = u32_at(b, 24) as i32;
        let y = u32_at(b, 28) as i32;
        let need = HEADER + stride * h as usize;
        if w == 0 || h == 0 || need > size {
            return None;
        }
        let px = &b[HEADER..need];
        let image = match format {
            1 => Image::from_bgra(w, h, px, stride, true),
            2 => {
                let device = glint_capture::monitors::enumerate()
                    .into_iter()
                    .find(|m| m.rect.x == x && m.rect.y == y)
                    .map(|m| m.device)
                    .unwrap_or_default();
                let white = glint_capture::hdr::sdr_white_level(&device).unwrap_or(1000);
                glint_capture::hdr::tone_map_fp16(w, h, px, stride, white)
            }
            _ => return None,
        };
        Some(WindFrame { image, x, y })
    })();
    let _ = UnmapViewOfFile(view);
    out
}
