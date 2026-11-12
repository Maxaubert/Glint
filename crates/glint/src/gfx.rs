//! Small GDI helpers: memory bitmaps, colours, fonts, text and alpha fills.

use glint_core::{Image, Rect};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::*;

pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn win_rect(r: &Rect) -> RECT {
    RECT { left: r.x, top: r.y, right: r.right(), bottom: r.bottom() }
}

/// A top-down 32-bit DIB selected into its own memory DC.
pub struct Dib {
    pub dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    pub w: i32,
    pub h: i32,
}

impl Dib {
    pub fn new(w: i32, h: i32) -> Option<Dib> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w.max(1),
                    biHeight: -h.max(1),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let Ok(bmp) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
                let _ = DeleteDC(dc);
                return None;
            };
            let old = SelectObject(dc, bmp.into());
            Some(Dib { dc, bmp, old, bits: bits as *mut u8, w: w.max(1), h: h.max(1) })
        }
    }

    /// From an RGBA image. `dim` multiplies colours (1.0 = unchanged).
    pub fn from_image(img: &Image, dim: f32) -> Option<Dib> {
        let mut d = Dib::new(img.width as i32, img.height as i32)?;
        let k = (dim.clamp(0.0, 1.0) * 256.0) as u32;
        for (o, p) in d.pixels_mut().chunks_exact_mut(4).zip(img.data.chunks_exact(4)) {
            o[0] = (p[2] as u32 * k >> 8) as u8;
            o[1] = (p[1] as u32 * k >> 8) as u8;
            o[2] = (p[0] as u32 * k >> 8) as u8;
            o[3] = 255;
        }
        Some(d)
    }

    pub fn pixels_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, self.w as usize * self.h as usize * 4) }
    }

    pub fn pixels(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.bits, self.w as usize * self.h as usize * 4) }
    }

    /// Tests only: with GLINT_TEST_DUMP set, writes what was painted to `GLINT_TEST_DIR/<name>.png`.
    pub fn dump(&self, name: &str) {
        let (Some(_), Some(dir)) = (std::env::var_os("GLINT_TEST_DUMP"), std::env::var_os("GLINT_TEST_DIR")) else { return };
        let img = Image::from_bgra(self.w as u32, self.h as u32, self.pixels(), self.w as usize * 4, true);
        let path = std::path::Path::new(&dir).join(format!("{name}.png"));
        let _ = std::fs::write(path, glint_core::encode::encode_png_fast(&img));
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bmp.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

pub fn fill(dc: HDC, r: &Rect, c: COLORREF) {
    unsafe {
        let b = CreateSolidBrush(c);
        FillRect(dc, &win_rect(r), b);
        let _ = DeleteObject(b.into());
    }
}

/// Fills `r` with colour `c` at `alpha` (0-255) over what is already there.
pub fn fill_alpha(dc: HDC, r: &Rect, c: COLORREF, alpha: u8) {
    if r.is_empty() {
        return;
    }
    thread_local! {
        static ONE: std::cell::RefCell<Option<Dib>> = const { std::cell::RefCell::new(None) };
    }
    ONE.with(|one| {
        let mut one = one.borrow_mut();
        let d = one.get_or_insert_with(|| Dib::new(1, 1).expect("1x1 dib"));
        let px = d.pixels_mut();
        px[0] = (c.0 >> 16) as u8;
        px[1] = (c.0 >> 8) as u8;
        px[2] = c.0 as u8;
        px[3] = 255;
        let bf = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: alpha, AlphaFormat: 0 };
        unsafe {
            let _ = AlphaBlend(dc, r.x, r.y, r.w, r.h, d.dc, 0, 0, 1, 1, bf);
        }
    });
}

pub fn frame(dc: HDC, r: &Rect, c: COLORREF, width: i32) {
    let w = width.max(1);
    fill(dc, &Rect::new(r.x - w, r.y - w, r.w + 2 * w, w), c);
    fill(dc, &Rect::new(r.x - w, r.bottom(), r.w + 2 * w, w), c);
    fill(dc, &Rect::new(r.x - w, r.y, w, r.h), c);
    fill(dc, &Rect::new(r.right(), r.y, w, r.h), c);
}

pub fn round_rect(dc: HDC, r: &Rect, radius: i32, fill_c: COLORREF, border: Option<COLORREF>) {
    unsafe {
        let brush = CreateSolidBrush(fill_c);
        let pen = match border {
            Some(c) => CreatePen(PS_SOLID, 1, c),
            None => CreatePen(PS_NULL, 0, fill_c),
        };
        let ob = SelectObject(dc, brush.into());
        let op = SelectObject(dc, pen.into());
        let _ = RoundRect(dc, r.x, r.y, r.right(), r.bottom(), radius * 2, radius * 2);
        SelectObject(dc, ob);
        SelectObject(dc, op);
        let _ = DeleteObject(brush.into());
        let _ = DeleteObject(pen.into());
    }
}

/// A GDI font, deleted on drop.
pub struct Font(pub HFONT);

impl Font {
    /// `px`: em height in physical pixels.
    pub fn new(face: &str, px: i32, bold: bool) -> Font {
        let f = wide(face);
        unsafe {
            Font(CreateFontW(
                -px.max(6),
                0,
                0,
                0,
                if bold { 600 } else { 400 },
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                PCWSTR(f.as_ptr()),
            ))
        }
    }

    pub fn ui(px: i32) -> Font {
        Font::new("Segoe UI Variable Text", px, false)
    }

    pub fn ui_bold(px: i32) -> Font {
        Font::new("Segoe UI Variable Text", px, true)
    }

    /// Segoe Fluent Icons (Windows 11), with the same code points as Segoe MDL2 Assets.
    pub fn icons(px: i32) -> Font {
        Font::new("Segoe Fluent Icons", px, false)
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.0.into());
        }
    }
}

/// Measures single-line text.
pub fn text_size(dc: HDC, font: &Font, s: &str) -> (i32, i32) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut r = RECT::default();
    unsafe {
        let old = SelectObject(dc, font.0.into());
        DrawTextW(dc, &mut w, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, old);
    }
    (r.right - r.left, r.bottom - r.top)
}

/// Draws single-line text centred in `r`.
pub fn text(dc: HDC, font: &Font, s: &str, r: &Rect, c: COLORREF) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut rc = win_rect(r);
    unsafe {
        let old = SelectObject(dc, font.0.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, c);
        DrawTextW(dc, &mut w, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, old);
    }
}

/// Scales a length in 96-dpi units to physical pixels.
pub fn px(v: i32, dpi: u32) -> i32 {
    (v as i64 * dpi as i64 / 96) as i32
}
