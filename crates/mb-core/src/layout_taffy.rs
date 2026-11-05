//! Block, flex and grid layout by Taffy (the layout engine Servo's and Blitz's
//! use), producing the same [`LayoutBox`] tree the painter reads.
//!
//! Taffy has no inline layout, so boxes that hold inline content (text, inline
//! elements, images, or a block whose children are all of those) are Taffy
//! leaves: they are measured, and finally laid out inside the rectangle Taffy
//! gave them, by the original layout code in [`crate::layout`]. A run of
//! inline-level children inside a block that also has block children becomes
//! one anonymous leaf, as CSS wraps it in an anonymous block box.

use std::collections::HashMap;

use taffy::prelude::*;
use taffy::AvailableSpace;

use crate::layout::{self, BoxContent, LayoutBox, StyledNode};

/// Lays out the styled tree for a viewport `viewport_width` wide; the height
/// grows with the content.
pub fn layout_tree(styled_root: &StyledNode, viewport_width: f32, viewport_height: f32) -> LayoutBox {
    let mut root = layout::build_layout_box(styled_root);
    let mut b = Builder { tree: TaffyTree::new(), leaves: Vec::new(), runs: HashMap::new() };
    let root_id = b.build(styled_root, &root, true);
    let Builder { mut tree, leaves, runs } = b;

    let available = Size { width: AvailableSpace::Definite(viewport_width), height: AvailableSpace::MaxContent };
    // Taffy measures a leaf many times (each flex/grid pass); a leaf's text lays
    // out the same at the same width, so each width runs once.
    let mut memo: Vec<Vec<(u32, (f32, f32))>> = vec![Vec::new(); leaves.len()];
    let measured = tree.compute_layout_with_measure(root_id, available, |inputs, _id, ctx, style| {
        taffy::compute_leaf_layout(inputs, style, |_, _| 0.0, |known, avail| match ctx {
            Some(i) => measure_leaf(&leaves[*i], &mut memo[*i], known, avail),
            None => Size::ZERO,
        })
    });
    if let Err(e) = measured {
        log::warn!("taffy: {e:?}; falling back to the built-in layout");
        return layout::layout_tree(styled_root, viewport_width, viewport_height);
    }
    place(&tree, &leaves, &runs, root_id, &mut root, 0.0, 0.0);
    root
}

struct Builder {
    tree: TaffyTree<usize>,
    /// Box templates of the leaves, indexed by the Taffy node context.
    leaves: Vec<LayoutBox>,
    /// Anonymous run leaves: which children of their parent box they hold.
    runs: HashMap<NodeId, Vec<usize>>,
}

/// True for boxes whose content is inline: they become Taffy leaves.
fn is_leaf(node: &StyledNode) -> bool {
    match (&node.content, node.display) {
        (layout::NodeContent::Text(_), _) | (layout::NodeContent::Image { .. }, _) => true,
        (_, layout::Display::Inline) => node.children.iter().all(is_inline_level),
        (_, layout::Display::Block) => !node.children.is_empty() && node.children.iter().all(is_inline_level),
        _ => false,
    }
}

/// Text, images, hidden boxes, and inline elements with only such content.
fn is_inline_level(node: &StyledNode) -> bool {
    node.display == layout::Display::None
        || matches!(node.content, layout::NodeContent::Text(_) | layout::NodeContent::Image { .. })
        || (node.display == layout::Display::Inline && node.children.iter().all(is_inline_level))
}

impl Builder {
    /// Mirrors the styled tree into Taffy nodes; leaf boxes are kept in `leaves`
    /// (their index is the Taffy node context).
    fn build(&mut self, node: &StyledNode, boxed: &LayoutBox, root: bool) -> NodeId {
        let style = style_of(node, root);
        if node.display != layout::Display::None && is_leaf(node) {
            self.leaves.push(boxed.clone());
            return self.tree.new_leaf_with_context(style, self.leaves.len() - 1).expect("taffy leaf");
        }
        if node.display == layout::Display::None {
            return self.tree.new_leaf(style).expect("taffy node");
        }
        // build_layout_box drops hidden children; skip them too to stay in step.
        let visible: Vec<&StyledNode> = node.children.iter().filter(|c| c.display != layout::Display::None).collect();
        let block_flow = matches!(node.display, layout::Display::Block | layout::Display::Inline);
        let mut children = Vec::new();
        let mut i = 0;
        while i < visible.len() {
            if block_flow && is_inline_level(visible[i]) {
                let start = i;
                while i < visible.len() && is_inline_level(visible[i]) {
                    i += 1;
                }
                children.push(self.run(boxed, (start..i).collect()));
            } else {
                children.push(self.build(visible[i], &boxed.children[i], false));
                i += 1;
            }
        }
        self.tree.new_with_children(style, &children).expect("taffy node")
    }

