//! Draws the document (base image + shapes) with tiny-skia, optionally with selection handles.

use crate::editor::Editor;
use crate::model::*;
use crate::text::{self, Fonts};
use glint_core::Image;
use tiny_skia::{
    BlendMode, Color, FillRule, IntRect, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Rect,
    Stroke, StrokeDash, Transform,
};

const ACCENT: [u8; 4] = [139, 92, 246, 255];

pub struct Renderer {
    base: Pixmap,
    opaque: bool,
    fonts: Option<Fonts>,
}

fn color(c: [u8; 4]) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], c[3])
}

fn paint(c: [u8; 4]) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(color(c));
    p.anti_alias = true;
    p
}

fn stroke(width: f32) -> Stroke {
    Stroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Default::default()
    }
}

fn polyline(pts: &[Pt]) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(pts.first()?.x, pts[0].y);
    if pts.len() == 1 {
        pb.line_to(pts[0].x + 0.01, pts[0].y);
    }
    for p in &pts[1..] {
        pb.line_to(p.x, p.y);
    }
    pb.finish()
}

impl Renderer {
    pub fn new(img: &Image) -> Renderer {
        let mut base = Pixmap::new(img.width.max(1), img.height.max(1)).expect("pixmap");
        let mut opaque = true;
        for (d, s) in base
            .data_mut()
            .chunks_exact_mut(4)
            .zip(img.data.chunks_exact(4))
        {
            let a = s[3] as u32;
            if a != 255 {
                opaque = false;
            }
            d[0] = (s[0] as u32 * a / 255) as u8;
            d[1] = (s[1] as u32 * a / 255) as u8;
            d[2] = (s[2] as u32 * a / 255) as u8;
            d[3] = s[3];
        }
        Renderer {
            base,
            opaque,
            fonts: Fonts::load(),
        }
    }

    /// The visible area (after crop) with all shapes. `ui`: also draw selection, handles and the
    /// pending crop.
    pub fn render(&self, ed: &Editor, ui: bool) -> Pixmap {
        let mut pm = self.base.clone();
        let hidden = ed.hidden();
        for (i, s) in ed.shapes.iter().enumerate() {
            if Some(i) != hidden {
                self.draw_shape(&mut pm, s);
            }
        }
        if ui {
            // Only the Select tool shows the box and handles; drawing shows just the shape.
            let sel = ed.sel.filter(|_| ed.tool == crate::model::Tool::Select);
            if let Some(s) = sel.and_then(|i| ed.shapes.get(i)) {
                draw_selection(&mut pm, s, ed.scale);
            }
            if let Some(c) = ed.crop_pending {
                draw_crop(&mut pm, &c, &ed.view(), ed.scale);
            }
        }
        let v = ed.view();
        if ed.crop.is_some() {
            let r = IntRect::from_xywh(v.x0 as i32, v.y0 as i32, v.w() as u32, v.h() as u32);
            if let Some(c) = r.and_then(|r| pm.clone_rect(r)) {
                return c;
            }
        }
        pm
    }

    /// The final image (no UI), straight alpha.
    pub fn export(&self, ed: &Editor) -> Image {
        let pm = self.render(ed, false);
        to_image(&pm)
    }

    pub fn is_opaque(&self) -> bool {
        self.opaque
    }

