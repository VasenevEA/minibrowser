//! WASM integration tests for v1.0 features.
//! Tests CSS Grid, position, border-radius, text-decoration, images, Canvas 2D, localStorage.

use wasm_bindgen_test::*;
use minibrowser::js_engine::{self, JsHost};
use minibrowser::BrowserJs;
use minibrowser::html_parser;
use minibrowser::css;

wasm_bindgen_test_configure!(run_in_browser);

// ---------------------------------------------------------------------------
// CSS <style> block support
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn style_block_applies_class_styles() {
    let html = r#"<html><head><style>
        .red { color: red; }
        .big { font-size: 24px; }
    </style></head><body>
        <div class="red big">Styled text</div>
    </body></html>"#;

    let doc = html_parser::parse_html(html);
    let styled = css::resolve_styles(&doc);

    // The div should have color:red and font-size:24px from the style block
    fn find_red(node: &css::StyledNode) -> bool {
        if let Some(css::CSSValue::Color(255, 0, 0, 255)) = node.get("color") {
            return true;
        }
        node.children.iter().any(|c| find_red(c))
    }
    assert!(find_red(&styled.root), "Should find red-colored element from .red class");
}

#[wasm_bindgen_test]
fn style_block_id_selector() {
    let html = r#"<html><head><style>
        #main { background-color: #ff0000; }
    </style></head><body>
        <div id="main">Content</div>
    </body></html>"#;

    let doc = html_parser::parse_html(html);
    let styled = css::resolve_styles(&doc);

    fn find_bg(node: &css::StyledNode) -> bool {
        if let Some(css::CSSValue::Color(255, 0, 0, 255)) = node.get("background-color") {
            return true;
        }
        node.children.iter().any(|c| find_bg(c))
    }
    assert!(find_bg(&styled.root), "Should find background-color from #main selector");
}

// ---------------------------------------------------------------------------
// CSS Grid
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn grid_layout_renders_without_crash() {
    let html = r#"<html><body>
        <div style="display:grid;grid-template-columns:1fr 1fr;grid-gap:10px;">
            <div style="background:red;padding:10px;">Cell 1</div>
            <div style="background:green;padding:10px;">Cell 2</div>
            <div style="background:blue;padding:10px;">Cell 3</div>
            <div style="background:purple;padding:10px;">Cell 4</div>
        </div>
    </body></html>"#;

    let doc = html_parser::parse_html(html);
    let styled = css::resolve_styles(&doc);
    // Just verify no crash — layout happens during render
    assert!(styled.root.children.len() > 0);
}

// ---------------------------------------------------------------------------
// localStorage
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn localstorage_set_and_get() {
    let html = r#"<html><body>
        <div id="out">none</div>
        <script>
            localStorage.setItem('test_key', 'hello');
            var val = localStorage.getItem('test_key');
            document.getElementById('out').textContent = val;
        </script>
    </body></html>"#;

    let mut doc = html_parser::parse_html(html);
    let mut js = BrowserJs;
    js.reset();
    js_engine::execute_scripts(&mut doc, &mut js, 0.0);

    assert_eq!(doc.root.get_element_by_id("out").unwrap().inner_text(), "hello");
}

#[wasm_bindgen_test]
fn localstorage_persists_across_calls() {
    let html = r#"<html><body>
        <div id="count">0</div>
        <script>
            var c = parseInt(localStorage.getItem('counter') || '0') + 1;
            localStorage.setItem('counter', '' + c);
            document.getElementById('count').textContent = '' + c;
        </script>
    </body></html>"#;

    let mut doc = html_parser::parse_html(html);
    let mut js = BrowserJs;
    js.reset();
    // Clear any previous state
    let _ = js_sys::eval("window.__mb_storage = {}");
    js_engine::execute_scripts(&mut doc, &mut js, 0.0);

    let val1 = doc.root.get_element_by_id("count").unwrap().inner_text();
    assert_eq!(val1, "1");

    // Second execution (simulating page reload with persistent storage)
    let mut doc2 = html_parser::parse_html(html);
    js_engine::execute_scripts(&mut doc2, &mut js, 0.0);
    let val2 = doc2.root.get_element_by_id("count").unwrap().inner_text();
    assert_eq!(val2, "2");
}

