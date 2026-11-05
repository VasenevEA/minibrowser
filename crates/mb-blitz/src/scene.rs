//! anyrender backend on tiny-skia: Blitz paints its tree through this.
//!
//! It paints one region of the page (the damaged rect, or the whole screen)
//! into a pixmap of that size: everything outside the region is culled before
//! rasterizing, so a small change costs little. Text comes from a per-glyph
//! coverage cache, blended by hand: rasterizing glyph outlines every frame is
//! too slow on a Cortex-A7.

use std::collections::HashMap;
use std::sync::Arc;

use anyrender::{Filter, Glyph, NormalizedCoord, Paint, PaintRef, PaintScene, RenderContext};
use kurbo::{Affine, PathEl, Rect, Shape, Stroke};
use peniko::{BlendMode, Color, Compose, Extend, Fill, FontData, GradientKind, ImageAlphaType, ImageFormat, Mix, StyleRef};
use skrifa::{instance::LocationRef, outline::DrawSettings, prelude::Size, GlyphId, MetadataProvider};
use tiny_skia as sk;

const TOLERANCE: f64 = 0.1;

/// One entry of the layer stack: a clip (as a mask over the region, or None
/// when it clips nothing there) and, for a compositing layer, its own pixmap.
struct Layer {
    mask: Option<sk::Mask>,
    /// It pushed a pixmap on `surfaces`.
    compositing: bool,
    /// Region pixels its clip can touch (what is composited on pop).
    bbox: Option<sk::IntRect>,
    alpha: f32,
    blend: sk::BlendMode,
    /// Nothing of this layer can show (its clip misses the region).
    empty: bool,
}

/// Coverage of one glyph at one size and subpixel offset, `left/top` from the
/// pen position.
struct GlyphMask {
    left: i32,
    top: i32,
    w: u32,
    h: u32,
    cov: Vec<u8>,
}

#[derive(Hash, PartialEq, Eq, Clone, Copy)]
struct GlyphKey {
    font: u64,
    index: u32,
    glyph: u32,
    size: u32,
    /// Quarter-pixel horizontal offset.
    sub: u8,
}

pub struct TinySkiaScene {
    /// The region of the page being painted, in page pixels.
    pub region: sk::IntRect,
    pub pixmap: sk::Pixmap,
    layers: Vec<Layer>,
    /// Pixmaps of the open compositing layers, innermost last.
    surfaces: Vec<sk::Pixmap>,
    glyphs: HashMap<GlyphKey, Option<GlyphMask>>,
    images: HashMap<u64, sk::Pixmap>,
}

