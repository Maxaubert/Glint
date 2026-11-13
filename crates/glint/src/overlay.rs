//! The capture overlay: one topmost window over the whole virtual desktop showing the frozen frame.
//!
//! Drag a box and let go to capture. With capture_on_release off, adjust it with the handles or
//! the arrow keys first, then Enter.
//! Right-click captures the whole monitor under the cursor; middle-click repeats the last region.

use crate::gfx::{self, px, rgb, Dib, Font};
use glint_capture::Frame;
use glint_core::geom::{drag_handle, hit_handle, monitor_at, virtual_bounds};
use glint_core::{Handle, Image, Monitor, Rect};
use std::cell::RefCell;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const ACCENT: windows::Win32::Foundation::COLORREF = rgb(139, 92, 246);
const DIM: f32 = 0.55;

/// What the overlay ended with.
pub enum Outcome {
    Capture {
        image: Image,
        rect: Rect,
        monitor: Monitor,
    },
    Cancel,
}

#[derive(Clone, Copy)]
enum Drag {
    None,
    New {
        ax: i32,
        ay: i32,
    },
    Handle {
        h: Handle,
        sx: i32,
        sy: i32,
        orig: Rect,
    },
}

struct Overlay {
    hwnd: HWND,
    frame: Frame,
    bright: Dib,
    dim: Dib,
    back: Dib,
    vb: Rect,
    /// The previous capture, for middle-click.
    last: Option<Rect>,
    sel: Option<Rect>,
    drag: Drag,
    moved: bool,
    cur: (i32, i32),
    capture_on_release: bool,
    /// The snip hotkey as key names ("Win", "Shift", "S"): pressing it again cancels, so the hint
    /// shows it next to Esc, drawn from the user's own setting.
    close_keys: Vec<String>,
    /// Ticks left of re-asserting top-most and the keyboard (see WM_TIMER).
    assert_ticks: u32,
    /// The window that had the keyboard before the overlay: it gets it back on close. Otherwise
    /// Windows activates the next window in z-order, often another app (owner report, 2026-11-13).
    prev: Option<HWND>,
    done: Option<Outcome>,
}

type DoneFn = Box<dyn FnOnce(Outcome)>;

thread_local! {
    static STATE: RefCell<Option<Overlay>> = const { RefCell::new(None) };
    static ON_DONE: RefCell<Option<DoneFn>> = RefCell::new(None);
}

pub fn is_open() -> bool {
    STATE.with(|s| s.borrow().is_some())
}

/// Closes the overlay without capturing (the hotkey pressed a second time).
pub fn cancel() {
    STATE.with(|s| {
        if let Ok(mut s) = s.try_borrow_mut() {
            if let Some(o) = s.as_mut() {
                o.finish(None);
            }
        }
    });
    close_if_done();
}

fn register() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: GetModuleHandleW(None).unwrap_or_default().into(),
            lpszClassName: w!("GlintOverlay"),
            hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&wc);
    });
}

/// Shows the overlay over `frame`. `done` runs once, after the overlay window is gone.
pub fn show(
    frame: Frame,
    last: Option<Rect>,
    capture_on_release: bool,
    close_hotkey: &str,
    done: impl FnOnce(Outcome) + 'static,
) -> Result<(), String> {
    if is_open() {
        return Err("overlay already open".into());
    }
    register();
    let prev = previous_foreground();
    let vb = virtual_bounds(&frame.monitors);
    let bright = Dib::from_image(&frame.image, 1.0).ok_or("out of memory (bright)")?;
    let dim = Dib::from_image(&frame.image, DIM).ok_or("out of memory (dim)")?;
    let back = Dib::new(vb.w, vb.h).ok_or("out of memory (back)")?;
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            w!("GlintOverlay"),
            w!("Glint"),
            WS_POPUP,
            vb.x,
            vb.y,
            vb.w,
            vb.h,
            None,
            None,
            Some(GetModuleHandleW(None).unwrap_or_default().into()),
            None,
        )
    }
    .map_err(|e| format!("CreateWindowEx: {e}"))?;
    let on: i32 = 1;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            &on as *const _ as _,
            4,
        );
    }
    STATE.with(|s| {
        *s.borrow_mut() = Some(Overlay {
            hwnd,
            frame,
            bright,
            dim,
            back,
            vb,
            last,
            sel: None,
            drag: Drag::None,
            moved: false,
            cur: (p.x, p.y),
            capture_on_release,
            close_keys: close_hotkey
                .split('+')
                .map(|k| k.trim().to_string())
                .filter(|k| !k.is_empty())
                .collect(),
            assert_ticks: ASSERT_TICKS,
            prev,
            done: None,
        })
    });
    ON_DONE.with(|d| *d.borrow_mut() = Some(Box::new(done)));
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        force_foreground(hwnd);
        let _ = UpdateWindow(hwnd);
        // The taskbar and a closing Start menu can end up above a top-most window, and Windows may
        // keep the keyboard elsewhere: for the first second, put the overlay back on top and take
        // the keyboard again whenever it lost either.
        let _ = SetTimer(Some(hwnd), ASSERT_TIMER, 80, None);
    }
    Ok(())
}

