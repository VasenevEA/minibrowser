//! CSS parser and style resolution for the MiniBrowser engine.
//!
//! Parses inline `style=""` attributes and resolves default per-element styles.
//! Uses the `cssparser` crate for tokenizing CSS property declarations.
//!
//! Supported properties: display, position, width, height, margin, padding,
//! background-color, color, font-size, border, overflow, text-align.

use std::collections::HashMap;
use std::fmt;

use cssparser::{Parser, ParserInput, Token};

use crate::dom::{Document, ElementData, Node, NodeType};

// ---------------------------------------------------------------------------
// CSS value types
// ---------------------------------------------------------------------------

/// A length unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unit {
    Px,
    Percent,
    Em,
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unit::Px => write!(f, "px"),
            Unit::Percent => write!(f, "%"),
            Unit::Em => write!(f, "em"),
        }
    }
}

/// A parsed CSS value.
#[derive(Debug, Clone, PartialEq)]
pub enum CSSValue {
    Length(f32, Unit),
    Color(u8, u8, u8, u8),
    Keyword(String),
    Number(f32),
    Auto,
}

impl CSSValue {
    /// Interpret as pixels given a base font size and containing block size.
    pub fn to_px(&self, base_font_size: f32, containing_size: f32) -> Option<f32> {
        match self {
            CSSValue::Length(v, Unit::Px) => Some(*v),
            CSSValue::Length(v, Unit::Em) => Some(*v * base_font_size),
            CSSValue::Length(v, Unit::Percent) => Some(*v / 100.0 * containing_size),
            CSSValue::Number(v) => Some(*v),
            _ => None,
        }
    }
}

impl fmt::Display for CSSValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CSSValue::Length(v, u) => write!(f, "{}{}", v, u),
            CSSValue::Color(r, g, b, a) => write!(f, "rgba({},{},{},{})", r, g, b, a),
            CSSValue::Keyword(k) => write!(f, "{}", k),
            CSSValue::Number(n) => write!(f, "{}", n),
            CSSValue::Auto => write!(f, "auto"),
        }
    }
}

/// Map of property name to CSS value.
pub type StyleMap = HashMap<String, CSSValue>;

// ---------------------------------------------------------------------------
// Color parsing
// ---------------------------------------------------------------------------

/// Parse a CSS color string into RGBA.
///
/// Supports hex (`#rgb`, `#rrggbb`, `#rgba`, `#rrggbbaa`), `rgb()`, `rgba()`,
/// and common named colors.
pub fn parse_color(s: &str) -> Option<CSSValue> {
    let s = s.trim().to_lowercase();

    // Named colors
    let named = match s.as_str() {
        "red" => Some((255, 0, 0, 255)),
        "green" => Some((0, 128, 0, 255)),
        "lime" => Some((0, 255, 0, 255)),
        "blue" => Some((0, 0, 255, 255)),
        "white" => Some((255, 255, 255, 255)),
        "black" => Some((0, 0, 0, 255)),
        "gray" | "grey" => Some((128, 128, 128, 255)),
        "yellow" => Some((255, 255, 0, 255)),
        "orange" => Some((255, 165, 0, 255)),
        "purple" => Some((128, 0, 128, 255)),
        "cyan" | "aqua" => Some((0, 255, 255, 255)),
        "magenta" | "fuchsia" => Some((255, 0, 255, 255)),
        "pink" => Some((255, 192, 203, 255)),
        "brown" => Some((165, 42, 42, 255)),
        "transparent" => Some((0, 0, 0, 0)),
        "silver" => Some((192, 192, 192, 255)),
        "navy" => Some((0, 0, 128, 255)),
        "teal" => Some((0, 128, 128, 255)),
        "maroon" => Some((128, 0, 0, 255)),
        "olive" => Some((128, 128, 0, 255)),
        _ => None,
    };

    if let Some((r, g, b, a)) = named {
        return Some(CSSValue::Color(r, g, b, a));
    }

    // Hex colors
    if s.starts_with('#') {
        return parse_hex_color(&s[1..]);
    }

    // rgb()/rgba()
    if s.starts_with("rgb") {
        return parse_rgb_function(&s);
    }

    None
}

fn parse_hex_color(hex: &str) -> Option<CSSValue> {
    let chars: Vec<char> = hex.chars().collect();
    match chars.len() {
        3 => {
            let r = hex_digit(chars[0])? * 17;
            let g = hex_digit(chars[1])? * 17;
            let b = hex_digit(chars[2])? * 17;
            Some(CSSValue::Color(r, g, b, 255))
        }
        4 => {
            let r = hex_digit(chars[0])? * 17;
            let g = hex_digit(chars[1])? * 17;
            let b = hex_digit(chars[2])? * 17;
            let a = hex_digit(chars[3])? * 17;
            Some(CSSValue::Color(r, g, b, a))
        }
        6 => {
            let r = hex_byte(chars[0], chars[1])?;
            let g = hex_byte(chars[2], chars[3])?;
            let b = hex_byte(chars[4], chars[5])?;
            Some(CSSValue::Color(r, g, b, 255))
        }
        8 => {
            let r = hex_byte(chars[0], chars[1])?;
            let g = hex_byte(chars[2], chars[3])?;
            let b = hex_byte(chars[4], chars[5])?;
            let a = hex_byte(chars[6], chars[7])?;
            Some(CSSValue::Color(r, g, b, a))
        }
        _ => None,
    }
}

fn hex_digit(c: char) -> Option<u8> {
    c.to_digit(16).map(|d| d as u8)
}

fn hex_byte(hi: char, lo: char) -> Option<u8> {
    Some(hex_digit(hi)? * 16 + hex_digit(lo)?)
}

fn parse_rgb_function(s: &str) -> Option<CSSValue> {
    let inner = s
        .trim_start_matches("rgba(")
        .trim_start_matches("rgb(")
        .trim_end_matches(')');
    let parts: Vec<&str> = inner.split(|c: char| c == ',' || c == '/').collect();
    if parts.len() < 3 {
        return None;
    }
    let r = parts[0].trim().parse::<f32>().ok()?.clamp(0.0, 255.0) as u8;
    let g = parts[1].trim().parse::<f32>().ok()?.clamp(0.0, 255.0) as u8;
    let b = parts[2].trim().parse::<f32>().ok()?.clamp(0.0, 255.0) as u8;
    let a = if parts.len() >= 4 {
        let av = parts[3].trim().parse::<f32>().ok()?;
        if av <= 1.0 { (av * 255.0) as u8 } else { av.clamp(0.0, 255.0) as u8 }
    } else {
        255
    };
    Some(CSSValue::Color(r, g, b, a))
}

// ---------------------------------------------------------------------------
// Length parsing
// ---------------------------------------------------------------------------

/// Parse a CSS length string like `"10px"`, `"2em"`, `"50%"`, `"0"`, `"auto"`.
pub fn parse_length(s: &str) -> Option<CSSValue> {
    let s = s.trim().to_lowercase();
    if s == "auto" {
        return Some(CSSValue::Auto);
    }
    if s == "0" {
        return Some(CSSValue::Length(0.0, Unit::Px));
    }
    if let Some(v) = s.strip_suffix("px") {
        return Some(CSSValue::Length(v.trim().parse::<f32>().ok()?, Unit::Px));
    }
    if let Some(v) = s.strip_suffix('%') {
        return Some(CSSValue::Length(v.trim().parse::<f32>().ok()?, Unit::Percent));
    }
    if let Some(v) = s.strip_suffix("em") {
        return Some(CSSValue::Length(v.trim().parse::<f32>().ok()?, Unit::Em));
    }
    // Bare number
    if let Ok(n) = s.parse::<f32>() {
        return Some(CSSValue::Number(n));
    }
    None
}

