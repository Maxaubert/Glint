//! A plain RGBA8 image (straight alpha) and the few operations Glint needs.

use crate::Rect;

#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// RGBA8, row-major, no padding.
    pub data: Vec<u8>,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image({}x{})", self.width, self.height)
    }
}

impl Image {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, data: vec![0; width as usize * height as usize * 4] }
    }

    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        let mut img = Self::new(width, height);
        for px in img.data.chunks_exact_mut(4) {
            px.copy_from_slice(&rgba);
        }
        img
    }

    /// From BGRA rows (as GDI and DXGI give them) with the given stride in bytes.
    /// `opaque` forces alpha to 255 (GDI leaves it 0).
    pub fn from_bgra(width: u32, height: u32, src: &[u8], stride: usize, opaque: bool) -> Self {
        let mut img = Self::new(width, height);
        let row = width as usize * 4;
        for y in 0..height as usize {
            let s = &src[y * stride..y * stride + row];
            let d = &mut img.data[y * row..(y + 1) * row];
            for (dp, sp) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
                dp[0] = sp[2];
                dp[1] = sp[1];
                dp[2] = sp[0];
                dp[3] = if opaque { 255 } else { sp[3] };
            }
        }
        img
    }

    /// BGRA bytes, top-down, no padding.
    pub fn to_bgra(&self) -> Vec<u8> {
        let mut out = self.data.clone();
        for px in out.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        out
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [self.data[i], self.data[i + 1], self.data[i + 2], self.data[i + 3]]
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width as i32, self.height as i32)
    }

    /// The part of the image inside `r` (clamped to the image).
    pub fn crop(&self, r: &Rect) -> Image {
        let r = r.clamp_to(&self.bounds());
        let mut out = Image::new(r.w.max(0) as u32, r.h.max(0) as u32);
        if r.is_empty() {
            return out;
        }
        let (sw, ow) = (self.width as usize * 4, out.width as usize * 4);
        for y in 0..r.h as usize {
            let s = (r.y as usize + y) * sw + r.x as usize * 4;
            out.data[y * ow..(y + 1) * ow].copy_from_slice(&self.data[s..s + ow]);
        }
        out
    }

    /// Copies `src` onto this image with its top-left at `x, y` (clipped, no blending).
    pub fn blit(&mut self, src: &Image, x: i32, y: i32) {
        let dst_rect = Rect::new(x, y, src.width as i32, src.height as i32);
        let Some(clip) = dst_rect.intersect(&self.bounds()) else { return };
        let (sw, dw) = (src.width as usize * 4, self.width as usize * 4);
        let n = clip.w as usize * 4;
        for row in 0..clip.h {
            let sy = (clip.y - y + row) as usize;
            let sx = (clip.x - x) as usize;
            let s = sy * sw + sx * 4;
            let d = (clip.y + row) as usize * dw + clip.x as usize * 4;
            self.data[d..d + n].copy_from_slice(&src.data[s..s + n]);
        }
    }

    /// A box-filtered thumbnail that fits inside `max_w x max_h` (never upscales).
    pub fn thumbnail(&self, max_w: u32, max_h: u32) -> Image {
        if self.width == 0 || self.height == 0 {
            return Image::new(1, 1);
        }
        let scale = (max_w as f64 / self.width as f64).min(max_h as f64 / self.height as f64).min(1.0);
        let tw = ((self.width as f64 * scale).round() as u32).max(1);
        let th = ((self.height as f64 * scale).round() as u32).max(1);
        let mut out = Image::new(tw, th);
        for ty in 0..th {
            let y0 = (ty as u64 * self.height as u64 / th as u64) as u32;
            let y1 = (((ty + 1) as u64 * self.height as u64 / th as u64) as u32).max(y0 + 1);
            for tx in 0..tw {
                let x0 = (tx as u64 * self.width as u64 / tw as u64) as u32;
                let x1 = (((tx + 1) as u64 * self.width as u64 / tw as u64) as u32).max(x0 + 1);
                // Sample at most about 4x4 points per output pixel: fast enough for big screens.
                let (sx, sy) = (((x1 - x0) / 4).max(1), ((y1 - y0) / 4).max(1));
                let mut acc = [0u64; 4];
                let mut n = 0u64;
                let mut y = y0;
                while y < y1 {
                    let mut x = x0;
                    while x < x1 {
                        let p = self.pixel(x, y);
                        for c in 0..4 {
                            acc[c] += p[c] as u64;
                        }
                        n += 1;
                        x += sx;
                    }
                    y += sy;
                }
                let i = (ty as usize * tw as usize + tx as usize) * 4;
                for c in 0..4 {
                    out.data[i + c] = (acc[c] / n) as u8;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> Image {
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                img.data[i..i + 4].copy_from_slice(&[x as u8, y as u8, 7, 255]);
            }
        }
        img
    }

    #[test]
    fn bgra_round_trip_with_stride() {
        let src = [1u8, 2, 3, 0, 4, 5, 6, 0, 9, 9, 9, 9];
        let img = Image::from_bgra(2, 1, &src, 12, true);
        assert_eq!(img.data, vec![3, 2, 1, 255, 6, 5, 4, 255]);
        assert_eq!(img.to_bgra(), vec![1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn crop_and_blit() {
        let img = gradient(10, 10);
        let c = img.crop(&Rect::new(2, 3, 4, 2));
        assert_eq!((c.width, c.height), (4, 2));
        assert_eq!(c.pixel(0, 0), [2, 3, 7, 255]);
        assert_eq!(c.pixel(3, 1), [5, 4, 7, 255]);
        let mut dst = Image::new(5, 5);
        dst.blit(&c, -1, 4);
        assert_eq!(dst.pixel(0, 4), [3, 3, 7, 255]);
        assert_eq!(dst.pixel(0, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn crop_outside_is_clamped() {
        let img = gradient(4, 4);
        assert_eq!(img.crop(&Rect::new(2, 2, 10, 10)).width, 2);
        assert!(img.crop(&Rect::new(10, 10, 2, 2)).data.is_empty());
    }

    #[test]
    fn thumbnail_fits() {
        let img = gradient(400, 100);
        let t = img.thumbnail(200, 200);
        assert_eq!((t.width, t.height), (200, 50));
        let small = gradient(10, 10).thumbnail(200, 200);
        assert_eq!((small.width, small.height), (10, 10));
    }
}
