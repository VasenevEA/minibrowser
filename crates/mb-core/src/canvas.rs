//! `<canvas>` 2D contexts: scripts' drawing calls arrive as commands and are
//! drawn into the canvas's own pixels (tiny-skia), which the page shows like
//! an image. A frame that only draws on canvases skips styling and layout.

use std::collections::HashMap;
use std::sync::Arc;

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

use crate::css::{parse_color, CSSValue};
use crate::font::FontduFont;

/// One drawing call: the method name, its numeric arguments, and the style,
/// text or font it needs.
#[derive(Debug, Clone, Default)]
pub struct Op {
    pub op: String,
    pub args: Vec<f32>,
    /// fillStyle or strokeStyle, whichever the call uses.
    pub style: String,
    /// fillText's text.
    pub text: String,
    pub font: String,
    pub line_width: f32,
    pub alpha: f32,
    pub align: String,
    /// putImageData's RGBA bytes.
    pub data: Vec<u8>,
    /// The JS host's handle for `data` when it came as binary.
    pub blob: u32,
}

pub struct Canvas {
    pixmap: Pixmap,
    path: Option<PathBuilder>,
    transform: Transform,
    saved: Vec<Transform>,
    /// Straight-alpha copy for the painter, made when the page next asks.
    image: Option<Arc<Vec<u8>>>,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Option<Canvas> {
        Some(Canvas { pixmap: Pixmap::new(w.max(1), h.max(1))?, path: None, transform: Transform::identity(), saved: Vec::new(), image: None })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.pixmap.width(), self.pixmap.height())
    }

    /// The canvas as straight-alpha RGBA, shared until it is drawn on again.
    pub fn image(&mut self) -> Arc<Vec<u8>> {
        self.image
            .get_or_insert_with(|| {
                let mut rgba = self.pixmap.data().to_vec();
                for p in rgba.chunks_exact_mut(4) {
                    let a = p[3] as u32;
                    if a != 0 && a != 255 {
                        for c in &mut p[..3] {
                            *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                        }
                    }
                }
                Arc::new(rgba)
            })
            .clone()
    }

    pub fn apply(&mut self, op: &Op) {
        self.image = None;
        let a = |i: usize| op.args.get(i).copied().unwrap_or(0.0);
        match op.op.as_str() {
            "fillRect" => {
                if let Some(r) = Rect::from_xywh(a(0), a(1), a(2), a(3)) {
                    self.pixmap.fill_rect(r, &paint(&op.style, op.alpha), self.transform, None);
                }
            }
            "strokeRect" => {
                if let Some(r) = Rect::from_xywh(a(0), a(1), a(2), a(3)) {
                    let path = PathBuilder::from_rect(r);
                    self.pixmap.stroke_path(&path, &paint(&op.style, op.alpha), &stroke(op.line_width), self.transform, None);
                }
            }
            "clearRect" => self.clear(a(0), a(1), a(2), a(3)),
            "beginPath" => self.path = Some(PathBuilder::new()),
            "moveTo" => self.path().move_to(a(0), a(1)),
            "lineTo" => {
                let p = self.path();
                if p.is_empty() {
                    p.move_to(a(0), a(1));
                } else {
                    p.line_to(a(0), a(1));
                }
            }
            "quadraticCurveTo" => self.path().quad_to(a(0), a(1), a(2), a(3)),
            "bezierCurveTo" => self.path().cubic_to(a(0), a(1), a(2), a(3), a(4), a(5)),
            "rect" => {
                if let Some(r) = Rect::from_xywh(a(0), a(1), a(2), a(3)) {
                    self.path().push_rect(r);
                }
            }
            "arc" => arc(self.path(), a(0), a(1), a(2), a(3), a(4), a(5) != 0.0),
            "closePath" => self.path().close(),
            "fill" | "stroke" => {
                let Some(path) = self.path.clone().and_then(|p| p.finish()) else { return };
                let paint = paint(&op.style, op.alpha);
                if op.op == "fill" {
                    self.pixmap.fill_path(&path, &paint, FillRule::Winding, self.transform, None);
                } else {
                    self.pixmap.stroke_path(&path, &paint, &stroke(op.line_width), self.transform, None);
                }
            }
            "fillText" => self.fill_text(op, a(0), a(1)),
            "save" => self.saved.push(self.transform),
            "restore" => self.transform = self.saved.pop().unwrap_or(self.transform),
            "translate" => self.transform = self.transform.pre_translate(a(0), a(1)),
            "rotate" => self.transform = self.transform.pre_concat(Transform::from_rotate(a(0).to_degrees())),
            "scale" => self.transform = self.transform.pre_scale(a(0), a(1)),
            "setTransform" => self.transform = Transform::from_row(a(0), a(1), a(2), a(3), a(4), a(5)),
            "resetTransform" => self.transform = Transform::identity(),
            "putImageData" => self.put(a(0) as i32, a(1) as i32, a(2) as u32, a(3) as u32, &op.data),
            _ => log::debug!("canvas: {} not supported", op.op),
        }
    }

    fn path(&mut self) -> &mut PathBuilder {
        self.path.get_or_insert_with(PathBuilder::new)
    }

    fn clear(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if self.transform.is_identity() {
            // The common "clear the whole canvas": no blending.
            let (cw, ch) = self.size();
            let (x0, y0) = (x.max(0.0) as u32, y.max(0.0) as u32);
            let (x1, y1) = (((x + w).max(0.0) as u32).min(cw), ((y + h).max(0.0) as u32).min(ch));
            let data = self.pixmap.data_mut();
            for row in y0..y1 {
                data[((row * cw + x0) * 4) as usize..((row * cw + x1.max(x0)) * 4) as usize].fill(0);
            }
            return;
        }
        if let Some(r) = Rect::from_xywh(x, y, w, h) {
            let mut p = Paint::default();
            p.set_color_rgba8(0, 0, 0, 0);
            p.blend_mode = tiny_skia::BlendMode::Clear;
            self.pixmap.fill_rect(r, &p, self.transform, None);
        }
    }

    fn fill_text(&mut self, op: &Op, x: f32, y: f32) {
        let (size, bold) = parse_font(&op.font);
        let font = FontduFont::shared();
        let (tx, ty) = (self.transform.tx, self.transform.ty);
        let width = font.measure_text_in(&op.text, size, bold);
        let x = x + tx - match op.align.as_str() {
            "center" => width / 2.0,
            "right" | "end" => width,
            _ => 0.0,
        };
        // fillText's y is the baseline; glyphs are drawn from their box top.
        let top = y + ty - size * 0.8;
        let [r, g, b, a] = color(&op.style, op.alpha);
        let (w, h) = self.size();
        let data = self.pixmap.data_mut();
        font.draw_text_in(data, w, x as i32, top.round() as i32, &op.text, size, [r, g, b, a], bold, [0, 0, w as i32, h as i32]);
    }

    fn put(&mut self, x: i32, y: i32, w: u32, h: u32, src: &[u8]) {
        let (cw, ch) = self.size();
        let data = self.pixmap.data_mut();
        for row in 0..h as i32 {
            let dy = y + row;
            if dy < 0 || dy >= ch as i32 {
                continue;
            }
            for col in 0..w as i32 {
                let dx = x + col;
                if dx < 0 || dx >= cw as i32 {
                    continue;
                }
                let s = ((row as u32 * w + col as u32) * 4) as usize;
                let Some(px) = src.get(s..s + 4) else { return };
                let d = ((dy as u32 * cw + dx as u32) * 4) as usize;
                // Stored premultiplied, as tiny-skia keeps it.
                let a = px[3] as u32;
                let pm = |c: u8| ((c as u32 * a + 127) / 255) as u8;
                data[d..d + 4].copy_from_slice(&[pm(px[0]), pm(px[1]), pm(px[2]), px[3]]);
            }
        }
    }
}

