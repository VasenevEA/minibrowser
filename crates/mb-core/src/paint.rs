//! Paint module -- renders a [`LayoutBox`] tree to an RGBA pixel buffer.
//!
//! Uses `tiny_skia` for rectangle fills and borders, and the built-in
//! [`BitmapFont`](crate::font::BitmapFont) for text rendering.

use tiny_skia::{FillRule, Mask, Paint, PathBuilder, Pixmap, PixmapMut, Transform};

use crate::font::FontduFont;
use crate::layout::{BoxContent, Color, LayoutBox, TextDecoration};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Render the layout tree into an RGBA pixel buffer (`width * height * 4` bytes).
pub fn paint(layout_root: &LayoutBox, width: u32, height: u32) -> Vec<u8> {
    let mut pixmap = Pixmap::new(width, height).expect("failed to create pixmap");

    // Fill canvas with root element's background color, or white as fallback.
    let bg = find_root_background(layout_root);
    pixmap.fill(bg);

    let font = FontduFont::shared();
    let clip = Clip { x0: 0, y0: 0, x1: width as i32, y1: height as i32 };
    paint_box(&mut pixmap.as_mut(), layout_root, font, clip);

    pixmap.take()
}

/// Repaints only `rect` (`[x0, y0, x1, y1)`) of an existing page buffer from the
/// layout tree; everything outside it is left as it is.
pub fn repaint(pixels: &mut [u8], width: u32, height: u32, layout_root: &LayoutBox, rect: [i32; 4]) {
    let Some(mut pm) = PixmapMut::from_bytes(pixels, width, height) else { return };
    let clip = Clip { x0: 0, y0: 0, x1: width as i32, y1: height as i32 }
        .intersect(Clip { x0: rect[0], y0: rect[1], x1: rect[2], y1: rect[3] });
    if clip.is_empty() {
        return;
    }
    let bg = find_root_background(layout_root);
    let mut paint = Paint::default();
    paint.set_color(bg);
    if let Some(r) = tiny_skia::Rect::from_ltrb(clip.x0 as f32, clip.y0 as f32, clip.x1 as f32, clip.y1 as f32) {
        pm.fill_rect(r, &paint, Transform::identity(), None);
    }
    paint_box(&mut pm, layout_root, FontduFont::shared(), clip);
}

