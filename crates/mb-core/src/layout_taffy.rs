//! Block, flex and grid layout by Taffy (the layout engine Servo's and Blitz's
//! use), producing the same [`LayoutBox`] tree the painter reads.
//!
//! Taffy has no inline layout, so boxes that hold inline content (text, inline
//! elements, images, or a block whose children are all of those) are Taffy
//! leaves: they are measured, and finally laid out inside the rectangle Taffy
//! gave them, by the original layout code in [`crate::layout`].

use taffy::prelude::*;
use taffy::AvailableSpace;

use crate::layout::{self, BoxContent, LayoutBox, StyledNode};

/// Lays out the styled tree for a viewport `viewport_width` wide; the height
/// grows with the content.
pub fn layout_tree(styled_root: &StyledNode, viewport_width: f32, viewport_height: f32) -> LayoutBox {
    let mut root = layout::build_layout_box(styled_root);
    let mut tree: TaffyTree<usize> = TaffyTree::new();
    let mut leaves: Vec<LayoutBox> = Vec::new();
    let root_id = build(&mut tree, &mut leaves, styled_root, &root, true);

    let available = Size { width: AvailableSpace::Definite(viewport_width), height: AvailableSpace::MaxContent };
    let measured = tree.compute_layout_with_measure(root_id, available, |inputs, _id, ctx, style| {
        taffy::compute_leaf_layout(inputs, style, |_, _| 0.0, |known, avail| match ctx {
            Some(i) => measure_leaf(&leaves[*i], known, avail),
            None => Size::ZERO,
        })
    });
    if let Err(e) = measured {
        log::warn!("taffy: {e:?}; falling back to the built-in layout");
        return layout::layout_tree(styled_root, viewport_width, viewport_height);
    }
    place(&tree, root_id, &mut root, 0.0, 0.0);
    root
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

/// Mirrors the styled tree into Taffy nodes; leaf boxes are kept in `leaves`
/// (their index is the Taffy node context).
fn build(tree: &mut TaffyTree<usize>, leaves: &mut Vec<LayoutBox>, node: &StyledNode, boxed: &LayoutBox, root: bool) -> NodeId {
    let style = style_of(node, root);
    if node.display != layout::Display::None && is_leaf(node) {
        leaves.push(boxed.clone());
        return tree.new_leaf_with_context(style, leaves.len() - 1).expect("taffy leaf");
    }
    let children: Vec<NodeId> = if node.display == layout::Display::None {
        Vec::new()
    } else {
        // build_layout_box drops hidden children; skip them too to stay in step.
        node.children
            .iter()
            .filter(|c| c.display != layout::Display::None)
            .zip(&boxed.children)
            .map(|(c, b)| build(tree, leaves, c, b, false))
            .collect()
    };
    tree.new_with_children(style, &children).expect("taffy node")
}

/// Content size of a leaf for Taffy: the built-in layout runs on a copy of it
/// whose own margin, border and padding are zeroed (Taffy adds those itself).
fn measure_leaf(template: &LayoutBox, known: Size<Option<f32>>, avail: Size<AvailableSpace>) -> Size<f32> {
    if let Size { width: Some(w), height: Some(h) } = known {
        return Size { width: w, height: h };
    }
    let run = |width: f32| -> (f32, f32) {
        let mut b = template.clone();
        b.margin = layout::EdgeSizes::ZERO;
        b.padding = layout::EdgeSizes::ZERO;
        b.border = layout::EdgeSizes::ZERO;
        b.explicit_width = layout::Dimension::Auto;
        b.explicit_height = layout::Dimension::Auto;
        b.position = layout::Position::Static;
        left_align(&mut b);
        b.layout(0.0, 0.0, width, 100_000.0);
        (extent(&b) - b.content_x(), b.height)
    };
    let width = match known.width {
        Some(w) => w,
        None => {
            let (max_content, _) = run(1.0e6);
            match avail.width {
                AvailableSpace::Definite(w) => max_content.min(w),
                AvailableSpace::MinContent => run(0.0).0,
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
        BoxContent::Text(_) | BoxContent::Image { .. } => b.x + b.margin_box_width() - b.margin.right,
        _ => b.content_x(),
    };
    b.children.iter().map(extent).fold(own, f32::max)
}

/// Copies Taffy's geometry into the box tree. `(px, py)` is the parent's
/// border-box origin in page coordinates.
fn place(tree: &TaffyTree<usize>, id: NodeId, b: &mut LayoutBox, px: f32, py: f32) {
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
    for (cid, child) in kids.into_iter().zip(b.children.iter_mut()) {
        place(tree, cid, child, bx, by);
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
    let (position, inset) = match n.position {
        layout::Position::Absolute | layout::Position::Fixed => (
            Position::Absolute,
            taffy::Rect {
                left: if n.offset_left != 0.0 { length(n.offset_left) } else { auto() },
                top: if n.offset_top != 0.0 { length(n.offset_top) } else { auto() },
                right: auto(),
                bottom: auto(),
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
        min_size: Size { width: auto(), height: lpa(n.min_height) },
        margin: taffy::Rect { left: margin_left, right: length(n.margin.right), top: length(n.margin.top), bottom: length(n.margin.bottom) },
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
        grid_template_columns: n
            .grid_template_columns
            .iter()
            .map(|t| match t {
                layout::GridTrack::Fr(v) => fr(*v),
                layout::GridTrack::Px(v) => length(*v),
            })
            .collect(),
        ..Default::default()
    }
}