impl TinySkiaScene {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            region: sk::IntRect::from_xywh(0, 0, width, height).unwrap(),
            pixmap: sk::Pixmap::new(width, height).unwrap(),
            layers: Vec::new(),
            surfaces: Vec::new(),
            glyphs: HashMap::new(),
            images: HashMap::new(),
        }
    }

    /// Start painting `region` (page pixels) on a cleared pixmap of its size.
    pub fn begin(&mut self, region: sk::IntRect) {
        if self.pixmap.width() != region.width() || self.pixmap.height() != region.height() {
            self.pixmap = sk::Pixmap::new(region.width(), region.height()).unwrap();
        } else {
            self.pixmap.fill(sk::Color::TRANSPARENT);
        }
        self.region = region;
        self.layers.clear();
        self.surfaces.clear();
        if self.glyphs.len() > 4096 {
            self.glyphs.clear();
        }
    }

    /// Paints recorded commands (a recorded scene, without consuming it).
    pub fn replay(&mut self, cmds: &[anyrender::recording::RenderCommand]) {
        use anyrender::recording::RenderCommand as C;
        let profile = std::env::var_os("MB_PROFILE").is_some();
        let mut spent = [0f64; 7];
        for cmd in cmds {
            let t = std::time::Instant::now();
            let kind = match cmd {
                C::PushLayer(_) => 0,
                C::PushClipLayer(_) => 1,
                C::PopLayer => 2,
                C::Stroke(_) => 3,
                C::Fill(_) => 4,
                C::GlyphRun(_) => 5,
                C::BoxShadow(_) => 6,
            };
            match cmd {
                C::PushLayer(c) => self.push_layer(c.fill, c.blend, c.alpha, c.transform, &c.clip, None, None),
                C::PushClipLayer(c) => self.push_clip_layer(c.fill, c.transform, &c.clip),
                C::PopLayer => self.pop_layer(),
                C::Stroke(c) => self.stroke(&c.style, c.transform, &c.brush, c.brush_transform, &c.shape),
                C::Fill(c) => self.fill(c.fill, c.transform, &c.brush, c.brush_transform, &c.shape),
                C::GlyphRun(c) => self.draw_glyphs(
                    &c.font_data,
                    c.font_size,
                    c.hint,
                    &c.normalized_coords,
                    c.embolden,
                    &c.style,
                    &c.brush,
                    c.brush_alpha,
                    c.transform,
                    c.glyph_transform,
                    c.glyphs.iter().copied(),
                ),
                C::BoxShadow(c) => self.draw_box_shadow(c.transform, c.rect, c.brush, c.radius, c.std_dev),
            }
            if profile {
                spent[kind] += t.elapsed().as_secs_f64() * 1e3;
            }
        }
        if profile {
            eprintln!(
                "replay ms: layers {:.2} clips {:.2} pops {:.2} strokes {:.2} fills {:.2} text {:.2} shadows {:.2}",
                spent[0], spent[1], spent[2], spent[3], spent[4], spent[5], spent[6]
            );
        }
    }

    /// Page transform to region pixels.
    fn to_region(&self, t: Affine) -> sk::Transform {
        let [a, b, c, d, e, f] = t.as_coeffs();
        sk::Transform::from_row(a as f32, b as f32, c as f32, d as f32, e as f32 - self.region.x() as f32, f as f32 - self.region.y() as f32)
    }

    fn region_rect(&self) -> Rect {
        let r = self.region;
        Rect::new(r.x() as f64, r.y() as f64, r.right() as f64, r.bottom() as f64)
    }

    /// Whether a shape drawn with `t` can touch the region at all.
    fn visible(&self, t: Affine, bounds: Rect, pad: f64) -> bool {
        let b = t.transform_rect_bbox(bounds).inflate(pad, pad);
        b.intersect(self.region_rect()).area() > 0.0 || (b.width() == 0.0 || b.height() == 0.0) && b.overlaps(self.region_rect())
    }

    fn top_empty(&self) -> bool {
        self.layers.last().is_some_and(|l| l.empty)
    }

    fn mask(&self) -> Option<&sk::Mask> {
        self.layers.last().and_then(|l| l.mask.as_ref())
    }


    /// Clip for a new layer: the parent's, narrowed by `clip`.
    fn clip_layer(&self, fill: Fill, transform: Affine, clip: &impl Shape) -> (Option<sk::Mask>, bool) {
        let parent = self.mask().cloned();
        if self.top_empty() {
            return (None, true);
        }
        let bounds = clip.bounding_box();
        if !self.visible(transform, bounds, 0.0) {
            return (None, true);
        }
        // An axis-aligned rect around the whole region clips nothing.
        if let Some(r) = clip.as_rect() {
            let [a, b, c, d, _, _] = transform.as_coeffs();
            if b == 0.0 && c == 0.0 && a > 0.0 && d > 0.0 {
                let r = transform.transform_rect_bbox(r);
                let reg = self.region_rect();
                if r.x0 <= reg.x0 && r.y0 <= reg.y0 && r.x1 >= reg.x1 && r.y1 >= reg.y1 {
                    return (parent, false);
                }
            }
        }
        let Some(path) = to_path(clip) else { return (None, true) };
        let t = self.to_region(transform);
        let mask = match parent {
            Some(mut m) => {
                m.intersect_path(&path, fill_rule(fill), true, t);
                m
            }
            None => {
                let mut m = sk::Mask::new(self.region.width(), self.region.height()).unwrap();
                m.fill_path(&path, fill_rule(fill), true, t);
                m
            }
        };
        (Some(mask), false)
    }

    /// The brush as a shader, and where to draw it: (shader, pixmap, clip).
    fn paint<'a>(
        &'a mut self,
        paint: PaintRef<'_>,
        brush_transform: Option<Affine>,
        transform: Affine,
    ) -> Option<(sk::Shader<'a>, &'a mut sk::Pixmap, Option<&'a sk::Mask>)> {
        // Shaders live in the shape's user space: tiny-skia applies the
        // shape's transform to them as well, so only the brush's own here.
        let _ = transform;
        let [a, b, c, d, e, f] = brush_transform.unwrap_or(Affine::IDENTITY).as_coeffs();
        let bt = sk::Transform::from_row(a as f32, b as f32, c as f32, d as f32, e as f32, f as f32);
        let shader = shader(&mut self.images, paint, bt)?;
        let mask = self.layers.last().and_then(|l| l.mask.as_ref());
        let pixmap = self.surfaces.last_mut().unwrap_or(&mut self.pixmap);
        Some((shader, pixmap, mask))
    }
}