    /// An anonymous block leaf holding `members` (indices into `parent.children`).
    fn run(&mut self, parent: &LayoutBox, members: Vec<usize>) -> NodeId {
        let mut w = parent.clone();
        w.children = members.iter().map(|&i| parent.children[i].clone()).collect();
        w.content = BoxContent::Block;
        w.margin = layout::EdgeSizes::ZERO;
        w.padding = layout::EdgeSizes::ZERO;
        w.border = layout::EdgeSizes::ZERO;
        w.background_color = None;
        w.border_color = None;
        w.border_radius = 0.0;
        w.explicit_width = layout::Dimension::Auto;
        w.explicit_height = layout::Dimension::Auto;
        w.min_height = layout::Dimension::Auto;
        w.position = layout::Position::Static;
        w.opacity = 1.0;
        self.leaves.push(w);
        let id = self.tree.new_leaf_with_context(Style::default(), self.leaves.len() - 1).expect("taffy run");
        self.runs.insert(id, members);
        id
    }
}

/// Content size of a leaf for Taffy: the built-in layout runs on a copy of it
/// whose own margin, border and padding are zeroed (Taffy adds those itself).
fn measure_leaf(template: &LayoutBox, memo: &mut Vec<(u32, (f32, f32))>, known: Size<Option<f32>>, avail: Size<AvailableSpace>) -> Size<f32> {
    if let Size { width: Some(w), height: Some(h) } = known {
        return Size { width: w, height: h };
    }
    let mut run = |width: f32| -> (f32, f32) {
        if let Some((_, r)) = memo.iter().find(|(w, _)| *w == width.to_bits()) {
            return *r;
        }
        let mut b = template.clone();
        b.margin = layout::EdgeSizes::ZERO;
        b.padding = layout::EdgeSizes::ZERO;
        b.border = layout::EdgeSizes::ZERO;
        b.explicit_width = layout::Dimension::Auto;
        b.explicit_height = layout::Dimension::Auto;
        b.position = layout::Position::Static;
        left_align(&mut b);
        b.layout(0.0, 0.0, width, 100_000.0);
        let r = (extent(&b) - b.content_x(), b.height);
        memo.push((width.to_bits(), r));
        r
    };
    let width = match known.width {
        Some(w) => w,
        None => {
            // Whole pixels: Taffy rounds the box, and a box 0.3px short of its
            // content would wrap the last word.
            let max_content = run(1.0e6).0.ceil();
            match avail.width {
                AvailableSpace::Definite(w) => max_content.min(w),
                AvailableSpace::MinContent => run(0.0).0.ceil(),
                AvailableSpace::MaxContent => max_content,
            }
        }
    };
    let height = match known.height {
        Some(h) => h,
        None => run(width.ceil()).1,
    };
    Size { width, height }
}

/// Measures with left alignment: centred text at a huge width would read as wide.
fn left_align(b: &mut LayoutBox) {
    b.text_align = layout::TextAlign::Left;
    for c in &mut b.children {
        left_align(c);
    }
}

/// The rightmost edge of everything inside `b` (absolute x).
fn extent(b: &LayoutBox) -> f32 {
    let own = match b.content {
        // An inline box's padding and border count too (a button's right padding).
        BoxContent::Text(_) | BoxContent::Image { .. } | BoxContent::Inline => b.x + b.margin_box_width(),
        _ => b.content_x(),
    };
    b.children.iter().map(extent).fold(own, f32::max)
}

