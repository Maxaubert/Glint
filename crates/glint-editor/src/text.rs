//! Text drawing onto a tiny-skia pixmap with ab_glyph (Segoe UI, with Arial as a fallback).

use ab_glyph::{point, Font, FontArc, PxScale, ScaleFont};
use tiny_skia::Pixmap;

pub struct Fonts {
    pub regular: FontArc,
    pub bold: FontArc,
}

fn load(names: &[&str]) -> Option<FontArc> {
    let dir = std::env::var_os("WINDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "C:\\Windows".into())
        .join("Fonts");
    names.iter().find_map(|n| {
        std::fs::read(dir.join(n))
            .ok()
            .and_then(|b| FontArc::try_from_vec(b).ok())
    })
}

impl Fonts {
    pub fn load() -> Option<Fonts> {
        let regular = load(&["segoeui.ttf", "arial.ttf"])?;
        let bold = load(&["seguisb.ttf", "segoeuib.ttf", "arialbd.ttf"])
            .unwrap_or_else(|| regular.clone());
        Some(Fonts { regular, bold })
    }
}

/// Width and height of `text` at `size` px (lines split on `\n`).
pub fn measure(font: &FontArc, text: &str, size: f32) -> (f32, f32) {
    let sf = font.as_scaled(PxScale::from(size));
    let line_h = sf.height() + sf.line_gap();
    let mut w: f32 = 0.0;
    let mut lines = 0;
    for line in text.split('\n') {
        lines += 1;
        let mut x = 0.0;
        let mut prev = None;
        for c in line.chars() {
            let id = sf.glyph_id(c);
            if let Some(p) = prev {
                x += sf.kern(p, id);
            }
            x += sf.h_advance(id);
            prev = Some(id);
        }
        w = w.max(x);
    }
    (w, line_h * lines as f32)
}

/// Blends `color` with coverage `cov` (0..1) into a premultiplied RGBA pixel.
#[inline]
pub fn blend(pm: &mut Pixmap, x: i32, y: i32, color: [u8; 4], cov: f32) {
    if x < 0 || y < 0 || x >= pm.width() as i32 || y >= pm.height() as i32 {
        return;
    }
    let a = (cov.clamp(0.0, 1.0) * color[3] as f32 / 255.0).min(1.0);
    if a <= 0.0 {
        return;
    }
    let i = (y as usize * pm.width() as usize + x as usize) * 4;
    let d = pm.data_mut();
    for c in 0..3 {
        d[i + c] = (color[c] as f32 * a + d[i + c] as f32 * (1.0 - a)).round() as u8;
    }
    d[i + 3] = (255.0 * a + d[i + 3] as f32 * (1.0 - a)).round() as u8;
}

/// Draws `text` with its top-left at `x, y`.
pub fn draw(
    pm: &mut Pixmap,
    font: &FontArc,
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    color: [u8; 4],
) {
    let sf = font.as_scaled(PxScale::from(size));
    let line_h = sf.height() + sf.line_gap();
    for (li, line) in text.split('\n').enumerate() {
        let mut caret = point(x, y + sf.ascent() + li as f32 * line_h);
        let mut prev = None;
        for c in line.chars() {
            let id = sf.glyph_id(c);
            if let Some(p) = prev {
                caret.x += sf.kern(p, id);
            }
            let g = id.with_scale_and_position(size, caret);
            caret.x += sf.h_advance(id);
            prev = Some(id);
            if let Some(og) = font.outline_glyph(g) {
                let b = og.px_bounds();
                let (bx, by) = (b.min.x as i32, b.min.y as i32);
                og.draw(|gx, gy, cov| blend(pm, bx + gx as i32, by + gy as i32, color, cov));
            }
        }
    }
}

/// Text with a contrasting halo so it reads on any background.
pub fn draw_with_halo(
    pm: &mut Pixmap,
    font: &FontArc,
    text: &str,
    x: f32,
    y: f32,
    size: f32,
    color: [u8; 4],
) {
    let lum = 0.299 * color[0] as f32 + 0.587 * color[1] as f32 + 0.114 * color[2] as f32;
    let halo = if lum > 140.0 {
        [0, 0, 0, 200]
    } else {
        [255, 255, 255, 230]
    };
    let r = (size / 14.0).max(1.0);
    for i in 0..12 {
        let a = i as f32 / 12.0 * std::f32::consts::TAU;
        draw(pm, font, text, x + r * a.cos(), y + r * a.sin(), size, halo);
    }
    draw(pm, font, text, x, y, size, color);
}