impl Overlay {
    fn monitor(&self) -> &Monitor {
        monitor_at(&self.frame.monitors, self.cur.0, self.cur.1).unwrap_or(&self.frame.monitors[0])
    }

    fn dpi(&self) -> u32 {
        self.monitor().dpi
    }

    /// The rectangle that would be captured right now.
    fn target(&self) -> Option<Rect> {
        self.sel.filter(|r| !r.is_empty())
    }

    fn finish(&mut self, rect: Option<Rect>) {
        self.done = Some(match rect.filter(|r| !r.is_empty()) {
            Some(r) => {
                let (cx, cy) = r.center();
                let monitor = monitor_at(&self.frame.monitors, cx, cy)
                    .cloned()
                    .unwrap_or_else(|| self.frame.monitors[0].clone());
                Outcome::Capture {
                    image: self.frame.crop_virtual(&r),
                    rect: r,
                    monitor,
                }
            }
            None => Outcome::Cancel,
        });
    }

    fn to_virtual(&self, lp: LPARAM) -> (i32, i32) {
        let x = (lp.0 & 0xFFFF) as i16 as i32;
        let y = ((lp.0 >> 16) & 0xFFFF) as i16 as i32;
        (x + self.vb.x, y + self.vb.y)
    }

    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn mouse_down(&mut self, x: i32, y: i32) {
        self.cur = (x, y);
        self.moved = false;
        unsafe { SetCapture(self.hwnd) };
        let tol = px(8, self.dpi());
        self.drag = match self
            .sel
            .and_then(|s| hit_handle(&s, x, y, tol).map(|h| (h, s)))
        {
            Some((h, orig)) => Drag::Handle {
                h,
                sx: x,
                sy: y,
                orig,
            },
            None => {
                self.sel = None;
                Drag::New { ax: x, ay: y }
            }
        };
    }

    fn mouse_move(&mut self, x: i32, y: i32) {
        self.cur = (x, y);
        match self.drag {
            Drag::New { ax, ay } => {
                if (x - ax).abs() > 2 || (y - ay).abs() > 2 {
                    self.moved = true;
                }
                if self.moved {
                    // Include the pixel under the cursor.
                    let (r, b) = (x.max(ax) + 1, y.max(ay) + 1);
                    self.sel = Some(Rect::from_ltrb(x.min(ax), y.min(ay), r, b).clamp_to(&self.vb));
                }
            }
            Drag::Handle { h, sx, sy, orig } => {
                self.moved = true;
                self.sel = Some(drag_handle(&orig, h, x - sx, y - sy, &self.vb));
            }
            Drag::None => {}
        }
        self.invalidate();
    }

    fn mouse_up(&mut self, x: i32, y: i32) {
        self.cur = (x, y);
        let _ = unsafe { ReleaseCapture() };
        let drag = std::mem::replace(&mut self.drag, Drag::None);
        if let Drag::New { .. } = drag {
            if !self.moved {
                // A click without a drag clears the selection.
                self.sel = None;
            } else if self.capture_on_release {
                let t = self.target();
                self.finish(t);
                return;
            }
        }
        self.invalidate();
    }

