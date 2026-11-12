//! Clipboard read and write. Writes PNG (with alpha), CF_DIBV5 and CF_DIB so every app can paste.

use crate::encode::{decode_dib, decode_png, encode_dib, encode_dibv5, encode_png_fast};
use crate::Image;
use windows::core::w;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};

const CF_DIB: u32 = 8;
const CF_DIBV5: u32 = 17;

fn png_format() -> u32 {
    unsafe { RegisterClipboardFormatW(w!("PNG")) }
}

/// Holds the clipboard open; closes it on drop.
struct Open;

impl Open {
    fn new(owner: Option<HWND>) -> Result<Self, String> {
        // Another app may hold the clipboard for a moment: retry briefly.
        for _ in 0..20 {
            if unsafe { OpenClipboard(owner) }.is_ok() {
                return Ok(Open);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Err("the clipboard is busy".into())
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

fn to_global(bytes: &[u8]) -> Result<HGLOBAL, String> {
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|e| e.to_string())?;
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            let _ = GlobalFree(Some(h));
            return Err("GlobalLock failed".into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(h);
        Ok(h)
    }
}

fn put(format: u32, bytes: &[u8]) -> Result<(), String> {
    let h = to_global(bytes)?;
    // On success the system owns the memory; on failure we free it.
    if unsafe { SetClipboardData(format, Some(HANDLE(h.0))) }.is_err() {
        let _ = unsafe { GlobalFree(Some(h)) };
        return Err(format!("SetClipboardData({format}) failed"));
    }
    Ok(())
}

/// Puts the image on the clipboard as PNG, CF_DIBV5 and CF_DIB.
pub fn set_image(img: &Image, owner: Option<HWND>) -> Result<(), String> {
    // Encode before opening, so the clipboard is held as briefly as possible.
    let png = encode_png_fast(img);
    let v5 = encode_dibv5(img);
    let dib = encode_dib(img);
    let _open = Open::new(owner)?;
    unsafe { EmptyClipboard() }.map_err(|e| e.to_string())?;
    put(png_format(), &png)?;
    put(CF_DIBV5, &v5)?;
    put(CF_DIB, &dib)?;
    Ok(())
}

fn read_format(format: u32) -> Option<Vec<u8>> {
    unsafe {
        IsClipboardFormatAvailable(format).ok()?;
        let h = GetClipboardData(format).ok()?;
        let g = HGLOBAL(h.0);
        let size = GlobalSize(g);
        let p = GlobalLock(g) as *const u8;
        if p.is_null() || size == 0 {
            return None;
        }
        let v = std::slice::from_raw_parts(p, size).to_vec();
        let _ = GlobalUnlock(g);
        Some(v)
    }
}

/// Reads an image from the clipboard (PNG first, then DIBV5, then DIB).
pub fn get_image(owner: Option<HWND>) -> Result<Option<Image>, String> {
    let _open = Open::new(owner)?;
    if let Some(b) = read_format(png_format()) {
        if let Ok(img) = decode_png(&b) {
            return Ok(Some(img));
        }
    }
    for f in [CF_DIBV5, CF_DIB] {
        if let Some(b) = read_format(f) {
            if let Ok(img) = decode_dib(&b) {
                return Ok(Some(img));
            }
        }
    }
    Ok(None)
}
