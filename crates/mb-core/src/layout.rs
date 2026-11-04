//! Block layout engine.
//!
//! Takes a simple styled-node tree and produces a tree of `LayoutBox` values
//! with fully resolved positions and sizes.

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// RGBA colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::rgba(r, g, b, 255)
    }
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
}

/// Edge sizes for margin / padding / border.
#[derive(Debug, Clone, Copy)]
pub struct EdgeSizes {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl EdgeSizes {
    pub const ZERO: Self = Self {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    };

    pub fn uniform(v: f32) -> Self {
        Self {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }
}

/// What kind of content a layout box contains.
#[derive(Debug, Clone)]
pub enum BoxContent {
    Block,
    Inline,
    Flex,
    Grid,
    Text(String),
    Image {
        alt: String,
        /// Decoded RGBA pixel data (width * height * 4 bytes). Empty if placeholder.
        data: Vec<u8>,
        /// Intrinsic pixel width of the decoded image (0 if placeholder).
        intrinsic_w: u32,
        /// Intrinsic pixel height of the decoded image (0 if placeholder).
        intrinsic_h: u32,
    },
    Anonymous,
}

/// A fully laid-out box with absolute positions.
#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub margin: EdgeSizes,
    pub padding: EdgeSizes,
    pub border: EdgeSizes,
    pub content: BoxContent,
    pub children: Vec<LayoutBox>,
    pub background_color: Option<Color>,
    pub text_color: Color,
    pub border_color: Option<Color>,
    pub font_size: f32,
    pub text_align: TextAlign,
    pub opacity: f32,
    pub explicit_width: Dimension,
    pub explicit_height: Dimension,
    pub gap: f32,
    pub flex_wrap: FlexWrap,
    pub align_items: AlignItems,
    pub min_height: Dimension,
    pub line_height: f32,
    pub white_space_nowrap: bool,
    pub float_right: bool,
    pub border_radius: f32,
    pub text_decoration: TextDecoration,
    pub position: Position,
    pub offset_top: f32,
    pub offset_left: f32,
    pub grid_template_columns: Vec<GridTrack>,
    pub grid_gap: f32,
}

impl LayoutBox {
    /// Total width including margin + border + padding + content.
    pub fn margin_box_width(&self) -> f32 {
        self.margin.left
            + self.border.left
            + self.padding.left
            + self.width
            + self.padding.right
            + self.border.right
            + self.margin.right
    }

    /// Total height including margin + border + padding + content.
    pub fn margin_box_height(&self) -> f32 {
        self.margin.top
            + self.border.top
            + self.padding.top
            + self.height
            + self.padding.bottom
            + self.border.bottom
            + self.margin.bottom
    }

    /// The left edge of the content area in absolute coordinates.
    pub fn content_x(&self) -> f32 {
        self.x + self.margin.left + self.border.left + self.padding.left
    }

