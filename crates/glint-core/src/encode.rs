//! PNG and DIB encoding and decoding. Pure byte work, so it is fully unit-tested.

use crate::Image;

/// PNG with fast compression (used for the clipboard, where speed matters).
pub fn encode_png_fast(img: &Image) -> Vec<u8> {
    encode_png(img, png::Compression::Fast)
}

/// PNG with good compression (used when saving to disk).
pub fn encode_png_best(img: &Image) -> Vec<u8> {
    encode_png(img, png::Compression::High)
}

fn encode_png(img: &Image, level: png::Compression) -> Vec<u8> {
    let mut out = Vec::with_capacity(img.data.len() / 3);
    {
        let mut enc = png::Encoder::new(&mut out, img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(level);
        enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut w = enc.write_header().expect("png header");
        w.write_image_data(&img.data).expect("png data");
    }
    out
}

/// Decodes any 8- or 16-bit PNG to RGBA8.
pub fn decode_png(bytes: &[u8]) -> Result<Image, String> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("png too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (w, h) = (info.width, info.height);
    let src = &buf[..info.buffer_size()];
    let mut img = Image::new(w, h);
    let n = (w * h) as usize;
    match info.color_type {
        png::ColorType::Rgba => img.data.copy_from_slice(&src[..n * 4]),
        png::ColorType::Rgb => {
            for i in 0..n {
                img.data[i * 4..i * 4 + 3].copy_from_slice(&src[i * 3..i * 3 + 3]);
                img.data[i * 4 + 3] = 255;
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for i in 0..n {
                let (g, a) = (src[i * 2], src[i * 2 + 1]);
                img.data[i * 4..i * 4 + 4].copy_from_slice(&[g, g, g, a]);
            }
        }
        png::ColorType::Grayscale => {
            for i in 0..n {
                let g = src[i];
                img.data[i * 4..i * 4 + 4].copy_from_slice(&[g, g, g, 255]);
            }
        }
        png::ColorType::Indexed => return Err("indexed png was not expanded".into()),
    }
    Ok(img)
}

const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;
const LCS_SRGB: u32 = 0x7352_4742;
const LCS_GM_IMAGES: u32 = 4;

/// Bottom-up BGRA rows, as DIBs store them.
fn dib_pixels(img: &Image) -> Vec<u8> {
    let row = img.width as usize * 4;
    let mut out = Vec::with_capacity(img.data.len());
    for y in (0..img.height as usize).rev() {
        for px in img.data[y * row..(y + 1) * row].chunks_exact(4) {
            out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    out
}

fn push(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

/// `CF_DIB`: BITMAPINFOHEADER + 32-bit BI_RGB pixels.
pub fn encode_dib(img: &Image) -> Vec<u8> {
    let px = dib_pixels(img);
    let mut v = Vec::with_capacity(40 + px.len());
    push(&mut v, 40);
    push(&mut v, img.width);
    push(&mut v, img.height); // positive = bottom-up
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&32u16.to_le_bytes());
    push(&mut v, BI_RGB);
    push(&mut v, px.len() as u32);
    push(&mut v, 3780); // 96 dpi in pixels per metre
    push(&mut v, 3780);
    push(&mut v, 0);
    push(&mut v, 0);
    v.extend_from_slice(&px);
    v
}

/// `CF_DIBV5`: BITMAPV5HEADER with an alpha mask and sRGB colour space.
pub fn encode_dibv5(img: &Image) -> Vec<u8> {
    let px = dib_pixels(img);
    let mut v = Vec::with_capacity(124 + px.len());
    push(&mut v, 124);
    push(&mut v, img.width);
    push(&mut v, img.height);
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&32u16.to_le_bytes());
    push(&mut v, BI_BITFIELDS);
    push(&mut v, px.len() as u32);
    push(&mut v, 3780);
    push(&mut v, 3780);
    push(&mut v, 0);
    push(&mut v, 0);
    push(&mut v, 0x00FF_0000); // red
    push(&mut v, 0x0000_FF00); // green
    push(&mut v, 0x0000_00FF); // blue
    push(&mut v, 0xFF00_0000); // alpha
    push(&mut v, LCS_SRGB);
    v.extend_from_slice(&[0u8; 36]); // endpoints
    v.extend_from_slice(&[0u8; 12]); // gamma
    push(&mut v, LCS_GM_IMAGES);
    push(&mut v, 0); // profile data
    push(&mut v, 0); // profile size
    push(&mut v, 0); // reserved
    debug_assert_eq!(v.len(), 124);
    v.extend_from_slice(&px);
    v
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    b.get(i..i + 4)
        .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
}

/// Decodes a packed DIB (`CF_DIB` or `CF_DIBV5`) with 24 or 32 bits per pixel.
pub fn decode_dib(b: &[u8]) -> Result<Image, String> {
    let hsize = u32_at(b, 0).ok_or("short dib")? as usize;
    let w = u32_at(b, 4).ok_or("short dib")? as i32;
    let h = u32_at(b, 8).ok_or("short dib")? as i32;
    let bpp = u16::from_le_bytes(b.get(14..16).ok_or("short dib")?.try_into().unwrap());
    let comp = u32_at(b, 16).ok_or("short dib")?;
    if w <= 0 || h == 0 || !(bpp == 24 || bpp == 32) {
        return Err(format!("unsupported dib {w}x{h} {bpp}bpp"));
    }
    // BITMAPINFOHEADER with BI_BITFIELDS stores three masks after the header.
    let mut off = hsize;
    if hsize == 40 && comp == BI_BITFIELDS {
        off += 12;
    }
    let clr_used = u32_at(b, 32).unwrap_or(0) as usize;
    off += clr_used * 4;
    let (wu, hu) = (w as usize, h.unsigned_abs() as usize);
    let stride = (wu * bpp as usize / 8).div_ceil(4) * 4;
    if b.len() < off + stride * hu {
        return Err("dib pixel data is short".into());
    }
    // 32-bit BI_RGB usually has no real alpha; treat all-zero alpha as opaque.
    let has_alpha = bpp == 32
        && (comp == BI_BITFIELDS || hsize >= 108)
        && (0..hu).any(|y| (0..wu).any(|x| b[off + y * stride + x * 4 + 3] != 0));
    let mut img = Image::new(wu as u32, hu as u32);
    for y in 0..hu {
        let src_row = if h > 0 { hu - 1 - y } else { y };
        let s = off + src_row * stride;
        for x in 0..wu {
            let p = s + x * bpp as usize / 8;
            let a = if has_alpha { b[p + 3] } else { 255 };
            let i = (y * wu + x) * 4;
            img.data[i..i + 4].copy_from_slice(&[b[p + 2], b[p + 1], b[p], a]);
        }
    }
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Image {
        let mut img = Image::new(3, 2);
        img.data = vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 128, //
            10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 0,
        ];
        img
    }

    #[test]
    fn png_round_trip() {
        let img = sample();
        assert_eq!(decode_png(&encode_png_fast(&img)).unwrap(), img);
        assert_eq!(decode_png(&encode_png_best(&img)).unwrap(), img);
    }

    #[test]
    fn dib_layout() {
        let d = encode_dib(&sample());
        assert_eq!(d.len(), 40 + 3 * 2 * 4);
        // First stored row is the bottom row; first pixel is (10,20,30) as BGRA.
        assert_eq!(&d[40..44], &[30, 20, 10, 255]);
        assert_eq!(decode_dib(&d).unwrap().pixel(0, 0), [255, 0, 0, 255]);
    }

    #[test]
    fn dibv5_keeps_alpha() {
        let d = encode_dibv5(&sample());
        assert_eq!(u32_at(&d, 0), Some(124));
        let back = decode_dib(&d).unwrap();
        assert_eq!(back, sample());
    }

    #[test]
    fn dib_24bit_with_padding() {
        // 1x2 image, 24 bpp: rows padded to 4 bytes.
        let mut d = Vec::new();
        for x in [40u32, 1, 2] {
            d.extend_from_slice(&x.to_le_bytes());
        }
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&24u16.to_le_bytes());
        d.extend_from_slice(&[0u8; 24]);
        d.extend_from_slice(&[1, 2, 3, 0, 4, 5, 6, 0]); // bottom row, then top row
        let img = decode_dib(&d).unwrap();
        assert_eq!(img.pixel(0, 0), [6, 5, 4, 255]);
        assert_eq!(img.pixel(0, 1), [3, 2, 1, 255]);
    }
}