    fn draw_shape(&self, pm: &mut Pixmap, s: &Shape) {
        let w = s.style.width;
        let c = s.style.color;
        let t = Transform::identity();
        match &s.kind {
            Kind::Arrow { a, b, bend } => draw_arrow(pm, *a, *b, *bend, w, c),
            Kind::Rect { a, b } => {
                if let Some(r) =
                    Rect::from_ltrb(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y))
                {
                    let path = PathBuilder::from_rect(r);
                    pm.stroke_path(&path, &paint(c), &stroke(w), t, None);
                }
            }
            Kind::Ellipse { a, b } => {
                if let Some(r) =
                    Rect::from_ltrb(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y))
                {
                    if let Some(path) = PathBuilder::from_oval(r) {
                        pm.stroke_path(&path, &paint(c), &stroke(w), t, None);
                    }
                }
            }
            Kind::Highlight { pts } => {
                if let Some(path) = polyline(pts) {
                    let mut p = paint(c);
                    p.blend_mode = BlendMode::Multiply;
                    let mut st = stroke(w * crate::editor::HIGHLIGHT_WIDTH);
                    st.line_cap = LineCap::Square;
                    pm.stroke_path(&path, &p, &st, t, None);
                }
            }
            Kind::Pen { pts } => {
                if let Some(path) = polyline(pts) {
                    pm.stroke_path(&path, &paint(c), &stroke(w), t, None);
                }
            }
            Kind::Text { at, text } => {
                if let Some(f) = &self.fonts {
                    text::draw_with_halo(pm, &f.regular, text, at.x, at.y, text_size(w), c);
                }
            }
            Kind::Step { at, n } => {
                let d = step_size(w);
                if let Some(path) = PathBuilder::from_circle(at.x, at.y, d / 2.0) {
                    pm.fill_path(&path, &paint(c), FillRule::Winding, t, None);
                    pm.stroke_path(
                        &path,
                        &paint([255, 255, 255, 255]),
                        &stroke((d * 0.07).max(1.0)),
                        t,
                        None,
                    );
                }
                if let Some(f) = &self.fonts {
                    let label = n.to_string();
                    let size = d * 0.58;
                    let (tw, th) = text::measure(&f.bold, &label, size);
                    let lum = 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
                    let fg = if lum > 160.0 {
                        [20, 20, 20, 255]
                    } else {
                        [255, 255, 255, 255]
                    };
                    text::draw(
                        pm,
                        &f.bold,
                        &label,
                        at.x - tw / 2.0,
                        at.y - th / 2.0,
                        size,
                        fg,
                    );
                }
            }
            Kind::Blur { a, b } => {
                let bx = Bx::new(*a, *b);
                let r = ((bx.w().min(bx.h()) / 6.0) as usize).clamp(6, 40);
                region(pm, &bx, |buf, w, h| box_blur(buf, w, h, r));
            }
            Kind::Pixelate { a, b } => {
                let bx = Bx::new(*a, *b);
                let block = ((bx.w().min(bx.h()) / 10.0) as usize).clamp(8, 48);
                region(pm, &bx, |buf, w, h| pixelate(buf, w, h, block));
            }
        }
    }
}

/// Converts a premultiplied pixmap to a straight-alpha image.
pub fn to_image(pm: &Pixmap) -> Image {
    let mut img = Image::new(pm.width(), pm.height());
    for (d, s) in img.data.chunks_exact_mut(4).zip(pm.data().chunks_exact(4)) {
        let a = s[3] as u32;
        if a == 255 || a == 0 {
            d.copy_from_slice(s);
        } else {
            for c in 0..3 {
                d[c] = ((s[c] as u32 * 255 + a / 2) / a).min(255) as u8;
            }
            d[3] = s[3];
        }
    }
    img
}

fn draw_arrow(pm: &mut Pixmap, a: Pt, b: Pt, bend: Option<Pt>, w: f32, c: [u8; 4]) {
    let path = arrow_path(a, b, bend);
    let n = path.len();
    // Direction at the tip, from a point a little way back along the path.
    let head_len = (w * 4.2).max(10.0);
    let mut back = path[0];
    for p in path.iter().rev() {
        if p.dist(b) >= head_len {
            back = *p;
            break;
        }
    }
    let (dx, dy) = (b.x - back.x, b.y - back.y);
    let len = (dx * dx + dy * dy).sqrt().max(0.001);
    let (ux, uy) = (dx / len, dy / len);
    let half = head_len * 0.62;
    let base = pt(b.x - ux * head_len, b.y - uy * head_len);
    // Shaft: stop inside the head so the tip stays sharp.
    let shaft_end = pt(b.x - ux * head_len * 0.6, b.y - uy * head_len * 0.6);
    let mut shaft: Vec<Pt> = path
        .iter()
        .copied()
        .filter(|p| p.dist(b) > head_len * 0.6)
        .collect();
    if shaft.is_empty() || n < 2 {
        shaft = vec![a];
    }
    shaft.push(shaft_end);
    if let Some(sp) = polyline(&shaft) {
        pm.stroke_path(&sp, &paint(c), &stroke(w), Transform::identity(), None);
    }
    let mut pb = PathBuilder::new();
    pb.move_to(b.x, b.y);
    pb.line_to(base.x - uy * half, base.y + ux * half);
    // A slight notch makes the head look less blunt.
    pb.line_to(base.x + ux * head_len * 0.18, base.y + uy * head_len * 0.18);
    pb.line_to(base.x + uy * half, base.y - ux * half);
    pb.close();
    if let Some(head) = pb.finish() {
        let mut p = paint(c);
        p.anti_alias = true;
        pm.fill_path(&head, &p, FillRule::Winding, Transform::identity(), None);
        let mut st = stroke((w * 0.35).max(1.0));
        st.line_join = LineJoin::Round;
        pm.stroke_path(&head, &p, &st, Transform::identity(), None);
    }
}

