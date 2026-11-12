//! The Glint icon, drawn in code: a violet-to-teal rounded square with a white four-point sparkle.

/// RGBA pixels of the icon at `n x n`.
pub fn pixels(n: u32) -> Vec<u8> {
    let mut out = vec![0u8; (n * n * 4) as usize];
    const SS: u32 = 4; // supersampling per axis
    for y in 0..n {
        for x in 0..n {
            let (mut bg, mut star) = (0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let u = (x as f32 + (sx as f32 + 0.5) / SS as f32) / n as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / SS as f32) / n as f32;
                    if in_rounded_square(u, v, 0.04, 0.24) {
                        bg += 1;
                        if in_sparkle(u - 0.5, v - 0.5, 0.33) {
                            star += 1;
                        }
                    }
                }
            }
            let total = (SS * SS) as f32;
            let a = bg as f32 / total;
            let s = if bg > 0 { star as f32 / bg as f32 } else { 0.0 };
            // Diagonal gradient: violet (top-left) to teal (bottom-right).
            let t = (x + y) as f32 / (2 * n) as f32;
            let base = [
                lerp(124.0, 20.0, t),
                lerp(77.0, 184.0, t),
                lerp(255.0, 166.0, t),
            ];
            let i = ((y * n + x) * 4) as usize;
            for c in 0..3 {
                out[i + c] = lerp(base[c], 255.0, s).round() as u8;
            }
            out[i + 3] = (a * 255.0).round() as u8;
        }
    }
    out
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn in_rounded_square(u: f32, v: f32, inset: f32, r: f32) -> bool {
    let (lo, hi) = (inset, 1.0 - inset);
    if u < lo || u > hi || v < lo || v > hi {
        return false;
    }
    let cx = u.clamp(lo + r, hi - r);
    let cy = v.clamp(lo + r, hi - r);
    (u - cx).powi(2) + (v - cy).powi(2) <= r * r
}

/// An astroid: |x|^(2/3) + |y|^(2/3) <= r^(2/3).
fn in_sparkle(x: f32, y: f32, r: f32) -> bool {
    x.abs().powf(2.0 / 3.0) + y.abs().powf(2.0 / 3.0) <= r.powf(2.0 / 3.0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn corners_are_clear_and_centre_is_white() {
        let n = 32;
        let p = super::pixels(n);
        assert_eq!(p[3], 0, "top-left corner is transparent");
        let c = ((n / 2 * n + n / 2) * 4) as usize;
        assert_eq!(&p[c..c + 4], &[255, 255, 255, 255]);
    }
}