    /// The top edge of the content area in absolute coordinates.
    pub fn content_y(&self) -> f32 {
        self.y + self.margin.top + self.border.top + self.padding.top
    }
}

// ---------------------------------------------------------------------------
// Styled node — lightweight input representation
// ---------------------------------------------------------------------------

/// Minimal styled node used as input to the layout engine.
///
/// Higher-level code (the CSS cascade) should resolve property values and
/// build this tree before handing it to [`layout_tree`].
#[derive(Debug, Clone)]
pub struct StyledNode {
    pub display: Display,
    pub position: Position,
    pub width: Dimension,
    pub height: Dimension,
    pub margin: EdgeSizes,
    pub padding: EdgeSizes,
    pub border: EdgeSizes,
    pub background_color: Option<Color>,
    pub text_color: Color,
    pub border_color: Option<Color>,
    pub font_size: f32,
    pub text_align: TextAlign,
    pub opacity: f32,
    pub gap: f32,
    pub flex_wrap: FlexWrap,
    pub align_items: AlignItems,
    pub min_height: Dimension,
    pub line_height: f32,
    pub white_space_nowrap: bool,
    pub float_right: bool,
    pub border_radius: f32,
    pub text_decoration: TextDecoration,
    pub offset_top: f32,
    pub offset_left: f32,
    pub grid_template_columns: Vec<GridTrack>,
    pub grid_gap: f32,
    pub content: NodeContent,
    pub children: Vec<StyledNode>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FlexWrap {
    NoWrap,
    Wrap,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlignItems {
    Stretch,
    Center,
    FlexStart,
    FlexEnd,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextDecoration {
    None,
    Underline,
    LineThrough,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Display {
    Block,
    Inline,
    Flex,
    Grid,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Position {
    Static,
    Absolute,
    Relative,
    Fixed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GridTrack {
    Px(f32),
    Fr(f32),
}

#[derive(Debug, Clone, Copy)]
pub enum Dimension {
    Auto,
    Px(f32),
}

#[derive(Debug, Clone)]
pub enum NodeContent {
    Element,
    Text(String),
    Image {
        alt: String,
        data: Vec<u8>,
        intrinsic_w: u32,
        intrinsic_h: u32,
    },
}

impl StyledNode {
    /// Convenience: create a simple text node with default styling.
    pub fn text(s: &str) -> Self {
        Self {
            display: Display::Inline,
            position: Position::Static,
            width: Dimension::Auto,
            height: Dimension::Auto,
            margin: EdgeSizes::ZERO,
            padding: EdgeSizes::ZERO,
            border: EdgeSizes::ZERO,
            background_color: None,
            text_color: Color::BLACK,
            border_color: None,
            font_size: 16.0,
            text_align: TextAlign::Left,
            opacity: 1.0,
            gap: 0.0,
            flex_wrap: FlexWrap::NoWrap,
            align_items: AlignItems::Stretch,
            min_height: Dimension::Auto,
            line_height: 0.0,
            white_space_nowrap: false,
            float_right: false,
            border_radius: 0.0,
            text_decoration: TextDecoration::None,
            offset_top: 0.0,
            offset_left: 0.0,
            grid_template_columns: Vec::new(),
            grid_gap: 0.0,
            content: NodeContent::Text(s.to_string()),
            children: Vec::new(),
        }
    }

    /// Convenience: create a block container with children.
    pub fn block(children: Vec<StyledNode>) -> Self {
        Self {
            display: Display::Block,
            position: Position::Static,
            width: Dimension::Auto,
            height: Dimension::Auto,
            margin: EdgeSizes::ZERO,
            padding: EdgeSizes::ZERO,
            border: EdgeSizes::ZERO,
            background_color: None,
            text_color: Color::BLACK,
            border_color: None,
            font_size: 16.0,
            text_align: TextAlign::Left,
            opacity: 1.0,
            gap: 0.0,
            flex_wrap: FlexWrap::NoWrap,
            align_items: AlignItems::Stretch,
            min_height: Dimension::Auto,
            line_height: 0.0,
            white_space_nowrap: false,
            float_right: false,
            border_radius: 0.0,
            text_decoration: TextDecoration::None,
            offset_top: 0.0,
            offset_left: 0.0,
            grid_template_columns: Vec::new(),
            grid_gap: 0.0,
            content: NodeContent::Element,
            children,
        }
    }
}

// ---------------------------------------------------------------------------
// Estimated character width for text measurement
// ---------------------------------------------------------------------------

/// Approximate width of a single character at the given font size.
/// Use fontdue for accurate text measurement (matching paint module).
fn measure_text_width(text: &str, font_size: f32) -> f32 {
    crate::font::FontduFont::shared().measure_text(text, font_size).0
}

/// Single character width via fontdue.
fn estimated_char_width(font_size: f32) -> f32 {
    measure_text_width("M", font_size)
}

// ---------------------------------------------------------------------------
// Layout algorithm
// ---------------------------------------------------------------------------

/// Lay out the entire styled tree within the given viewport.
pub fn layout_tree(
    styled_root: &StyledNode,
    viewport_width: f32,
    viewport_height: f32,
) -> LayoutBox {
    let mut root = build_layout_box(styled_root);
    // The root box starts at (0, 0) and its containing block is the viewport.
    root.layout(0.0, 0.0, viewport_width, viewport_height);
    root
}

/// Count total layout boxes in the tree (useful for diagnostics).
pub fn count_boxes(layout: &LayoutBox) -> usize {
    1 + layout
        .children
        .iter()
        .map(|c| count_boxes(c))
        .sum::<usize>()
}

/// Recursively convert the styled tree into an un-laid-out `LayoutBox` tree.
fn build_layout_box(node: &StyledNode) -> LayoutBox {
    if node.display == Display::None {
        // Hidden nodes produce a zero-sized anonymous box with no children.
        return LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            margin: EdgeSizes::ZERO,
            padding: EdgeSizes::ZERO,
            border: EdgeSizes::ZERO,
            content: BoxContent::Anonymous,
            children: Vec::new(),
            background_color: None,
            text_color: Color::BLACK,
            border_color: None,
            font_size: 0.0,
            text_align: TextAlign::Left,
            opacity: 1.0,
            explicit_width: Dimension::Auto,
            explicit_height: Dimension::Auto,
            gap: 0.0,
            flex_wrap: FlexWrap::NoWrap,
            align_items: AlignItems::Stretch,
            min_height: Dimension::Auto,
            line_height: 0.0,
            white_space_nowrap: false,
            float_right: false,
            border_radius: 0.0,
            text_decoration: TextDecoration::None,
            position: Position::Static,
            offset_top: 0.0,
            offset_left: 0.0,
            grid_template_columns: Vec::new(),
            grid_gap: 0.0,
        };
    }

    let content = match &node.content {
        NodeContent::Text(s) => BoxContent::Text(s.clone()),
        NodeContent::Image { alt, data, intrinsic_w, intrinsic_h } => BoxContent::Image {
            alt: alt.clone(),
            data: data.clone(),
            intrinsic_w: *intrinsic_w,
            intrinsic_h: *intrinsic_h,
        },
        NodeContent::Element => match node.display {
            Display::Block => BoxContent::Block,
            Display::Inline => BoxContent::Inline,
            Display::Flex => BoxContent::Flex,
            Display::Grid => BoxContent::Grid,
            Display::None => BoxContent::Anonymous, // already handled above
        },
    };

    let children: Vec<LayoutBox> = node
        .children
        .iter()
        .filter(|c| c.display != Display::None)
        .map(|c| build_layout_box(c))
        .collect();

    LayoutBox {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
        margin: node.margin,
        padding: node.padding,
        border: node.border,
        content,
        children,
        background_color: node.background_color,
        text_color: node.text_color,
        border_color: node.border_color,
        font_size: node.font_size,
        text_align: node.text_align,
        opacity: node.opacity,
        explicit_width: node.width,
        explicit_height: node.height,
        gap: node.gap,
        flex_wrap: node.flex_wrap,
        align_items: node.align_items,
        min_height: node.min_height,
        line_height: node.line_height,
        white_space_nowrap: node.white_space_nowrap,
        float_right: node.float_right,
        border_radius: node.border_radius,
        text_decoration: node.text_decoration,
        position: node.position,
        offset_top: node.offset_top,
        offset_left: node.offset_left,
        grid_template_columns: node.grid_template_columns.clone(),
        grid_gap: node.grid_gap,
    }
}

impl LayoutBox {
    /// Perform layout relative to containing block at (`cx`, `cy`) with the
    /// given available width and height.
    fn layout(
        &mut self,
        cx: f32,
        cy: f32,
        containing_width: f32,
        _containing_height: f32,
    ) {
        // Fixed elements are positioned relative to viewport (0,0)
        let (eff_cx, eff_cy) = if self.position == Position::Fixed {
            (0.0, 0.0)
        } else {
            (cx, cy)
        };

        match self.content {
            BoxContent::Anonymous => {
                // Anonymous boxes from Display::None — keep zero size.
                self.x = eff_cx;
                self.y = eff_cy;
            }
            BoxContent::Block => {
                self.layout_block(eff_cx, eff_cy, containing_width);
            }
            BoxContent::Flex => {
                self.layout_flex(eff_cx, eff_cy, containing_width);
            }
            BoxContent::Grid => {
                self.layout_grid(eff_cx, eff_cy, containing_width);
            }
            BoxContent::Inline => {
                self.layout_inline(eff_cx, eff_cy, containing_width);
            }
            BoxContent::Text(_) => {
                self.layout_text(eff_cx, eff_cy, containing_width);
            }
            BoxContent::Image { .. } => {
                self.layout_image(eff_cx, eff_cy, containing_width);
            }
        }

        // For position: fixed/absolute, use top/left as absolute coordinates
        if self.position == Position::Fixed || self.position == Position::Absolute {
            if self.offset_top != 0.0 {
                self.y = self.offset_top;
            }
            if self.offset_left != 0.0 {
                self.x = self.offset_left;
            }
        }

        // For position: relative, offset from normal flow position
        if self.position == Position::Relative {
            self.x += self.offset_left;
            self.y += self.offset_top;
        }
    }

    // -----------------------------------------------------------------------
    // Block layout
    // -----------------------------------------------------------------------
    fn layout_block(&mut self, cx: f32, cy: f32, containing_width: f32) {
        // 1. Resolve width
        self.width = self.resolve_width(containing_width);

        // 2. Position
        self.x = cx;
        self.y = cy;

        // 3. Check if children are all inline elements — if so, use inline flow.
        //    Bare text children in a block context still stack vertically.
        let has_inline_elements = self.children.iter().any(|c| {
            matches!(c.content, BoxContent::Inline)
        });
        let all_inline = !self.children.is_empty() && has_inline_elements && self.children.iter().all(|c| {
            matches!(c.content, BoxContent::Inline | BoxContent::Text(_) | BoxContent::Image { .. })
        });

        if all_inline {
            self.layout_inline_children();
        } else {
            // Normal block flow: lay out children vertically with margin collapse
            let content_x = self.content_x();
            let mut child_y = self.content_y();
            let mut prev_margin_bottom: f32 = 0.0;

            for child in &mut self.children {

                // Margin collapse: the gap between siblings is max(prev_bottom, cur_top)
                // not prev_bottom + cur_top
                let cur_margin_top = child.margin.top;
                let collapsed = prev_margin_bottom.max(cur_margin_top);
                let adjustment = prev_margin_bottom + cur_margin_top - collapsed;

                child.layout(content_x, child_y - adjustment, self.width, 0.0);

                // float:right — push child to the right edge of content area
                if child.float_right {
                    let child_total_w = child.margin_box_width();
                    let new_x = content_x + self.width - child_total_w;
                    let dx = new_x - child.x;
                    child.x = new_x;
                    fn shift_descendants_block(node: &mut LayoutBox, dx: f32) {
                        for c in &mut node.children {
                            c.x += dx;
                            shift_descendants_block(c, dx);
                        }
                    }
                    shift_descendants_block(child, dx);
                }

                child_y += child.margin_box_height() - adjustment;
                prev_margin_bottom = child.margin.bottom;
            }
        }

        // 4. Resolve height
        let children_height = if all_inline {
            // Inline children are laid out horizontally — height is based on
            // the bottom-most child relative to the content area top.
            let content_top = self.content_y();
            let mut max_bottom: f32 = content_top;
            for child in &self.children {
                let child_bottom = child.y + child.margin_box_height();
                if child_bottom > max_bottom {
                    max_bottom = child_bottom;
                }
            }
            max_bottom - content_top
        } else {
            // Block flow: account for margin collapse in children
            let mut h: f32 = 0.0;
            let mut prev_mb: f32 = 0.0;
            for child in &self.children {
                let cur_mt = child.margin.top;
                let collapsed = prev_mb.max(cur_mt);
                let adj = prev_mb + cur_mt - collapsed;
                h += child.margin_box_height() - adj;
                prev_mb = child.margin.bottom;
            }
            h
        };

        self.height = match self.explicit_height {
            Dimension::Px(h) => h,
            Dimension::Auto => children_height,
        };

        // Clamp to min-height
        if let Dimension::Px(min_h) = self.min_height {
            if self.height < min_h {
                self.height = min_h;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Inline children flow layout
    // -----------------------------------------------------------------------
    fn layout_inline_children(&mut self) {
        let content_x = self.content_x();
        let content_y = self.content_y();
        let available_width = self.width;

        let mut cursor_x = content_x;
        let mut cursor_y = content_y;
        let mut line_height: f32 = 0.0;

        for child in &mut self.children {
            // For text nodes with leading whitespace, advance cursor by one space
            // so that inline siblings are properly separated (e.g. "with <b>bold</b>")
            if let BoxContent::Text(ref s) = child.content {
                if s.starts_with(|c: char| c.is_whitespace()) && cursor_x > content_x {
                    cursor_x += measure_text_width(" ", child.font_size);
                }
            }

            // Measure the child's desired width
            let child_width = match &child.content {
                BoxContent::Text(s) => {
                    // Use the same measurement approach as layout_text:
                    // word widths + inter-word spaces + trailing space
                    let words: Vec<&str> = s.split_whitespace().collect();
                    if words.is_empty() {
                        0.0
                    } else {
                        let space_w = measure_text_width(" ", child.font_size);
                        let mut w: f32 = 0.0;
                        for (i, word) in words.iter().enumerate() {
                            if i > 0 { w += space_w; }
                            w += measure_text_width(word, child.font_size);
                        }
                        if s.ends_with(|c: char| c.is_whitespace()) {
                            w += space_w;
                        }
                        w
                    }
                }
                BoxContent::Inline => {
                    // Inline elements: fit-content width (just content, no extras)
                    // child_margin_h below will add margin+border+padding
                    match child.explicit_width {
                        Dimension::Px(w) => w,
                        Dimension::Auto => child.measure_inline_content_width(),
                    }
                }
                BoxContent::Image { intrinsic_w, .. } => {
                    match child.explicit_width {
                        Dimension::Px(w) => w,
                        Dimension::Auto => if *intrinsic_w > 0 { *intrinsic_w as f32 } else { 200.0 },
                    }
                }
                _ => child.resolve_width(available_width),
            };

            let child_margin_h = child.margin.left + child.border.left + child.padding.left
                + child.padding.right + child.border.right + child.margin.right;
            let total_child_width = child_width + child_margin_h;

            // Wrap to next line if doesn't fit
            if cursor_x + total_child_width > content_x + available_width
                && cursor_x > content_x
            {
                cursor_x = content_x;
                cursor_y += line_height;
                line_height = 0.0;
            }

            // For text nodes, use remaining available width so text can wrap properly
            let layout_width = if matches!(child.content, BoxContent::Text(_)) {
                let remaining = content_x + available_width - cursor_x;
                remaining.max(child_width + child_margin_h)
            } else {
                child_width + child_margin_h
            };
            child.layout(cursor_x, cursor_y, layout_width, 0.0);
            cursor_x += child.margin_box_width();
            line_height = line_height.max(child.margin_box_height());
        }

        // Post-pass: reposition float:right children to the right edge
        for child in &mut self.children {
            if child.float_right {
                let child_total_w = child.margin_box_width();
                let new_x = content_x + available_width - child_total_w;
                let dx = new_x - child.x;
                child.x = new_x;
                // Shift all descendants to match the new position
                fn shift_descendants(node: &mut LayoutBox, dx: f32) {
                    for c in &mut node.children {
                        c.x += dx;
                        shift_descendants(c, dx);
                    }
                }
                shift_descendants(child, dx);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Flex layout
    // -----------------------------------------------------------------------
    fn layout_flex(&mut self, cx: f32, cy: f32, containing_width: f32) {
        // 1. Resolve width
        self.width = self.resolve_width(containing_width);

        // 2. Position
        self.x = cx;
        self.y = cy;

        // In flex layout, whitespace-only text nodes don't participate as flex items.
        // Collapse them to zero-size anonymous boxes.
        for child in &mut self.children {
            if let BoxContent::Text(ref s) = child.content {
                if s.chars().all(|c| c.is_whitespace()) {
                    child.width = 0.0;
                    child.height = 0.0;
                    child.content = BoxContent::Anonymous;
                }
            }
        }

        if self.children.is_empty() {
            self.height = 0.0;
            return;
        }

        let content_x = self.content_x();
        let content_y = self.content_y();
        let available_width = self.width;
        let gap = self.gap;
        let wrap = self.flex_wrap == FlexWrap::Wrap;
        let align = self.align_items;

        // Detect and zero out auto margin-left sentinels before layout
        let auto_sentinel = f32::MAX / 2.0;
        let mut auto_margin_left_indices: Vec<usize> = Vec::new();
        for (i, child) in self.children.iter_mut().enumerate() {
            if child.margin.left >= auto_sentinel * 0.9 {
                child.margin.left = 0.0;
                auto_margin_left_indices.push(i);
            }
        }

        // Pre-compute desired widths for each child
        let child_desired_widths: Vec<f32> = self.children.iter().map(|child| {
            let child_horiz = child.margin.left + child.border.left + child.padding.left
                + child.padding.right + child.border.right + child.margin.right;
            if child.has_explicit_width() {
                child.resolve_width(available_width) + child_horiz
            } else {
                match &child.content {
                    BoxContent::Text(s) => {
                        measure_text_width(s, child.font_size) + child_horiz
                    }
                    BoxContent::Inline => {
                        // Inline children in flex: content width + horizontal extras
                        // measure_inline_content_width returns just content,
                        // child_horiz has margin+border+padding
                        child.measure_inline_content_width() + child_horiz
                    }
                    _ => {
                        // Block/other children in flex without explicit width:
                        // measure content width (text children, nested elements)
                        let content_w = Self::measure_subtree_content_width(child);
                        if content_w > 0.0 {
                            content_w + child_horiz
                        } else {
                            0.0 // truly auto — will share remaining space
                        }
                    }
                }
            }
        }).collect();

        // Build lines (for wrapping)
        // Each line is a vec of (child_index, desired_width)
        let mut lines: Vec<Vec<(usize, f32)>> = Vec::new();
        let mut current_line: Vec<(usize, f32)> = Vec::new();
        let mut current_line_width: f32 = 0.0;

        for (i, &dw) in child_desired_widths.iter().enumerate() {
            // Skip collapsed whitespace nodes (now Anonymous)
            if matches!(self.children[i].content, BoxContent::Anonymous) {
                continue;
            }
            let child_w = if dw == 0.0 { 50.0 } else { dw }; // estimate for auto-width
            let needed = if current_line.is_empty() { child_w } else { gap + child_w };

            if wrap && !current_line.is_empty() && current_line_width + needed > available_width {
                lines.push(current_line);
                current_line = vec![(i, dw)];
                current_line_width = child_w;
            } else {
                if !current_line.is_empty() {
                    current_line_width += gap;
                }
                current_line.push((i, dw));
                current_line_width += child_w;
            }
        }
        if !current_line.is_empty() {
            lines.push(current_line);
        }

        // Lay out each line
        let mut y_cursor = content_y;
        let mut total_height: f32 = 0.0;

        for line in &lines {
            // Count auto-width children and explicit width total for this line
            let mut explicit_total: f32 = 0.0;
            let mut auto_count: usize = 0;
            for &(_, dw) in line {
                if dw > 0.0 {
                    explicit_total += dw;
                } else {
                    auto_count += 1;
                }
            }
            let line_gap_total = gap * (line.len() as f32 - 1.0).max(0.0);
            let total_desired = explicit_total + line_gap_total;

            // Flex-shrink: when items with positive desired widths overflow
            // the container, shrink them proportionally (CSS flex-shrink: 1 default).
            let overflow = if auto_count == 0 && explicit_total + line_gap_total > available_width {
                explicit_total + line_gap_total - available_width
            } else {
                0.0
            };
            let shrink_per_item = if overflow > 0.0 && !line.is_empty() {
                overflow / line.len() as f32
            } else {
                0.0
            };

            let remaining = if overflow > 0.0 {
                0.0
            } else {
                (available_width - line_gap_total - explicit_total).max(0.0)
            };
            let auto_child_width = if auto_count > 0 {
                remaining / auto_count as f32
            } else {
                0.0
            };

            let mut x_cursor = content_x;

            // First pass: lay out children in this line
            let mut line_max_height: f32 = 0.0;
            let child_indices: Vec<usize> = line.iter().map(|&(i, _)| i).collect();

            for &(idx, dw) in line {
                let child = &mut self.children[idx];
                let child_width = if dw > 0.0 {
                    (dw - shrink_per_item).max(0.0)
                } else {
                    auto_child_width
                };

                // When shrinking, temporarily adjust explicit_width so
                // resolve_width uses the shrunk content size
                let saved_width = child.explicit_width;
                if shrink_per_item > 0.0 {
                    if let Dimension::Px(w) = child.explicit_width {
                        child.explicit_width = Dimension::Px((w - shrink_per_item).max(0.0));
                    }
                }

                child.layout(x_cursor, y_cursor, child_width, 0.0);

                child.explicit_width = saved_width;

                // Use explicit height if set
                if let Dimension::Px(h) = child.explicit_height {
                    child.height = h;
                }

                x_cursor += child.margin_box_width() + gap;
                line_max_height = line_max_height.max(child.margin_box_height());
            }

            // Second pass: apply align-items within this line
            match align {
                AlignItems::Center | AlignItems::FlexEnd => {
                    for &idx in &child_indices {
                        let child = &mut self.children[idx];
                        let child_h = child.margin_box_height();
                        match align {
                            AlignItems::Center => {
                                let offset = (line_max_height - child_h) / 2.0;
                                child.y += offset;
                            }
                            AlignItems::FlexEnd => {
                                let offset = line_max_height - child_h;
                                child.y += offset;
                            }
                            _ => {}
                        }
                    }
                }
                AlignItems::Stretch => {
                    // Stretch: children without explicit height fill the line height
                    for &idx in &child_indices {
                        let child = &mut self.children[idx];
                        if matches!(child.explicit_height, Dimension::Auto) {
                            let target_h = line_max_height
                                - child.margin.top - child.margin.bottom
                                - child.border.top - child.border.bottom
                                - child.padding.top - child.padding.bottom;
                            if target_h > child.height {
                                child.height = target_h;
                            }
                        }
                    }
                }
                _ => {} // FlexStart — no adjustment needed
            }

            y_cursor += line_max_height;
            if total_height > 0.0 {
                // Add gap between lines (use gap for row gap as well)
                y_cursor += gap;
                total_height += gap;
            }
            total_height += line_max_height;
        }

        self.height = total_height;

        // Clamp to min-height
        if let Dimension::Px(min_h) = self.min_height {
            if self.height < min_h {
                self.height = min_h;
            }
        }

        // Handle margin-left: auto — push element to the right of the flex line
        if !auto_margin_left_indices.is_empty() {
            for line in &lines {
                for &(idx, _) in line {
                    if !auto_margin_left_indices.contains(&idx) {
                        continue;
                    }
                    // Compute total used width of all children in this line (excluding auto margins)
                    let mut used_width: f32 = 0.0;
                    for &(other_idx, _) in line {
                        used_width += self.children[other_idx].margin_box_width();
                    }
                    used_width += gap * (line.len() as f32 - 1.0).max(0.0);
                    let auto_margin = (available_width - used_width).max(0.0);

                    // Set the auto margin on this child. content_x() will add
                    // margin.left to self.x, positioning content correctly.
                    // We do NOT shift self.x (that would double-count the margin).
                    // But we must shift all descendants since they have absolute positions
                    // that were computed without the auto margin.
                    fn shift_subtree(node: &mut LayoutBox, dx: f32) {
                        node.x += dx;
                        for child in &mut node.children {
                            shift_subtree(child, dx);
                        }
                    }

                    let child = &mut self.children[idx];
                    child.margin.left = auto_margin;
                    // Shift all children (not self) to account for the new margin
                    for desc in &mut child.children {
                        shift_subtree(desc, auto_margin);
                    }

                    // Shift subsequent children in the line by auto_margin
                    // (their positions need to account for the enlarged margin box)
                    let mut found = false;
                    for &(other_idx, _) in line {
                        if other_idx == idx {
                            found = true;
                            continue;
                        }
                        if found {
                            shift_subtree(&mut self.children[other_idx], auto_margin);
                        }
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Grid layout
    // -----------------------------------------------------------------------
    fn layout_grid(&mut self, cx: f32, cy: f32, containing_width: f32) {
        // 1. Resolve width
        self.width = self.resolve_width(containing_width);

        // 2. Position
        self.x = cx;
        self.y = cy;

        if self.children.is_empty() {
            self.height = 0.0;
            return;
        }

        let content_x = self.content_x();
        let content_y = self.content_y();
        let available_width = self.width;
        let gap = self.grid_gap;

        // Parse column template into concrete widths
        let col_count = if self.grid_template_columns.is_empty() {
            1
        } else {
            self.grid_template_columns.len()
        };

        let col_widths = if self.grid_template_columns.is_empty() {
            vec![available_width]
        } else {
            // Calculate fixed widths and total fr units
            let mut fixed_total: f32 = 0.0;
            let mut fr_total: f32 = 0.0;
            let gap_total = gap * (col_count as f32 - 1.0).max(0.0);
            for track in &self.grid_template_columns {
                match track {
                    GridTrack::Px(px) => fixed_total += px,
                    GridTrack::Fr(fr) => fr_total += fr,
                }
            }
            let remaining = (available_width - gap_total - fixed_total).max(0.0);
            let per_fr = if fr_total > 0.0 { remaining / fr_total } else { 0.0 };

            self.grid_template_columns.iter().map(|track| match track {
                GridTrack::Px(px) => *px,
                GridTrack::Fr(fr) => fr * per_fr,
            }).collect::<Vec<f32>>()
        };

        // Place children into grid cells
        let mut row_y = content_y;
        let num_children = self.children.len();
        let mut child_idx = 0;

        while child_idx < num_children {
            let mut row_height: f32 = 0.0;
            let mut col_x = content_x;

            for col in 0..col_count {
                if child_idx >= num_children {
                    break;
                }
                let col_w = col_widths[col];
                let child = &mut self.children[child_idx];
                child.layout(col_x, row_y, col_w, 0.0);
                row_height = row_height.max(child.margin_box_height());
                col_x += col_w;
                if col < col_count - 1 {
                    col_x += gap;
                }
                child_idx += 1;
            }

            row_y += row_height;
            if child_idx < num_children {
                row_y += gap;
            }
        }

        // Height
        self.height = match self.explicit_height {
            Dimension::Px(h) => h,
            Dimension::Auto => (row_y - content_y).max(0.0),
        };

        // Clamp to min-height
        if let Dimension::Px(min_h) = self.min_height {
            if self.height < min_h {
                self.height = min_h;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Inline layout
    // -----------------------------------------------------------------------
    fn layout_inline(&mut self, cx: f32, cy: f32, containing_width: f32) {
        self.x = cx;
        self.y = cy;

        // Inline elements: width = fit-content (not 100% of container)
        match self.explicit_width {
            Dimension::Px(w) => {
                self.width = w;
            }
            Dimension::Auto => {
                // Calculate content-based width (just children content, no self padding)
                // self.width is the content area; padding/border are in margin_box_width()
                let content_width = self.measure_inline_content_width();
                let max_content = (containing_width
                    - self.padding.left - self.padding.right
                    - self.border.left - self.border.right).max(0.0);
                self.width = content_width.min(max_content);
            }
        }

        if self.children.is_empty() {
            self.height = match self.explicit_height {
                Dimension::Px(h) => h,
                Dimension::Auto => self.computed_line_height(),
            };
            // Clamp to min-height
            if let Dimension::Px(min_h) = self.min_height {
                if self.height < min_h {
                    self.height = min_h;
                }
            }
            return;
        }

        // If we have inline children, lay them out horizontally
        self.layout_inline_children();

        // Height = max child height (not sum — inline children are horizontal)
        let max_child_height: f32 = self
            .children
            .iter()
            .map(|c| c.margin_box_height())
            .fold(0.0f32, |a, b| a.max(b));

        self.height = match self.explicit_height {
            Dimension::Px(h) => h,
            Dimension::Auto => {
                if max_child_height > 0.0 {
                    max_child_height
                } else {
                    self.computed_line_height()
                }
            }
        };

        // Clamp to min-height
        if let Dimension::Px(min_h) = self.min_height {
            if self.height < min_h {
                self.height = min_h;
            }
        }
    }

    /// Measure the natural content width of an inline element.
    fn measure_inline_content_width(&self) -> f32 {
        if self.children.is_empty() {
            return 0.0;
        }
        let mut total = 0.0;
        for child in &self.children {
            total += match &child.content {
                BoxContent::Text(s) => {
                    measure_text_width(s, child.font_size)
                        + child.margin.left + child.margin.right
                        + child.padding.left + child.padding.right
                        + child.border.left + child.border.right
                }
                _ => {
                    match child.explicit_width {
                        Dimension::Px(w) => w + child.margin.left + child.margin.right
                            + child.padding.left + child.padding.right
                            + child.border.left + child.border.right,
                        Dimension::Auto => child.measure_inline_content_width()
                            + child.padding.left + child.padding.right
                            + child.border.left + child.border.right
                            + child.margin.left + child.margin.right,
                    }
                }
            };
        }
        total
    }

    // -----------------------------------------------------------------------
    // Text layout with word wrapping
    // -----------------------------------------------------------------------
    /// Compute line height: use explicit line_height if set (>0), else font_size * factor.
    /// The 0.76 factor approximates the rendered glyph height from DejaVu Sans,
    /// providing consistent text layout matching the paint module's line spacing.
    fn computed_line_height(&self) -> f32 {
        if self.line_height > 0.0 {
            self.line_height
        } else {
            self.font_size * 0.76
        }
    }

    fn layout_text(&mut self, cx: f32, cy: f32, containing_width: f32) {
        let text = match &self.content {
            BoxContent::Text(s) => s.clone(),
            _ => return,
        };

        self.x = cx;
        self.y = cy;

        let line_height = self.computed_line_height();
        let font_size = self.font_size;

        // Word wrapping: split text into words and wrap at container boundary
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.is_empty() {
            self.width = 0.0;
            // Truly empty text (no characters at all) should have zero height;
            // whitespace-only text still occupies one line.
            self.height = if text.is_empty() { 0.0 } else { line_height };
            return;
        }

        let max_width = if self.white_space_nowrap || containing_width <= 0.0 { f32::MAX } else { containing_width };
        let mut current_line_width: f32 = 0.0;
        let mut max_line_width: f32 = 0.0;
        let mut line_count: u32 = 1;
        let space_width = measure_text_width(" ", font_size);

        // Preserve trailing whitespace for inline flow:
        // when text ends with space, add it after the last word so the
        // next sibling starts after the space.
        let has_trailing_space = text.ends_with(|c: char| c.is_whitespace());

        for (i, word) in words.iter().enumerate() {
            let word_width = measure_text_width(word, font_size);

            let needed = if i == 0 || current_line_width == 0.0 {
                word_width
            } else {
                space_width + word_width
            };

            if current_line_width + needed > max_width && current_line_width > 0.0 {
                // Wrap to next line
                max_line_width = max_line_width.max(current_line_width);
                current_line_width = word_width;
                line_count += 1;
            } else {
                if i > 0 && current_line_width > 0.0 {
                    current_line_width += space_width;
                }
                current_line_width += word_width;
            }
        }

        if has_trailing_space {
            current_line_width += space_width;
        }
        max_line_width = max_line_width.max(current_line_width);

        self.width = match self.explicit_width {
            Dimension::Px(w) => w,
            Dimension::Auto => max_line_width.min(max_width),
        };
        self.height = match self.explicit_height {
            Dimension::Px(h) => h,
            Dimension::Auto => line_count as f32 * line_height,
        };
    }

    // -----------------------------------------------------------------------
    // Image layout
    // -----------------------------------------------------------------------
    fn layout_image(&mut self, cx: f32, cy: f32, containing_width: f32) {
        self.x = cx;
        self.y = cy;

        // Use explicit width/height from HTML attributes (already set as explicit_width/height),
        // or fall back to intrinsic image dimensions, or default placeholder size.
        let (iw, ih) = match &self.content {
            BoxContent::Image { intrinsic_w, intrinsic_h, .. } if *intrinsic_w > 0 => {
                (*intrinsic_w as f32, *intrinsic_h as f32)
            }
            _ => (200.0, 150.0), // default placeholder
        };

        self.width = match self.explicit_width {
            Dimension::Px(w) => w,
            Dimension::Auto => iw,
        };

        self.height = match self.explicit_height {
            Dimension::Px(h) => h,
            Dimension::Auto => {
                // If width was explicitly set but height wasn't, scale proportionally
                if let Dimension::Px(w) = self.explicit_width {
                    if iw > 0.0 { w * ih / iw } else { ih }
                } else {
                    ih
                }
            }
        };
    }

    // -----------------------------------------------------------------------
    // Width resolution
    // -----------------------------------------------------------------------

    /// Resolve the content width of this box given the containing block width.
    fn resolve_width(&self, containing_width: f32) -> f32 {
        match self.explicit_width {
            Dimension::Px(w) => w,
            Dimension::Auto => {
                let used_horizontal = self.margin.left
                    + self.border.left
                    + self.padding.left
                    + self.padding.right
                    + self.border.right
                    + self.margin.right;
                (containing_width - used_horizontal).max(0.0)
            }
        }
    }

    fn has_explicit_width(&self) -> bool {
        matches!(self.explicit_width, Dimension::Px(_))
    }

    /// Measure the natural content width of a subtree (for flex auto-sizing).
    /// Walks children recursively to find text content and compute their width.
    fn measure_subtree_content_width(node: &LayoutBox) -> f32 {
        match &node.content {
            BoxContent::Text(s) => {
                measure_text_width(s, node.font_size)
                    + node.padding.left + node.padding.right
                    + node.border.left + node.border.right
            }
            BoxContent::Image { intrinsic_w, .. } => {
                let w = match node.explicit_width {
                    Dimension::Px(w) => w,
                    Dimension::Auto => if *intrinsic_w > 0 { *intrinsic_w as f32 } else { 200.0 },
                };
                w + node.padding.left + node.padding.right
                    + node.border.left + node.border.right
            }
            _ => {
                if let Dimension::Px(w) = node.explicit_width {
                    return w + node.padding.left + node.padding.right
                        + node.border.left + node.border.right;
                }
                // Sum children widths (for inline flow) or take max (for block flow)
                let children_width: f32 = node.children.iter()
                    .map(|c| Self::measure_subtree_content_width(c))
                    .fold(0.0f32, |a, b| a.max(b));
                if children_width > 0.0 {
                    children_width + node.padding.left + node.padding.right
                        + node.border.left + node.border.right
                } else {
                    0.0
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_block_has_zero_height() {
        let node = StyledNode::block(vec![]);
        let layout = layout_tree(&node, 800.0, 600.0);
        assert_eq!(layout.height, 0.0);
        assert_eq!(layout.width, 800.0);
    }

    #[test]
    fn single_text_child() {
        let node = StyledNode::block(vec![StyledNode::text("Hello")]);
        let layout = layout_tree(&node, 800.0, 600.0);
        assert!(layout.height > 0.0);
        assert_eq!(layout.children.len(), 1);
    }

    #[test]
    fn nested_blocks() {
        let inner = StyledNode::block(vec![StyledNode::text("A")]);
        let outer = StyledNode::block(vec![inner]);
        let layout = layout_tree(&outer, 400.0, 300.0);
        assert!(layout.height > 0.0);
    }

    #[test]
    fn display_none_produces_zero_box() {
        let mut node = StyledNode::block(vec![]);
        node.display = Display::None;
        let layout = layout_tree(&node, 800.0, 600.0);
        assert_eq!(layout.width, 0.0);
        assert_eq!(layout.height, 0.0);
    }

    #[test]
    fn padding_reduces_content_width() {
        let mut node = StyledNode::block(vec![]);
        node.padding = EdgeSizes::uniform(10.0);
        let layout = layout_tree(&node, 200.0, 100.0);
        // Content width = 200 - 10*2 (padding left+right) = 180
        assert!((layout.width - 180.0).abs() < 0.01);
    }

    #[test]
    fn children_stack_vertically() {
        let node = StyledNode::block(vec![
            StyledNode::text("Line 1"),
            StyledNode::text("Line 2"),
            StyledNode::text("Line 3"),
        ]);
        let layout = layout_tree(&node, 800.0, 600.0);
        let c = &layout.children;
        assert!(c[1].y > c[0].y);
        assert!(c[2].y > c[1].y);
    }

    #[test]
    fn debug_demo3_layout() {
        // Simulate Demo 3: html > body(margin:0,padding:10) > div(padding:8)
        // The header div should start at y=10 (body padding)
        let mut body = StyledNode::block(vec![
            {
                let mut header = StyledNode::block(vec![StyledNode::text("Test")]);
                header.padding = EdgeSizes { top: 8.0, right: 8.0, bottom: 8.0, left: 8.0 };
                header.margin = EdgeSizes { top: 0.0, right: 0.0, bottom: 10.0, left: 0.0 };
                header
            },
        ]);
        body.padding = EdgeSizes { top: 10.0, right: 10.0, bottom: 10.0, left: 10.0 };
        // body margin = 0

        // Wrap in html element (also a block)
        let html_elem = StyledNode::block(vec![body]);
        // Wrap in document root
        let doc = StyledNode::block(vec![html_elem]);

        let layout = layout_tree(&doc, 637.0, 600.0);
        // doc > html > body > header
        let html_box = &layout.children[0];
        let body_box = &html_box.children[0];
        let header_box = &body_box.children[0];

        eprintln!("doc: x={} y={} w={} h={}", layout.x, layout.y, layout.width, layout.height);
        eprintln!("html: x={} y={} w={} h={}", html_box.x, html_box.y, html_box.width, html_box.height);
        eprintln!("body: x={} y={} w={} h={} padding.top={}", body_box.x, body_box.y, body_box.width, body_box.height, body_box.padding.top);
        eprintln!("header: x={} y={} w={} h={}", header_box.x, header_box.y, header_box.width, header_box.height);

        // Header should be at y=10 (body padding)
        assert_eq!(header_box.y, 10.0, "Header should be at y=10 (body padding)");
    }
}