/// Copies Taffy's geometry into the box tree. `(px, py)` is the parent's
/// border-box origin in page coordinates.
fn place(tree: &TaffyTree<usize>, leaves: &[LayoutBox], runs: &HashMap<NodeId, Vec<usize>>, id: NodeId, b: &mut LayoutBox, px: f32, py: f32) {
    let l = tree.layout(id).expect("taffy layout");
    let (bx, by) = (px + l.location.x, py + l.location.y);
    let edges = |r: taffy::Rect<f32>| layout::EdgeSizes { top: r.top, right: r.right, bottom: r.bottom, left: r.left };
    b.margin = edges(l.margin);
    b.border = edges(l.border);
    b.padding = edges(l.padding);
    let content_w = (l.size.width - l.padding.left - l.padding.right - l.border.left - l.border.right).max(0.0);
    let content_h = (l.size.height - l.padding.top - l.padding.bottom - l.border.top - l.border.bottom).max(0.0);
    let (x, y) = (bx - l.margin.left, by - l.margin.top);

    if tree.get_node_context(id).is_some() {
        // A leaf: the built-in layout fills in its inline content at this spot.
        b.position = layout::Position::Static;
        b.offset_top = 0.0;
        b.offset_left = 0.0;
        b.explicit_width = layout::Dimension::Px(content_w);
        b.layout(x, y, l.size.width + l.margin.left + l.margin.right, 100_000.0);
        b.x = x;
        b.y = y;
        b.width = content_w;
        b.height = content_h;
        return;
    }
    b.x = x;
    b.y = y;
    b.width = content_w;
    b.height = content_h;
    let kids = tree.children(id).expect("taffy children");
    let mut next = 0;
    for cid in kids {
        if let Some(members) = runs.get(&cid) {
            // Lay the run out as its anonymous block, then hand the boxes back.
            let ctx = *tree.get_node_context(cid).expect("run context");
            let mut w = leaves[ctx].clone();
            place(tree, leaves, runs, cid, &mut w, bx, by);
            for (&i, laid) in members.iter().zip(w.children) {
                b.children[i] = laid;
            }
            next = members.last().map_or(next, |&i| i + 1);
        } else if let Some(child) = b.children.get_mut(next) {
            place(tree, leaves, runs, cid, child, bx, by);
            next += 1;
        }
    }
}

fn dim(d: layout::Dimension) -> Dimension {
    match d {
        layout::Dimension::Px(v) => length(v),
        layout::Dimension::Percent(p) => percent(p / 100.0),
        layout::Dimension::Auto => auto(),
    }
}

fn lpa(d: layout::Dimension) -> LengthPercentageAuto {
    match d {
        layout::Dimension::Px(v) => length(v),
        layout::Dimension::Percent(p) => percent(p / 100.0),
        layout::Dimension::Auto => auto(),
    }
}

fn align(a: layout::AlignItems) -> AlignItems {
    match a {
        layout::AlignItems::Stretch => AlignItems::STRETCH,
        layout::AlignItems::Center => AlignItems::CENTER,
        layout::AlignItems::FlexStart => AlignItems::FLEX_START,
        layout::AlignItems::FlexEnd => AlignItems::FLEX_END,
    }
}

