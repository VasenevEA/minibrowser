//! WASM integration tests for the JS engine.
//!
//! These tests run in a real browser (headless Firefox/Chrome) via wasm-pack test,
//! verifying that the JS↔Rust bridge works correctly: mock DOM API interception,
//! script execution, DOM command generation, and state persistence.

use wasm_bindgen_test::*;
use minibrowser::js_engine::{self, JsHost};
use minibrowser::BrowserJs;
use minibrowser::html_parser;

wasm_bindgen_test_configure!(run_in_browser);

// ---------------------------------------------------------------------------
// Script extraction + execution
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn execute_scripts_sets_text() {
    let html = r#"<html><body>
        <div id="msg">original</div>
        <script>document.getElementById('msg').textContent = 'updated';</script>
    </body></html>"#;

    let mut doc = html_parser::parse_html(html);
    let mut js = BrowserJs;
    js.reset();
    let count = js_engine::execute_scripts(&mut doc, &mut js, 0.0);

    assert_eq!(count, 1);
    let msg = doc.root.get_element_by_id("msg").unwrap();
    assert_eq!(msg.inner_text(), "updated");
}

#[wasm_bindgen_test]
fn execute_scripts_sets_style() {
    let html = r#"<html><body>
        <div id="box"></div>
        <script>document.getElementById('box').style.backgroundColor = 'red';</script>
    </body></html>"#;

    let mut doc = html_parser::parse_html(html);
    let mut js = BrowserJs;
    js.reset();
    js_engine::execute_scripts(&mut doc, &mut js, 0.0);

    let box_el = doc.root.get_element_by_id("box").unwrap();
    let style = box_el.element_data().unwrap().attributes.get("style").unwrap();
    assert!(style.contains("background-color:red"));
}

#[wasm_bindgen_test]
fn execute_scripts_registers_onclick() {
    let html = r#"<html><body>
        <button id="btn"></button>
        <script>
            document.getElementById('btn').onclick = function() {
                document.getElementById('btn').textContent = 'clicked';
            };
        </script>
    </body></html>"#;

    let mut doc = html_parser::parse_html(html);
    let mut js = BrowserJs;
    js.reset();
    js_engine::execute_scripts(&mut doc, &mut js, 0.0);

    let btn = doc.root.get_element_by_id("btn").unwrap();
    let onclick = btn.element_data().unwrap().attributes.get("onclick");
    assert!(onclick.is_some(), "onclick attribute should be set by JS");
    assert!(onclick.unwrap().contains("__mb_click_btn"));
}

#[wasm_bindgen_test]
fn execute_scripts_addeventlistener_click() {
    let html = r#"<html><body>
        <div id="target"></div>
        <script>
            document.getElementById('target').addEventListener('click', function() {
                document.getElementById('target').textContent = 'handled';
            });
        </script>
    </body></html>"#;

    let mut doc = html_parser::parse_html(html);
    let mut js = BrowserJs;
    js.reset();
    js_engine::execute_scripts(&mut doc, &mut js, 0.0);

    let target = doc.root.get_element_by_id("target").unwrap();
    let onclick = target.element_data().unwrap().attributes.get("onclick");
    assert!(onclick.is_some(), "addEventListener('click') should set onclick");
}

// ---------------------------------------------------------------------------
// State persistence across calls
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn var_persists_across_execute_onclick() {
    // Initial script sets var counter = 0
    let html = r#"<html><body>
        <div id="count">0</div>
        <script>var counter = 0;</script>
    </body></html>"#;

    let mut doc = html_parser::parse_html(html);
    let mut js = BrowserJs;
    js.reset();
    js_engine::execute_scripts(&mut doc, &mut js, 0.0);

    // Simulate onclick: counter++ and update text
    js_engine::execute_onclick(&mut doc, &mut js,
        "counter++; document.getElementById('count').textContent = '' + counter;", 0.0);

    assert_eq!(doc.root.get_element_by_id("count").unwrap().inner_text(), "1");

    // Click again
    js_engine::execute_onclick(&mut doc, &mut js,
        "counter++; document.getElementById('count').textContent = '' + counter;", 0.0);

    assert_eq!(doc.root.get_element_by_id("count").unwrap().inner_text(), "2");

    // Click again
    js_engine::execute_onclick(&mut doc, &mut js,
        "counter++; document.getElementById('count').textContent = '' + counter;", 0.0);

    assert_eq!(doc.root.get_element_by_id("count").unwrap().inner_text(), "3");
}