    fn key(&mut self, vk: u16) {
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        let step = if ctrl { 10 } else { 1 };
        let (dx, dy) = match VIRTUAL_KEY(vk) {
            VK_LEFT => (-step, 0),
            VK_RIGHT => (step, 0),
            VK_UP => (0, -step),
            VK_DOWN => (0, step),
            _ => (0, 0),
        };
        if dx != 0 || dy != 0 {
            match self.sel {
                Some(s) => {
                    self.sel = Some(if shift {
                        s.resize_by(dx, dy, &self.vb)
                    } else {
                        s.nudge(dx, dy, &self.vb)
                    });
                }
                _ => unsafe {
                    // No selection yet: move the cursor itself, pixel by pixel.
                    let _ = SetCursorPos(self.cur.0 + dx, self.cur.1 + dy);
                },
            }
            self.invalidate();
            return;
        }
        match VIRTUAL_KEY(vk) {
            VK_ESCAPE => self.finish(None),
            VK_RETURN => {
                let t = self.target();
                if t.is_some() {
                    self.finish(t);
                }
            }
            k if k.0 == b'C' as u16 && ctrl => {
                let t = self.target();
                if t.is_some() {
                    self.finish(t);
                }
            }
            k if k.0 == b'A' as u16 && ctrl => self.sel = Some(self.monitor().rect),
            k if k.0 == b'F' as u16 => {
                let r = if shift { self.vb } else { self.monitor().rect };
                self.finish(Some(r));
            }
            _ => {}
        }
        self.invalidate();
    }

    fn cursor_for(&self) -> PCWSTRCursor {
        let h = match self.drag {
            Drag::Handle { h, .. } => Some(h),
            Drag::New { .. } => None,
            Drag::None => self
                .sel
                .and_then(|s| hit_handle(&s, self.cur.0, self.cur.1, px(8, self.dpi()))),
        };
        match h {
            Some(Handle::N | Handle::S) => IDC_SIZENS_,
            Some(Handle::E | Handle::W) => IDC_SIZEWE_,
            Some(Handle::NW | Handle::SE) => IDC_SIZENWSE_,
            Some(Handle::NE | Handle::SW) => IDC_SIZENESW_,
            Some(Handle::Move) => IDC_SIZEALL_,
            None => IDC_CROSS_,
        }
    }

    fn paint(&mut self, hdc: HDC) {
        let dc = self.back.dc;
        let (vx, vy) = (self.vb.x, self.vb.y);
        let local = |r: &Rect| r.translate(-vx, -vy);
        let dpi = self.dpi();
        unsafe {
            let _ = BitBlt(
                dc,
                0,
                0,
                self.vb.w,
                self.vb.h,
                Some(self.dim.dc),
                0,
                0,
                SRCCOPY,
            );
        }
        let target = self.target();
        if let Some(t) = target {
            let l = local(&t);
            unsafe {
                let _ = BitBlt(
                    dc,
                    l.x,
                    l.y,
                    l.w,
                    l.h,
                    Some(self.bright.dc),
                    l.x,
                    l.y,
                    SRCCOPY,
                );
            }
            gfx::frame(dc, &l, ACCENT, px(2, dpi).max(1));
            if !matches!(self.drag, Drag::New { .. }) {
                self.paint_handles(&l, dpi);
            }
            self.paint_size(&l, &t, dpi);
        }
        self.paint_hint(dpi);
        unsafe {
            let _ = BitBlt(hdc, 0, 0, self.vb.w, self.vb.h, Some(dc), 0, 0, SRCCOPY);
        }
        self.back.dump("overlay-paint");
    }

    fn paint_handles(&self, l: &Rect, dpi: u32) {
        let s = px(8, dpi);
        let (cx, cy) = l.center();
        for (x, y) in [
            (l.x, l.y),
            (cx, l.y),
            (l.right(), l.y),
            (l.x, cy),
            (l.right(), cy),
            (l.x, l.bottom()),
            (cx, l.bottom()),
            (l.right(), l.bottom()),
        ] {
            let r = Rect::new(x - s / 2, y - s / 2, s, s);
            gfx::fill(self.back.dc, &r, rgb(255, 255, 255));
            gfx::frame(
                self.back.dc,
                &Rect::new(r.x + 1, r.y + 1, r.w - 2, r.h - 2),
                ACCENT,
                1,
            );
        }
    }

