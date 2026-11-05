//! What changed on screen between two renders of the same page: the box trees
//! are compared node by node, and only the rectangles that differ get repainted.

use crate::layout::{BoxContent, LayoutBox};

/// A pixel rectangle `[x0, y0, x1, y1)`.
pub type Rect = [i32; 4];

/// The rectangles to repaint to turn `old`'s picture into `new`'s.
pub fn damage(old: &LayoutBox, new: &LayoutBox) -> Vec<Rect> {
    let mut out = Vec::new();
    walk(old, new, &mut out);
    merge(out)
}

fn walk(o: &LayoutBox, n: &LayoutBox, out: &mut Vec<Rect>) {
    if o.children.len() != n.children.len() || std::mem::discriminant(&o.content) != std::mem::discriminant(&n.content) {
        // Different structure (innerHTML, a shown/hidden child): repaint both subtrees.
        out.push(union(subtree(o), subtree(n)));
        return;
    }
    if !same_look(o, n) {
        out.push(union(own(o), own(n)));
    }
    for (a, b) in o.children.iter().zip(&n.children) {
        walk(a, b, out);
    }
}

/// Everything about the box itself that painting reads (not its children).
fn same_look(a: &LayoutBox, b: &LayoutBox) -> bool {
    let content = match (&a.content, &b.content) {
        (BoxContent::Text(x), BoxContent::Text(y)) => x == y,
        (BoxContent::Image { alt: x, data: dx, intrinsic_w: wx, intrinsic_h: hx },
         BoxContent::Image { alt: y, data: dy, intrinsic_w: wy, intrinsic_h: hy }) => x == y && wx == wy && hx == hy && dx == dy,
        _ => true,
    };
    content
        && a.x == b.x && a.y == b.y && a.width == b.width && a.height == b.height
        && a.margin == b.margin && a.padding == b.padding && a.border == b.border
        && a.background_color == b.background_color && a.text_color == b.text_color
        && a.border_color == b.border_color && a.border_side_colors == b.border_side_colors
        && a.font_size == b.font_size && a.text_align == b.text_align && a.opacity == b.opacity
        && a.line_height == b.line_height && a.white_space_nowrap == b.white_space_nowrap
        && a.border_radius == b.border_radius && a.text_decoration == b.text_decoration
        && a.bold == b.bold && a.overflow_hidden == b.overflow_hidden
}

/// The area a box paints itself: its border box, and for text a margin for
/// glyphs that reach past the line box.
fn own(b: &LayoutBox) -> Rect {
    let slack = match b.content {
        BoxContent::Text(_) | BoxContent::Image { .. } => (b.font_size * 0.5).ceil() + 1.0,
        _ => 1.0,
    };
    let x = b.x + b.margin.left;
    let y = b.y + b.margin.top;
    let w = b.border.left + b.padding.left + b.width + b.padding.right + b.border.right;
    let h = b.border.top + b.padding.top + b.height + b.padding.bottom + b.border.bottom;
    [(x - slack).floor() as i32, (y - slack).floor() as i32, (x + w + slack).ceil() as i32, (y + h + slack).ceil() as i32]
}

/// The area a box and everything inside it may paint.
fn subtree(b: &LayoutBox) -> Rect {
    b.children.iter().map(subtree).fold(own(b), union)
}

fn union(a: Rect, b: Rect) -> Rect {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

/// Joins overlapping rectangles until none overlap.
fn merge(mut rects: Vec<Rect>) -> Vec<Rect> {
    loop {
        let mut joined = false;
        let mut out: Vec<Rect> = Vec::with_capacity(rects.len());
        for r in rects {
            if let Some(o) = out.iter_mut().find(|o| overlaps(**o, r)) {
                *o = union(*o, r);
                joined = true;
            } else {
                out.push(r);
            }
        }
        rects = out;
        if !joined {
            return rects;
        }
    }
}

/// Total area of the rectangles (they do not overlap after `damage`).
pub fn area(rects: &[Rect]) -> i64 {
    rects.iter().map(|r| ((r[2] - r[0]).max(0) as i64) * ((r[3] - r[1]).max(0) as i64)).sum()
}