#[wasm_bindgen_test]
fn function_persists_across_calls() {
    let html = r#"<html><body>
        <div id="out"></div>
        <script>
            var count = 10;
            function inc() { count++; document.getElementById('out').textContent = '' + count; }
        </script>
    </body></html>"#;

    let mut doc = html_parser::parse_html(html);
    let mut js = BrowserJs;
    js.reset();
    js_engine::execute_scripts(&mut doc, &mut js, 0.0);

    // Call inc() via onclick
    js_engine::execute_onclick(&mut doc, &mut js, "inc()", 0.0);
    assert_eq!(doc.root.get_element_by_id("out").unwrap().inner_text(), "11");

    js_engine::execute_onclick(&mut doc, &mut js, "inc()", 0.0);
    assert_eq!(doc.root.get_element_by_id("out").unwrap().inner_text(), "12");
}

// ---------------------------------------------------------------------------
// onclick handler invocation
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn onclick_handler_callable() {
    let html = r#"<html><body>
        <div id="result">none</div>
        <button id="btn"></button>
        <script>
            document.getElementById('btn').onclick = function() {
                document.getElementById('result').textContent = 'clicked!';
            };
        </script>
    </body></html>"#;

    let mut doc = html_parser::parse_html(html);
    let mut js = BrowserJs;
    js.reset();
    js_engine::execute_scripts(&mut doc, &mut js, 0.0);

    // Get the onclick attribute (should be __mb_click_btn())
    let onclick = doc.root.get_element_by_id("btn").unwrap()
        .element_data().unwrap()
        .attributes.get("onclick").unwrap().clone();

    // Execute it
    js_engine::execute_onclick(&mut doc, &mut js, &onclick, 0.0);

    assert_eq!(doc.root.get_element_by_id("result").unwrap().inner_text(), "clicked!");
}

// ---------------------------------------------------------------------------
// Counter demo simulation (full flow)
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn full_counter_demo_flow() {
    let html = r#"<html><body>
        <div id="count">0</div>
        <button id="inc"></button>
        <button id="dec"></button>
        <button id="reset"></button>
        <script>
            var count = 0;
            function render() {
                document.getElementById('count').textContent = '' + count;
            }
            document.getElementById('inc').onclick = function() { count++; render(); };
            document.getElementById('dec').onclick = function() { count--; render(); };
            document.getElementById('reset').onclick = function() { count = 0; render(); };
            render();
        </script>
    </body></html>"#;

    let mut doc = html_parser::parse_html(html);
    let mut js = BrowserJs;
    js.reset();
    js_engine::execute_scripts(&mut doc, &mut js, 0.0);

    // Initial state
    assert_eq!(doc.root.get_element_by_id("count").unwrap().inner_text(), "0");

    // Get onclick handlers
    let inc_handler = doc.root.get_element_by_id("inc").unwrap()
        .element_data().unwrap().attributes.get("onclick").unwrap().clone();
    let dec_handler = doc.root.get_element_by_id("dec").unwrap()
        .element_data().unwrap().attributes.get("onclick").unwrap().clone();
    let reset_handler = doc.root.get_element_by_id("reset").unwrap()
        .element_data().unwrap().attributes.get("onclick").unwrap().clone();

    // Click + three times
    js_engine::execute_onclick(&mut doc, &mut js, &inc_handler, 0.0);
    assert_eq!(doc.root.get_element_by_id("count").unwrap().inner_text(), "1");

    js_engine::execute_onclick(&mut doc, &mut js, &inc_handler, 0.0);
    assert_eq!(doc.root.get_element_by_id("count").unwrap().inner_text(), "2");

    js_engine::execute_onclick(&mut doc, &mut js, &inc_handler, 0.0);
    assert_eq!(doc.root.get_element_by_id("count").unwrap().inner_text(), "3");

    // Click - once
    js_engine::execute_onclick(&mut doc, &mut js, &dec_handler, 0.0);
    assert_eq!(doc.root.get_element_by_id("count").unwrap().inner_text(), "2");

    // Reset
    js_engine::execute_onclick(&mut doc, &mut js, &reset_handler, 0.0);
    assert_eq!(doc.root.get_element_by_id("count").unwrap().inner_text(), "0");

    // Click - to go negative
    js_engine::execute_onclick(&mut doc, &mut js, &dec_handler, 0.0);
    assert_eq!(doc.root.get_element_by_id("count").unwrap().inner_text(), "-1");
}

