//! Integer rectangles in physical pixels, and the selection math the overlay uses.

/// A half-open rectangle: covers `x..x+w` and `y..y+h`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// The rectangle spanned by two corner points, in any order.
    pub fn from_points(ax: i32, ay: i32, bx: i32, by: i32) -> Self {
        Self::new(ax.min(bx), ay.min(by), (ax - bx).abs(), (ay - by).abs())
    }

    pub fn from_ltrb(l: i32, t: i32, r: i32, b: i32) -> Self {
        Self::from_points(l, t, r, b)
    }

    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub const fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub const fn area(&self) -> i64 {
        self.w as i64 * self.h as i64
    }

    pub const fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.right() && py < self.bottom()
    }

    pub fn intersect(&self, o: &Rect) -> Option<Rect> {
        let l = self.x.max(o.x);
        let t = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        (r > l && b > t).then(|| Rect::new(l, t, r - l, b - t))
    }

    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        Rect::from_ltrb(
            self.x.min(o.x),
            self.y.min(o.y),
            self.right().max(o.right()),
            self.bottom().max(o.bottom()),
        )
    }

    pub const fn translate(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// Moves the rectangle by `dx, dy` but keeps it inside `bounds` (size is kept).
    pub fn nudge(&self, dx: i32, dy: i32, bounds: &Rect) -> Rect {
        let w = self.w.min(bounds.w);
        let h = self.h.min(bounds.h);
        let x = (self.x + dx).clamp(bounds.x, bounds.right() - w);
        let y = (self.y + dy).clamp(bounds.y, bounds.bottom() - h);
        Rect::new(x, y, w, h)
    }

    /// Grows or shrinks the right and bottom edges, never below 1 px, clamped to `bounds`.
    pub fn resize_by(&self, dw: i32, dh: i32, bounds: &Rect) -> Rect {
        let w = (self.w + dw).clamp(1, (bounds.right() - self.x).max(1));
        let h = (self.h + dh).clamp(1, (bounds.bottom() - self.y).max(1));
        Rect::new(self.x, self.y, w, h)
    }

    /// Clamps the rectangle to `bounds`, returning an empty rect if they do not overlap.
    pub fn clamp_to(&self, bounds: &Rect) -> Rect {
        self.intersect(bounds).unwrap_or_default()
    }

    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }
}

/// A grab point on a live selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
    Move,
}

/// Which handle (if any) of `r` is under the point, with `tol` px of slack around edges.
pub fn hit_handle(r: &Rect, px: i32, py: i32, tol: i32) -> Option<Handle> {
    if r.is_empty() {
        return None;
    }
    let near = |a: i32, b: i32| (a - b).abs() <= tol;
    let in_x = px >= r.x - tol && px <= r.right() + tol;
    let in_y = py >= r.y - tol && py <= r.bottom() + tol;
    if !(in_x && in_y) {
        return None;
    }
    let (l, rr, t, b) = (
        near(px, r.x),
        near(px, r.right()),
        near(py, r.y),
        near(py, r.bottom()),
    );
    Some(match (l, rr, t, b) {
        (true, _, true, _) => Handle::NW,
        (_, true, true, _) => Handle::NE,
        (true, _, _, true) => Handle::SW,
        (_, true, _, true) => Handle::SE,
        (true, _, _, _) => Handle::W,
        (_, true, _, _) => Handle::E,
        (_, _, true, _) => Handle::N,
        (_, _, _, true) => Handle::S,
        _ if r.contains(px, py) => Handle::Move,
        _ => return None,
    })
}

/// The rectangle after dragging handle `h` of `orig` by `dx, dy`, clamped to `bounds`.
/// Dragging an edge past the opposite edge flips the rectangle, as in most editors.
pub fn drag_handle(orig: &Rect, h: Handle, dx: i32, dy: i32, bounds: &Rect) -> Rect {
    if h == Handle::Move {
        return orig.nudge(dx, dy, bounds);
    }
    let (mut l, mut t, mut r, mut b) = (orig.x, orig.y, orig.right(), orig.bottom());
    match h {
        Handle::N => t += dy,
        Handle::S => b += dy,
        Handle::W => l += dx,
        Handle::E => r += dx,
        Handle::NW => {
            l += dx;
            t += dy
        }
        Handle::NE => {
            r += dx;
            t += dy
        }
        Handle::SW => {
            l += dx;
            b += dy
        }
        Handle::SE => {
            r += dx;
            b += dy
        }
        Handle::Move => unreachable!(),
    }
    Rect::from_ltrb(l, t, r, b).clamp_to(bounds)
}