/// The pixel rectangle `[x0, y0, x1, y1)` drawing may touch.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Clip {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl Clip {
    fn intersect(self, o: Clip) -> Clip {
        Clip { x0: self.x0.max(o.x0), y0: self.y0.max(o.y0), x1: self.x1.min(o.x1), y1: self.y1.min(o.y1) }
    }

    fn is_empty(self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }

    fn arr(self) -> [i32; 4] {
        [self.x0, self.y0, self.x1, self.y1]
    }

    /// Whether the clip covers the whole pixmap (no masking needed).
    fn covers(self, pm: &PixmapMut) -> bool {
        self.x0 <= 0 && self.y0 <= 0 && self.x1 >= pm.width() as i32 && self.y1 >= pm.height() as i32
    }

    /// Whether a box's subtree (approximated by `b`'s border box grown by
    /// `slack`) could draw inside the clip.
    fn touches(self, x: f32, y: f32, w: f32, h: f32) -> bool {
        !(x + w < self.x0 as f32 || y + h < self.y0 as f32 || x > self.x1 as f32 || y > self.y1 as f32)
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Walk the tree (up to 5 levels deep) looking for the first background color.
fn find_root_background(root: &LayoutBox) -> tiny_skia::Color {
    fn search(node: &LayoutBox, depth: u32) -> Option<tiny_skia::Color> {
        if depth > 5 { return None; }
        if let Some(c) = node.background_color {
            if c.a > 0 {
                return Some(tiny_skia::Color::from_rgba8(c.r, c.g, c.b, c.a));
            }
        }
        // Only follow first child path (html→body path)
        for child in &node.children {
            if let Some(c) = search(child, depth + 1) {
                return Some(c);
            }
        }
        None
    }
    search(root, 0).unwrap_or(tiny_skia::Color::WHITE)
}

fn paint_box(pixmap: &mut PixmapMut, layout_box: &LayoutBox, font: &FontduFont, clip: Clip) {
    if clip.is_empty() {
        return;
    }
    paint_background(pixmap, layout_box, clip);
    paint_borders(pixmap, layout_box, clip);
    paint_text(pixmap, layout_box, font, clip);
    paint_image(pixmap, layout_box, font, clip);

    // overflow: hidden clips the content to the padding box.
    let inner = if layout_box.overflow_hidden {
        let x = layout_box.x + layout_box.margin.left + layout_box.border.left;
        let y = layout_box.y + layout_box.margin.top + layout_box.border.top;
        let w = layout_box.padding.left + layout_box.width + layout_box.padding.right;
        let h = layout_box.padding.top + layout_box.height + layout_box.padding.bottom;
        clip.intersect(Clip { x0: x.floor() as i32, y0: y.floor() as i32, x1: (x + w).ceil() as i32, y1: (y + h).ceil() as i32 })
    } else {
        clip
    };
    for child in &layout_box.children {
        paint_box(pixmap, child, font, inner);
    }
}

// -- background -------------------------------------------------------------

fn paint_background(pixmap: &mut PixmapMut, layout_box: &LayoutBox, clip: Clip) {
    let color = match layout_box.background_color {
        Some(c) if c.a > 0 => c,
        _ => return,
    };

    let x = layout_box.x + layout_box.margin.left;
    let y = layout_box.y + layout_box.margin.top;
    let w = layout_box.border.left
        + layout_box.padding.left
        + layout_box.width
        + layout_box.padding.right
        + layout_box.border.right;
    let h = layout_box.border.top
        + layout_box.padding.top
        + layout_box.height
        + layout_box.padding.bottom
        + layout_box.border.bottom;

    fill_rect(pixmap, x, y, w, h, color, layout_box.border_radius, clip);
}

// -- borders ----------------------------------------------------------------

fn paint_borders(pixmap: &mut PixmapMut, layout_box: &LayoutBox, clip: Clip) {
    let b = &layout_box.border;
    if b.top == 0.0 && b.right == 0.0 && b.bottom == 0.0 && b.left == 0.0 {
        return;
    }

    // Use border-color if set, otherwise fall back to text color.
    let color = layout_box.border_color.unwrap_or(layout_box.text_color);

    let bx = layout_box.x + layout_box.margin.left;
    let by = layout_box.y + layout_box.margin.top;
    let outer_w = b.left
        + layout_box.padding.left
        + layout_box.width
        + layout_box.padding.right
        + b.right;
    let outer_h = b.top
        + layout_box.padding.top
        + layout_box.height
        + layout_box.padding.bottom
        + b.bottom;

    let side = |i: usize| layout_box.border_side_colors[i].unwrap_or(color);
    // Top border
    if b.top > 0.0 {
        fill_rect(pixmap, bx, by, outer_w, b.top, side(0), 0.0, clip);
    }
    // Bottom border
    if b.bottom > 0.0 {
        fill_rect(pixmap, bx, by + outer_h - b.bottom, outer_w, b.bottom, side(2), 0.0, clip);
    }
    // Left border
    if b.left > 0.0 {
        fill_rect(pixmap, bx, by, b.left, outer_h, side(3), 0.0, clip);
    }
    // Right border
    if b.right > 0.0 {
        fill_rect(pixmap, bx + outer_w - b.right, by, b.right, outer_h, side(1), 0.0, clip);
    }
}

// -- text -------------------------------------------------------------------

fn paint_text(pixmap: &mut PixmapMut, layout_box: &LayoutBox, font: &FontduFont, clip: Clip) {
    let text = match &layout_box.content {
        BoxContent::Text(s) => s,
        _ => return,
    };
    // Glyphs reach a little past the line box (descenders, italics, overhang).
    let slack = layout_box.font_size * 0.5;
    if !clip.touches(layout_box.x - slack, layout_box.y - slack, layout_box.margin_box_width() + 2.0 * slack, layout_box.margin_box_height() + 2.0 * slack) {
        return;
    }

    let color = layout_box.text_color;

    // Apply opacity
    let alpha = (color.a as f32 * layout_box.opacity) as u8;
    let rgba = [color.r, color.g, color.b, alpha];

    // Content area origin.
    let cx = layout_box.content_x();
    let cy = layout_box.content_y();
    let max_width = layout_box.width;
    let font_size = layout_box.font_size;

    let line_h_f = if layout_box.line_height > 0.0 {
        layout_box.line_height
    } else {
        font.line_metrics(font_size).2
    };
    // draw_text puts the baseline 0.8em below the y it is given.
    let baseline = font.baseline_in_line(font_size, line_h_f);
    let glyph_dy = (baseline - font_size * 0.8).round() as i32;

    let w = pixmap.width();
    let data = pixmap.data_mut();

    // Word-wrap rendering: split into words and draw with wrapping
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return;
    }

    let bold = layout_box.bold;
    let space_w = font.measure_text_in(" ", font_size, bold);

    // Build lines for word wrapping (matching layout logic)
    let effective_max_width = if layout_box.white_space_nowrap { f32::MAX } else { max_width };
    let mut lines: Vec<Vec<&str>> = Vec::new();
    let mut current_line: Vec<&str> = Vec::new();
    let mut current_line_width: f32 = 0.0;

    for word in &words {
        let word_width = font.measure_text_in(word, font_size, bold);
        let needed = if current_line_width == 0.0 {
            word_width
        } else {
            space_w + word_width
        };

        if current_line_width + needed > effective_max_width && current_line_width > 0.0 {
            lines.push(current_line);
            current_line = vec![word];
            current_line_width = word_width;
        } else {
            if !current_line.is_empty() {
                current_line_width += space_w;
            }
            current_line.push(word);
            current_line_width += word_width;
        }
    }
    if !current_line.is_empty() {
        lines.push(current_line);
    }

    // Render each line
    let mut line_top = cy;
    for line_words in &lines {
        let line_text = line_words.join(" ");
        let line_pixel_width = font.measure_text_in(&line_text, font_size, bold);

        // Apply text-align
        let draw_x = match layout_box.text_align {
            crate::layout::TextAlign::Center => {
                let offset = (max_width - line_pixel_width).max(0.0) / 2.0;
                cx as i32 + offset as i32
            }
            crate::layout::TextAlign::Right => {
                cx as i32 + (max_width - line_pixel_width).max(0.0) as i32
            }
            crate::layout::TextAlign::Left => cx as i32,
        };

        let draw_y = line_top.round() as i32 + glyph_dy;
        let baseline_y = (line_top + baseline).round() as i32;
        font.draw_text_in(data, w, draw_x, draw_y, &line_text, font_size, rgba, bold, clip.arr());

        // Draw text-decoration lines
        match layout_box.text_decoration {
            TextDecoration::Underline => {
                let line_y = baseline_y + (font_size * 0.1).max(1.0) as i32;
                draw_horizontal_line(data, w, draw_x, line_y, line_pixel_width as i32, rgba, clip);
            }
            TextDecoration::LineThrough => {
                let line_y = baseline_y - (font_size * 0.3) as i32;
                draw_horizontal_line(data, w, draw_x, line_y, line_pixel_width as i32, rgba, clip);
            }
            TextDecoration::None => {}
        }

        line_top += line_h_f;
    }
}