// ---------------------------------------------------------------------------
// innerHTML via JS
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn innerhtml_set_via_script() {
    let html = r#"<html><body>
        <div id="container"></div>
        <script>
            document.getElementById('container').innerHTML = '<span>hello</span>';
        </script>
    </body></html>"#;

    let mut doc = html_parser::parse_html(html);
    let mut js = BrowserJs;
    js.reset();
    js_engine::execute_scripts(&mut doc, &mut js, 0.0);

    let text = doc.root.get_element_by_id("container").unwrap().inner_text();
    assert!(text.contains("hello"));
}

// ---------------------------------------------------------------------------
// Timer registration
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn setinterval_executes_immediately() {
    let html = r#"<html><body>
        <div id="clock">-</div>
        <script>
            setInterval(function() {
                document.getElementById('clock').textContent = 'tick';
            }, 1000);
        </script>
    </body></html>"#;

    let mut doc = html_parser::parse_html(html);
    let mut js = BrowserJs;
    js.reset();
    js_engine::execute_scripts(&mut doc, &mut js, 0.0);

    // setInterval callback should have run immediately
    assert_eq!(doc.root.get_element_by_id("clock").unwrap().inner_text(), "tick");
}

#[wasm_bindgen_test]
fn tick_timers_runs_intervals() {
    let html = r#"<html><body>
        <div id="val">0</div>
        <script>
            var n = 0;
            setInterval(function() {
                n++;
                document.getElementById('val').textContent = '' + n;
            }, 1000);
        </script>
    </body></html>"#;

    let mut doc = html_parser::parse_html(html);
    let mut js = BrowserJs;
    js.reset();
    js_engine::execute_scripts(&mut doc, &mut js, 0.0);

    // After initial execution, n=1
    assert_eq!(doc.root.get_element_by_id("val").unwrap().inner_text(), "1");

    // Tick timers (simulates re-render)
    js_engine::tick_timers(&mut doc, &mut js, 0.0);
    assert_eq!(doc.root.get_element_by_id("val").unwrap().inner_text(), "2");

    js_engine::tick_timers(&mut doc, &mut js, 0.0);
    assert_eq!(doc.root.get_element_by_id("val").unwrap().inner_text(), "3");
}

// ---------------------------------------------------------------------------
// Script removal
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn scripts_removed_after_execution() {
    let html = r#"<html><body>
        <div>content</div>
        <script>var x = 1;</script>
    </body></html>"#;

    let mut doc = html_parser::parse_html(html);
    let mut js = BrowserJs;
    js.reset();

    // Before: scripts exist
    assert!(!js_engine::extract_scripts(&doc).is_empty());

    js_engine::execute_scripts(&mut doc, &mut js, 0.0);

    // After: scripts removed
    assert!(js_engine::extract_scripts(&doc).is_empty());
}

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn js_error_does_not_crash() {
    let html = r#"<html><body>
        <div id="ok">original</div>
        <script>
            throw new Error('intentional');
        </script>
    </body></html>"#;

    let mut doc = html_parser::parse_html(html);
    let mut js = BrowserJs;
    js.reset();
    js_engine::execute_scripts(&mut doc, &mut js, 0.0);

    // Should not crash, DOM unchanged
    assert_eq!(doc.root.get_element_by_id("ok").unwrap().inner_text(), "original");
}

#[wasm_bindgen_test]
fn js_error_in_onclick_does_not_crash() {
    let html = r#"<html><body>
        <div id="out">safe</div>
        <script>var x = 1;</script>
    </body></html>"#;

    let mut doc = html_parser::parse_html(html);
    let mut js = BrowserJs;
    js.reset();
    js_engine::execute_scripts(&mut doc, &mut js, 0.0);

    // Execute broken onclick
    js_engine::execute_onclick(&mut doc, &mut js, "undefined_function()", 0.0);

    // Should not crash
    assert_eq!(doc.root.get_element_by_id("out").unwrap().inner_text(), "safe");
}