fn draw_selection(pm: &mut Pixmap, s: &Shape, scale: f32) {
    let bx = s.bounds().grow(4.0 * scale);
    if let Some(r) = Rect::from_ltrb(bx.x0, bx.y0, bx.x1, bx.y1) {
        let path = PathBuilder::from_rect(r);
        let mut st = Stroke {
            width: 1.5 * scale,
            ..Default::default()
        };
        st.dash = StrokeDash::new(vec![5.0 * scale, 4.0 * scale], 0.0);
        pm.stroke_path(
            &path,
            &paint([255, 255, 255, 220]),
            &Stroke {
                width: 3.0 * scale,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
        pm.stroke_path(&path, &paint(ACCENT), &st, Transform::identity(), None);
    }
    for h in s.handles() {
        if let Some(c) = PathBuilder::from_circle(h.x, h.y, 5.0 * scale) {
            pm.fill_path(
                &c,
                &paint([255, 255, 255, 255]),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
            pm.stroke_path(
                &c,
                &paint(ACCENT),
                &stroke(1.5 * scale),
                Transform::identity(),
                None,
            );
        }
    }
}

fn draw_crop(pm: &mut Pixmap, c: &Bx, view: &Bx, scale: f32) {
    let dim = paint([0, 0, 0, 140]);
    let t = Transform::identity();
    for r in [
        Rect::from_ltrb(view.x0, view.y0, view.x1, c.y0),
        Rect::from_ltrb(view.x0, c.y1, view.x1, view.y1),
        Rect::from_ltrb(view.x0, c.y0, c.x0, c.y1),
        Rect::from_ltrb(c.x1, c.y0, view.x1, c.y1),
    ]
    .into_iter()
    .flatten()
    {
        pm.fill_rect(r, &dim, t, None);
    }
    if let Some(r) = Rect::from_ltrb(c.x0, c.y0, c.x1, c.y1) {
        pm.stroke_path(
            &PathBuilder::from_rect(r),
            &paint([255, 255, 255, 255]),
            &Stroke {
                width: 1.5 * scale,
                ..Default::default()
            },
            t,
            None,
        );
    }
}

/// Runs `f` on the pixels inside `bx` (clamped), as a tight RGBA buffer.
fn region(pm: &mut Pixmap, bx: &Bx, f: impl Fn(&mut [u8], usize, usize)) {
    let x0 = bx.x0.max(0.0).floor() as usize;
    let y0 = bx.y0.max(0.0).floor() as usize;
    let x1 = (bx.x1.ceil() as usize).min(pm.width() as usize);
    let y1 = (bx.y1.ceil() as usize).min(pm.height() as usize);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let (w, h) = (x1 - x0, y1 - y0);
    let stride = pm.width() as usize * 4;
    let mut buf = vec![0u8; w * h * 4];
    {
        let d = pm.data();
        for y in 0..h {
            let s = (y0 + y) * stride + x0 * 4;
            buf[y * w * 4..(y + 1) * w * 4].copy_from_slice(&d[s..s + w * 4]);
        }
    }
    f(&mut buf, w, h);
    let d = pm.data_mut();
    for y in 0..h {
        let s = (y0 + y) * stride + x0 * 4;
        d[s..s + w * 4].copy_from_slice(&buf[y * w * 4..(y + 1) * w * 4]);
    }
}

/// Three passes of a box blur (close to a Gaussian). Pixels outside the box are not used,
/// so nothing of the original survives inside it.
pub fn box_blur(buf: &mut [u8], w: usize, h: usize, r: usize) {
    let mut tmp = vec![0u8; buf.len()];
    for _ in 0..3 {
        blur_pass(buf, &mut tmp, w, h, r, true);
        blur_pass(&tmp, buf, w, h, r, false);
    }
}

fn blur_pass(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, horizontal: bool) {
    let (outer, inner) = if horizontal { (h, w) } else { (w, h) };
    let idx = |o: usize, i: usize| {
        if horizontal {
            (o * w + i) * 4
        } else {
            (i * w + o) * 4
        }
    };
    for o in 0..outer {
        let mut acc = [0u32; 4];
        let mut count = 0u32;
        // Window [i - r, i + r], clamped.
        for i in 0..=r.min(inner - 1) {
            let k = idx(o, i);
            for c in 0..4 {
                acc[c] += src[k + c] as u32;
            }
            count += 1;
        }
        for i in 0..inner {
            let k = idx(o, i);
            for c in 0..4 {
                dst[k + c] = (acc[c] / count) as u8;
            }
            let add = i + r + 1;
            if add < inner {
                let k = idx(o, add);
                for c in 0..4 {
                    acc[c] += src[k + c] as u32;
                }
                count += 1;
            }
            if i >= r {
                let k = idx(o, i - r);
                for c in 0..4 {
                    acc[c] -= src[k + c] as u32;
                }
                count -= 1;
            }
        }
    }
}

pub fn pixelate(buf: &mut [u8], w: usize, h: usize, block: usize) {
    let block = block.max(1);
    for by in (0..h).step_by(block) {
        for bx in (0..w).step_by(block) {
            let (ex, ey) = ((bx + block).min(w), (by + block).min(h));
            let mut acc = [0u64; 4];
            let n = ((ex - bx) * (ey - by)) as u64;
            for y in by..ey {
                for x in bx..ex {
                    let k = (y * w + x) * 4;
                    for c in 0..4 {
                        acc[c] += buf[k + c] as u64;
                    }
                }
            }
            let avg = acc.map(|v| (v / n) as u8);
            for y in by..ey {
                for x in bx..ex {
                    let k = (y * w + x) * 4;
                    buf[k..k + 4].copy_from_slice(&avg);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{Editor, Mods};

    fn checker(w: u32, h: u32) -> Image {
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = if (x / 4 + y / 4) % 2 == 0 { 255 } else { 0 };
                let i = ((y * w + x) * 4) as usize;
                img.data[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        img
    }

    #[test]
    fn blur_removes_detail() {
        let img = checker(64, 64);
        let mut buf = img.data.clone();
        box_blur(&mut buf, 64, 64, 8);
        let mid = (32 * 64 + 32) * 4;
        assert!((100..160).contains(&buf[mid]), "{}", buf[mid]);
    }

    #[test]
    fn pixelate_makes_blocks() {
        let img = checker(16, 16);
        let mut buf = img.data.clone();
        pixelate(&mut buf, 16, 16, 8);
        assert_eq!(&buf[0..4], &buf[(7 * 16 + 7) * 4..(7 * 16 + 7) * 4 + 4]);
    }

    #[test]
    fn export_draws_arrow_and_crops() {
        let img = Image::filled(200, 100, [255, 255, 255, 255]);
        let r = Renderer::new(&img);
        let mut e = Editor::new(200, 100, 1.0);
        let m = Mods::default();
        e.pointer_down(20.0, 50.0, m);
        e.pointer_up(180.0, 50.0, m);
        let out = r.export(&e);
        assert_eq!((out.width, out.height), (200, 100));
        let p = out.pixel(100, 50);
        assert!(p[0] > 200 && p[1] < 100, "arrow is red: {p:?}");
        assert_eq!(out.pixel(100, 10), [255, 255, 255, 255]);
        e.set_tool(crate::model::Tool::Crop);
        e.pointer_down(50.0, 20.0, m);
        e.pointer_up(150.0, 80.0, m);
        e.apply_crop();
        let out = r.export(&e);
        assert_eq!((out.width, out.height), (100, 60));
    }

    #[test]
    fn highlighter_multiplies() {
        let mut img = Image::filled(100, 40, [255, 255, 255, 255]);
        for x in 0..100 {
            for y in 18..22 {
                let i = ((y * 100 + x) * 4) as usize;
                img.data[i..i + 3].copy_from_slice(&[0, 0, 0]);
            }
        }
        let r = Renderer::new(&img);
        let mut e = Editor::new(100, 40, 1.0);
        e.set_tool(crate::model::Tool::Highlight);
        let m = Mods::default();
        e.pointer_down(5.0, 20.0, m);
        e.pointer_move(50.0, 20.0, m);
        e.pointer_up(95.0, 20.0, m);
        let out = r.export(&e);
        assert_eq!(&out.pixel(50, 20)[..3], &[0, 0, 0], "dark text stays dark");
        let bg = out.pixel(50, 15);
        assert!(bg[0] > 200 && bg[2] < 60, "white turns yellow: {bg:?}");
    }

    #[test]
    fn text_and_steps_render() {
        let img = Image::filled(200, 100, [255, 255, 255, 255]);
        let r = Renderer::new(&img);
        if r.fonts.is_none() {
            return; // no system fonts (CI without fonts)
        }
        let mut e = Editor::new(200, 100, 1.0);
        e.set_tool(crate::model::Tool::Step);
        let m = Mods::default();
        e.pointer_down(30.0, 30.0, m);
        e.pointer_up(30.0, 30.0, m);
        e.set_tool(crate::model::Tool::Text);
        e.pointer_down(80.0, 50.0, m);
        e.pointer_up(80.0, 50.0, m);
        e.commit_text("Hi");
        let out = r.export(&e);
        let red = out
            .data
            .chunks_exact(4)
            .filter(|p| p[0] > 200 && p[1] < 80)
            .count();
        assert!(red > 200, "step and text drawn ({red} red pixels)");
    }
}
