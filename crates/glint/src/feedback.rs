//! Capture feedback: a short shutter sound and a frame that flashes around the snipped area.
//! Both are optional (`capture_sound`, `capture_frame` in config.toml).

use crate::gfx::{self, px, rgb};
use glint_core::Rect;
use std::cell::RefCell;
use std::sync::OnceLock;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

// ---------------------------------------------------------------- sound

/// A synthesized camera-shutter "ch-chk": two short noise bursts with a fast decay, 16-bit mono.
fn shutter_wav() -> &'static [u8] {
    static WAV: OnceLock<Vec<u8>> = OnceLock::new();
    WAV.get_or_init(|| {
        const RATE: u32 = 44_100;
        let n = (RATE as f32 * 0.14) as usize;
        let mut seed: u32 = 0x1234_5678;
        let mut samples = Vec::with_capacity(n);
        let mut lp = 0.0f32;
        for i in 0..n {
            let t = i as f32 / RATE as f32;
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (seed >> 9) as f32 / (1u32 << 23) as f32 * 2.0 - 1.0;
            // Soften the hiss a little with a one-pole low-pass.
            lp += (noise - lp) * 0.45;
            let burst = |start: f32, len: f32| -> f32 {
                if t < start {
                    0.0
                } else {
                    (-(t - start) / len).exp()
                }
            };
            let env = 0.9 * burst(0.0, 0.012) + 0.6 * burst(0.055, 0.018);
            samples.push((lp * env * 0.5 * i16::MAX as f32) as i16);
        }
        let data_len = (samples.len() * 2) as u32;
        let mut v = Vec::with_capacity(44 + data_len as usize);
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data_len).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes()); // PCM
        v.extend_from_slice(&1u16.to_le_bytes()); // mono
        v.extend_from_slice(&RATE.to_le_bytes());
        v.extend_from_slice(&(RATE * 2).to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            v.extend_from_slice(&s.to_le_bytes());
        }
        v
    })
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