fn style_of(n: &StyledNode, root: bool) -> Style {
    let e = |v: f32| -> LengthPercentage { length(v) };
    // The styled tree's pixel sizes are content-box already (border-box was
    // converted); percentages of border-box elements still include the edges.
    let has_percent = matches!(n.width, layout::Dimension::Percent(_)) || matches!(n.height, layout::Dimension::Percent(_));
    let (box_sizing, width, height) = if n.border_box && has_percent {
        let px = |d: layout::Dimension, edges: f32| match d {
            layout::Dimension::Px(v) => layout::Dimension::Px(v + edges),
            d => d,
        };
        let h = n.padding.left + n.padding.right + n.border.left + n.border.right;
        let v = n.padding.top + n.padding.bottom + n.border.top + n.border.bottom;
        (BoxSizing::BorderBox, px(n.width, h), px(n.height, v))
    } else {
        (BoxSizing::ContentBox, n.width, n.height)
    };
    let margin_left = if n.margin.left >= f32::MAX / 4.0 { auto() } else { length(n.margin.left) };
    let margin_right = if n.margin.right >= f32::MAX / 4.0 { auto() } else { length(n.margin.right) };
    let (position, inset) = match n.position {
        layout::Position::Absolute | layout::Position::Fixed => (
            Position::Absolute,
            {
                let side = |v: Option<f32>| v.map(length).unwrap_or(auto());
                taffy::Rect { top: side(n.inset[0]), right: side(n.inset[1]), bottom: side(n.inset[2]), left: side(n.inset[3]) }
            },
        ),
        layout::Position::Relative => (
            Position::Relative,
            taffy::Rect { left: length(n.offset_left), top: length(n.offset_top), right: auto(), bottom: auto() },
        ),
        layout::Position::Static => (Position::Relative, taffy::Rect { left: auto(), right: auto(), top: auto(), bottom: auto() }),
    };
    let f = n.flex;
    Style {
        display: match n.display {
            layout::Display::None => Display::None,
            layout::Display::Flex => Display::Flex,
            layout::Display::Grid => Display::Grid,
            layout::Display::Block | layout::Display::Inline => Display::Block,
        },
        box_sizing,
        position,
        inset,
        size: Size { width: if root { percent(1.0) } else { dim(width) }, height: dim(height) },
        min_size: Size { width: lpa(n.min_width), height: lpa(n.min_height) },
        max_size: Size { width: lpa(n.max_width), height: lpa(n.max_height) },
        margin: taffy::Rect { left: margin_left, right: margin_right, top: length(n.margin.top), bottom: length(n.margin.bottom) },
        padding: taffy::Rect { left: e(n.padding.left), right: e(n.padding.right), top: e(n.padding.top), bottom: e(n.padding.bottom) },
        border: taffy::Rect { left: e(n.border.left), right: e(n.border.right), top: e(n.border.top), bottom: e(n.border.bottom) },
        gap: if n.display == layout::Display::Grid {
            Size { width: length(n.grid_gap), height: length(n.grid_gap) }
        } else {
            Size { width: length(n.gap), height: length(n.gap) }
        },
        flex_direction: match f.direction {
            layout::FlexDirection::Row => FlexDirection::Row,
            layout::FlexDirection::RowReverse => FlexDirection::RowReverse,
            layout::FlexDirection::Column => FlexDirection::Column,
            layout::FlexDirection::ColumnReverse => FlexDirection::ColumnReverse,
        },
        flex_wrap: match n.flex_wrap {
            layout::FlexWrap::Wrap => FlexWrap::Wrap,
            layout::FlexWrap::NoWrap => FlexWrap::NoWrap,
        },
        justify_content: match f.justify {
            layout::JustifyContent::FlexStart => JustifyContent::FLEX_START,
            layout::JustifyContent::FlexEnd => JustifyContent::FLEX_END,
            layout::JustifyContent::Center => JustifyContent::CENTER,
            layout::JustifyContent::SpaceBetween => JustifyContent::SPACE_BETWEEN,
            layout::JustifyContent::SpaceAround => JustifyContent::SPACE_AROUND,
            layout::JustifyContent::SpaceEvenly => JustifyContent::SPACE_EVENLY,
        },
        align_items: align(n.align_items),
        align_self: f.align_self.map(align),
        flex_grow: f.grow,
        flex_shrink: f.shrink,
        flex_basis: dim(f.basis),
        grid_template_columns: parse_tracks(&n.grid.columns),
        grid_template_rows: parse_tracks(&n.grid.rows),
        grid_column: parse_line(&n.grid.column),
        grid_row: parse_line(&n.grid.row),
        ..Default::default()
    }
}

/// Splits on top-level whitespace and commas, keeping `f(a, b)` together.
fn split_top(s: &str, sep: fn(char) -> bool) -> Vec<String> {
    let (mut out, mut cur, mut depth) = (Vec::new(), String::new(), 0);
    for c in s.chars() {
        match c {
            '(' => { depth += 1; cur.push(c); }
            ')' => { depth -= 1; cur.push(c); }
            c if depth == 0 && sep(c) => {
                if !cur.trim().is_empty() { out.push(cur.trim().to_string()); }
                cur.clear();
            }
            c => cur.push(c),
        }
    }
    if !cur.trim().is_empty() { out.push(cur.trim().to_string()); }
    out
}