// ---------------------------------------------------------------------------
// Canvas 2D API mock
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn canvas_getcontext_returns_object() {
    let html = r#"<html><body>
        <div id="c"></div>
        <div id="result">no</div>
        <script>
            var ctx = document.getElementById('c').getContext('2d');
            if (ctx && typeof ctx.fillRect === 'function') {
                document.getElementById('result').textContent = 'yes';
            }
        </script>
    </body></html>"#;

    let mut doc = html_parser::parse_html(html);
    let mut js = BrowserJs;
    js.reset();
    js_engine::execute_scripts(&mut doc, &mut js, 0.0);

    assert_eq!(doc.root.get_element_by_id("result").unwrap().inner_text(), "yes");
}

#[wasm_bindgen_test]
fn canvas_fillrect_produces_command() {
    let html = r#"<html><body>
        <div id="canvas"></div>
        <script>
            var ctx = document.getElementById('canvas').getContext('2d');
            ctx.fillStyle = '#ff0000';
            ctx.fillRect(10, 20, 100, 50);
        </script>
    </body></html>"#;

    let mut doc = html_parser::parse_html(html);
    let mut js = BrowserJs;
    js.reset();
    js_engine::execute_scripts(&mut doc, &mut js, 0.0);

    // Canvas commands are stored as data-canvas-cmds attribute
    let canvas_el = doc.root.get_element_by_id("canvas").unwrap();
    let cmds = canvas_el.element_data().unwrap().attributes.get("data-canvas-cmds");
    assert!(cmds.is_some(), "Canvas commands should be stored as attribute");
    let cmds_str = cmds.unwrap();
    assert!(cmds_str.contains("fillRect") || cmds_str.contains("CanvasFillRect"),
            "Should contain fillRect command");
}

// ---------------------------------------------------------------------------
// border-radius (just verify parsing doesn't crash)
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn border_radius_parsed() {
    let html = r#"<html><body>
        <div style="border-radius: 10px; background: red; padding: 20px;">Rounded</div>
    </body></html>"#;

    let doc = html_parser::parse_html(html);
    let styled = css::resolve_styles(&doc);

    fn find_radius(node: &css::StyledNode) -> bool {
        if let Some(css::CSSValue::Length(r, _)) = node.get("border-radius") {
            return *r > 0.0;
        }
        node.children.iter().any(|c| find_radius(c))
    }
    assert!(find_radius(&styled.root), "border-radius should be parsed");
}

// ---------------------------------------------------------------------------
// text-decoration
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn text_decoration_inherited() {
    let html = r#"<html><body>
        <div style="text-decoration: underline;">
            <span>Child text</span>
        </div>
    </body></html>"#;

    let doc = html_parser::parse_html(html);
    let styled = css::resolve_styles(&doc);

    fn find_underline(node: &css::StyledNode) -> bool {
        if let Some(css::CSSValue::Keyword(k)) = node.get("text-decoration") {
            if k == "underline" { return true; }
        }
        node.children.iter().any(|c| find_underline(c))
    }
    assert!(find_underline(&styled.root), "text-decoration should be inherited");
}

// ---------------------------------------------------------------------------
// Image placeholder
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn img_element_creates_layout_box() {
    let html = r#"<html><body>
        <img width="200" height="100" alt="Test image">
        <div id="after">After image</div>
    </body></html>"#;

    let doc = html_parser::parse_html(html);
    let styled = css::resolve_styles(&doc);
    // Verify it parses without crash
    assert!(doc.root.get_element_by_id("after").is_some());
}

