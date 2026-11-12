//! The preview card in the bottom-right corner after a capture.
//!
//! It never takes focus. Hover shows Edit, Save and Close. Click opens the editor. Drag it into
//! another app (Explorer, Discord, Slack, a browser) to drop the capture there as a PNG file.

use crate::gfx::{self, px, rgb, Dib, Font};
use glint_core::encode::{encode_png_best, encode_png_fast};
use glint_core::naming::{file_name, unique_path, Stamp};
use glint_core::{paths, Image, Monitor, Rect};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::DROPEFFECT_COPY;
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::Shell::{BHID_DataObject, IShellItem, SHCreateItemFromParsingName, SHDoDragDrop};
use windows::Win32::UI::WindowsAndMessaging::*;

const MAX_TOASTS: usize = 3;
const WM_MOUSELEAVE: u32 = 0x02A3;
const TICK_MS: u32 = 15;
const SLIDE_MS: i32 = 160;

/// Settings the card needs, copied from the config at capture time.
#[derive(Clone)]
pub struct Opts {
    pub seconds: u32,
    pub save_dir: PathBuf,
    /// Process name of the captured app, for file names.
    pub app: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Btn {
    Edit,
    Save,
    Close,
}

struct Toast {
    hwnd: HWND,
    image: Arc<Image>,
    thumb: Dib,
    opts: Opts,
    monitor: Monitor,
    w: i32,
    h: i32,
    x: i32,
    y: i32,
    shown_ms: i32,
    left_ms: i32,
    hover: bool,
    hover_btn: Option<Btn>,
    press: Option<(i32, i32)>,
    saved: Option<PathBuf>,
    closing: bool,
}

enum Action {
    None,
    Edit,
    Save,
    Close,
    Drag,
    Reveal(PathBuf),
}

thread_local! {
    static TOASTS: RefCell<Vec<Toast>> = const { RefCell::new(Vec::new()) };
}

fn register() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(wndproc),
            hInstance: GetModuleHandleW(None).unwrap_or_default().into(),
            lpszClassName: w!("GlintPreview"),
            hCursor: LoadCursorW(None, IDC_HAND).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&wc);
    });
}

/// Shows a card for `image` on `monitor`.
pub fn show(image: Arc<Image>, monitor: Monitor, opts: Opts) {
    register();
    let dpi = monitor.dpi;
    let pad = px(6, dpi);
    let thumb_img = image.thumbnail(px(300, dpi) as u32, px(190, dpi) as u32);
    // Very small captures get a minimum card size so the buttons fit.
    let min_w = px(200, dpi);
    let min_h = px(110, dpi);
    let w = (thumb_img.width as i32 + 2 * pad).max(min_w);
    let h = (thumb_img.height as i32 + 2 * pad).max(min_h);
    let Some(thumb) = Dib::from_image(&thumb_img, 1.0) else { return };
    // Close the oldest if there are too many.
    let oldest = TOASTS.with(|t| {
        let t = t.borrow();
        (t.len() >= MAX_TOASTS).then(|| t[0].hwnd)
    });
    if let Some(h) = oldest {
        close(h);
    }
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("GlintPreview"),
            w!("Glint preview"),
            WS_POPUP,
            monitor.work.right(),
            monitor.work.bottom() - h,
            w,
            h,
            None,
            None,
            Some(GetModuleHandleW(None).unwrap_or_default().into()),
            None,
        )
    };
    let Ok(hwnd) = hwnd else { return };
    unsafe {
        let pref = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const _ as _, 4);
    }
    let seconds = opts.seconds.max(1) as i32;
    TOASTS.with(|t| {
        t.borrow_mut().push(Toast {
            hwnd,
            image,
            thumb,
            opts,
            monitor,
            w,
            h,
            x: 0,
            y: 0,
            shown_ms: 0,
            left_ms: seconds * 1000,
            hover: false,
            hover_btn: None,
            press: None,
            saved: None,
            closing: false,
        })
    });
    layout();
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(Some(hwnd), 1, TICK_MS, None);
    }
}