    fn pill(&self, text: &str, font: &Font, x: i32, y: i32, dpi: u32, clamp: &Rect) -> Rect {
        let dc = self.back.dc;
        let (tw, th) = gfx::text_size(dc, font, text);
        let (padx, pady) = (px(8, dpi), px(4, dpi));
        let mut r = Rect::new(x, y, tw + 2 * padx, th + 2 * pady);
        r = r.nudge(0, 0, clamp);
        gfx::round_rect(dc, &r, px(6, dpi), rgb(24, 24, 28), Some(rgb(60, 60, 70)));
        gfx::text(dc, font, text, &r, rgb(240, 240, 245));
        r
    }

    fn paint_size(&self, l: &Rect, t: &Rect, dpi: u32) {
        let font = Font::ui(px(12, dpi));
        let label = format!("{} \u{00D7} {}", t.w, t.h);
        let mon = local_rect(self.monitor().rect, &self.vb);
        let gap = px(6, dpi);
        let (_, th) = gfx::text_size(self.back.dc, &font, &label);
        let h = th + px(8, dpi);
        let y = if l.y - gap - h >= mon.y {
            l.y - gap - h
        } else {
            l.y + gap
        };
        self.pill(&label, &font, l.x, y, dpi, &mon);
    }

    fn paint_hint(&self, dpi: u32) {
        use gfx::MouseButton::{Left, Middle, Right};
        let mon = local_rect(self.monitor().rect, &self.vb);
        // What each input does: icons (a mouse with the button lit, or key caps), then a label.
        // Several icons in one group mean "any of these".
        let keys = |names: &[&str]| Hint::Keys(names.iter().map(|s| s.to_string()).collect());
        let mut items: Vec<(Vec<Hint>, &str)> = Vec::new();
        if self.sel.is_some() && !self.capture_on_release {
            items.push((vec![keys(&["Enter"])], "Snip"));
            items.push((vec![Hint::Mouse(Left)], "Adjust with the handles"));
        } else if self.sel.is_some() {
            items.push((vec![Hint::Mouse(Left)], "Let go to snip"));
        } else {
            items.push((vec![Hint::Mouse(Left)], "Drag to snip"));
            items.push((vec![Hint::Mouse(Right)], "Full screen"));
            if self.last.is_some() {
                items.push((vec![Hint::Mouse(Middle)], "Last area"));
            }
        }
        let mut cancel = vec![keys(&["Esc"])];
        if !self.close_keys.is_empty() {
            cancel.push(Hint::Keys(self.close_keys.clone()));
        }
        items.push((cancel, "Cancel"));

        let dc = self.back.dc;
        let font = Font::ui(px(12, dpi));
        let small = Font::ui(px(11, dpi));
        let key_font = Font::ui_bold(px(10, dpi));
        let (mw, mh) = gfx::mouse_icon_size(dpi);
        let key_h = px(19, dpi);
        let cap_gap = px(3, dpi);
        let or_w = gfx::text_size(dc, &small, "or").0 + px(10, dpi);
        let cap_w = |k: &str| {
            if k == "Win" {
                key_h + px(4, dpi)
            } else {
                (gfx::text_size(dc, &key_font, k).0 + px(12, dpi)).max(key_h)
            }
        };
        let icon_w = |h: &Hint| match h {
            Hint::Mouse(_) => mw,
            Hint::Keys(ks) => {
                ks.iter().map(|k| cap_w(k)).sum::<i32>() + cap_gap * (ks.len() as i32 - 1)
            }
        };
        let group_w = |g: &[Hint]| g.iter().map(&icon_w).sum::<i32>() + or_w * (g.len() as i32 - 1);
        let (pad, icon_gap, group_gap) = (px(12, dpi), px(7, dpi), px(18, dpi));
        let widths: Vec<(i32, i32)> = items
            .iter()
            .map(|(g, label)| (group_w(g), gfx::text_size(dc, &font, label).0))
            .collect();
        let w = 2 * pad
            + widths.iter().map(|(a, b)| a + icon_gap + b).sum::<i32>()
            + group_gap * (items.len() as i32 - 1);
        let h = px(32, dpi);
        let x = mon.x + (mon.w - w) / 2;
        let y = mon.y + px(16, dpi);
        // Always shown, also with the cursor under it (owner, 2026-11-12).
        let pill = Rect::new(x, y, w, h);
        gfx::round_rect(
            dc,
            &pill,
            px(8, dpi),
            rgb(24, 24, 28),
            Some(rgb(60, 60, 70)),
        );
        let mut at = x + pad;
        let mid = y + h / 2;
        for ((group, label), (_, tw)) in items.iter().zip(&widths) {
            for (n, hint) in group.iter().enumerate() {
                if n > 0 {
                    let r = Rect::new(at, y, or_w, h);
                    gfx::text(dc, &small, "or", &r, rgb(150, 150, 160));
                    at += or_w;
                }
                match hint {
                    Hint::Mouse(b) => gfx::mouse_icon(
                        &self.back,
                        at,
                        mid - mh / 2,
                        dpi,
                        *b,
                        rgb(225, 225, 232),
                        ACCENT,
                    ),
                    Hint::Keys(ks) => {
                        let mut kx = at;
                        for k in ks {
                            let cw = cap_w(k);
                            let r = Rect::new(kx, mid - key_h / 2, cw, key_h);
                            gfx::key_cap(dc, &key_font, k, &r, dpi);
                            kx += cw + cap_gap;
                        }
                    }
                }
                at += icon_w(hint);
            }
            at += icon_gap;
            let r = Rect::new(at, y, *tw, h);
            gfx::text(dc, &font, label, &r, rgb(240, 240, 245));
            at += tw + group_gap;
        }
    }
}