fn shader<'a>(images: &'a mut HashMap<u64, sk::Pixmap>, paint: PaintRef<'_>, bt: sk::Transform) -> Option<sk::Shader<'a>> {
    {
        match paint {
            Paint::Solid(c) => Some(sk::Shader::SolidColor(color(c))),
            Paint::Gradient(g) => {
                let stops: Vec<_> = g
                    .stops
                    .iter()
                    .map(|s| sk::GradientStop::new(s.offset, color(s.color.to_alpha_color::<peniko::color::Srgb>())))
                    .collect();
                let first = stops.first().map(|_| color(g.stops[0].color.to_alpha_color::<peniko::color::Srgb>()));
                let mode = match g.extend {
                    Extend::Pad => sk::SpreadMode::Pad,
                    Extend::Repeat => sk::SpreadMode::Repeat,
                    Extend::Reflect => sk::SpreadMode::Reflect,
                };
                let shader = match g.kind {
                    GradientKind::Linear(p) => sk::LinearGradient::new(pt(p.start), pt(p.end), stops, mode, bt),
                    GradientKind::Radial(p) => {
                        sk::RadialGradient::new(pt(p.start_center), p.start_radius, pt(p.end_center), p.end_radius, stops, mode, bt)
                    }
                    // No sweep gradients in tiny-skia: the first color.
                    GradientKind::Sweep(_) => None,
                };
                shader.or(first.map(sk::Shader::SolidColor))
            }
            Paint::Image(img) => {
                let data = img.image;
                let key = data.data.id();
                if !images.contains_key(&key) {
                    images.insert(key, image_pixmap(data)?);
                }
                let quality = match img.sampler.quality {
                    peniko::ImageQuality::Low => sk::FilterQuality::Nearest,
                    _ => sk::FilterQuality::Bilinear,
                };
                let mode = match img.sampler.x_extend {
                    Extend::Repeat => sk::SpreadMode::Repeat,
                    Extend::Reflect => sk::SpreadMode::Reflect,
                    Extend::Pad => sk::SpreadMode::Pad,
                };
                let p = images.get(&key)?;
                Some(sk::Pattern::new(p.as_ref(), mode, quality, img.sampler.alpha, bt))
            }
            Paint::Resource(_) | Paint::Custom(_) => None,
        }
    }
}

fn pt(p: kurbo::Point) -> sk::Point {
    sk::Point::from_xy(p.x as f32, p.y as f32)
}

fn color(c: Color) -> sk::Color {
    let [r, g, b, a] = c.components;
    sk::Color::from_rgba(r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0), a.clamp(0.0, 1.0)).unwrap_or(sk::Color::BLACK)
}

fn fill_rule(f: Fill) -> sk::FillRule {
    match f {
        Fill::NonZero => sk::FillRule::Winding,
        Fill::EvenOdd => sk::FillRule::EvenOdd,
    }
}