/// Places the cards: newest at the bottom, older ones stacked above, per monitor.
fn layout() {
    TOASTS.with(|t| {
        let mut t = t.borrow_mut();
        let n = t.len();
        let mut used: Vec<(String, i32)> = Vec::new();
        for i in (0..n).rev() {
            let dpi = t[i].monitor.dpi;
            let margin = px(14, dpi);
            let key = t[i].monitor.device.clone();
            let below = used.iter().find(|(k, _)| *k == key).map(|(_, v)| *v).unwrap_or(0);
            let work = t[i].monitor.work;
            t[i].y = work.bottom() - margin - below - t[i].h;
            t[i].x = work.right() - margin - t[i].w;
            let add = t[i].h + px(10, dpi);
            match used.iter_mut().find(|(k, _)| *k == key) {
                Some(e) => e.1 += add,
                None => used.push((key, add)),
            }
            position(&t[i]);
        }
    });
}

fn ease_out(p: f32) -> f32 {
    1.0 - (1.0 - p).powi(3)
}

fn position(t: &Toast) {
    let p = (t.shown_ms as f32 / SLIDE_MS as f32).clamp(0.0, 1.0);
    let off = ((1.0 - ease_out(p)) * (t.w as f32 + px(20, t.monitor.dpi) as f32)) as i32;
    unsafe {
        let _ = SetWindowPos(t.hwnd, Some(HWND_TOPMOST), t.x + off, t.y, t.w, t.h, SWP_NOACTIVATE);
    }
}

pub fn close(hwnd: HWND) {
    let t = TOASTS.with(|t| {
        let mut t = t.try_borrow_mut().ok()?;
        let i = t.iter().position(|x| x.hwnd == hwnd)?;
        Some(t.remove(i))
    });
    if let Some(t) = t {
        unsafe {
            let _ = KillTimer(Some(t.hwnd), 1);
            let _ = DestroyWindow(t.hwnd);
        }
        drop(t);
        layout();
    }
}

pub fn close_all() {
    let all: Vec<HWND> = TOASTS.with(|t| t.borrow().iter().map(|t| t.hwnd).collect());
    for h in all {
        close(h);
    }
}

impl Toast {
    fn dpi(&self) -> u32 {
        self.monitor.dpi
    }

    fn bar(&self) -> Rect {
        let bh = px(40, self.dpi());
        Rect::new(0, self.h - bh, self.w, bh)
    }

    fn buttons(&self) -> [(Btn, Rect); 3] {
        let dpi = self.dpi();
        let bar = self.bar();
        let bw = px(76, dpi);
        let gap = px(6, dpi);
        let bh = bar.h - 2 * px(6, dpi);
        let y = bar.y + px(6, dpi);
        let edit = Rect::new(bar.x + px(8, dpi), y, bw, bh);
        let save = Rect::new(edit.right() + gap, y, bw, bh);
        let cs = px(26, dpi);
        let close = Rect::new(self.w - cs - px(6, dpi), px(6, dpi), cs, cs);
        [(Btn::Edit, edit), (Btn::Save, save), (Btn::Close, close)]
    }

    fn btn_at(&self, x: i32, y: i32) -> Option<Btn> {
        self.buttons().into_iter().find(|(_, r)| r.contains(x, y)).map(|(b, _)| b)
    }