/// Draw a 1px horizontal line into the pixel buffer.
fn draw_horizontal_line(data: &mut [u8], buf_width: u32, x: i32, y: i32, length: i32, rgba: [u8; 4], clip: Clip) {
    if y < 0 || length <= 0 || y < clip.y0 || y >= clip.y1 {
        return;
    }
    let y = y as u32;
    let bw = buf_width;
    for dx in 0..length {
        if x + dx < clip.x0 || x + dx >= clip.x1 {
            continue;
        }
        let px = (x + dx) as u32;
        if px < bw {
            let idx = ((y * bw + px) * 4) as usize;
            if idx + 3 < data.len() {
                data[idx] = rgba[0];
                data[idx + 1] = rgba[1];
                data[idx + 2] = rgba[2];
                data[idx + 3] = rgba[3];
            }
        }
    }
}

// -- image ------------------------------------------------------------------

fn paint_image(pixmap: &mut PixmapMut, layout_box: &LayoutBox, font: &FontduFont, clip: Clip) {
    let (alt, img_data, intrinsic_w, intrinsic_h) = match &layout_box.content {
        BoxContent::Image { alt, data, intrinsic_w, intrinsic_h } => {
            (alt.as_str(), data, *intrinsic_w, *intrinsic_h)
        }
        _ => return,
    };

    let cx = layout_box.content_x();
    let cy = layout_box.content_y();
    let box_w = layout_box.width;
    let box_h = layout_box.height;

    if !img_data.is_empty() && intrinsic_w > 0 && intrinsic_h > 0 {
        // Blit decoded image data (nearest-neighbor scaling)
        let pw = pixmap.width();
        let ph = pixmap.height();
        let data = pixmap.data_mut();

        let dst_w = box_w as u32;
        let dst_h = box_h as u32;

        for dy in 0..dst_h {
            for dx in 0..dst_w {
                let px = cx as u32 + dx;
                let py = cy as u32 + dy;
                if px >= pw || py >= ph || (px as i32) < clip.x0 || (px as i32) >= clip.x1 || (py as i32) < clip.y0 || (py as i32) >= clip.y1 {
                    continue;
                }

                // Nearest-neighbor sample from source
                let sx = (dx as f32 * intrinsic_w as f32 / dst_w as f32) as u32;
                let sy = (dy as f32 * intrinsic_h as f32 / dst_h as f32) as u32;
                let sx = sx.min(intrinsic_w - 1);
                let sy = sy.min(intrinsic_h - 1);

                let src_idx = ((sy * intrinsic_w + sx) * 4) as usize;
                let dst_idx = ((py * pw + px) * 4) as usize;

                if src_idx + 3 < img_data.len() && dst_idx + 3 < data.len() {
                    let sa = img_data[src_idx + 3] as u32;
                    if sa == 255 {
                        data[dst_idx] = img_data[src_idx];
                        data[dst_idx + 1] = img_data[src_idx + 1];
                        data[dst_idx + 2] = img_data[src_idx + 2];
                        data[dst_idx + 3] = 255;
                    } else if sa > 0 {
                        // Alpha blend
                        let da = data[dst_idx + 3] as u32;
                        let inv_sa = 255 - sa;
                        data[dst_idx]     = ((img_data[src_idx] as u32 * sa + data[dst_idx] as u32 * inv_sa) / 255) as u8;
                        data[dst_idx + 1] = ((img_data[src_idx + 1] as u32 * sa + data[dst_idx + 1] as u32 * inv_sa) / 255) as u8;
                        data[dst_idx + 2] = ((img_data[src_idx + 2] as u32 * sa + data[dst_idx + 2] as u32 * inv_sa) / 255) as u8;
                        data[dst_idx + 3] = ((sa + da * inv_sa / 255).min(255)) as u8;
                    }
                }
            }
        }
    } else {
        // No image data: like browsers, show the alt text at the top left of the
        // box (the box's own background and border are painted as usual).
        if alt.is_empty() {
            return;
        }
        let font_size = layout_box.font_size.max(1.0);
        let c = layout_box.text_color;
        let rgba = [c.r, c.g, c.b, (c.a as f32 * layout_box.opacity) as u8];
        let glyph_dy = (font.baseline_in_line(font_size, font.line_metrics(font_size).2) - font_size * 0.8).round() as i32;
        let pw = pixmap.width();
        let data = pixmap.data_mut();
        font.draw_text_in(data, pw, cx as i32, cy as i32 + glyph_dy, alt, font_size, rgba, layout_box.bold, clip.arr());
    }
}

