//! Editor state: the shape list, the current tool, selection, dragging, crop and undo.
//! Positions here are in document pixels (the full image, before any crop).

use crate::model::*;
use std::collections::HashMap;

pub const PALETTE: [[u8; 4]; 6] = [
    [255, 59, 48, 255],   // red
    [255, 214, 10, 255],  // yellow
    [48, 209, 88, 255],   // green
    [10, 132, 255, 255],  // blue
    [255, 255, 255, 255], // white
    [28, 28, 30, 255],    // black
];

/// Stroke widths in screen (logical) pixels; multiplied by the display scale.
pub const SIZES: [f32; 7] = [2.0, 3.0, 5.0, 8.0, 12.0, 18.0, 26.0];

#[derive(Clone, PartialEq)]
struct Snapshot {
    shapes: Vec<Shape>,
    crop: Option<Bx>,
}

#[derive(Clone, Copy, Debug)]
enum Drag {
    None,
    Create { idx: usize, start: Pt },
    Move { last: Pt },
    Handle { idx: usize, h: usize },
    Crop { start: Pt },
}

/// Text being typed in the on-canvas text box.
#[derive(Clone, Debug, PartialEq)]
pub struct TextEdit {
    /// The shape being edited, or None for a new one.
    pub idx: Option<usize>,
    pub at: Pt,
    pub text: String,
    pub style: Style,
}

#[derive(Clone, Copy, Default)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

pub struct Editor {
    pub width: u32,
    pub height: u32,
    pub shapes: Vec<Shape>,
    pub crop: Option<Bx>,
    pub crop_pending: Option<Bx>,
    pub tool: Tool,
    pub sel: Option<usize>,
    pub color: usize,
    pub size: usize,
    /// Physical pixels per logical pixel of the editor window (stroke widths scale with it).
    pub scale: f32,
    pub text_edit: Option<TextEdit>,
    /// Changed since the last copy or save.
    pub dirty: bool,
    styles: HashMap<Tool, (usize, usize)>,
    drag: Drag,
    before: Option<Snapshot>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

impl Editor {
    pub fn new(width: u32, height: u32, scale: f32) -> Editor {
        let mut styles = HashMap::new();
        styles.insert(Tool::Arrow, (0, 2));
        styles.insert(Tool::Rect, (0, 1));
        styles.insert(Tool::Ellipse, (0, 1));
        styles.insert(Tool::Highlight, (1, 3));
        styles.insert(Tool::Pen, (0, 1));
        styles.insert(Tool::Text, (0, 2));
        styles.insert(Tool::Step, (0, 2));
        Editor {
            width,
            height,
            shapes: Vec::new(),
            crop: None,
            crop_pending: None,
            tool: Tool::Arrow,
            sel: None,
            color: 0,
            size: 2,
            scale: scale.max(0.5),
            text_edit: None,
            dirty: false,
            styles,
            drag: Drag::None,
            before: None,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    // ------------------------------------------------------------ geometry

    /// The visible area in document pixels (the crop, or the whole image).
    pub fn view(&self) -> Bx {
        self.crop.unwrap_or(Bx {
            x0: 0.0,
            y0: 0.0,
            x1: self.width as f32,
            y1: self.height as f32,
        })
    }

    pub fn view_size(&self) -> (u32, u32) {
        let v = self.view();
        (v.w().round().max(1.0) as u32, v.h().round().max(1.0) as u32)
    }

    /// Display pixels (top-left of the visible area = 0,0) to document pixels.
    pub fn to_doc(&self, x: f32, y: f32) -> Pt {
        let v = self.view();
        pt(x + v.x0, y + v.y0)
    }

    pub fn style(&self) -> Style {
        Style {
            color: PALETTE[self.color],
            width: SIZES[self.size] * self.scale,
        }
    }

    /// Grab tolerance in document pixels.
    fn tol(&self) -> f32 {
        6.0 * self.scale
    }

    // ------------------------------------------------------------ undo

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            shapes: self.shapes.clone(),
            crop: self.crop,
        }
    }

    fn restore(&mut self, s: Snapshot) {
        self.shapes = s.shapes;
        self.crop = s.crop;
        self.sel = None;
        self.dirty = true;
    }

    /// Call before a change; the change becomes one undo step.
    fn checkpoint(&mut self) {
        let s = self.snapshot();
        self.undo.push(s);
        self.redo.clear();
        self.dirty = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        self.commit_pending_crop_cancel();
        if let Some(s) = self.undo.pop() {
            let now = self.snapshot();
            self.redo.push(now);
            self.restore(s);
        }
    }