    fn paint(&self, hdc: HDC) {
        let Some(back) = Dib::new(self.w, self.h) else { return };
        let dc = back.dc;
        let dpi = self.dpi();
        gfx::fill(dc, &Rect::new(0, 0, self.w, self.h), rgb(30, 30, 34));
        let (tw, th) = (self.thumb.w, self.thumb.h);
        let (tx, ty) = ((self.w - tw) / 2, (self.h - th) / 2);
        unsafe {
            let _ = BitBlt(dc, tx, ty, tw, th, Some(self.thumb.dc), 0, 0, SRCCOPY);
        }
        if let Some(path) = &self.saved {
            gfx::fill_alpha(dc, &Rect::new(0, 0, self.w, self.h), rgb(0, 0, 0), 150);
            let font = Font::ui_bold(px(13, dpi));
            let small = Font::ui(px(11, dpi));
            let mid = self.h / 2;
            gfx::text(dc, &font, "Saved", &Rect::new(0, mid - px(22, dpi), self.w, px(20, dpi)), rgb(255, 255, 255));
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            gfx::text(dc, &small, &name, &Rect::new(px(8, dpi), mid, self.w - px(16, dpi), px(18, dpi)), rgb(220, 220, 228));
            gfx::text(dc, &small, "Click to show in folder", &Rect::new(0, mid + px(18, dpi), self.w, px(18, dpi)), rgb(170, 170, 180));
        } else if self.hover || std::env::var_os("GLINT_TEST_HOVER").is_some() {
            gfx::fill_alpha(dc, &self.bar(), rgb(0, 0, 0), 170);
            let icons = Font::icons(px(14, dpi));
            let label = Font::ui(px(12, dpi));
            for (b, r) in self.buttons() {
                let hot = self.hover_btn == Some(b);
                match b {
                    Btn::Close => {
                        gfx::round_rect(dc, &r, r.w / 2, if hot { rgb(196, 43, 28) } else { rgb(24, 24, 28) }, None);
                        gfx::text(dc, &Font::icons(px(10, dpi)), "\u{E8BB}", &r, rgb(255, 255, 255));
                    }
                    _ => {
                        let bg = if hot { rgb(139, 92, 246) } else { rgb(52, 52, 60) };
                        gfx::round_rect(dc, &r, px(5, dpi), bg, None);
                        let (glyph, name) = if b == Btn::Edit { ("\u{E70F}", "Edit") } else { ("\u{E74E}", "Save") };
                        let icon_r = Rect::new(r.x + px(8, dpi), r.y, px(18, dpi), r.h);
                        gfx::text(dc, &icons, glyph, &icon_r, rgb(255, 255, 255));
                        let text_r = Rect::new(icon_r.right(), r.y, r.right() - icon_r.right() - px(6, dpi), r.h);
                        gfx::text(dc, &label, name, &text_r, rgb(255, 255, 255));
                    }
                }
            }
        }
        unsafe {
            let _ = BitBlt(hdc, 0, 0, self.w, self.h, Some(dc), 0, 0, SRCCOPY);
        }
        back.dump("preview-paint");
    }

    fn suggested_name(&self) -> String {
        file_name(self.opts.app.as_deref().unwrap_or(""), Stamp::now(), "png")
    }
}

fn with_toast<R>(hwnd: HWND, f: impl FnOnce(&mut Toast) -> R) -> Option<R> {
    TOASTS.with(|t| {
        let mut t = t.try_borrow_mut().ok()?;
        t.iter_mut().find(|t| t.hwnd == hwnd).map(f)
    })
}

/// Writes the capture to the temp folder (for the editor or a drag) and returns the path.
fn write_temp(img: &Image, name: &str) -> Option<PathBuf> {
    let dir = paths::temp_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let path = unique_path(&dir, name);
    std::fs::write(&path, encode_png_fast(img)).ok()?;
    Some(path)
}

