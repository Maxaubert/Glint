//! The annotation model: shapes, hit-testing, handles, and the editor's input state machine.
//! No UI code here, so it is unit-tested directly.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

pub const fn pt(x: f32, y: f32) -> Pt {
    Pt { x, y }
}

impl Pt {
    pub fn dist(self, o: Pt) -> f32 {
        ((self.x - o.x).powi(2) + (self.y - o.y).powi(2)).sqrt()
    }
    pub fn add(self, dx: f32, dy: f32) -> Pt {
        pt(self.x + dx, self.y + dy)
    }
    pub fn mid(self, o: Pt) -> Pt {
        pt((self.x + o.x) / 2.0, (self.y + o.y) / 2.0)
    }
}

/// An axis-aligned box from two corners, in any order.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bx {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Bx {
    pub fn new(a: Pt, b: Pt) -> Bx {
        Bx {
            x0: a.x.min(b.x),
            y0: a.y.min(b.y),
            x1: a.x.max(b.x),
            y1: a.y.max(b.y),
        }
    }
    pub fn w(&self) -> f32 {
        self.x1 - self.x0
    }
    pub fn h(&self) -> f32 {
        self.y1 - self.y0
    }
    pub fn contains(&self, p: Pt, tol: f32) -> bool {
        p.x >= self.x0 - tol && p.x <= self.x1 + tol && p.y >= self.y0 - tol && p.y <= self.y1 + tol
    }
    pub fn grow(&self, d: f32) -> Bx {
        Bx {
            x0: self.x0 - d,
            y0: self.y0 - d,
            x1: self.x1 + d,
            y1: self.y1 + d,
        }
    }
    pub fn union(&self, o: &Bx) -> Bx {
        Bx {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }
    pub fn intersect(&self, o: &Bx) -> Option<Bx> {
        let b = Bx {
            x0: self.x0.max(o.x0),
            y0: self.y0.max(o.y0),
            x1: self.x1.min(o.x1),
            y1: self.y1.min(o.y1),
        };
        (b.x1 > b.x0 && b.y1 > b.y0).then_some(b)
    }
    /// The 8 handle points: NW, N, NE, E, SE, S, SW, W.
    pub fn handles(&self) -> [Pt; 8] {
        let (cx, cy) = ((self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0);
        [
            pt(self.x0, self.y0),
            pt(cx, self.y0),
            pt(self.x1, self.y0),
            pt(self.x1, cy),
            pt(self.x1, self.y1),
            pt(cx, self.y1),
            pt(self.x0, self.y1),
            pt(self.x0, cy),
        ]
    }
    /// Moves handle `i` (see `handles`) to `p`.
    pub fn with_handle(&self, i: usize, p: Pt) -> Bx {
        let (mut x0, mut y0, mut x1, mut y1) = (self.x0, self.y0, self.x1, self.y1);
        match i {
            0 => (x0, y0) = (p.x, p.y),
            1 => y0 = p.y,
            2 => (x1, y0) = (p.x, p.y),
            3 => x1 = p.x,
            4 => (x1, y1) = (p.x, p.y),
            5 => y1 = p.y,
            6 => (x0, y1) = (p.x, p.y),
            _ => x0 = p.x,
        }
        Bx::new(pt(x0, y0), pt(x1, y1))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Select,
    Arrow,
    Rect,
    Ellipse,
    Highlight,
    Pen,
    Text,
    Step,
    Blur,
    Pixelate,
    Crop,
}

impl Tool {
    pub const ALL: [Tool; 11] = [
        Tool::Select,
        Tool::Arrow,
        Tool::Rect,
        Tool::Ellipse,
        Tool::Highlight,
        Tool::Pen,
        Tool::Text,
        Tool::Step,
        Tool::Blur,
        Tool::Pixelate,
        Tool::Crop,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Tool::Select => "select",
            Tool::Arrow => "arrow",
            Tool::Rect => "rect",
            Tool::Ellipse => "ellipse",
            Tool::Highlight => "highlight",
            Tool::Pen => "pen",
            Tool::Text => "text",
            Tool::Step => "step",
            Tool::Blur => "blur",
            Tool::Pixelate => "pixelate",
            Tool::Crop => "crop",
        }
    }

    pub fn from_id(s: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.id() == s)
    }

    pub fn key(self) -> char {
        match self {
            Tool::Select => 'V',
            Tool::Arrow => 'A',
            Tool::Rect => 'R',
            Tool::Ellipse => 'O',
            Tool::Highlight => 'H',
            Tool::Pen => 'P',
            Tool::Text => 'T',
            Tool::Step => 'N',
            Tool::Blur => 'B',
            Tool::Pixelate => 'X',
            Tool::Crop => 'C',
        }
    }

    pub fn from_key(c: char) -> Option<Tool> {
        let c = c.to_ascii_uppercase();
        Tool::ALL.into_iter().find(|t| t.key() == c)
    }

    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select and move",
            Tool::Arrow => "Arrow",
            Tool::Rect => "Rectangle",
            Tool::Ellipse => "Ellipse (circle things)",
            Tool::Highlight => "Highlighter",
            Tool::Pen => "Pen",
            Tool::Text => "Text",
            Tool::Step => "Numbered step",
            Tool::Blur => "Blur",
            Tool::Pixelate => "Pixelate",
            Tool::Crop => "Crop",
        }
    }

    /// Segoe Fluent Icons code point.
    pub fn glyph(self) -> &'static str {
        match self {
            Tool::Select => "\u{E8B0}",
            Tool::Arrow => "\u{E72A}",
            Tool::Rect => "\u{E739}",
            Tool::Ellipse => "\u{EA3A}",
            Tool::Highlight => "\u{E7E6}",
            Tool::Pen => "\u{EDFB}",
            Tool::Text => "\u{E8D2}",
            Tool::Step => "\u{F146}",
            Tool::Blur => "\u{E727}",
            Tool::Pixelate => "\u{F0E2}",
            Tool::Crop => "\u{E7A8}",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub color: [u8; 4],
    /// Stroke width in image pixels. Text size and step size derive from it.
    pub width: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// `bend`: the curve passes through this point (none = straight).
    Arrow {
        a: Pt,
        b: Pt,
        bend: Option<Pt>,
    },
    Rect {
        a: Pt,
        b: Pt,
    },
    Ellipse {
        a: Pt,
        b: Pt,
    },
    Highlight {
        pts: Vec<Pt>,
    },
    Pen {
        pts: Vec<Pt>,
    },
    Text {
        at: Pt,
        text: String,
    },
    Step {
        at: Pt,
        n: u32,
    },
    Blur {
        a: Pt,
        b: Pt,
    },
    Pixelate {
        a: Pt,
        b: Pt,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub kind: Kind,
    pub style: Style,
}

/// Font size in image pixels for a text shape of this stroke width.
pub fn text_size(width: f32) -> f32 {
    8.0 + width * 3.5
}

/// Diameter of a step badge for this stroke width.
pub fn step_size(width: f32) -> f32 {
    16.0 + width * 3.2
}

fn dist_to_segment(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return p.dist(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    p.dist(pt(a.x + t * dx, a.y + t * dy))
}

/// Points along the arrow's path: a straight line, or a quadratic curve through `bend`.
pub fn arrow_path(a: Pt, b: Pt, bend: Option<Pt>) -> Vec<Pt> {
    match bend {
        None => vec![a, b],
        Some(m) => {
            // Control point so the curve passes through `m` at t = 0.5.
            let c = pt(2.0 * m.x - (a.x + b.x) / 2.0, 2.0 * m.y - (a.y + b.y) / 2.0);
            (0..=32)
                .map(|i| {
                    let t = i as f32 / 32.0;
                    let u = 1.0 - t;
                    pt(
                        u * u * a.x + 2.0 * u * t * c.x + t * t * b.x,
                        u * u * a.y + 2.0 * u * t * c.y + t * t * b.y,
                    )
                })
                .collect()
        }
    }
}

fn near_polyline(p: Pt, pts: &[Pt], tol: f32) -> bool {
    match pts.len() {
        0 => false,
        1 => p.dist(pts[0]) <= tol,
        _ => pts
            .windows(2)
            .any(|w| dist_to_segment(p, w[0], w[1]) <= tol),
    }
}

impl Shape {
    pub fn bounds(&self) -> Bx {
        let w = self.style.width;
        match &self.kind {
            Kind::Arrow { a, b, bend } => {
                let path = arrow_path(*a, *b, *bend);
                path.iter()
                    .fold(Bx::new(*a, *a), |acc, p| acc.union(&Bx::new(*p, *p)))
                    .grow(w * 2.5)
            }
            Kind::Rect { a, b } | Kind::Ellipse { a, b } => Bx::new(*a, *b).grow(w / 2.0),
            Kind::Blur { a, b } | Kind::Pixelate { a, b } => Bx::new(*a, *b),
            Kind::Highlight { pts } | Kind::Pen { pts } => {
                let k = if matches!(self.kind, Kind::Highlight { .. }) {
                    w * 3.0
                } else {
                    w
                } / 2.0;
                pts.iter()
                    .fold(Bx::new(pts[0], pts[0]), |acc, p| {
                        acc.union(&Bx::new(*p, *p))
                    })
                    .grow(k)
            }
            Kind::Text { at, text } => {
                let size = text_size(w);
                let chars = text.chars().count().max(1) as f32;
                Bx {
                    x0: at.x,
                    y0: at.y,
                    x1: at.x + chars * size * 0.55,
                    y1: at.y + size * 1.3,
                }
            }
            Kind::Step { at, .. } => {
                let r = step_size(w) / 2.0;
                Bx {
                    x0: at.x - r,
                    y0: at.y - r,
                    x1: at.x + r,
                    y1: at.y + r,
                }
            }
        }
    }

    /// Whether `p` touches the shape (`tol` in image pixels).
    pub fn hit(&self, p: Pt, tol: f32) -> bool {
        let w = self.style.width;
        match &self.kind {
            Kind::Arrow { a, b, bend } => {
                near_polyline(p, &arrow_path(*a, *b, *bend), w * 1.5 + tol)
            }
            Kind::Rect { a, b } => {
                let bx = Bx::new(*a, *b);
                bx.contains(p, w / 2.0 + tol) && !bx.grow(-(w / 2.0 + tol)).contains(p, 0.0)
            }
            Kind::Ellipse { a, b } => {
                let bx = Bx::new(*a, *b);
                let (cx, cy) = ((bx.x0 + bx.x1) / 2.0, (bx.y0 + bx.y1) / 2.0);
                let (rx, ry) = ((bx.w() / 2.0).max(1.0), (bx.h() / 2.0).max(1.0));
                // Distance from the outline, approximately, in pixels.
                let d = (((p.x - cx) / rx).powi(2) + ((p.y - cy) / ry).powi(2)).sqrt();
                ((d - 1.0) * rx.min(ry)).abs() <= w / 2.0 + tol
            }
            Kind::Highlight { pts } => near_polyline(p, pts, w * 1.5 + tol),
            Kind::Pen { pts } => near_polyline(p, pts, w / 2.0 + tol),
            Kind::Blur { .. } | Kind::Pixelate { .. } | Kind::Text { .. } | Kind::Step { .. } => {
                self.bounds().contains(p, tol)
            }
        }
    }

    pub fn translate(&mut self, dx: f32, dy: f32) {
        let mv = |p: &mut Pt| *p = p.add(dx, dy);
        match &mut self.kind {
            Kind::Arrow { a, b, bend } => {
                mv(a);
                mv(b);
                if let Some(m) = bend {
                    mv(m);
                }
            }
            Kind::Rect { a, b }
            | Kind::Ellipse { a, b }
            | Kind::Blur { a, b }
            | Kind::Pixelate { a, b } => {
                mv(a);
                mv(b);
            }
            Kind::Highlight { pts } | Kind::Pen { pts } => pts.iter_mut().for_each(mv),
            Kind::Text { at, .. } | Kind::Step { at, .. } => mv(at),
        }
    }

    /// Drag points shown when the shape is selected.
    pub fn handles(&self) -> Vec<Pt> {
        match &self.kind {
            Kind::Arrow { a, b, bend } => vec![*a, *b, bend.unwrap_or(a.mid(*b))],
            Kind::Rect { a, b }
            | Kind::Ellipse { a, b }
            | Kind::Blur { a, b }
            | Kind::Pixelate { a, b } => Bx::new(*a, *b).handles().to_vec(),
            _ => vec![],
        }
    }

    pub fn move_handle(&mut self, i: usize, p: Pt) {
        match &mut self.kind {
            Kind::Arrow { a, b, bend } => match i {
                0 => *a = p,
                1 => *b = p,
                _ => {
                    // Snap back to straight when the bend is dragged close to the middle.
                    let straight = dist_to_segment(p, *a, *b) < 4.0;
                    *bend = if straight { None } else { Some(p) };
                }
            },
            Kind::Rect { a, b }
            | Kind::Ellipse { a, b }
            | Kind::Blur { a, b }
            | Kind::Pixelate { a, b } => {
                let bx = Bx::new(*a, *b).with_handle(i, p);
                *a = pt(bx.x0, bx.y0);
                *b = pt(bx.x1, bx.y1);
            }
            _ => {}
        }
    }

    /// Too small to keep (a click without a drag).
    pub fn is_degenerate(&self) -> bool {
        match &self.kind {
            Kind::Arrow { a, b, .. } => a.dist(*b) < 4.0,
            Kind::Rect { a, b }
            | Kind::Ellipse { a, b }
            | Kind::Blur { a, b }
            | Kind::Pixelate { a, b } => {
                let bx = Bx::new(*a, *b);
                bx.w() < 3.0 || bx.h() < 3.0
            }
            Kind::Highlight { pts } | Kind::Pen { pts } => pts.len() < 2,
            Kind::Text { text, .. } => text.trim().is_empty(),
            Kind::Step { .. } => false,
        }
    }

    pub fn has_color(&self) -> bool {
        !matches!(self.kind, Kind::Blur { .. } | Kind::Pixelate { .. })
    }
}

/// Snaps the direction from `a` to `b` to steps of 15 degrees.
pub fn snap_angle(a: Pt, b: Pt) -> Pt {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx * dx + dy * dy).sqrt();
    let step = std::f32::consts::PI / 12.0;
    let ang = (dy.atan2(dx) / step).round() * step;
    pt(a.x + len * ang.cos(), a.y + len * ang.sin())
}

/// Makes the box from `a` to `b` square (for Shift with rectangle and ellipse).
pub fn square(a: Pt, b: Pt) -> Pt {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let s = dx.abs().max(dy.abs());
    pt(a.x + s * dx.signum(), a.y + s * dy.signum())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> Style {
        Style {
            color: [255, 0, 0, 255],
            width: 4.0,
        }
    }

    #[test]
    fn box_handles_round_trip() {
        let b = Bx::new(pt(10.0, 10.0), pt(110.0, 60.0));
        let h = b.handles();
        assert_eq!(h[0], pt(10.0, 10.0));
        assert_eq!(h[4], pt(110.0, 60.0));
        assert_eq!(
            b.with_handle(4, pt(200.0, 100.0)),
            Bx::new(pt(10.0, 10.0), pt(200.0, 100.0))
        );
        // Dragging past the opposite edge flips.
        assert_eq!(
            b.with_handle(3, pt(0.0, 0.0)),
            Bx::new(pt(0.0, 10.0), pt(10.0, 60.0))
        );
    }

    #[test]
    fn arrow_hit_and_bend() {
        let mut s = Shape {
            kind: Kind::Arrow {
                a: pt(0.0, 0.0),
                b: pt(100.0, 0.0),
                bend: None,
            },
            style: style(),
        };
        assert!(s.hit(pt(50.0, 3.0), 2.0));
        assert!(!s.hit(pt(50.0, 30.0), 2.0));
        s.move_handle(2, pt(50.0, 40.0));
        assert!(
            s.hit(pt(50.0, 40.0), 2.0),
            "the curve passes through the bend point"
        );
        s.move_handle(2, pt(50.0, 1.0));
        assert!(
            matches!(s.kind, Kind::Arrow { bend: None, .. }),
            "snaps back to straight"
        );
    }

    #[test]
    fn rect_hits_border_not_inside() {
        let s = Shape {
            kind: Kind::Rect {
                a: pt(0.0, 0.0),
                b: pt(100.0, 100.0),
            },
            style: style(),
        };
        assert!(s.hit(pt(0.0, 50.0), 2.0));
        assert!(!s.hit(pt(50.0, 50.0), 2.0));
    }

    #[test]
    fn ellipse_hits_outline() {
        let s = Shape {
            kind: Kind::Ellipse {
                a: pt(0.0, 0.0),
                b: pt(200.0, 100.0),
            },
            style: style(),
        };
        assert!(s.hit(pt(100.0, 0.0), 2.0));
        assert!(s.hit(pt(200.0, 50.0), 2.0));
        assert!(!s.hit(pt(100.0, 50.0), 2.0));
    }

    #[test]
    fn translate_moves_everything() {
        let mut s = Shape {
            kind: Kind::Pen {
                pts: vec![pt(0.0, 0.0), pt(1.0, 1.0)],
            },
            style: style(),
        };
        s.translate(5.0, -1.0);
        assert_eq!(
            s.kind,
            Kind::Pen {
                pts: vec![pt(5.0, -1.0), pt(6.0, 0.0)]
            }
        );
    }

    #[test]
    fn snapping() {
        let p = snap_angle(pt(0.0, 0.0), pt(100.0, 3.0));
        assert!((p.y).abs() < 0.01 && (p.x - 100.04).abs() < 0.1);
        assert_eq!(square(pt(0.0, 0.0), pt(10.0, -4.0)), pt(10.0, -10.0));
    }

    #[test]
    fn tool_keys() {
        for t in Tool::ALL {
            assert_eq!(Tool::from_key(t.key()), Some(t));
            assert_eq!(Tool::from_id(t.id()), Some(t));
        }
    }
}