/// The canvases of a page, by element id.
#[derive(Default)]
pub struct Canvases {
    map: HashMap<String, Canvas>,
}

impl Canvases {
    pub fn apply(&mut self, id: &str, size: (u32, u32), op: &Op) {
        let c = match self.map.get_mut(id) {
            Some(c) if c.size() == size => c,
            _ => {
                let Some(c) = Canvas::new(size.0, size.1) else { return };
                self.map.insert(id.to_string(), c);
                self.map.get_mut(id).unwrap()
            }
        };
        c.apply(op);
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Canvas> {
        self.map.get_mut(id)
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
}

fn color(style: &str, alpha: f32) -> [u8; 4] {
    match parse_color(style) {
        Some(CSSValue::Color(r, g, b, a)) => [r, g, b, (a as f32 * alpha.clamp(0.0, 1.0)) as u8],
        _ => [0, 0, 0, (255.0 * alpha.clamp(0.0, 1.0)) as u8],
    }
}

fn paint(style: &str, alpha: f32) -> Paint<'static> {
    let [r, g, b, a] = color(style, alpha);
    let mut p = Paint::default();
    p.set_color_rgba8(r, g, b, a);
    p.anti_alias = true;
    p
}

fn stroke(width: f32) -> Stroke {
    Stroke { width: if width > 0.0 { width } else { 1.0 }, ..Stroke::default() }
}

/// "bold 16px sans-serif" → (16, true).
fn parse_font(font: &str) -> (f32, bool) {
    let size = font
        .split_whitespace()
        .find_map(|w| w.strip_suffix("px").and_then(|n| n.parse::<f32>().ok()))
        .unwrap_or(10.0);
    let bold = font.split_whitespace().any(|w| w == "bold" || w.parse::<u32>().is_ok_and(|n| n >= 600));
    (size, bold)
}

/// Canvas arc(): from angle a0 to a1 (radians, clockwise unless `ccw`), joined
/// to the current point by a line, as browsers do. Drawn as quarter-or-less cubic pieces.
fn arc(p: &mut PathBuilder, cx: f32, cy: f32, r: f32, a0: f32, a1: f32, ccw: bool) {
    use std::f32::consts::TAU;
    let mut sweep = a1 - a0;
    if !ccw && sweep < 0.0 {
        sweep = sweep.rem_euclid(TAU);
        if sweep == 0.0 && a1 != a0 { sweep = TAU }
    } else if ccw && sweep > 0.0 {
        sweep = -(-sweep).rem_euclid(TAU);
        if sweep == 0.0 && a1 != a0 { sweep = -TAU }
    }
    if !ccw && (a1 - a0) >= TAU { sweep = TAU }
    if ccw && (a0 - a1) >= TAU { sweep = -TAU }
    let start = (cx + r * a0.cos(), cy + r * a0.sin());
    if p.is_empty() {
        p.move_to(start.0, start.1);
    } else {
        p.line_to(start.0, start.1);
    }
    let pieces = (sweep.abs() / (std::f32::consts::FRAC_PI_2)).ceil().max(1.0) as usize;
    let step = sweep / pieces as f32;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let mut a = a0;
    for _ in 0..pieces {
        let b = a + step;
        let (ca, sa, cb, sb) = (a.cos(), a.sin(), b.cos(), b.sin());
        p.cubic_to(
            cx + r * (ca - k * sa), cy + r * (sa + k * ca),
            cx + r * (cb + k * sb), cy + r * (sb - k * cb),
            cx + r * cb, cy + r * sb,
        );
        a = b;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(name: &str, args: &[f32], style: &str) -> Op {
        Op { op: name.into(), args: args.to_vec(), style: style.into(), line_width: 1.0, alpha: 1.0, ..Default::default() }
    }

    #[test]
    fn fill_clear_and_put() {
        let mut cs = Canvases::default();
        cs.apply("c", (10, 10), &op("fillRect", &[0.0, 0.0, 10.0, 10.0], "#ff0000"));
        cs.apply("c", (10, 10), &op("clearRect", &[0.0, 0.0, 5.0, 10.0], ""));
        let put = Op { data: vec![0, 0, 255, 255], ..op("putImageData", &[9.0, 9.0, 1.0, 1.0], "") };
        cs.apply("c", (10, 10), &put);
        let img = cs.get_mut("c").unwrap().image();
        let px = |x: usize, y: usize| &img[(y * 10 + x) * 4..(y * 10 + x) * 4 + 4];
        assert_eq!(px(1, 1), [0, 0, 0, 0]);
        assert_eq!(px(7, 1), [255, 0, 0, 255]);
        assert_eq!(px(9, 9), [0, 0, 255, 255]);
    }

    #[test]
    fn transformed_path_fill() {
        let mut cs = Canvases::default();
        for o in [op("translate", &[5.0, 5.0], ""), op("beginPath", &[], ""), op("rect", &[-2.0, -2.0, 4.0, 4.0], ""), op("fill", &[], "#00ff00")] {
            cs.apply("c", (10, 10), &o);
        }
        let img = cs.get_mut("c").unwrap().image();
        assert_eq!(&img[(5 * 10 + 5) * 4..(5 * 10 + 5) * 4 + 4], [0, 255, 0, 255]);
        assert_eq!(img[3], 0);
    }
}