#[test]
fn debug_demo3_css_and_layout() {
    use minibrowser::html_parser;
    use minibrowser::css;
    
    let html = r#"<html><body style="background-color: #1a1a2e; color: #e0e0e0; font-size: 14px; margin: 0; padding: 10px;">
  <div style="background-color: #2d2d44; padding: 8px; margin-bottom: 10px;">
    <span style="color: #4fc3f7; font-size: 18px;">Test</span>
  </div>
</body></html>"#;
    
    let doc = html_parser::parse_html(html);
    let styled = css::resolve_styles(&doc);
    
    // Check body styles
    fn find_body(node: &css::StyledNode) -> Option<&css::StyledNode> {
        if let minibrowser::dom::NodeType::Element(ref data) = node.node_type {
            if data.tag_name == "body" {
                return Some(node);
            }
        }
        for child in &node.children {
            if let Some(b) = find_body(child) {
                return Some(b);
            }
        }
        None
    }
    
    let body = find_body(&styled.root).unwrap();
    eprintln!("Body styles:");
    for (k, v) in body.styles.iter() {
        eprintln!("  {}: {:?}", k, v);
    }

    // Print styled tree structure
    fn print_tree(node: &css::StyledNode, depth: usize) {
        let indent = "  ".repeat(depth);
        let tag = match &node.node_type {
            minibrowser::dom::NodeType::Element(data) => data.tag_name.clone(),
            minibrowser::dom::NodeType::Text(s) => format!("TEXT({})", &s[..s.len().min(20)]),
            minibrowser::dom::NodeType::Document => "DOCUMENT".to_string(),
        };
        let display = node.display();
        eprintln!("{}tag={} display={} children={}", indent, tag, display, node.children.len());
        for child in &node.children {
            print_tree(child, depth + 1);
        }
    }
    eprintln!("\nStyled tree:");
    print_tree(&styled.root, 0);
}

#[test]
fn debug_demo8_flex_header() {
    use minibrowser::html_parser;
    use minibrowser::css;
    
    let html = r#"<html><body style="background:#1a1a2e;color:#e0e0e0;font-size:14px;margin:0;padding:10px;">
  <div style="background:#2d2d44;padding:8px;margin-bottom:10px;display:flex;">
    <span style="color:#4fc3f7;font-size:18px;">HMI Control Panel</span>
    <span id="status" style="color:#81c784;margin-left:auto;">Online</span>
  </div>
</body></html>"#;
    
    let doc = html_parser::parse_html(html);
    let styled = css::resolve_styles(&doc);
    
    fn find_div_flex(node: &css::StyledNode) -> Option<&css::StyledNode> {
        if let minibrowser::dom::NodeType::Element(ref data) = node.node_type {
            if data.tag_name == "div" {
                if node.display() == "flex" {
                    return Some(node);
                }
            }
        }
        for child in &node.children {
            if let Some(d) = find_div_flex(child) {
                return Some(d);
            }
        }
        None
    }
    
    let flex_div = find_div_flex(&styled.root);
    if let Some(fd) = flex_div {
        eprintln!("Found flex div with {} children", fd.children.len());
        for (i, child) in fd.children.iter().enumerate() {
            let tag = match &child.node_type {
                minibrowser::dom::NodeType::Element(data) => data.tag_name.clone(),
                minibrowser::dom::NodeType::Text(s) => format!("TEXT({})", &s[..s.len().min(20)]),
                _ => "?".to_string(),
            };
            let ml = child.styles.get("margin-left").map(|v| format!("{:?}", v)).unwrap_or_default();
            eprintln!("  [{}] {} display={} margin-left={}", i, tag, child.display(), ml);
        }
    } else {
        eprintln!("No flex div found!");
    }
}