/// One track size: `1fr`, `200px`, `25%`, `auto`, `min-content`, `max-content`, `minmax(a, b)`.
fn parse_track(t: &str) -> Option<TrackSizingFunction> {
    let t = t.trim();
    if let Some(args) = t.strip_prefix("minmax(").and_then(|r| r.strip_suffix(')')) {
        let parts = split_top(args, |c| c == ',');
        let (lo, hi) = (parts.first()?, parts.get(1)?);
        let min: MinTrackSizingFunction = match lo.as_str() {
            "auto" => auto(),
            "min-content" => min_content(),
            "max-content" => max_content(),
            v => num(v, "px").map(length).or_else(|| num(v, "%").map(|p| percent(p / 100.0)))?,
        };
        let max: MaxTrackSizingFunction = match hi.as_str() {
            "auto" => auto(),
            "min-content" => min_content(),
            "max-content" => max_content(),
            v => num(v, "fr").map(fr).or_else(|| num(v, "px").map(length)).or_else(|| num(v, "%").map(|p| percent(p / 100.0)))?,
        };
        return Some(minmax(min, max));
    }
    Some(match t {
        "auto" => auto(),
        "min-content" => min_content(),
        "max-content" => max_content(),
        v => num(v, "fr").map(fr).or_else(|| num(v, "px").map(length)).or_else(|| num(v, "%").map(|p| percent(p / 100.0)))
            .or_else(|| v.parse::<f32>().ok().map(length))?,
    })
}

fn num(v: &str, unit: &str) -> Option<f32> {
    v.strip_suffix(unit)?.trim().parse().ok()
}

/// `grid-template-columns`/`-rows`: track sizes and `repeat(n | auto-fill | auto-fit, ...)`.
fn parse_tracks(s: &str) -> Vec<GridTemplateComponent<String>> {
    let mut out = Vec::new();
    for item in split_top(s, char::is_whitespace) {
        if let Some(args) = item.strip_prefix("repeat(").and_then(|r| r.strip_suffix(')')) {
            let parts = split_top(args, |c| c == ',');
            let Some(count) = parts.first() else { continue };
            let tracks: Vec<TrackSizingFunction> = parts[1..].iter()
                .flat_map(|p| split_top(p, char::is_whitespace))
                .filter_map(|t| parse_track(&t))
                .collect();
            match count.as_str() {
                "auto-fill" => out.push(repeat(taffy::RepetitionCount::AutoFill, tracks)),
                "auto-fit" => out.push(repeat(taffy::RepetitionCount::AutoFit, tracks)),
                n => {
                    for _ in 0..n.parse::<usize>().unwrap_or(1).min(64) {
                        out.extend(tracks.iter().map(|t| GridTemplateComponent::Single(*t)));
                    }
                }
            }
        } else if let Some(t) = parse_track(&item) {
            out.push(GridTemplateComponent::Single(t));
        }
    }
    out
}

/// `grid-column`/`grid-row`: `span 2`, `1 / 3`, `2 / span 2`, `1 / -1`, `3`.
fn parse_line(s: &str) -> Line<GridPlacement<String>> {
    fn one(p: &str) -> GridPlacement<String> {
        let p = p.trim();
        if let Some(n) = p.strip_prefix("span").and_then(|n| n.trim().parse::<u16>().ok()) {
            return span(n);
        }
        match p.parse::<i16>() {
            Ok(n) if n != 0 => line(n),
            _ => GridPlacement::Auto,
        }
    }
    match s.split_once('/') {
        Some((a, b)) => Line { start: one(a), end: one(b) },
        None if s.trim().is_empty() => Line { start: GridPlacement::Auto, end: GridPlacement::Auto },
        None => Line { start: one(s), end: GridPlacement::Auto },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_expands() {
        let t = parse_tracks("repeat(3, 1fr)");
        assert_eq!(t.len(), 3, "{t:?}");
        assert_eq!(parse_tracks("200px 1fr minmax(100px, 1fr)").len(), 3);
        assert_eq!(parse_tracks("repeat(auto-fill, 120px)").len(), 1);
    }
}