fn blend_mode(b: BlendMode) -> sk::BlendMode {
    use sk::BlendMode as B;
    match b.mix {
        Mix::Normal => match b.compose {
            Compose::Clear => B::Clear,
            Compose::Copy => B::Source,
            Compose::Dest => B::Destination,
            Compose::SrcOver => B::SourceOver,
            Compose::DestOver => B::DestinationOver,
            Compose::SrcIn => B::SourceIn,
            Compose::DestIn => B::DestinationIn,
            Compose::SrcOut => B::SourceOut,
            Compose::DestOut => B::DestinationOut,
            Compose::SrcAtop => B::SourceAtop,
            Compose::DestAtop => B::DestinationAtop,
            Compose::Xor => B::Xor,
            Compose::Plus => B::Plus,
            Compose::PlusLighter => B::Plus,
        },
        Mix::Multiply => B::Multiply,
        Mix::Screen => B::Screen,
        Mix::Overlay => B::Overlay,
        Mix::Darken => B::Darken,
        Mix::Lighten => B::Lighten,
        Mix::ColorDodge => B::ColorDodge,
        Mix::ColorBurn => B::ColorBurn,
        Mix::HardLight => B::HardLight,
        Mix::SoftLight => B::SoftLight,
        Mix::Difference => B::Difference,
        Mix::Exclusion => B::Exclusion,
        Mix::Hue => B::Hue,
        Mix::Saturation => B::Saturation,
        Mix::Color => B::Color,
        Mix::Luminosity => B::Luminosity,
        #[allow(unreachable_patterns)]
        _ => B::SourceOver,
    }
}

/// A path that is an axis-aligned rect (as recorded scenes store rects).
fn path_rect(shape: &impl Shape) -> Option<Rect> {
    let mut pts = [kurbo::Point::ZERO; 4];
    let mut n = 0;
    for (i, el) in shape.path_elements(TOLERANCE).enumerate() {
        match (i, el) {
            (0, PathEl::MoveTo(p)) | (1..=3, PathEl::LineTo(p)) => {
                pts[n] = p;
                n += 1;
            }
            (4, PathEl::ClosePath) | (4, PathEl::LineTo(_)) | (5, PathEl::ClosePath) => {}
            _ => return None,
        }
    }
    if n != 4 {
        return None;
    }
    let [a, b, c, d] = pts;
    let horiz_first = a.y == b.y && b.x == c.x && c.y == d.y && d.x == a.x;
    let vert_first = a.x == b.x && b.y == c.y && c.x == d.x && d.y == a.y;
    (horiz_first || vert_first).then(|| Rect::from_points(a, c))
}

fn to_path(shape: &impl Shape) -> Option<sk::Path> {
    let mut pb = sk::PathBuilder::new();
    for el in shape.path_elements(TOLERANCE) {
        match el {
            PathEl::MoveTo(p) => pb.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => pb.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(a, p) => pb.quad_to(a.x as f32, a.y as f32, p.x as f32, p.y as f32),
            PathEl::CurveTo(a, b, p) => pb.cubic_to(a.x as f32, a.y as f32, b.x as f32, b.y as f32, p.x as f32, p.y as f32),
            PathEl::ClosePath => pb.close(),
        }
    }
    pb.finish()
}

/// Premultiplied RGBA pixmap of a peniko image.
fn image_pixmap(img: &peniko::ImageData) -> Option<sk::Pixmap> {
    let mut p = sk::Pixmap::new(img.width, img.height)?;
    let src = img.data.data();
    for (d, s) in p.data_mut().chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let (r, g, b, a) = match img.format {
            ImageFormat::Rgba8 => (s[0], s[1], s[2], s[3]),
            ImageFormat::Bgra8 => (s[2], s[1], s[0], s[3]),
            #[allow(unreachable_patterns)]
            _ => (s[0], s[1], s[2], s[3]),
        };
        if img.alpha_type == ImageAlphaType::AlphaPremultiplied || a == 255 {
            d.copy_from_slice(&[r, g, b, a]);
        } else {
            let m = |c: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
            d.copy_from_slice(&[m(r), m(g), m(b), a]);
        }
    }
    Some(p)
}