#[test]
fn debug_demo8_layout_positions() {
    use minibrowser::html_parser;
    use minibrowser::css;
    use minibrowser::layout;
    
    let html = r#"<html><body style="background:#1a1a2e;color:#e0e0e0;font-size:14px;margin:0;padding:10px;">
  <div style="background:#2d2d44;padding:8px;margin-bottom:10px;display:flex;">
    <span style="color:#4fc3f7;font-size:18px;">HMI Control Panel</span>
    <span id="status" style="color:#81c784;margin-left:auto;">Online</span>
  </div>
</body></html>"#;
    
    let doc = html_parser::parse_html(html);
    let styled = css::resolve_styles(&doc);
    
    // Convert to layout tree - we need to use the same convert_styled function
    // that lib.rs uses, but it's private. Let's instead call layout directly.
    // Actually, we can test the CSS conversion through layout::StyledNode construction.
    
    // For simplicity, build a manual layout tree matching the structure
    let mut status_span = layout::StyledNode {
        display: layout::Display::Inline,
        position: layout::Position::Static,
        width: layout::Dimension::Auto,
        height: layout::Dimension::Auto,
        margin: layout::EdgeSizes { top: 0.0, right: 0.0, bottom: 0.0, left: f32::MAX / 2.0 },
        padding: layout::EdgeSizes::ZERO,
        border: layout::EdgeSizes::ZERO,
        background_color: None,
        text_color: layout::Color::rgb(129, 199, 132),
        border_color: None,
        font_size: 14.0,
        text_align: layout::TextAlign::Left,
        opacity: 1.0,
        gap: 0.0,
        flex_wrap: layout::FlexWrap::NoWrap,
        align_items: layout::AlignItems::Stretch,
        min_height: layout::Dimension::Auto,
        line_height: 0.0,
        white_space_nowrap: false,
        float_right: false,
        border_radius: 0.0,
        text_decoration: layout::TextDecoration::None,
        offset_top: 0.0,
        offset_left: 0.0,
        grid_template_columns: Vec::new(),
        grid_gap: 0.0,
        content: layout::NodeContent::Element,
        children: vec![layout::StyledNode::text("Online")],
    };
    status_span.children[0].font_size = 14.0;
    
    let mut title_span = layout::StyledNode {
        display: layout::Display::Inline,
        position: layout::Position::Static,
        width: layout::Dimension::Auto,
        height: layout::Dimension::Auto,
        margin: layout::EdgeSizes::ZERO,
        padding: layout::EdgeSizes::ZERO,
        border: layout::EdgeSizes::ZERO,
        background_color: None,
        text_color: layout::Color::rgb(79, 195, 247),
        border_color: None,
        font_size: 18.0,
        text_align: layout::TextAlign::Left,
        opacity: 1.0,
        gap: 0.0,
        flex_wrap: layout::FlexWrap::NoWrap,
        align_items: layout::AlignItems::Stretch,
        min_height: layout::Dimension::Auto,
        line_height: 0.0,
        white_space_nowrap: false,
        float_right: false,
        border_radius: 0.0,
        text_decoration: layout::TextDecoration::None,
        offset_top: 0.0,
        offset_left: 0.0,
        grid_template_columns: Vec::new(),
        grid_gap: 0.0,
        content: layout::NodeContent::Element,
        children: vec![layout::StyledNode::text("HMI Control Panel")],
    };
    title_span.children[0].font_size = 18.0;
    
    let mut flex_div = layout::StyledNode::block(vec![title_span, status_span]);
    flex_div.display = layout::Display::Flex;
    flex_div.padding = layout::EdgeSizes { top: 8.0, right: 8.0, bottom: 8.0, left: 8.0 };
    flex_div.margin = layout::EdgeSizes { top: 0.0, right: 0.0, bottom: 10.0, left: 0.0 };
    
    let mut body = layout::StyledNode::block(vec![flex_div]);
    body.padding = layout::EdgeSizes { top: 10.0, right: 10.0, bottom: 10.0, left: 10.0 };
    
    let root = layout::StyledNode::block(vec![body]);
    
    let layout_root = layout::layout_tree(&root, 637.0, 600.0);
    
    fn print_layout(b: &layout::LayoutBox, depth: usize) {
        let indent = "  ".repeat(depth);
        let content_type = match &b.content {
            layout::BoxContent::Block => "Block",
            layout::BoxContent::Flex => "Flex",
            layout::BoxContent::Inline => "Inline",
            layout::BoxContent::Text(s) => "Text",
            layout::BoxContent::Anonymous => "Anon",
            _ => "Other",
        };
        eprintln!("{}[{}] x={:.1} y={:.1} w={:.1} h={:.1} ml={:.1}",
            indent, content_type, b.x, b.y, b.width, b.height, b.margin.left);
        for child in &b.children {
            print_layout(child, depth + 1);
        }
    }
    
    eprintln!("\nLayout tree:");
    print_layout(&layout_root, 0);
    
    // The status span (second child of flex div) should be right-aligned
    let body_box = &layout_root.children[0];
    let flex_box = &body_box.children[0];
    eprintln!("\nFlex container: x={:.1} w={:.1} content_x={:.1}", flex_box.x, flex_box.width, flex_box.content_x());
    for (i, child) in flex_box.children.iter().enumerate() {
        eprintln!("  child[{}]: x={:.1} w={:.1} margin_box_w={:.1} margin.left={:.1}", 
            i, child.x, child.width, child.margin_box_width(), child.margin.left);
    }
}