// ---------------------------------------------------------------------------
// Inline style parsing via cssparser
// ---------------------------------------------------------------------------

/// Parse an inline `style` attribute value into a `StyleMap`.
///
/// Example: `"display: block; width: 100px; background-color: #ff0000"`
pub fn parse_inline_style(style: &str) -> StyleMap {
    let mut map = StyleMap::new();
    let mut input = ParserInput::new(style);
    let mut parser = Parser::new(&mut input);

    loop {
        parser.skip_whitespace();

        // Read property name (an ident token).
        let name = match parser.expect_ident() {
            Ok(name) => name.to_string().to_lowercase(),
            Err(_) => break,
        };

        // Expect colon separator.
        if parser.expect_colon().is_err() {
            skip_to_semicolon(&mut parser);
            continue;
        }

        // Collect raw value text until `;` or EOF.
        let value_str = collect_value(&mut parser);

        // Parse value according to property.
        // Border shorthand needs special expansion.
        if name == "border" {
            let expanded = parse_border_shorthand(&value_str);
            for (k, v) in expanded {
                map.insert(k, v);
            }
        } else if let Some(side) = name.strip_prefix("border-").filter(|s| matches!(*s, "top" | "right" | "bottom" | "left")) {
            // border-<side>: <width> <style> <color>, for that side only.
            for (k, v) in parse_border_shorthand(&value_str) {
                if k == format!("border-{side}-width") {
                    map.insert(k, v);
                } else if k == "border-color" {
                    map.insert(format!("border-{side}-color"), v);
                }
            }
        } else if name == "background" {
            // background shorthand: extract color and store as background-color
            if let Some(val) = parse_property_value(&name, &value_str) {
                map.insert("background-color".to_string(), val);
            }
        } else if name == "margin" || name == "padding" {
            // Handle multi-value shorthand: 1, 2, 3, or 4 values
            let expanded = expand_box_shorthand(&name, &value_str);
            for (k, v) in expanded {
                map.insert(k, v);
            }
        } else if let Some(val) = parse_property_value(&name, &value_str) {
            map.insert(name, val);
        }

        // Consume optional semicolon.
        let _ = parser.expect_semicolon();
    }

    map
}

/// Expand margin/padding shorthand with 1-4 values.
/// `padding: 10px` → all four sides
/// `padding: 10px 20px` → top/bottom=10, left/right=20
/// `padding: 10px 20px 30px` → top=10, left/right=20, bottom=30
/// `padding: 10px 20px 30px 40px` → top=10, right=20, bottom=30, left=40
fn expand_box_shorthand(property: &str, value: &str) -> Vec<(String, CSSValue)> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let values: Vec<CSSValue> = parts.iter()
        .filter_map(|p| if *p == "auto" { Some(CSSValue::Auto) } else { parse_length(p) })
        .collect();

    if values.is_empty() {
        // Try single value parse (might be a keyword like "auto")
        if let Some(v) = parse_length(value.trim()) {
            return vec![
                (format!("{}-top", property), v.clone()),
                (format!("{}-right", property), v.clone()),
                (format!("{}-bottom", property), v.clone()),
                (format!("{}-left", property), v),
            ];
        }
        return Vec::new();
    }

    let (top, right, bottom, left) = match values.len() {
        1 => (values[0].clone(), values[0].clone(), values[0].clone(), values[0].clone()),
        2 => (values[0].clone(), values[1].clone(), values[0].clone(), values[1].clone()),
        3 => (values[0].clone(), values[1].clone(), values[2].clone(), values[1].clone()),
        _ => (values[0].clone(), values[1].clone(), values[2].clone(), values[3].clone()),
    };

    vec![
        (format!("{}-top", property), top),
        (format!("{}-right", property), right),
        (format!("{}-bottom", property), bottom),
        (format!("{}-left", property), left),
    ]
}

/// Advance past tokens until we see a semicolon or reach EOF.
fn skip_to_semicolon(parser: &mut Parser<'_, '_>) {
    loop {
        match parser.next() {
            Ok(&Token::Semicolon) | Err(_) => break,
            _ => {}
        }
    }
}