/// One icon in the overlay hint.
enum Hint {
    Mouse(gfx::MouseButton),
    /// Key caps for a key or a combination ("Win", "Shift", "S"); Win is drawn as its logo.
    Keys(Vec<String>),
}

const ASSERT_TIMER: usize = 7;
const ASSERT_TICKS: u32 = 13;

/// Brings `hwnd` to the front with the keyboard, also when another app has the foreground: joins
/// that app's input queue for the moment, which Windows allows for a window the user just asked for.
fn force_foreground(hwnd: HWND) {
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    unsafe {
        let fg = GetForegroundWindow();
        let me = GetCurrentThreadId();
        let other = if fg.is_invalid() {
            0
        } else {
            GetWindowThreadProcessId(fg, None)
        };
        let joined = other != 0 && other != me && AttachThreadInput(me, other, true).as_bool();
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(hwnd));
        if joined {
            let _ = AttachThreadInput(me, other, false);
        }
    }
}

/// The window the user was in, unless it is a shell flyout (Start, Search) that is being closed.
fn previous_foreground() -> Option<HWND> {
    let fg = unsafe { GetForegroundWindow() };
    if fg.is_invalid() {
        return None;
    }
    let app = glint_capture::windows_list::foreground_app()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let shell = [
        "startmenuexperiencehost",
        "searchhost",
        "searchapp",
        "shellexperiencehost",
    ];
    (!shell.iter().any(|s| app.contains(s))).then_some(fg)
}