/// Outline pen building a tiny-skia path, font units scaled, y down.
struct Pen(sk::PathBuilder);

impl skrifa::outline::OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, -y)
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, -y)
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.0.quad_to(cx, -cy, x, -y)
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.cubic_to(cx0, -cy0, cx1, -cy1, x, -y)
    }
    fn close(&mut self) {
        self.0.close()
    }
}

fn rasterize_glyph(font: &FontData, coords: &[NormalizedCoord], glyph: u32, size: f32, sub: f32) -> Option<GlyphMask> {
    let f = skrifa::FontRef::from_index(font.data.data(), font.index).ok()?;
    let outline = f.outline_glyphs().get(GlyphId::new(glyph))?;
    let mut pen = Pen(sk::PathBuilder::new());
    let coords: Vec<_> = coords.iter().map(|c| skrifa::raw::types::F2Dot14::from_bits(*c)).collect();
    let loc = LocationRef::new(&coords);
    outline.draw(DrawSettings::unhinted(Size::new(size), loc), &mut pen).ok()?;
    let path = pen.0.finish()?;
    let b = path.bounds();
    let left = (b.left() + sub).floor() as i32 - 1;
    let top = b.top().floor() as i32 - 1;
    let w = ((b.right() + sub).ceil() as i32 + 1 - left) as u32;
    let h = (b.bottom().ceil() as i32 + 1 - top) as u32;
    let mut m = sk::Mask::new(w.max(1), h.max(1))?;
    m.fill_path(&path, sk::FillRule::Winding, true, sk::Transform::from_translate(sub - left as f32, -top as f32));
    Some(GlyphMask { left, top, w, h, cov: m.take() })
}

/// Blend `cov * color` into premultiplied `dst` at (x, y), through the clip mask.
fn blend_coverage(dst: &mut sk::Pixmap, clip: Option<&sk::Mask>, g: &GlyphMask, x: i32, y: i32, c: sk::PremultipliedColorU8) {
    let (dw, dh) = (dst.width() as i32, dst.height() as i32);
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + g.w as i32).min(dw);
    let y1 = (y + g.h as i32).min(dh);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let (cr, cg, cb, ca) = (c.red() as u32, c.green() as u32, c.blue() as u32, c.alpha() as u32);
    let data = dst.data_mut();
    for py in y0..y1 {
        let row = ((py - y) as u32 * g.w) as usize;
        for px in x0..x1 {
            let mut k = g.cov[row + (px - x) as usize] as u32;
            if k == 0 {
                continue;
            }
            if let Some(m) = clip {
                k = k * m.data()[(py * dw + px) as usize] as u32 / 255;
                if k == 0 {
                    continue;
                }
            }
            let i = ((py * dw + px) * 4) as usize;
            let sa = ca * k / 255;
            let inv = 255 - sa;
            let d = &mut data[i..i + 4];
            d[0] = ((cr * k / 255) + d[0] as u32 * inv / 255) as u8;
            d[1] = ((cg * k / 255) + d[1] as u32 * inv / 255) as u8;
            d[2] = ((cb * k / 255) + d[2] as u32 * inv / 255) as u8;
            d[3] = (sa + d[3] as u32 * inv / 255) as u8;
        }
    }
}

impl RenderContext for TinySkiaScene {}

impl PaintScene for TinySkiaScene {
    fn reset(&mut self) {
        self.layers.clear();
    }