    pub fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            let now = self.snapshot();
            self.undo.push(now);
            self.restore(s);
        }
    }

    fn commit_pending_crop_cancel(&mut self) {
        self.crop_pending = None;
    }

    // ------------------------------------------------------------ tools and styles

    pub fn set_tool(&mut self, t: Tool) {
        if self.tool == Tool::Crop && t != Tool::Crop {
            self.apply_crop();
        }
        self.tool = t;
        if let Some(&(c, s)) = self.styles.get(&t) {
            self.color = c;
            self.size = s;
        }
        if t != Tool::Select {
            self.sel = None;
        }
    }

    /// Picks a colour; also recolours the selected shape.
    pub fn set_color(&mut self, i: usize) {
        self.color = i.min(PALETTE.len() - 1);
        self.remember_style();
        if let Some(idx) = self.sel {
            if self.shapes[idx].has_color() && self.shapes[idx].style.color != PALETTE[self.color] {
                self.checkpoint();
                self.shapes[idx].style.color = PALETTE[self.color];
            }
        }
    }

    /// Changes the size by `delta` steps; also resizes the selected shape.
    pub fn bump_size(&mut self, delta: i32) {
        self.size = (self.size as i32 + delta).clamp(0, SIZES.len() as i32 - 1) as usize;
        self.remember_style();
        if let Some(idx) = self.sel {
            let w = SIZES[self.size] * self.scale;
            if self.shapes[idx].style.width != w {
                self.checkpoint();
                self.shapes[idx].style.width = w;
            }
        }
    }

    fn remember_style(&mut self) {
        let tool = match self.sel.map(|i| &self.shapes[i].kind) {
            Some(Kind::Arrow { .. }) => Tool::Arrow,
            Some(Kind::Rect { .. }) => Tool::Rect,
            Some(Kind::Ellipse { .. }) => Tool::Ellipse,
            Some(Kind::Highlight { .. }) => Tool::Highlight,
            Some(Kind::Pen { .. }) => Tool::Pen,
            Some(Kind::Text { .. }) => Tool::Text,
            Some(Kind::Step { .. }) => Tool::Step,
            _ => self.tool,
        };
        self.styles.insert(tool, (self.color, self.size));
    }

    pub fn delete_selected(&mut self) -> bool {
        let Some(i) = self.sel else { return false };
        self.checkpoint();
        self.shapes.remove(i);
        self.sel = None;
        self.renumber_steps();
        true
    }

    /// Moves shape `i` by a few pixels (arrow keys).
    pub fn nudge(&mut self, i: usize, dx: f32, dy: f32) {
        if i < self.shapes.len() {
            self.checkpoint();
            self.shapes[i].translate(dx * self.scale, dy * self.scale);
        }
    }

    fn renumber_steps(&mut self) {
        let mut n = 0;
        for s in &mut self.shapes {
            if let Kind::Step { n: k, .. } = &mut s.kind {
                n += 1;
                *k = n;
            }
        }
    }

    fn next_step(&self) -> u32 {
        self.shapes
            .iter()
            .filter(|s| matches!(s.kind, Kind::Step { .. }))
            .count() as u32
            + 1
    }

    // ------------------------------------------------------------ hit-testing

    fn shape_at(&self, p: Pt) -> Option<usize> {
        let tol = self.tol();
        (0..self.shapes.len())
            .rev()
            .find(|&i| self.shapes[i].hit(p, tol))
    }

    fn handle_at(&self, p: Pt) -> Option<(usize, usize)> {
        let idx = self.sel?;
        let tol = self.tol() * 1.5;
        self.shapes[idx]
            .handles()
            .iter()
            .position(|h| h.dist(p) <= tol)
            .map(|h| (idx, h))
    }

    /// The mouse cursor to show over `p` (display pixels).
    pub fn cursor(&self, x: f32, y: f32) -> &'static str {
        let p = self.to_doc(x, y);
        if let Some((idx, h)) = self.handle_at(p) {
            return match (&self.shapes[idx].kind, h) {
                (Kind::Arrow { .. }, _) => "move",
                (_, 0 | 4) => "nwse",
                (_, 2 | 6) => "nesw",
                (_, 1 | 5) => "ns",
                _ => "ew",
            };
        }
        if let Some(i) = self.shape_at(p) {
            if self.tool == Tool::Select || Some(i) == self.sel {
                return "move";
            }
        }
        match self.tool {
            Tool::Select => "default",
            Tool::Text => "text",
            _ => "cross",
        }
    }

    // ------------------------------------------------------------ pointer input (display pixels)

    pub fn pointer_down(&mut self, x: f32, y: f32, m: Mods) {
        let p = self.to_doc(x, y);
        self.before = Some(self.snapshot());
        if self.tool == Tool::Crop {
            self.drag = Drag::Crop { start: p };
            self.crop_pending = Some(Bx::new(p, p));
            return;
        }
        if let Some((idx, h)) = self.handle_at(p) {
            self.drag = Drag::Handle { idx, h };
            return;
        }
        let hit = self.shape_at(p);
        if let Some(i) = hit {
            if self.tool == Tool::Select || Some(i) == self.sel {
                self.sel = Some(i);
                self.drag = Drag::Move { last: p };
                return;
            }
        }
        let style = self.style();
        let kind = match self.tool {
            Tool::Select => {
                self.sel = None;
                return;
            }
            Tool::Text => {
                self.begin_text(p, hit);
                return;
            }
            Tool::Step => {
                self.checkpoint();
                self.shapes.push(Shape {
                    kind: Kind::Step {
                        at: p,
                        n: self.next_step(),
                    },
                    style,
                });
                self.sel = Some(self.shapes.len() - 1);
                // Steps can be dragged into place right away.
                self.drag = Drag::Move { last: p };
                self.before = None;
                return;
            }
            Tool::Arrow => Kind::Arrow {
                a: p,
                b: p,
                bend: None,
            },
            Tool::Rect => Kind::Rect { a: p, b: p },
            Tool::Ellipse => Kind::Ellipse { a: p, b: p },
            Tool::Highlight => Kind::Highlight { pts: vec![p] },
            Tool::Pen => Kind::Pen { pts: vec![p] },
            Tool::Blur => Kind::Blur { a: p, b: p },
            Tool::Pixelate => Kind::Pixelate { a: p, b: p },
            Tool::Crop => unreachable!(),
        };
        let _ = m;
        self.shapes.push(Shape { kind, style });
        let idx = self.shapes.len() - 1;
        self.sel = Some(idx);
        self.drag = Drag::Create { idx, start: p };
    }

    pub fn pointer_move(&mut self, x: f32, y: f32, m: Mods) {
        let p = self.to_doc(x, y);
        match self.drag {
            Drag::None => {}
            Drag::Crop { start } => {
                let q = if m.shift { square(start, p) } else { p };
                let v = self.view();
                self.crop_pending = Bx::new(start, q).intersect(&v);
            }
            Drag::Move { last } => {
                if let Some(i) = self.sel {
                    self.shapes[i].translate(p.x - last.x, p.y - last.y);
                }
                self.drag = Drag::Move { last: p };
            }
            Drag::Handle { idx, h } => {
                let q = match (&self.shapes[idx].kind, h, m.shift) {
                    (Kind::Arrow { b, .. }, 0, true) => snap_angle(*b, p),
                    (Kind::Arrow { a, .. }, 1, true) => snap_angle(*a, p),
                    _ => p,
                };
                self.shapes[idx].move_handle(h, q);
            }
            Drag::Create { idx, start } => {
                let shape = &mut self.shapes[idx];
                match &mut shape.kind {
                    Kind::Arrow { b, .. } => *b = if m.shift { snap_angle(start, p) } else { p },
                    Kind::Rect { a, b }
                    | Kind::Ellipse { a, b }
                    | Kind::Blur { a, b }
                    | Kind::Pixelate { a, b } => {
                        let q = if m.shift { square(start, p) } else { p };
                        if m.alt {
                            // Draw from the centre.
                            *a = pt(2.0 * start.x - q.x, 2.0 * start.y - q.y);
                        } else {
                            *a = start;
                        }
                        *b = q;
                    }
                    Kind::Highlight { pts } => {
                        if m.shift {
                            *pts = vec![start, pt(p.x, start.y)];
                        } else if pts.last().is_none_or(|l| l.dist(p) >= 1.5) {
                            pts.push(p);
                        }
                    }
                    Kind::Pen { pts } => {
                        if m.shift {
                            *pts = vec![start, snap_angle(start, p)];
                        } else if pts.last().is_none_or(|l| l.dist(p) >= 1.5) {
                            pts.push(p);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    pub fn pointer_up(&mut self, x: f32, y: f32, m: Mods) {
        self.pointer_move(x, y, m);
        let drag = std::mem::replace(&mut self.drag, Drag::None);
        if let Drag::Create { idx, .. } = drag {
            if self.shapes[idx].is_degenerate() {
                self.shapes.remove(idx);
                self.sel = None;
            }
        }
        if let Drag::Crop { .. } = drag {
            if self
                .crop_pending
                .is_some_and(|c| c.w() < 3.0 || c.h() < 3.0)
            {
                self.crop_pending = None;
            }
            return;
        }
        // One undo step for the whole drag, if anything changed.
        if let Some(before) = self.before.take() {
            if before.shapes != self.shapes || before.crop != self.crop {
                self.undo.push(before);
                self.redo.clear();
                self.dirty = true;
            }
        }
    }

    pub fn double_click(&mut self, x: f32, y: f32) {
        let p = self.to_doc(x, y);
        if let Some(i) = self.shape_at(p) {
            if matches!(self.shapes[i].kind, Kind::Text { .. }) {
                self.begin_text(p, Some(i));
            }
        }
    }

    // ------------------------------------------------------------ text

    fn begin_text(&mut self, p: Pt, hit: Option<usize>) {
        if let Some(i) = hit {
            if let Kind::Text { at, text } = &self.shapes[i].kind {
                self.text_edit = Some(TextEdit {
                    idx: Some(i),
                    at: *at,
                    text: text.clone(),
                    style: self.shapes[i].style,
                });
                self.sel = None;
                return;
            }
        }
        let style = self.style();
        // Put the click point at the middle of the first line.
        let at = pt(p.x, p.y - text_size(style.width) * 0.65);
        self.text_edit = Some(TextEdit {
            idx: None,
            at,
            text: String::new(),
            style,
        });
        self.sel = None;
    }

    pub fn commit_text(&mut self, text: &str) {
        let Some(edit) = self.text_edit.take() else {
            return;
        };
        let text = text.trim_end().to_string();
        match edit.idx {
            Some(i) => {
                let old = match &self.shapes[i].kind {
                    Kind::Text { text, .. } => text.clone(),
                    _ => return,
                };
                if old == text {
                    return;
                }
                self.checkpoint();
                if text.trim().is_empty() {
                    self.shapes.remove(i);
                } else if let Kind::Text { text: t, .. } = &mut self.shapes[i].kind {
                    *t = text;
                }
            }
            None => {
                if text.trim().is_empty() {
                    return;
                }
                self.checkpoint();
                self.shapes.push(Shape {
                    kind: Kind::Text { at: edit.at, text },
                    style: edit.style,
                });
                self.sel = Some(self.shapes.len() - 1);
            }
        }
    }

    pub fn cancel_text(&mut self) {
        self.text_edit = None;
    }

    /// Shapes to draw: the one being edited as text is hidden (the text box shows it).
    pub fn hidden(&self) -> Option<usize> {
        self.text_edit.as_ref().and_then(|t| t.idx)
    }

    // ------------------------------------------------------------ crop

    pub fn apply_crop(&mut self) -> bool {
        let Some(c) = self.crop_pending.take() else {
            return false;
        };
        let c = Bx {
            x0: c.x0.round(),
            y0: c.y0.round(),
            x1: c.x1.round(),
            y1: c.y1.round(),
        };
        if c.w() < 1.0 || c.h() < 1.0 {
            return false;
        }
        self.checkpoint();
        self.crop = Some(c);
        true
    }

    pub fn cancel_crop(&mut self) -> bool {
        self.crop_pending.take().is_some()
    }

    pub fn is_dragging(&self) -> bool {
        !matches!(self.drag, Drag::None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO: Mods = Mods {
        shift: false,
        ctrl: false,
        alt: false,
    };
    const SHIFT: Mods = Mods {
        shift: true,
        ctrl: false,
        alt: false,
    };

    fn ed() -> Editor {
        Editor::new(800, 600, 1.0)
    }

    fn drag(e: &mut Editor, a: (f32, f32), b: (f32, f32), m: Mods) {
        e.pointer_down(a.0, a.1, m);
        e.pointer_move((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0, m);
        e.pointer_up(b.0, b.1, m);
    }

    #[test]
    fn draw_arrow_then_undo_redo() {
        let mut e = ed();
        drag(&mut e, (10.0, 10.0), (200.0, 100.0), NO);
        assert_eq!(e.shapes.len(), 1);
        assert!(matches!(e.shapes[0].kind, Kind::Arrow { b, .. } if b == pt(200.0, 100.0)));
        assert!(e.dirty && e.can_undo());
        e.undo();
        assert!(e.shapes.is_empty());
        e.redo();
        assert_eq!(e.shapes.len(), 1);
    }

    #[test]
    fn click_without_drag_adds_nothing() {
        let mut e = ed();
        e.pointer_down(50.0, 50.0, NO);
        e.pointer_up(50.0, 50.0, NO);
        assert!(e.shapes.is_empty());
        assert!(!e.can_undo());
    }

    #[test]
    fn shift_makes_circles() {
        let mut e = ed();
        e.set_tool(Tool::Ellipse);
        drag(&mut e, (0.0, 0.0), (100.0, 40.0), SHIFT);
        assert!(
            matches!(e.shapes[0].kind, Kind::Ellipse { a, b } if a == pt(0.0, 0.0) && b == pt(100.0, 100.0))
        );
    }

    #[test]
    fn selected_shape_moves_with_any_tool() {
        let mut e = ed();
        e.set_tool(Tool::Rect);
        drag(&mut e, (100.0, 100.0), (200.0, 200.0), NO);
        assert_eq!(e.sel, Some(0));
        // Grab the border of the selected rectangle and move it.
        drag(&mut e, (100.0, 130.0), (110.0, 140.0), NO);
        assert_eq!(e.shapes.len(), 1);
        assert!(matches!(e.shapes[0].kind, Kind::Rect { a, .. } if a == pt(110.0, 110.0)));
        assert_eq!(e.undo.len(), 2);
    }

    #[test]
    fn handle_resizes() {
        let mut e = ed();
        e.set_tool(Tool::Rect);
        drag(&mut e, (100.0, 100.0), (200.0, 200.0), NO);
        drag(&mut e, (200.0, 200.0), (300.0, 250.0), NO);
        assert!(matches!(e.shapes[0].kind, Kind::Rect { b, .. } if b == pt(300.0, 250.0)));
    }

    #[test]
    fn steps_count_and_renumber() {
        let mut e = ed();
        e.set_tool(Tool::Step);
        for x in [100.0, 200.0, 300.0] {
            e.pointer_down(x, 100.0, NO);
            e.pointer_up(x, 100.0, NO);
        }
        let nums = |e: &Editor| {
            e.shapes
                .iter()
                .map(|s| {
                    if let Kind::Step { n, .. } = s.kind {
                        n
                    } else {
                        0
                    }
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(nums(&e), vec![1, 2, 3]);
        e.sel = Some(0);
        e.delete_selected();
        assert_eq!(nums(&e), vec![1, 2]);
        e.undo();
        assert_eq!(nums(&e), vec![1, 2, 3]);
    }

    #[test]
    fn text_commit_and_edit() {
        let mut e = ed();
        e.set_tool(Tool::Text);
        e.pointer_down(50.0, 50.0, NO);
        e.pointer_up(50.0, 50.0, NO);
        assert!(e.text_edit.is_some());
        e.commit_text("Hello");
        assert!(matches!(&e.shapes[0].kind, Kind::Text { text, .. } if text == "Hello"));
        let at = match &e.shapes[0].kind {
            Kind::Text { at, .. } => *at,
            _ => unreachable!(),
        };
        e.double_click(at.x + 5.0, at.y + 5.0);
        assert_eq!(e.hidden(), Some(0));
        e.commit_text("");
        assert!(e.shapes.is_empty(), "clearing the text deletes it");
    }

    #[test]
    fn crop_maps_coordinates() {
        let mut e = ed();
        e.set_tool(Tool::Crop);
        drag(&mut e, (100.0, 50.0), (300.0, 250.0), NO);
        assert!(e.apply_crop());
        assert_eq!(e.view_size(), (200, 200));
        assert_eq!(e.to_doc(0.0, 0.0), pt(100.0, 50.0));
        e.set_tool(Tool::Arrow);
        drag(&mut e, (0.0, 0.0), (10.0, 10.0), NO);
        assert!(matches!(e.shapes[0].kind, Kind::Arrow { a, .. } if a == pt(100.0, 50.0)));
        e.undo();
        e.undo();
        assert_eq!(e.view_size(), (800, 600));
    }

    #[test]
    fn styles_are_remembered_per_tool() {
        let mut e = ed();
        e.set_tool(Tool::Arrow);
        e.set_color(3);
        e.set_tool(Tool::Highlight);
        assert_eq!(e.color, 1);
        e.set_tool(Tool::Arrow);
        assert_eq!(e.color, 3);
    }

    #[test]
    fn recolor_selected_is_undoable() {
        let mut e = ed();
        drag(&mut e, (10.0, 10.0), (200.0, 100.0), NO);
        e.set_color(2);
        assert_eq!(e.shapes[0].style.color, PALETTE[2]);
        e.undo();
        assert_eq!(e.shapes[0].style.color, PALETTE[0]);
    }
}