/// The app that owns `hwnd`, for the log.
fn focus_owner(hwnd: HWND) -> String {
    if hwnd.is_invalid() {
        return "no window".into();
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    glint_capture::windows_list::process_name(pid).unwrap_or_else(|| format!("pid {pid}"))
}

fn local_rect(r: Rect, vb: &Rect) -> Rect {
    r.translate(-vb.x, -vb.y)
}

type PCWSTRCursor = windows::core::PCWSTR;
const IDC_CROSS_: PCWSTRCursor = IDC_CROSS;
const IDC_SIZENS_: PCWSTRCursor = IDC_SIZENS;
const IDC_SIZEWE_: PCWSTRCursor = IDC_SIZEWE;
const IDC_SIZENWSE_: PCWSTRCursor = IDC_SIZENWSE;
const IDC_SIZENESW_: PCWSTRCursor = IDC_SIZENESW;
const IDC_SIZEALL_: PCWSTRCursor = IDC_SIZEALL;

fn close_if_done() {
    // Take the state out first: DestroyWindow sends messages back into wndproc.
    let taken = STATE.with(|s| {
        let mut s = s.try_borrow_mut().ok()?;
        s.as_ref()?.done.as_ref()?;
        s.take()
    });
    let outcome = taken.map(|mut o| {
        let done = o.done.take().unwrap_or(Outcome::Cancel);
        unsafe {
            // Hand the keyboard back first, while the overlay still has it (Windows allows that).
            if let Some(p) = o.prev.filter(|&p| IsWindow(Some(p)).as_bool()) {
                let _ = SetForegroundWindow(p);
            }
            let _ = DestroyWindow(o.hwnd);
        }
        drop(o); // frees the frozen frame and bitmaps
        done
    });
    if let Some(outcome) = outcome {
        if let Some(f) = ON_DONE.with(|d| d.borrow_mut().take()) {
            f(outcome);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let handled = STATE.with(|s| {
        let Ok(mut guard) = s.try_borrow_mut() else {
            return None;
        };
        let o = guard.as_mut().filter(|o| o.hwnd == hwnd)?;
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                o.paint(hdc);
                let _ = EndPaint(hwnd, &ps);
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_TIMER if wp.0 == ASSERT_TIMER => {
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
                );
                if GetForegroundWindow() != hwnd || GetFocus() != hwnd {
                    force_foreground(hwnd);
                }
                o.assert_ticks = o.assert_ticks.saturating_sub(1);
                if o.assert_ticks == 0 {
                    let _ = KillTimer(Some(hwnd), ASSERT_TIMER);
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                let (x, y) = o.to_virtual(lp);
                o.mouse_down(x, y);
                o.invalidate();
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                let (x, y) = o.to_virtual(lp);
                o.mouse_move(x, y);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let (x, y) = o.to_virtual(lp);
                o.mouse_up(x, y);
                Some(LRESULT(0))
            }
            WM_LBUTTONDBLCLK => {
                let (x, y) = o.to_virtual(lp);
                o.cur = (x, y);
                if o.sel.is_some_and(|s| s.contains(x, y)) {
                    let t = o.target();
                    o.finish(t);
                }
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                let (x, y) = o.to_virtual(lp);
                o.cur = (x, y);
                if matches!(o.drag, Drag::None) {
                    // Right-click: the whole monitor under the cursor.
                    let r = o.monitor().rect;
                    o.finish(Some(r));
                } else {
                    // Right-click during a drag drops the drag.
                    let _ = ReleaseCapture();
                    o.drag = Drag::None;
                    o.sel = None;
                    o.invalidate();
                }
                Some(LRESULT(0))
            }
            WM_MBUTTONUP => {
                // Middle-click: the same area as the last capture.
                if let Some(r) = o.last.and_then(|r| r.intersect(&o.vb)) {
                    o.finish(Some(r));
                }
                Some(LRESULT(0))
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                o.key(wp.0 as u16);
                Some(LRESULT(0))
            }
            WM_SETCURSOR => {
                let c = o.cursor_for();
                SetCursor(LoadCursorW(None, c).ok());
                Some(LRESULT(1))
            }
            WM_KILLFOCUS => {
                if o.done.is_none() {
                    let to = focus_owner(HWND(wp.0 as *mut _));
                    if o.assert_ticks > 0 {
                        // Just opened: Windows can move the focus for a moment while the overlay
                        // takes it (it closed at once, so the hotkey seemed dead; owner report,
                        // 2026-11-13). The assert timer takes it back.
                        crate::log::line(&format!(
                            "overlay lost focus early to {to}, taking it back"
                        ));
                    } else {
                        // Another window took focus later (for example a UAC prompt): give up.
                        crate::log::line(&format!("overlay lost focus to {to}, closing"));
                        o.finish(None);
                    }
                }
                Some(LRESULT(0))
            }
            WM_DPICHANGED => Some(LRESULT(0)), // keep covering the whole desktop
            _ => None,
        }
    });
    close_if_done();
    handled.unwrap_or_else(|| DefWindowProcW(hwnd, msg, wp, lp))
}