    fn push_layer(
        &mut self,
        fill: Fill,
        blend: impl Into<BlendMode>,
        alpha: f32,
        transform: Affine,
        clip: &impl Shape,
        _filter: Option<Arc<Filter>>,
        _backdrop_filter: Option<Arc<Filter>>,
    ) {
        let (mask, empty) = self.clip_layer(fill, transform, clip);
        let blend = blend_mode(blend.into());
        let compositing = !empty && (alpha < 1.0 || blend != sk::BlendMode::SourceOver);
        if compositing {
            self.surfaces.push(sk::Pixmap::new(self.region.width(), self.region.height()).unwrap());
        }
        let b = transform.transform_rect_bbox(clip.bounding_box());
        let r = self.region;
        let bbox = sk::IntRect::from_ltrb(
            (b.x0.floor() as i32 - r.x() - 1).max(0),
            (b.y0.floor() as i32 - r.y() - 1).max(0),
            (b.x1.ceil() as i32 - r.x() + 1).min(r.width() as i32),
            (b.y1.ceil() as i32 - r.y() + 1).min(r.height() as i32),
        );
        self.layers.push(Layer { mask, compositing, bbox, alpha, blend, empty });
    }

    fn push_clip_layer(&mut self, fill: Fill, transform: Affine, clip: &impl Shape) {
        let (mask, empty) = self.clip_layer(fill, transform, clip);
        self.layers.push(Layer { mask, compositing: false, bbox: None, alpha: 1.0, blend: sk::BlendMode::SourceOver, empty });
    }

    fn pop_layer(&mut self) {
        let Some(layer) = self.layers.pop() else { return };
        if layer.compositing {
            let Some(p) = self.surfaces.pop() else { return };
            let paint = sk::PixmapPaint { opacity: layer.alpha, blend_mode: layer.blend, quality: sk::FilterQuality::Nearest };
            let dst = self.surfaces.last_mut().unwrap_or(&mut self.pixmap);
            // Only the part the layer's clip covers.
            match layer.bbox.and_then(|b| p.clone_rect(b).map(|c| (b, c))) {
                Some((b, part)) => dst.draw_pixmap(b.x(), b.y(), part.as_ref(), &paint, sk::Transform::identity(), None),
                None => dst.draw_pixmap(0, 0, p.as_ref(), &paint, sk::Transform::identity(), None),
            }
        }
    }