/// Collect all tokens between the current position and the next `;` (or EOF)
/// as a trimmed string slice from the original input.
fn collect_value(parser: &mut Parser<'_, '_>) -> String {
    let start = parser.position();
    loop {
        let state = parser.state();
        match parser.next_including_whitespace() {
            Ok(&Token::Semicolon) => {
                // Put the semicolon back so the caller can consume it.
                parser.reset(&state);
                break;
            }
            // Step over `f(...)`, `(...)`, `[...]` whole: `is_exhausted` and
            // `reset` inside one would lose the block and cut the value short
            // (`repeat(3, 1fr)` came out as `repeat(`).
            Ok(&Token::Function(_)) | Ok(&Token::ParenthesisBlock) | Ok(&Token::SquareBracketBlock) => {
                let _ = parser.parse_nested_block(|p| {
                    while p.next_including_whitespace().is_ok() {}
                    Ok::<(), cssparser::ParseError<'_, ()>>(())
                });
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    parser.slice_from(start).trim().to_string()
}

/// Map a CSS property name and raw value string to a typed `CSSValue`.
fn parse_property_value(property: &str, value: &str) -> Option<CSSValue> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    match property {
        // Color properties
        "color" | "background-color" | "border-color" | "border-top-color" | "border-right-color"
        | "border-bottom-color" | "border-left-color" => parse_color(value),

        // background shorthand: extract just the color part
        "background" => parse_color(value),

        // Sizing / positioning
        "width" | "height" | "min-width" | "min-height" | "max-width" | "max-height"
        | "top" | "right" | "bottom" | "left"
        | "border-radius" => parse_length(value),

        // Spacing (single-value shorthand only)
        "margin" | "padding" | "margin-top" | "margin-right" | "margin-bottom"
        | "margin-left" | "padding-top" | "padding-right" | "padding-bottom"
        | "padding-left" => parse_length(value),

        // Font size (supports named sizes)
        "font-size" => parse_font_size(value),

        // Line height: "normal", multiplier (1.5), or absolute (24px)
        "line-height" => {
            if value.to_lowercase() == "normal" {
                Some(CSSValue::Keyword("normal".to_string()))
            } else if let Some(len) = parse_length(value) {
                Some(len)
            } else if let Ok(n) = value.parse::<f32>() {
                Some(CSSValue::Number(n))
            } else {
                None
            }
        }

        // Border shorthand: handled specially in parse_inline_style; fallback here
        "border" => {
            parse_length(value).or_else(|| Some(CSSValue::Keyword(value.to_string())))
        }

        // Border width (individual sides or shorthand)
        "border-width" | "border-top-width" | "border-right-width"
        | "border-bottom-width" | "border-left-width" => {
            parse_length(value).or_else(|| Some(CSSValue::Keyword(value.to_string())))
        }

        // Opacity: 0.0 to 1.0
        "opacity" => {
            value.parse::<f32>().ok().map(|v| CSSValue::Number(v.clamp(0.0, 1.0)))
        }

        // Gap (flex/grid)
        "gap" | "row-gap" | "column-gap" | "grid-gap" => parse_length(value),

        // Grid template columns: stored as keyword string, parsed in lib.rs
        "grid-template-columns" => Some(CSSValue::Keyword(value.to_string())),

        // Keyword-only properties
        "display" | "position" | "overflow" | "text-align" | "visibility"
        | "float" | "clear" | "box-sizing" | "white-space" | "font-weight"
        | "font-style" | "text-decoration" | "vertical-align"
        | "flex-wrap" | "align-items" | "justify-content" | "flex-direction"
        | "cursor" | "pointer-events" | "user-select" => {
            Some(CSSValue::Keyword(value.to_lowercase()))
        }

        // Fallback: store anything else as a keyword
        _ => Some(CSSValue::Keyword(value.to_string())),
    }
}

/// Parse `font-size`, supporting named absolute sizes and length values.
fn parse_font_size(value: &str) -> Option<CSSValue> {
    match value.to_lowercase().as_str() {
        "xx-small" => return Some(CSSValue::Length(9.0, Unit::Px)),
        "x-small" => return Some(CSSValue::Length(10.0, Unit::Px)),
        "small" => return Some(CSSValue::Length(13.0, Unit::Px)),
        "medium" => return Some(CSSValue::Length(16.0, Unit::Px)),
        "large" => return Some(CSSValue::Length(18.0, Unit::Px)),
        "x-large" => return Some(CSSValue::Length(24.0, Unit::Px)),
        "xx-large" => return Some(CSSValue::Length(32.0, Unit::Px)),
        _ => {}
    }
    parse_length(value)
}

// ---------------------------------------------------------------------------
// Border shorthand parsing
// ---------------------------------------------------------------------------

/// Parse `border: 2px solid #333` into individual border properties.
/// Returns a vec of (property, value) pairs for border-*-width, border-style,
/// and border-color.
fn parse_border_shorthand(value: &str) -> Vec<(String, CSSValue)> {
    let mut result = Vec::new();
    let parts: Vec<&str> = value.split_whitespace().collect();

    let mut width: Option<CSSValue> = None;
    let mut _style: Option<String> = None;
    let mut color: Option<CSSValue> = None;

    for part in &parts {
        // Try as border style keyword
        match *part {
            "none" | "solid" | "dashed" | "dotted" | "double" | "groove"
            | "ridge" | "inset" | "outset" | "hidden" => {
                _style = Some(part.to_string());
                continue;
            }
            _ => {}
        }

        // Try as length (width)
        if width.is_none() {
            if let Some(len) = parse_length(part) {
                width = Some(len);
                continue;
            }
        }

        // Try as color
        if color.is_none() {
            if let Some(c) = parse_color(part) {
                color = Some(c);
                continue;
            }
        }
    }

    // If style is "none" or "hidden", border width is 0
    let w = if _style.as_deref() == Some("none") || _style.as_deref() == Some("hidden") {
        CSSValue::Length(0.0, Unit::Px)
    } else {
        width.unwrap_or(CSSValue::Length(3.0, Unit::Px))
    };
    for side in &["border-top-width", "border-right-width", "border-bottom-width", "border-left-width"] {
        result.push((side.to_string(), w.clone()));
    }

    if let Some(c) = color {
        result.push(("border-color".to_string(), c));
    }

    result
}

// ---------------------------------------------------------------------------
// Properties that are inherited by default
// ---------------------------------------------------------------------------

const INHERITED_PROPERTIES: &[&str] = &[
    "color",
    "font-size",
    "font-family",
    "font-weight",
    "font-style",
    "text-align",
    "text-decoration",
    "line-height",
    "white-space",
    "visibility",
];

// ---------------------------------------------------------------------------
// Style resolution: walk the DOM and produce a StyledTree
// ---------------------------------------------------------------------------

/// A node paired with its computed styles.
#[derive(Debug, Clone)]
pub struct StyledNode {
    /// The original node type (tag name or text).
    pub node_type: NodeType,
    /// The DOM node's uid (see `dom::Node::uid`).
    pub uid: u32,
    /// Computed style properties.
    pub styles: StyleMap,
    /// Styled children.
    pub children: Vec<StyledNode>,
}

impl StyledNode {
    /// Look up a style property by name.
    pub fn get(&self, property: &str) -> Option<&CSSValue> {
        self.styles.get(property)
    }

    /// Get the `display` value, defaulting to `"inline"`.
    pub fn display(&self) -> &str {
        match self.styles.get("display") {
            Some(CSSValue::Keyword(k)) => k.as_str(),
            _ => "inline",
        }
    }
}

/// A fully styled document tree.
#[derive(Debug, Clone)]
pub struct StyledTree {
    pub root: StyledNode,
}

/// Walk the DOM tree and compute styles for every node.
pub fn resolve_styles(doc: &Document) -> StyledTree {
    // Extract and parse <style> blocks
    let rules = resolve_stylesheet_styles(doc);
    let parent_styles = StyleMap::new();
    let root = style_node(&doc.root, &parent_styles, &rules, &[]);
    StyledTree { root }
}

/// Marks a margin-top that came from the UA defaults of a quirky element.
const QUIRKY_MARGIN_TOP: &str = "-mb-quirky-margin-top";

/// Quirks mode (a page without a doctype, as browsers treat it): the default
/// top margin of the first block at the top of `<body>` is ignored, as is that
/// of a first child whose margin would collapse through its parent's top.
pub fn apply_body_margin_quirk(tree: &mut StyledTree) {
    fn find_body(n: &mut StyledNode) -> Option<&mut StyledNode> {
        if matches!(&n.node_type, NodeType::Element(d) if d.tag_name == "body") {
            return Some(n);
        }
        n.children.iter_mut().find_map(find_body)
    }
    fn px(n: &StyledNode, k: &str) -> f32 {
        match n.styles.get(k) {
            Some(CSSValue::Length(v, Unit::Px)) => *v,
            _ => 0.0,
        }
    }
    let Some(body) = find_body(&mut tree.root) else { return };
    let mut node = body;
    let mut at_body = true;
    loop {
        // Body is the quirk container whatever its padding; below it, a margin only
        // reaches the body's top if it collapses through its parent's.
        let sealed = px(node, "padding-top") > 0.0 || px(node, "border-top-width") > 0.0;
        if (sealed && !at_body) || node.display() == "flex" || node.display() == "grid" {
            return;
        }
        at_body = false;
        let first = node.children.iter_mut().find(|c| match &c.node_type {
            NodeType::Text(t) => !t.trim().is_empty(),
            NodeType::Element(_) => c.display() != "none",
            _ => false,
        });
        let Some(child) = first else { return };
        if !matches!(child.node_type, NodeType::Element(_)) || child.display() == "inline" {
            return;
        }
        if child.styles.contains_key(QUIRKY_MARGIN_TOP) {
            child.styles.insert("margin-top".into(), CSSValue::Length(0.0, Unit::Px));
        } else if px(child, "margin-top") != 0.0 {
            return;
        }
        node = child;
    }
}

/// The px font size of a style map (16 when unset).
fn font_size_px(styles: &StyleMap) -> f32 {
    match styles.get("font-size") {
        Some(CSSValue::Length(v, Unit::Px)) | Some(CSSValue::Number(v)) => *v,
        _ => 16.0,
    }
}

/// Turns em (and font-size %) lengths into px, so later stages only see px and %.
fn resolve_em(styles: &mut StyleMap, parent_styles: &StyleMap) {
    let parent_fs = font_size_px(parent_styles);
    let fs = match styles.get("font-size") {
        Some(CSSValue::Length(v, Unit::Em)) => v * parent_fs,
        Some(CSSValue::Length(v, Unit::Percent)) => v / 100.0 * parent_fs,
        Some(CSSValue::Keyword(k)) => match k.as_str() {
            "xx-small" => 9.0, "x-small" => 10.0, "small" => 13.0, "medium" => 16.0,
            "large" => 18.0, "x-large" => 24.0, "xx-large" => 32.0,
            "smaller" => parent_fs / 1.2, "larger" => parent_fs * 1.2,
            _ => parent_fs,
        },
        _ => font_size_px(styles),
    };
    styles.insert("font-size".into(), CSSValue::Length(fs, Unit::Px));
    for (k, v) in styles.iter_mut() {
        if let CSSValue::Length(n, Unit::Em) = v {
            if k != "font-size" {
                *v = CSSValue::Length(*n * fs, Unit::Px);
            }
        }
    }
}

/// Recursively style a single node.
fn style_node(node: &Node, parent_styles: &StyleMap, rules: &[CSSRule], ancestors: &[&ElementData]) -> StyledNode {
    let mut styles = StyleMap::new();

    match &node.node_type {
        NodeType::Element(data) => {
            // 1. Default styles for the tag.
            apply_default_styles(&data.tag_name, &mut styles);
            let ua_margin_top = styles.get("margin-top").cloned();

            // 1b. Map HTML presentational attributes to CSS (e.g. <img width="200">).
            if data.tag_name == "img" {
                if let Some(w) = data.attributes.get("width") {
                    if let Ok(n) = w.trim().trim_end_matches("px").parse::<f32>() {
                        styles.insert("width".into(), CSSValue::Length(n, Unit::Px));
                    }
                }
                if let Some(h) = data.attributes.get("height") {
                    if let Ok(n) = h.trim().trim_end_matches("px").parse::<f32>() {
                        styles.insert("height".into(), CSSValue::Length(n, Unit::Px));
                    }
                }
            }

            // 2. Inherit inheritable properties from parent.
            for &prop in INHERITED_PROPERTIES {
                if !styles.contains_key(prop) {
                    if let Some(val) = parent_styles.get(prop) {
                        styles.insert(prop.to_string(), val.clone());
                    }
                }
            }

            // 2.5. Apply matching stylesheet rules (sorted by specificity).
            let mut matching_rules: Vec<&CSSRule> = rules.iter()
                .filter(|rule| selector_matches_in(&rule.selector, data, ancestors))
                .collect();
            matching_rules.sort_by_key(|rule| specificity(&rule.selector));
            for rule in matching_rules {
                for (k, v) in &rule.declarations {
                    styles.insert(k.clone(), v.clone());
                }
            }

            // 3. Inline styles override everything.
            if let Some(style_attr) = data.style() {
                let inline = parse_inline_style(style_attr);
                for (k, v) in inline {
                    styles.insert(k, v);
                }
            }

            // A default (not author) top margin of the elements whose margins are
            // "quirky" in browsers: quirks mode drops it at the top of the body.
            if ua_margin_top.is_some()
                && styles.get("margin-top") == ua_margin_top.as_ref()
                && matches!(data.tag_name.as_str(), "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                    | "ul" | "ol" | "dl" | "blockquote" | "figure" | "pre")
            {
                styles.insert(QUIRKY_MARGIN_TOP.into(), CSSValue::Keyword("1".into()));
            }

            // 4. Computed values: font-size in em/% is relative to the parent's,
            //    other em lengths to this element's own font size.
            resolve_em(&mut styles, parent_styles);
        }
        NodeType::Text(_) => {
            // Text nodes inherit all inheritable properties.
            for &prop in INHERITED_PROPERTIES {
                if let Some(val) = parent_styles.get(prop) {
                    styles.insert(prop.to_string(), val.clone());
                }
            }
        }
        NodeType::Document => {}
    }

    // Children see this element as their nearest ancestor.
    let mut chain: Vec<&ElementData> = ancestors.to_vec();
    if let NodeType::Element(data) = &node.node_type {
        chain.push(data);
    }
    let children = node
        .children
        .iter()
        .map(|child| style_node(child, &styles, rules, &chain))
        .collect();

    StyledNode {
        node_type: node.node_type.clone(),
        uid: node.uid,
        styles,
        children,
    }
}

/// Apply browser-like default styles for known HTML elements.
fn apply_default_styles(tag: &str, styles: &mut StyleMap) {
    match tag {
        // Block containers. Values follow the HTML spec's rendering section
        // (https://html.spec.whatwg.org/multipage/rendering.html), as Chrome uses them;
        // em lengths resolve against the element's font size after the cascade.
        "div" | "article" | "section" | "nav" | "aside" | "header" | "footer" | "main"
        | "form" | "fieldset" | "figcaption" | "details" | "summary" | "table" | "address"
        | "html" | "dt" | "li" => {
            styles.insert("display".into(), CSSValue::Keyword("block".into()));
        }

        "body" => {
            styles.insert("display".into(), CSSValue::Keyword("block".into()));
            styles.insert("margin".into(), CSSValue::Length(8.0, Unit::Px));
        }

        "p" | "dl" | "pre" => {
            styles.insert("display".into(), CSSValue::Keyword("block".into()));
            styles.insert("margin-top".into(), CSSValue::Length(1.0, Unit::Em));
            styles.insert("margin-bottom".into(), CSSValue::Length(1.0, Unit::Em));
            if tag == "pre" {
                styles.insert("font-family".into(), CSSValue::Keyword("monospace".into()));
                styles.insert("white-space".into(), CSSValue::Keyword("pre".into()));
            }
        }

        "ul" | "ol" => {
            styles.insert("display".into(), CSSValue::Keyword("block".into()));
            styles.insert("margin-top".into(), CSSValue::Length(1.0, Unit::Em));
            styles.insert("margin-bottom".into(), CSSValue::Length(1.0, Unit::Em));
            styles.insert("padding-left".into(), CSSValue::Length(40.0, Unit::Px));
        }

        "dd" => {
            styles.insert("display".into(), CSSValue::Keyword("block".into()));
            styles.insert("margin-left".into(), CSSValue::Length(40.0, Unit::Px));
        }

        "blockquote" | "figure" => {
            styles.insert("display".into(), CSSValue::Keyword("block".into()));
            styles.insert("margin-top".into(), CSSValue::Length(1.0, Unit::Em));
            styles.insert("margin-bottom".into(), CSSValue::Length(1.0, Unit::Em));
            styles.insert("margin-left".into(), CSSValue::Length(40.0, Unit::Px));
            styles.insert("margin-right".into(), CSSValue::Length(40.0, Unit::Px));
        }

        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let (size, margin) = match tag {
                "h1" => (2.0, 0.67),
                "h2" => (1.5, 0.83),
                "h3" => (1.17, 1.0),
                "h4" => (1.0, 1.33),
                "h5" => (0.83, 1.67),
                _ => (0.67, 2.33),
            };
            styles.insert("display".into(), CSSValue::Keyword("block".into()));
            styles.insert("font-size".into(), CSSValue::Length(size, Unit::Em));
            styles.insert("font-weight".into(), CSSValue::Keyword("bold".into()));
            styles.insert("margin-top".into(), CSSValue::Length(margin, Unit::Em));
            styles.insert("margin-bottom".into(), CSSValue::Length(margin, Unit::Em));
        }

        "hr" => {
            styles.insert("display".into(), CSSValue::Keyword("block".into()));
            styles.insert("margin-top".into(), CSSValue::Length(0.5, Unit::Em));
            styles.insert("margin-bottom".into(), CSSValue::Length(0.5, Unit::Em));
            styles.insert("border-top-width".into(), CSSValue::Length(1.0, Unit::Px));
            styles.insert("border-bottom-width".into(), CSSValue::Length(1.0, Unit::Px));
            styles.insert("border-color".into(), CSSValue::Color(128, 128, 128, 255));
        }

        // Inline elements
        "span" | "a" | "label" | "abbr" | "cite" | "q" | "mark" | "s" | "del" | "ins"
        | "small" | "big" | "sub" | "sup" | "u" => {
            styles.insert("display".into(), CSSValue::Keyword("inline".into()));
        }

        "code" | "kbd" | "samp" | "tt" => {
            styles.insert("display".into(), CSSValue::Keyword("inline".into()));
            styles.insert("font-family".into(), CSSValue::Keyword("monospace".into()));
        }

        "strong" | "b" => {
            styles.insert("display".into(), CSSValue::Keyword("inline".into()));
            styles.insert("font-weight".into(), CSSValue::Keyword("bold".into()));
        }

        "em" | "i" => {
            styles.insert("display".into(), CSSValue::Keyword("inline".into()));
            styles.insert("font-style".into(), CSSValue::Keyword("italic".into()));
        }

        "img" => {
            styles.insert("display".into(), CSSValue::Keyword("block".into()));
        }

        "button" => {
            styles.insert("display".into(), CSSValue::Keyword("inline".into()));
            styles.insert("background-color".into(), CSSValue::Color(224, 224, 224, 255));
            styles.insert("padding-top".into(), CSSValue::Length(4.0, Unit::Px));
            styles.insert("padding-bottom".into(), CSSValue::Length(4.0, Unit::Px));
            styles.insert("padding-left".into(), CSSValue::Length(8.0, Unit::Px));
            styles.insert("padding-right".into(), CSSValue::Length(8.0, Unit::Px));
            styles.insert("border-top-width".into(), CSSValue::Length(1.0, Unit::Px));
            styles.insert("border-right-width".into(), CSSValue::Length(1.0, Unit::Px));
            styles.insert("border-bottom-width".into(), CSSValue::Length(1.0, Unit::Px));
            styles.insert("border-left-width".into(), CSSValue::Length(1.0, Unit::Px));
            styles.insert("border-color".into(), CSSValue::Color(153, 153, 153, 255));
            styles.insert("color".into(), CSSValue::Color(0, 0, 0, 255));
        }

        "input" => {
            styles.insert("display".into(), CSSValue::Keyword("inline".into()));
            styles.insert("background-color".into(), CSSValue::Color(255, 255, 255, 255));
            styles.insert("padding".into(), CSSValue::Length(4.0, Unit::Px));
            styles.insert("border-top-width".into(), CSSValue::Length(1.0, Unit::Px));
            styles.insert("border-right-width".into(), CSSValue::Length(1.0, Unit::Px));
            styles.insert("border-bottom-width".into(), CSSValue::Length(1.0, Unit::Px));
            styles.insert("border-left-width".into(), CSSValue::Length(1.0, Unit::Px));
            styles.insert("border-color".into(), CSSValue::Color(153, 153, 153, 255));
            styles.insert("color".into(), CSSValue::Color(0, 0, 0, 255));
            styles.insert("width".into(), CSSValue::Length(200.0, Unit::Px));
        }

        "select" => {
            styles.insert("display".into(), CSSValue::Keyword("inline".into()));
            styles.insert("background-color".into(), CSSValue::Color(255, 255, 255, 255));
            styles.insert("padding".into(), CSSValue::Length(4.0, Unit::Px));
            styles.insert("border-top-width".into(), CSSValue::Length(1.0, Unit::Px));
            styles.insert("border-right-width".into(), CSSValue::Length(1.0, Unit::Px));
            styles.insert("border-bottom-width".into(), CSSValue::Length(1.0, Unit::Px));
            styles.insert("border-left-width".into(), CSSValue::Length(1.0, Unit::Px));
            styles.insert("border-color".into(), CSSValue::Color(153, 153, 153, 255));
        }

        "textarea" => {
            styles.insert("display".into(), CSSValue::Keyword("block".into()));
            styles.insert("background-color".into(), CSSValue::Color(255, 255, 255, 255));
            styles.insert("padding".into(), CSSValue::Length(4.0, Unit::Px));
            styles.insert("border-top-width".into(), CSSValue::Length(1.0, Unit::Px));
            styles.insert("border-right-width".into(), CSSValue::Length(1.0, Unit::Px));
            styles.insert("border-bottom-width".into(), CSSValue::Length(1.0, Unit::Px));
            styles.insert("border-left-width".into(), CSSValue::Length(1.0, Unit::Px));
            styles.insert("border-color".into(), CSSValue::Color(153, 153, 153, 255));
            styles.insert("width".into(), CSSValue::Length(100.0, Unit::Percent));
            styles.insert("height".into(), CSSValue::Length(60.0, Unit::Px));
        }

        "head" | "script" | "style" | "link" | "meta" | "title" => {
            styles.insert("display".into(), CSSValue::Keyword("none".into()));
        }

        _ => {
            styles.insert("display".into(), CSSValue::Keyword("inline".into()));
        }
    }
}

// ---------------------------------------------------------------------------
// Stylesheet parsing: <style> block support (Phase 2 / v0.9)
// ---------------------------------------------------------------------------

/// A parsed CSS rule: selector + declaration block.
#[derive(Debug, Clone)]
pub struct CSSRule {
    pub selector: Selector,
    pub declarations: StyleMap,
}

/// A simple CSS selector supporting tag, id, and class components.
///
/// Matches selectors like `h1`, `.btn`, `#main`, `div.container`, `h1#title.big`.
#[derive(Debug, Clone, PartialEq)]
pub struct Selector {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    /// For `a > b c`: what must hold up the tree from the subject `c`, nearest
    /// first: `[(Descendant, b), (Child, a)]`.
    pub ancestors: Vec<(Combinator, Selector)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Combinator {
    /// `a b`: some ancestor matches.
    Descendant,
    /// `a > b`: the parent matches.
    Child,
}

impl Selector {
    /// Parse a simple selector string into its components.
    ///
    /// Supports: `h1`, `.btn`, `#main`, `div.container`, `h1#title`.
    fn parse(input: &str) -> Self {
        let input = input.trim();
        let mut tag = None;
        let mut id = None;
        let mut classes = Vec::new();

        let mut current = String::new();
        let mut kind = 't'; // 't' = tag, '#' = id, '.' = class

        for ch in input.chars() {
            if ch == '#' || ch == '.' {
                apply_selector_part(&mut tag, &mut id, &mut classes, kind, &current);
                current.clear();
                kind = ch;
            } else {
                current.push(ch);
            }
        }
        apply_selector_part(&mut tag, &mut id, &mut classes, kind, &current);

        Selector { tag, id, classes, ancestors: Vec::new() }
    }

    /// Parses a complex selector (`.a > .b c`). None for what this engine cannot
    /// match (pseudo-classes, attribute selectors, sibling combinators), so the
    /// rule is skipped rather than applied too widely.
    pub(crate) fn parse_complex(input: &str) -> Option<Self> {
        if input.contains([':', '[', '+', '~']) {
            return None;
        }
        let spaced = input.replace('>', " > ");
        let mut parts: Vec<&str> = spaced.split_whitespace().collect();
        let subject = parts.pop()?;
        if subject == ">" {
            return None;
        }
        let mut sel = Selector::parse(subject);
        let mut comb = Combinator::Descendant;
        while let Some(p) = parts.pop() {
            if p == ">" {
                comb = Combinator::Child;
                continue;
            }
            sel.ancestors.push((comb, Selector::parse(p)));
            comb = Combinator::Descendant;
        }
        Some(sel)
    }
}

fn apply_selector_part(
    tag: &mut Option<String>,
    id: &mut Option<String>,
    classes: &mut Vec<String>,
    kind: char,
    value: &str,
) {
    if value.is_empty() {
        return;
    }
    match kind {
        't' if value == "*" => {}
        't' => *tag = Some(value.to_lowercase()),
        '#' => *id = Some(value.to_string()),
        '.' => classes.push(value.to_string()),
        _ => {}
    }
}

/// Extract the text content of all `<style>` elements in the document.
///
/// Walks the DOM tree depth-first, finds every `<style>` element, concatenates
/// the text content of each one (separated by newlines), and returns the result.
pub fn extract_style_blocks(doc: &Document) -> String {
    let mut css = String::new();
    for node in doc.iter() {
        if let Some(data) = node.element_data() {
            if data.tag_name == "style" {
                let text = node.inner_text();
                if !text.is_empty() {
                    if !css.is_empty() {
                        css.push('\n');
                    }
                    css.push_str(&text);
                }
            }
        }
    }
    css
}

/// Parse a CSS stylesheet string into a list of rules.
///
/// Handles simple selectors (tag, class, id, and combinations) with
/// declaration blocks. Uses `parse_inline_style` for the declarations.
///
/// ```css
/// h1 { color: red; font-size: 32px; }
/// .highlight { background-color: yellow; }
/// #main { width: 100%; }
/// div.container { padding: 20px; }
/// ```
pub fn parse_stylesheet(css: &str) -> Vec<CSSRule> {
    let mut rules = Vec::new();
    let mut remaining = css.trim();

    while !remaining.is_empty() {
        // Skip whitespace and comments.
        remaining = skip_whitespace_and_comments(remaining);
        if remaining.is_empty() {
            break;
        }

        // Find the opening brace.
        let brace_open = match remaining.find('{') {
            Some(pos) => pos,
            None => break,
        };

        let selector_str = remaining[..brace_open].trim();
        if selector_str.is_empty() {
            // Malformed: skip past the block.
            remaining = skip_block(&remaining[brace_open..]);
            continue;
        }

        // Find the matching closing brace.
        let after_open = &remaining[brace_open + 1..];
        let brace_close = match after_open.find('}') {
            Some(pos) => pos,
            None => break, // Unterminated block; give up.
        };

        let declarations_str = &after_open[..brace_close];
        remaining = after_open[brace_close + 1..].trim();

        let declarations = parse_inline_style(declarations_str);
        // A selector list applies the block to each selector.
        for one in selector_str.split(',') {
            if let Some(selector) = Selector::parse_complex(one) {
                rules.push(CSSRule { selector, declarations: declarations.clone() });
            }
        }
    }

    rules
}

/// Skip leading whitespace and `/* ... */` comments.
fn skip_whitespace_and_comments(mut s: &str) -> &str {
    loop {
        s = s.trim_start();
        if s.starts_with("/*") {
            if let Some(end) = s.find("*/") {
                s = &s[end + 2..];
            } else {
                return ""; // Unterminated comment; consume everything.
            }
        } else {
            return s;
        }
    }
}

/// Skip past the next `{...}` block. Input should start at `{`.
fn skip_block(s: &str) -> &str {
    if let Some(close) = s.find('}') {
        s[close + 1..].trim()
    } else {
        ""
    }
}

/// Check whether a selector matches a given element.
pub fn selector_matches(selector: &Selector, node: &ElementData) -> bool {
    // Tag must match if specified.
    if let Some(ref tag) = selector.tag {
        if node.tag_name != *tag {
            return false;
        }
    }

    // ID must match if specified.
    if let Some(ref id) = selector.id {
        if node.id() != Some(id.as_str()) {
            return false;
        }
    }

    // All classes in the selector must be present on the element.
    let node_classes = node.classes();
    for cls in &selector.classes {
        if !node_classes.contains(&cls.as_str()) {
            return false;
        }
    }

    true
}

/// Compute the specificity of a selector as `(id_count, class_count, tag_count)`.
///
/// Higher specificity wins when multiple rules match the same element.
pub fn specificity(selector: &Selector) -> (u32, u32, u32) {
    let id_count = if selector.id.is_some() { 1 } else { 0 };
    let class_count = selector.classes.len() as u32;
    let tag_count = if selector.tag.is_some() { 1 } else { 0 };
    selector.ancestors.iter().fold((id_count, class_count, tag_count), |(a, b, c), (_, s)| {
        let (x, y, z) = specificity(s);
        (a + x, b + y, c + z)
    })
}

/// Whether `selector` matches `node`, whose ancestors are `ancestors` (root
/// first, parent last).
pub fn selector_matches_in(selector: &Selector, node: &ElementData, ancestors: &[&ElementData]) -> bool {
    fn up(chain: &[(Combinator, Selector)], ancestors: &[&ElementData]) -> bool {
        let Some(((comb, sel), rest)) = chain.split_first() else { return true };
        match comb {
            Combinator::Child => match ancestors.split_last() {
                Some((parent, above)) => selector_matches(sel, parent) && up(rest, above),
                None => false,
            },
            Combinator::Descendant => (0..ancestors.len()).rev().any(|i| {
                selector_matches(sel, ancestors[i]) && up(rest, &ancestors[..i])
            }),
        }
    }
    selector_matches(selector, node) && up(&selector.ancestors, ancestors)
}

/// Extract `<style>` blocks from the document, parse them, and return the
/// resulting CSS rules. This is the main integration entry point for Phase 2.
///
/// The returned rules can later be used by the style resolution pass to apply
/// stylesheet styles before inline styles.
pub fn resolve_stylesheet_styles(doc: &Document) -> Vec<CSSRule> {
    let css_text = extract_style_blocks(doc);
    parse_stylesheet(&css_text)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_color_named() {
        assert_eq!(parse_color("red"), Some(CSSValue::Color(255, 0, 0, 255)));
        assert_eq!(parse_color("transparent"), Some(CSSValue::Color(0, 0, 0, 0)));
        assert_eq!(parse_color("WHITE"), Some(CSSValue::Color(255, 255, 255, 255)));
    }

    #[test]
    fn test_parse_color_hex() {
        assert_eq!(parse_color("#fff"), Some(CSSValue::Color(255, 255, 255, 255)));
        assert_eq!(parse_color("#ff0000"), Some(CSSValue::Color(255, 0, 0, 255)));
        assert_eq!(
            parse_color("#00ff00ff"),
            Some(CSSValue::Color(0, 255, 0, 255))
        );
    }

    #[test]
    fn test_parse_color_rgb_func() {
        assert_eq!(
            parse_color("rgb(255, 0, 0)"),
            Some(CSSValue::Color(255, 0, 0, 255))
        );
        assert_eq!(
            parse_color("rgba(0, 0, 255, 0.5)"),
            Some(CSSValue::Color(0, 0, 255, 127))
        );
    }

    #[test]
    fn test_parse_length_values() {
        assert_eq!(parse_length("10px"), Some(CSSValue::Length(10.0, Unit::Px)));
        assert_eq!(
            parse_length("50%"),
            Some(CSSValue::Length(50.0, Unit::Percent))
        );
        assert_eq!(parse_length("2em"), Some(CSSValue::Length(2.0, Unit::Em)));
        assert_eq!(parse_length("auto"), Some(CSSValue::Auto));
        assert_eq!(parse_length("0"), Some(CSSValue::Length(0.0, Unit::Px)));
    }

    #[test]
    fn test_parse_inline_style_basic() {
        let styles =
            parse_inline_style("display: block; width: 100px; background-color: #ff0000");
        assert_eq!(
            styles.get("display"),
            Some(&CSSValue::Keyword("block".into()))
        );
        assert_eq!(
            styles.get("width"),
            Some(&CSSValue::Length(100.0, Unit::Px))
        );
        assert_eq!(
            styles.get("background-color"),
            Some(&CSSValue::Color(255, 0, 0, 255))
        );
    }

    #[test]
    fn test_parse_inline_style_multiple() {
        let styles = parse_inline_style(
            "margin: 10px; padding: 5px; color: blue; font-size: 14px; text-align: center",
        );
        // margin/padding shorthands now expand to individual sides
        assert_eq!(
            styles.get("margin-top"),
            Some(&CSSValue::Length(10.0, Unit::Px))
        );
        assert_eq!(
            styles.get("padding-left"),
            Some(&CSSValue::Length(5.0, Unit::Px))
        );
        assert_eq!(
            styles.get("color"),
            Some(&CSSValue::Color(0, 0, 255, 255))
        );
        assert_eq!(
            styles.get("font-size"),
            Some(&CSSValue::Length(14.0, Unit::Px))
        );
        assert_eq!(
            styles.get("text-align"),
            Some(&CSSValue::Keyword("center".into()))
        );
    }

    #[test]
    fn descendant_and_child_combinators() {
        let html = r#"<div class="card red"><p><span class="v">a</span></p></div><span class="v">b</span>"#;
        let doc = crate::html_parser::parse_html(&format!(
            "<html><head><style>.v {{ color: #00ff00 }} .red .v {{ color: #ff0000 }} div > .v {{ color: #0000ff }}</style></head><body>{html}</body></html>"
        ));
        let tree = resolve_styles(&doc);
        fn find<'a>(n: &'a StyledNode, text: &str) -> Option<&'a StyledNode> {
            if n.children.iter().any(|c| matches!(&c.node_type, NodeType::Text(t) if t == text)) {
                return Some(n);
            }
            n.children.iter().find_map(|c| find(c, text))
        }
        // `.red .v` (0,2,0) beats `.v`; `div > .v` does not apply (span's parent is p).
        assert_eq!(find(&tree.root, "a").unwrap().get("color"), Some(&CSSValue::Color(255, 0, 0, 255)));
        assert_eq!(find(&tree.root, "b").unwrap().get("color"), Some(&CSSValue::Color(0, 255, 0, 255)));
    }

    #[test]
    fn test_default_styles_block_elements() {
        let mut s = StyleMap::new();
        apply_default_styles("div", &mut s);
        assert_eq!(s.get("display"), Some(&CSSValue::Keyword("block".into())));

        let mut s = StyleMap::new();
        apply_default_styles("h1", &mut s);
        assert_eq!(s.get("display"), Some(&CSSValue::Keyword("block".into())));
        assert_eq!(
            s.get("font-size"),
            Some(&CSSValue::Length(2.0, Unit::Em))
        );
        // After the cascade: 2em of the 16px default, margins 0.67em of 32px.
        resolve_em(&mut s, &StyleMap::new());
        assert_eq!(s.get("font-size"), Some(&CSSValue::Length(32.0, Unit::Px)));
        assert_eq!(s.get("margin-top"), Some(&CSSValue::Length(0.67 * 32.0, Unit::Px)));
    }

    #[test]
    fn test_default_styles_inline_elements() {
        let mut s = StyleMap::new();
        apply_default_styles("span", &mut s);
        assert_eq!(s.get("display"), Some(&CSSValue::Keyword("inline".into())));
    }

    #[test]
    fn test_default_styles_hidden() {
        let mut s = StyleMap::new();
        apply_default_styles("head", &mut s);
        assert_eq!(s.get("display"), Some(&CSSValue::Keyword("none".into())));
    }

    #[test]
    fn test_resolve_styles_inline_overrides_default() {
        let mut doc = Document::new();
        let mut div = Node::new_element(
            "div",
            [("style".to_string(), "width: 200px; color: red".to_string())]
                .into_iter()
                .collect(),
        );
        div.append_child(Node::new_text("hello"));
        doc.root.append_child(div);

        let tree = resolve_styles(&doc);
        let div_styled = &tree.root.children[0];
        assert_eq!(
            div_styled.get("display"),
            Some(&CSSValue::Keyword("block".into()))
        );
        assert_eq!(
            div_styled.get("width"),
            Some(&CSSValue::Length(200.0, Unit::Px))
        );
        assert_eq!(
            div_styled.get("color"),
            Some(&CSSValue::Color(255, 0, 0, 255))
        );
    }

    #[test]
    fn test_resolve_styles_inheritance() {
        let mut doc = Document::new();
        let mut div = Node::new_element(
            "div",
            [("style".to_string(), "color: blue; font-size: 20px".to_string())]
                .into_iter()
                .collect(),
        );
        let span = Node::new_element("span", HashMap::new());
        div.append_child(span);
        doc.root.append_child(div);

        let tree = resolve_styles(&doc);
        let span_styled = &tree.root.children[0].children[0];
        // span should inherit color and font-size from parent div
        assert_eq!(
            span_styled.get("color"),
            Some(&CSSValue::Color(0, 0, 255, 255))
        );
        assert_eq!(
            span_styled.get("font-size"),
            Some(&CSSValue::Length(20.0, Unit::Px))
        );
    }

    // -----------------------------------------------------------------------
    // Phase 2: Stylesheet parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_extract_style_blocks_single() {
        let mut doc = Document::new();
        let mut html = Node::new_element("html", HashMap::new());
        let mut head = Node::new_element("head", HashMap::new());
        let mut style = Node::new_element("style", HashMap::new());
        style.append_child(Node::new_text("h1 { color: red; }"));
        head.append_child(style);
        html.append_child(head);
        doc.root.append_child(html);

        let css = extract_style_blocks(&doc);
        assert_eq!(css, "h1 { color: red; }");
    }

    #[test]
    fn test_extract_style_blocks_multiple() {
        let mut doc = Document::new();
        let mut html = Node::new_element("html", HashMap::new());
        let mut head = Node::new_element("head", HashMap::new());

        let mut style1 = Node::new_element("style", HashMap::new());
        style1.append_child(Node::new_text("h1 { color: red; }"));
        head.append_child(style1);

        let mut style2 = Node::new_element("style", HashMap::new());
        style2.append_child(Node::new_text("p { font-size: 14px; }"));
        head.append_child(style2);

        html.append_child(head);
        doc.root.append_child(html);

        let css = extract_style_blocks(&doc);
        assert!(css.contains("h1 { color: red; }"));
        assert!(css.contains("p { font-size: 14px; }"));
    }

    #[test]
    fn test_extract_style_blocks_none() {
        let mut doc = Document::new();
        let div = Node::new_element("div", HashMap::new());
        doc.root.append_child(div);

        let css = extract_style_blocks(&doc);
        assert!(css.is_empty());
    }

    #[test]
    fn test_parse_stylesheet_tag_selector() {
        let rules = parse_stylesheet("h1 { color: red; font-size: 32px; }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector.tag, Some("h1".to_string()));
        assert_eq!(rules[0].selector.id, None);
        assert!(rules[0].selector.classes.is_empty());
        assert_eq!(
            rules[0].declarations.get("color"),
            Some(&CSSValue::Color(255, 0, 0, 255))
        );
        assert_eq!(
            rules[0].declarations.get("font-size"),
            Some(&CSSValue::Length(32.0, Unit::Px))
        );
    }

    #[test]
    fn test_parse_stylesheet_class_selector() {
        let rules = parse_stylesheet(".highlight { background-color: yellow; }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector.tag, None);
        assert_eq!(rules[0].selector.classes, vec!["highlight".to_string()]);
        assert_eq!(
            rules[0].declarations.get("background-color"),
            Some(&CSSValue::Color(255, 255, 0, 255))
        );
    }

    #[test]
    fn test_parse_stylesheet_id_selector() {
        let rules = parse_stylesheet("#main { width: 100%; }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector.id, Some("main".to_string()));
        assert_eq!(rules[0].selector.tag, None);
        assert_eq!(
            rules[0].declarations.get("width"),
            Some(&CSSValue::Length(100.0, Unit::Percent))
        );
    }

    #[test]
    fn test_parse_stylesheet_combined_selector() {
        let rules = parse_stylesheet("div.container { padding: 20px; }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector.tag, Some("div".to_string()));
        assert_eq!(rules[0].selector.classes, vec!["container".to_string()]);
    }

    #[test]
    fn test_parse_stylesheet_tag_id_combined() {
        let rules = parse_stylesheet("h1#title { color: blue; }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector.tag, Some("h1".to_string()));
        assert_eq!(rules[0].selector.id, Some("title".to_string()));
    }

    #[test]
    fn test_parse_stylesheet_multiple_rules() {
        let css = "
            h1 { color: red; }
            .btn { background-color: blue; }
            #nav { width: 200px; }
        ";
        let rules = parse_stylesheet(css);
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].selector.tag, Some("h1".to_string()));
        assert_eq!(rules[1].selector.classes, vec!["btn".to_string()]);
        assert_eq!(rules[2].selector.id, Some("nav".to_string()));
    }

    #[test]
    fn test_parse_stylesheet_with_comments() {
        let css = "/* heading */ h1 { color: red; } /* end */";
        let rules = parse_stylesheet(css);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector.tag, Some("h1".to_string()));
    }

    #[test]
    fn test_parse_stylesheet_empty() {
        let rules = parse_stylesheet("");
        assert!(rules.is_empty());
        let rules = parse_stylesheet("   ");
        assert!(rules.is_empty());
    }

    #[test]
    fn test_selector_matches_tag() {
        let sel = Selector { tag: Some("div".into()), id: None, classes: vec![], ancestors: vec![] };
        let el = ElementData::new("div".into(), HashMap::new());
        assert!(selector_matches(&sel, &el));

        let el2 = ElementData::new("span".into(), HashMap::new());
        assert!(!selector_matches(&sel, &el2));
    }

    #[test]
    fn test_selector_matches_class() {
        let sel = Selector { tag: None, id: None, classes: vec!["highlight".into()], ancestors: vec![] };
        let el = ElementData::new(
            "div".into(),
            HashMap::from([("class".into(), "highlight extra".into())]),
        );
        assert!(selector_matches(&sel, &el));

        let el2 = ElementData::new("div".into(), HashMap::new());
        assert!(!selector_matches(&sel, &el2));
    }

    #[test]
    fn test_selector_matches_id() {
        let sel = Selector { tag: None, id: Some("main".into()), classes: vec![], ancestors: vec![] };
        let el = ElementData::new(
            "div".into(),
            HashMap::from([("id".into(), "main".into())]),
        );
        assert!(selector_matches(&sel, &el));

        let el2 = ElementData::new(
            "div".into(),
            HashMap::from([("id".into(), "sidebar".into())]),
        );
        assert!(!selector_matches(&sel, &el2));
    }

    #[test]
    fn test_selector_matches_combined() {
        let sel = Selector {
            tag: Some("div".into()),
            id: None,
            classes: vec!["container".into()],
            ancestors: vec![],
        };
        let el = ElementData::new(
            "div".into(),
            HashMap::from([("class".into(), "container".into())]),
        );
        assert!(selector_matches(&sel, &el));

        // Wrong tag
        let el2 = ElementData::new(
            "span".into(),
            HashMap::from([("class".into(), "container".into())]),
        );
        assert!(!selector_matches(&sel, &el2));

        // Missing class
        let el3 = ElementData::new("div".into(), HashMap::new());
        assert!(!selector_matches(&sel, &el3));
    }

    #[test]
    fn test_selector_matches_universal() {
        // Selector with no tag, no id, no classes matches everything.
        let sel = Selector { tag: None, id: None, classes: vec![], ancestors: vec![] };
        let el = ElementData::new("anything".into(), HashMap::new());
        assert!(selector_matches(&sel, &el));
    }

    #[test]
    fn test_specificity_tag_only() {
        let sel = Selector { tag: Some("h1".into()), id: None, classes: vec![], ancestors: vec![] };
        assert_eq!(specificity(&sel), (0, 0, 1));
    }

    #[test]
    fn test_specificity_class_only() {
        let sel = Selector { tag: None, id: None, classes: vec!["btn".into()], ancestors: vec![] };
        assert_eq!(specificity(&sel), (0, 1, 0));
    }

    #[test]
    fn test_specificity_id_only() {
        let sel = Selector { tag: None, id: Some("main".into()), classes: vec![], ancestors: vec![] };
        assert_eq!(specificity(&sel), (1, 0, 0));
    }

    #[test]
    fn test_specificity_combined() {
        let sel = Selector {
            tag: Some("div".into()),
            id: Some("main".into()),
            classes: vec!["container".into(), "wide".into()],
            ancestors: vec![],
        };
        assert_eq!(specificity(&sel), (1, 2, 1));
    }

    #[test]
    fn test_specificity_ordering() {
        let id_sel = Selector { tag: None, id: Some("x".into()), classes: vec![], ancestors: vec![] };
        let class_sel = Selector { tag: None, id: None, classes: vec!["x".into()], ancestors: vec![] };
        let tag_sel = Selector { tag: Some("x".into()), id: None, classes: vec![], ancestors: vec![] };

        // ID > class > tag when compared as tuples.
        assert!(specificity(&id_sel) > specificity(&class_sel));
        assert!(specificity(&class_sel) > specificity(&tag_sel));
    }

    #[test]
    fn test_resolve_stylesheet_styles_integration() {
        let mut doc = Document::new();
        let mut html = Node::new_element("html", HashMap::new());
        let mut head = Node::new_element("head", HashMap::new());
        let mut style = Node::new_element("style", HashMap::new());
        style.append_child(Node::new_text(
            "h1 { color: red; } .btn { background-color: blue; }",
        ));
        head.append_child(style);
        html.append_child(head);
        doc.root.append_child(html);

        let rules = resolve_stylesheet_styles(&doc);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].selector.tag, Some("h1".to_string()));
        assert_eq!(rules[1].selector.classes, vec!["btn".to_string()]);
    }
}