// -- helpers ----------------------------------------------------------------

/// Fill an axis-aligned rectangle using `tiny_skia`, with optional rounded corners.
#[allow(clippy::too_many_arguments)]
fn fill_rect(pixmap: &mut PixmapMut, x: f32, y: f32, w: f32, h: f32, color: Color, border_radius: f32, clip: Clip) {
    if w <= 0.0 || h <= 0.0 || !clip.touches(x, y, w, h) {
        return;
    }

    let r = border_radius.min(w / 2.0).min(h / 2.0);
    // A clip that misses all four corner squares sees a plain rectangle.
    let misses_corners = || {
        let c = |cx: f32, cy: f32| clip.touches(cx, cy, r, r);
        !(c(x, y) || c(x + w - r, y) || c(x, y + h - r) || c(x + w - r, y + h - r))
    };
    if r <= 0.0 || misses_corners() {
        // Square corners: tiny-skia's rectangle fast path, no path rasterizing.
        let mut paint = Paint::default();
        paint.set_color_rgba8(color.r, color.g, color.b, color.a);
        paint.anti_alias = false;
        let (x0, y0) = (x.max(clip.x0 as f32), y.max(clip.y0 as f32));
        let (x1, y1) = ((x + w).min(clip.x1 as f32), (y + h).min(clip.y1 as f32));
        if let Some(rect) = tiny_skia::Rect::from_ltrb(x0, y0, x1, y1) {
            pixmap.fill_rect(rect, &paint, Transform::identity(), None);
        }
        return;
    }
    let mut pb = PathBuilder::new();
    if r > 0.0 {
        pb.move_to(x + r, y);
        pb.line_to(x + w - r, y);
        pb.quad_to(x + w, y, x + w, y + r);
        pb.line_to(x + w, y + h - r);
        pb.quad_to(x + w, y + h, x + w - r, y + h);
        pb.line_to(x + r, y + h);
        pb.quad_to(x, y + h, x, y + h - r);
        pb.line_to(x, y + r);
        pb.quad_to(x, y, x + r, y);
    } else {
        pb.move_to(x, y);
        pb.line_to(x + w, y);
        pb.line_to(x + w, y + h);
        pb.line_to(x, y + h);
    }
    pb.close();

    let path = match pb.finish() {
        Some(p) => p,
        None => return,
    };

    let mut paint = Paint::default();
    paint.set_color_rgba8(color.r, color.g, color.b, color.a);
    // Rounded corners look like browsers' only when antialiased.
    paint.anti_alias = true;

    // A rounded box cut by the clip: mask the path fill to the clip rectangle.
    let inside = x >= clip.x0 as f32 && y >= clip.y0 as f32 && x + w <= clip.x1 as f32 && y + h <= clip.y1 as f32;
    let mask = if inside || clip.covers(pixmap) {
        None
    } else {
        let mut m = Mask::new(pixmap.width(), pixmap.height());
        if let (Some(m), Some(r)) = (m.as_mut(), tiny_skia::Rect::from_ltrb(clip.x0 as f32, clip.y0 as f32, clip.x1 as f32, clip.y1 as f32)) {
            m.fill_path(&PathBuilder::from_rect(r), FillRule::Winding, false, Transform::identity());
        }
        m
    };
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        mask.as_ref(),
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{self, EdgeSizes, StyledNode};

    #[test]
    fn paint_produces_correct_buffer_size() {
        let node = StyledNode::block(vec![]);
        let root = layout::layout_tree(&node, 100.0, 80.0);
        let buf = paint(&root, 100, 80);
        assert_eq!(buf.len(), 100 * 80 * 4);
    }

    #[test]
    fn paint_white_background_by_default() {
        let node = StyledNode::block(vec![]);
        let root = layout::layout_tree(&node, 4.0, 4.0);
        let buf = paint(&root, 4, 4);
        // Every pixel should be white (255,255,255,255).
        for chunk in buf.chunks(4) {
            assert_eq!(chunk, &[255, 255, 255, 255]);
        }
    }

    #[test]
    fn paint_with_background_color() {
        let mut node = StyledNode::block(vec![]);
        node.background_color = Some(Color::rgb(255, 0, 0));
        // Give it explicit area via padding (block with no children has zero
        // content height, but padding makes the border box non-zero).
        node.padding = EdgeSizes::uniform(5.0);
        let root = layout::layout_tree(&node, 20.0, 20.0);
        let buf = paint(&root, 20, 20);
        // At least some pixels should be red.
        let has_red = buf
            .chunks(4)
            .any(|px| px[0] == 255 && px[1] == 0 && px[2] == 0);
        assert!(has_red);
    }

    #[test]
    fn paint_text_leaves_pixels() {
        let node = StyledNode::block(vec![StyledNode::text("Hi")]);
        let root = layout::layout_tree(&node, 200.0, 100.0);
        let buf = paint(&root, 200, 100);
        // Should have some non-white pixels from the text.
        let has_non_white = buf
            .chunks(4)
            .any(|px| px[0] != 255 || px[1] != 255 || px[2] != 255);
        assert!(has_non_white);
    }
}