fn do_action(hwnd: HWND, action: Action) {
    match action {
        Action::None => {}
        Action::Close => close(hwnd),
        Action::Edit => {
            let Some((img, name, opts)) = with_toast(hwnd, |t| (t.image.clone(), t.suggested_name(), t.opts.clone())) else { return };
            close(hwnd);
            match write_temp(&img, &name) {
                Some(path) => crate::app::open_editor(&path, &name, &opts.save_dir, true),
                None => crate::log::line("could not write the temp file for the editor"),
            }
        }
        Action::Save => {
            let Some((img, name, opts)) = with_toast(hwnd, |t| (t.image.clone(), t.suggested_name(), t.opts.clone())) else { return };
            let dir = opts.save_dir.clone();
            if std::fs::create_dir_all(&dir).is_err() {
                crate::log::line(&format!("cannot create {}", dir.display()));
                return;
            }
            let path = unique_path(&dir, &name);
            // Encode on a worker: the best PNG compression of a 4K image takes a moment.
            let p2 = path.clone();
            std::thread::spawn(move || {
                if let Err(e) = std::fs::write(&p2, encode_png_best(&img)) {
                    crate::log::line(&format!("save failed: {e}"));
                }
            });
            with_toast(hwnd, |t| {
                t.saved = Some(path);
                t.left_ms = t.left_ms.max(2500);
            });
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
        Action::Reveal(path) => {
            let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{}", path.display())).spawn();
            close(hwnd);
        }
        Action::Drag => {
            let Some((img, name)) = with_toast(hwnd, |t| (t.image.clone(), t.suggested_name())) else { return };
            let Some(path) = write_temp(&img, &name) else { return };
            let dropped = unsafe { drag_file(hwnd, &path) };
            with_toast(hwnd, |t| t.press = None);
            if dropped {
                close(hwnd);
            }
        }
    }
}

/// Starts a shell drag of a file. Returns true if something accepted the drop.
unsafe fn drag_file(hwnd: HWND, path: &std::path::Path) -> bool {
    let item: windows::core::Result<IShellItem> = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None);
    let Ok(item) = item else { return false };
    let Ok(data) = item.BindToHandler::<_, IDataObject>(None, &BHID_DataObject) else { return false };
    match SHDoDragDrop(Some(hwnd), &data, None, DROPEFFECT_COPY) {
        Ok(effect) => effect.0 != 0,
        Err(_) => false,
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let xy = || ((lp.0 & 0xFFFF) as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as i16 as i32);
    let mut action = Action::None;
    let res = match msg {
        WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            with_toast(hwnd, |t| t.paint(hdc));
            let _ = EndPaint(hwnd, &ps);
            Some(LRESULT(0))
        }
        WM_ERASEBKGND => Some(LRESULT(1)),
        WM_TIMER => {
            let mut done = false;
            with_toast(hwnd, |t| {
                if t.shown_ms < SLIDE_MS {
                    t.shown_ms += TICK_MS as i32;
                    position(t);
                }
                if !t.hover && !t.closing {
                    t.left_ms -= TICK_MS as i32;
                    if t.left_ms <= 0 {
                        t.closing = true;
                        done = true;
                    }
                }
            });
            if done {
                action = Action::Close;
            }
            Some(LRESULT(0))
        }
        WM_MOUSEMOVE => {
            let (x, y) = xy();
            let mut drag = false;
            with_toast(hwnd, |t| {
                if !t.hover {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                }
                t.hover = true;
                t.hover_btn = t.btn_at(x, y);
                if let Some((px0, py0)) = t.press {
                    let (dx, dy) = (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG));
                    if (x - px0).abs() > dx || (y - py0).abs() > dy {
                        drag = t.saved.is_none();
                        t.press = None;
                    }
                }
            });
            let _ = InvalidateRect(Some(hwnd), None, false);
            if drag {
                action = Action::Drag;
            }
            Some(LRESULT(0))
        }
        WM_MOUSELEAVE => {
            with_toast(hwnd, |t| {
                t.hover = false;
                t.hover_btn = None;
                t.left_ms = t.left_ms.max(1500);
            });
            let _ = InvalidateRect(Some(hwnd), None, false);
            Some(LRESULT(0))
        }
        WM_LBUTTONDOWN => {
            let (x, y) = xy();
            with_toast(hwnd, |t| t.press = Some((x, y)));
            Some(LRESULT(0))
        }
        WM_LBUTTONUP => {
            let (x, y) = xy();
            if let Some(a) = with_toast(hwnd, |t| {
                let was_pressed = t.press.take().is_some();
                if !was_pressed {
                    return Action::None;
                }
                if let Some(p) = &t.saved {
                    return Action::Reveal(p.clone());
                }
                match t.btn_at(x, y) {
                    Some(Btn::Edit) | None => Action::Edit,
                    Some(Btn::Save) => Action::Save,
                    Some(Btn::Close) => Action::Close,
                }
            }) {
                action = a;
            }
            Some(LRESULT(0))
        }
        WM_RBUTTONUP => {
            action = Action::Close;
            Some(LRESULT(0))
        }
        _ => None,
    };
    do_action(hwnd, action);
    res.unwrap_or_else(|| DefWindowProcW(hwnd, msg, wp, lp))
}