/// One display, in physical pixels of the virtual desktop.
#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub rect: Rect,
    /// Work area (screen minus taskbar), for placing the preview.
    pub work: Rect,
    /// Effective DPI (96 = 100 % scale).
    pub dpi: u32,
    pub primary: bool,
    /// GDI device name, for example `\\.\DISPLAY1`.
    pub device: String,
}

impl Monitor {
    pub fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }
}

/// The bounding box of all monitors (can start at negative coordinates).
pub fn virtual_bounds(monitors: &[Monitor]) -> Rect {
    monitors
        .iter()
        .fold(Rect::default(), |acc, m| acc.union(&m.rect))
}

/// The monitor containing the point, or the nearest one.
pub fn monitor_at(monitors: &[Monitor], px: i32, py: i32) -> Option<&Monitor> {
    monitors
        .iter()
        .find(|m| m.rect.contains(px, py))
        .or_else(|| {
            monitors.iter().min_by_key(|m| {
                let (cx, cy) = m.rect.center();
                let (dx, dy) = ((cx - px) as i64, (cy - py) as i64);
                dx * dx + dy * dy
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: Rect = Rect::new(-1920, 0, 4480, 1440);

    fn mon(rect: Rect, dpi: u32, device: &str) -> Monitor {
        Monitor {
            rect,
            work: rect,
            dpi,
            primary: false,
            device: device.into(),
        }
    }

    #[test]
    fn from_points_normalizes() {
        assert_eq!(Rect::from_points(10, 20, 5, 2), Rect::new(5, 2, 5, 18));
    }

    #[test]
    fn intersect_and_union() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.intersect(&b), Some(Rect::new(5, 5, 5, 5)));
        assert_eq!(a.union(&b), Rect::new(0, 0, 15, 15));
        assert_eq!(a.intersect(&Rect::new(10, 0, 5, 5)), None);
    }

    #[test]
    fn nudge_stays_inside_negative_bounds() {
        let r = Rect::new(-1920, 10, 100, 100);
        assert_eq!(r.nudge(-5, 0, &B), Rect::new(-1920, 10, 100, 100));
        assert_eq!(r.nudge(1, -50, &B), Rect::new(-1919, 0, 100, 100));
        let far = Rect::new(2500, 1300, 60, 60);
        assert_eq!(far.nudge(100, 100, &B), Rect::new(2500, 1380, 60, 60));
    }

    #[test]
    fn resize_never_below_one() {
        let r = Rect::new(0, 0, 3, 3);
        assert_eq!(r.resize_by(-10, -10, &B), Rect::new(0, 0, 1, 1));
        assert_eq!(r.resize_by(10, 2, &B), Rect::new(0, 0, 13, 5));
    }

    #[test]
    fn handles_are_found() {
        let r = Rect::new(100, 100, 200, 100);
        assert_eq!(hit_handle(&r, 100, 100, 4), Some(Handle::NW));
        assert_eq!(hit_handle(&r, 300, 200, 4), Some(Handle::SE));
        assert_eq!(hit_handle(&r, 200, 98, 4), Some(Handle::N));
        assert_eq!(hit_handle(&r, 302, 150, 4), Some(Handle::E));
        assert_eq!(hit_handle(&r, 150, 150, 4), Some(Handle::Move));
        assert_eq!(hit_handle(&r, 50, 50, 4), None);
    }

    #[test]
    fn drag_edge_flips_past_opposite() {
        let r = Rect::new(100, 100, 50, 50);
        assert_eq!(
            drag_handle(&r, Handle::E, -80, 0, &B),
            Rect::new(70, 100, 30, 50)
        );
        assert_eq!(
            drag_handle(&r, Handle::SE, 10, 20, &B),
            Rect::new(100, 100, 60, 70)
        );
        assert_eq!(
            drag_handle(&r, Handle::Move, 5, 5, &B),
            Rect::new(105, 105, 50, 50)
        );
    }

    #[test]
    fn virtual_bounds_and_lookup() {
        let mons = vec![
            mon(Rect::new(0, 0, 2560, 1440), 144, "A"),
            mon(Rect::new(-1920, 200, 1920, 1080), 96, "B"),
        ];
        assert_eq!(virtual_bounds(&mons), Rect::new(-1920, 0, 4480, 1440));
        assert_eq!(monitor_at(&mons, -10, 500).unwrap().device, "B");
        assert_eq!(monitor_at(&mons, -10, 50).unwrap().device, "B"); // gap: nearest
        assert_eq!(monitor_at(&mons, 10, 50).unwrap().device, "A");
    }
}
