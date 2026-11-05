//! Inline `<svg>`: the element's subtree goes back to SVG markup, resvg renders
//! it, and the page lays it out and paints it like an image. Scripts change it
//! with `setAttribute` (a gauge needle's `transform`, a level's `height`), and the
//! next render redraws it.
//!
//! Rendering is without text support (no font database on the device); SVG
//! `<text>` is skipped, so labels belong in HTML next to the drawing.

use std::collections::HashMap;
use std::fmt::Write;

use crate::css::StyledNode;
use crate::dom::NodeType;

/// Rendered drawings by markup and size: a page redraws the same SVG every
/// frame until a script changes it.
#[derive(Default)]
pub struct SvgCache {
    entries: HashMap<(String, u32, u32), (Vec<u8>, u32, u32)>,
}

impl SvgCache {
    /// Drops drawings not used by the current render (call after each render).
    pub fn retain_used(&mut self, used: &[(String, u32, u32)]) {
        self.entries.retain(|k, _| used.contains(k));
    }
}

/// Renders the `<svg>` element `node` to straight-alpha RGBA. The size is the
/// `width`/`height` attributes, else the viewBox, else 300×150 (as browsers).
/// Returns the pixels, their size and the cache key it used.
pub fn render(node: &StyledNode, cache: &mut SvgCache) -> Option<((Vec<u8>, u32, u32), (String, u32, u32))> {
    let NodeType::Element(data) = &node.node_type else { return None };
    let attr = |k: &str| data.attributes.get(k).and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok());
    let view_box: Option<Vec<f32>> = data.attributes.get("viewBox").map(|v| {
        v.split(|c: char| c == ',' || c.is_whitespace()).filter_map(|p| p.parse().ok()).collect()
    });
    let (vw, vh) = match view_box.as_deref() {
        Some([_, _, w, h]) if *w > 0.0 && *h > 0.0 => (Some(*w), Some(*h)),
        _ => (None, None),
    };
    let (w, h) = match (attr("width"), attr("height"), vw, vh) {
        (Some(w), Some(h), _, _) => (w, h),
        (Some(w), None, Some(a), Some(b)) => (w, w * b / a),
        (None, Some(h), Some(a), Some(b)) => (h * a / b, h),
        (None, None, Some(a), Some(b)) => (a, b),
        _ => (300.0, 150.0),
    };
    let (w, h) = (w.round().clamp(1.0, 4000.0) as u32, h.round().clamp(1.0, 4000.0) as u32);

    let mut xml = String::new();
    write_node(node, &mut xml, true);
    let key = (xml, w, h);
    if let Some(hit) = cache.entries.get(&key) {
        return Some((hit.clone(), key));
    }

    let tree = match resvg::usvg::Tree::from_str(&key.0, &resvg::usvg::Options::default()) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("svg: {e}");
            return None;
        }
    };
    let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
    let size = tree.size();
    let transform = tiny_skia::Transform::from_scale(w as f32 / size.width(), h as f32 / size.height());
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    // The painter blends straight alpha, tiny-skia keeps premultiplied.
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for p in pixmap.pixels() {
        let c = p.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    let out = (rgba, w, h);
    cache.entries.insert(key.clone(), out.clone());
    Some((out, key))
}

/// SVG element names with capitals, which the HTML parser lowercased.
fn svg_name(tag: &str) -> &str {
    const NAMES: &[&str] = &[
        "linearGradient", "radialGradient", "clipPath", "textPath", "foreignObject", "feGaussianBlur",
        "feOffset", "feBlend", "feColorMatrix", "feComposite", "feFlood", "feMerge", "feMergeNode",
        "feDropShadow", "animateTransform", "animateMotion",
    ];
    NAMES.iter().find(|n| n.eq_ignore_ascii_case(tag)).copied().unwrap_or(tag)
}

fn escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

fn write_node(node: &StyledNode, out: &mut String, root: bool) {
    match &node.node_type {
        NodeType::Text(t) => escape(t, out),
        NodeType::Element(d) => {
            let name = svg_name(&d.tag_name);
            let _ = write!(out, "<{name}");
            if root && !d.attributes.contains_key("xmlns") {
                out.push_str(" xmlns=\"http://www.w3.org/2000/svg\"");
            }
            // Sorted, so the same drawing always gives the same cache key.
            let mut attrs: Vec<(&String, &String)> = d.attributes.iter().collect();
            attrs.sort();
            for (k, v) in attrs {
                let _ = write!(out, " {k}=\"");
                escape(v, out);
                out.push('"');
            }
            out.push('>');
            for c in &node.children {
                write_node(c, out, false);
            }
            let _ = write!(out, "</{name}>");
        }
        NodeType::Document => {}
    }
}
