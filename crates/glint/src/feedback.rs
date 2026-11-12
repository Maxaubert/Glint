//! Capture feedback: a short shutter sound and a frame that flashes around the snipped area.
//! Both are optional (`capture_sound`, `capture_frame` in config.toml).

use crate::gfx::{self, px, rgb};
use glint_core::Rect;
use std::cell::RefCell;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

// ---------------------------------------------------------------- sound

/// A real camera shutter (Pixabay "camera-shutter-6305", pitched up one semitone), 16-bit mono WAV.
/// Credit and license: README, "Credits".
fn shutter_wav() -> &'static [u8] {
    static WAV: &[u8] = include_bytes!("../../../assets/shutter.wav");
    WAV
}

pub fn play_sound() {
    let wav = shutter_wav();
    unsafe {
        // SND_MEMORY reads from our buffer, which is static, so SND_ASYNC is safe.
        let _ = PlaySoundW(
            PCWSTR(wav.as_ptr() as *const u16),
            None,
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

// ---------------------------------------------------------------- frame flash

const KEY: COLORREF = rgb(255, 0, 255); // transparent colour key
const HOLD_MS: u32 = 180;
const FADE_MS: u32 = 320;
const TICK_MS: u32 = 16;

struct Flash {
    hwnd: HWND,
    age_ms: u32,
    border: i32,
}

thread_local! {
    static FLASHES: RefCell<Vec<Flash>> = const { RefCell::new(Vec::new()) };
}

fn register() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: GetModuleHandleW(None).unwrap_or_default().into(),
            lpszClassName: w!("GlintFlash"),
            ..Default::default()
        };
        RegisterClassExW(&wc);
    });
}

/// Flashes a frame just inside `r` (virtual-desktop pixels), then fades it out.
pub fn flash(r: &Rect, dpi: u32) {
    if r.w < 4 || r.h < 4 {
        return;
    }
    register();
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("GlintFlash"),
            w!(""),
            WS_POPUP,
            r.x,
            r.y,
            r.w,
            r.h,
            None,
            None,
            Some(GetModuleHandleW(None).unwrap_or_default().into()),
            None,
        )
    };
    let Ok(hwnd) = hwnd else { return };
    let border = px(3, dpi).clamp(2, r.w.min(r.h) / 2);
    unsafe {
        let _ = SetLayeredWindowAttributes(hwnd, KEY, 255, LWA_COLORKEY | LWA_ALPHA);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(Some(hwnd), 1, TICK_MS, None);
    }
    FLASHES.with(|f| {
        f.borrow_mut().push(Flash {
            hwnd,
            age_ms: 0,
            border,
        })
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut rc = windows::Win32::Foundation::RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let r = Rect::from_ltrb(rc.left, rc.top, rc.right, rc.bottom);
            let b = FLASHES
                .with(|f| f.borrow().iter().find(|x| x.hwnd == hwnd).map(|x| x.border))
                .unwrap_or(3);
            gfx::fill(hdc, &r, KEY);
            // White outside, accent inside, so it reads on light and dark content.
            let outer = (b / 3).max(1);
            gfx::frame(
                hdc,
                &Rect::new(r.x + outer, r.y + outer, r.w - 2 * outer, r.h - 2 * outer),
                rgb(255, 255, 255),
                outer,
            );
            gfx::frame(
                hdc,
                &Rect::new(r.x + b, r.y + b, r.w - 2 * b, r.h - 2 * b),
                rgb(139, 92, 246),
                b - outer,
            );
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_TIMER => {
            let age = FLASHES.with(|f| {
                let mut f = f.borrow_mut();
                f.iter_mut().find(|x| x.hwnd == hwnd).map(|x| {
                    x.age_ms += TICK_MS;
                    x.age_ms
                })
            });
            match age {
                Some(a) if a < HOLD_MS + FADE_MS => {
                    if a > HOLD_MS {
                        let alpha = 255 - ((a - HOLD_MS) * 255 / FADE_MS).min(255);
                        let _ = SetLayeredWindowAttributes(
                            hwnd,
                            KEY,
                            alpha as u8,
                            LWA_COLORKEY | LWA_ALPHA,
                        );
                    }
                }
                _ => {
                    let _ = KillTimer(Some(hwnd), 1);
                    FLASHES.with(|f| f.borrow_mut().retain(|x| x.hwnd != hwnd));
                    let _ = DestroyWindow(hwnd);
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shutter_is_a_valid_wav() {
        let w = super::shutter_wav();
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        let data_len = u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize;
        assert_eq!(w.len(), 44 + data_len);
        assert!(data_len > 10_000);
    }
}