    fn stroke<'a>(&mut self, style: &Stroke, transform: Affine, brush: impl Into<PaintRef<'a>>, brush_transform: Option<Affine>, shape: &impl Shape) {
        if self.top_empty() || !self.visible(transform, shape.bounding_box(), style.width) {
            return;
        }
        let Some(path) = to_path(shape) else { return };
        let t = self.to_region(transform);
        let mut stroke = sk::Stroke {
            width: style.width as f32,
            miter_limit: style.miter_limit as f32,
            line_cap: match style.start_cap {
                kurbo::Cap::Butt => sk::LineCap::Butt,
                kurbo::Cap::Round => sk::LineCap::Round,
                kurbo::Cap::Square => sk::LineCap::Square,
            },
            line_join: match style.join {
                kurbo::Join::Bevel => sk::LineJoin::Bevel,
                kurbo::Join::Miter => sk::LineJoin::Miter,
                kurbo::Join::Round => sk::LineJoin::Round,
            },
            dash: None,
        };
        if !style.dash_pattern.is_empty() {
            stroke.dash = sk::StrokeDash::new(style.dash_pattern.iter().map(|d| *d as f32).collect(), style.dash_offset as f32);
        }
        let Some((shader, dst, mask)) = self.paint(brush.into(), brush_transform, transform) else { return };
        let paint = sk::Paint { shader, anti_alias: true, ..Default::default() };
        dst.stroke_path(&path, &paint, &stroke, t, mask);
    }

    fn fill<'a>(&mut self, style: Fill, transform: Affine, brush: impl Into<PaintRef<'a>>, brush_transform: Option<Affine>, shape: &impl Shape) {
        if self.top_empty() || !self.visible(transform, shape.bounding_box(), 1.0) {
            return;
        }
        let t = self.to_region(transform);
        let rect = shape.as_rect().or_else(|| path_rect(shape));
        let path = if rect.is_none() { to_path(shape) } else { None };
        if rect.is_none() && path.is_none() {
            return;
        }
        let Some((shader, dst, mask)) = self.paint(brush.into(), brush_transform, transform) else { return };
        let paint = sk::Paint { shader, anti_alias: true, ..Default::default() };
        match (rect, path) {
            (Some(r), _) => {
                if let Some(r) = sk::Rect::from_ltrb(r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32) {
                    dst.fill_rect(r, &paint, t, mask);
                }
            }
            (None, Some(p)) => dst.fill_path(&p, &paint, fill_rule(style), t, mask),
            _ => {}
        }
    }

    fn draw_glyphs<'a, 's: 'a>(
        &'s mut self,
        font: &'a FontData,
        font_size: f32,
        _hint: bool,
        normalized_coords: &'a [NormalizedCoord],
        _embolden: kurbo::Vec2,
        style: impl Into<StyleRef<'a>>,
        brush: impl Into<PaintRef<'a>>,
        brush_alpha: f32,
        transform: Affine,
        glyph_transform: Option<Affine>,
        glyphs: impl Iterator<Item = Glyph> + Clone,
    ) {
        if self.top_empty() {
            return;
        }
        // Only solid fills at an unrotated, unscaled transform take the cache;
        // other text is rare on these pages and skipped.
        let StyleRef::Fill(_) = style.into() else { return };
        let Paint::Solid(c) = brush.into() else { return };
        let [a, b, c2, d, e, f] = transform.as_coeffs();
        if b != 0.0 || c2 != 0.0 || glyph_transform.is_some_and(|g| g != Affine::IDENTITY) {
            return;
        }
        let scale = a.abs().max(d.abs()) as f32;
        let size = font_size * scale;
        let mut col = color(c);
        col.apply_opacity(brush_alpha);
        let pc = col.premultiply().to_color_u8();
        let key0 = GlyphKey { font: font.data.id(), index: font.index, glyph: 0, size: (size * 64.0) as u32, sub: 0 };
        let (rx, ry) = (self.region.x(), self.region.y());
        let (rw, rh) = (self.region.width() as i32, self.region.height() as i32);
        for g in glyphs {
            let px = e as f32 + g.x * a as f32 - rx as f32;
            let py = f as f32 + g.y * d as f32 - ry as f32;
            // Rough cull: a glyph is at most ~2 em around its origin.
            if px < -2.0 * size || py < -2.0 * size || px > rw as f32 + size || py > rh as f32 + 2.0 * size {
                continue;
            }
            let ix = px.floor();
            let sub = ((px - ix) * 4.0).round() as u8 % 4;
            let key = GlyphKey { glyph: g.id, sub, ..key0 };
            let mask = self
                .glyphs
                .entry(key)
                .or_insert_with(|| rasterize_glyph(font, normalized_coords, g.id, size, sub as f32 / 4.0));
            let Some(gm) = mask.as_ref() else { continue };
            let x = ix as i32 + gm.left;
            let y = py.round() as i32 + gm.top;
            let clip = self.layers.last().and_then(|l| l.mask.as_ref());
            let dst = self.surfaces.last_mut().unwrap_or(&mut self.pixmap);
            blend_coverage(dst, clip, gm, x, y, pc);
        }
    }

    fn draw_box_shadow(&mut self, transform: Affine, rect: Rect, brush: Color, radius: f64, std_dev: f64) {
        // No blur in tiny-skia: a few widening rounded rects of falling alpha.
        if self.top_empty() || !self.visible(transform, rect, 3.0 * std_dev) {
            return;
        }
        let steps = if std_dev < 0.5 { 1 } else { 4 };
        let [r, g, b, a] = brush.components;
        for i in 0..steps {
            let grow = if steps == 1 { 0.0 } else { std_dev * (1.5 - i as f64) };
            let rr = rect.inflate(grow, grow).to_rounded_rect((radius + grow).max(0.0));
            let c = Color::new([r, g, b, a / steps as f32]);
            self.fill(Fill::NonZero, transform, c, None, &rr);
        }
    }
}
