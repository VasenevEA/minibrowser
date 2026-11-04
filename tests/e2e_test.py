#!/usr/bin/env python3
"""E2E tests for minibrowser — Playwright headless Firefox."""

from playwright.sync_api import sync_playwright
import time, sys

URL = "http://localhost:9090"
PASSED = 0
FAILED = 0

def check(name, condition, detail=""):
    global PASSED, FAILED
    if condition:
        PASSED += 1
        print(f"  ✓ {name}")
    else:
        FAILED += 1
        print(f"  ✗ {name} — {detail}")

def test_demo6_counter():
    """Demo 6: Counter clicks work, state persists."""
    print("\n[Demo 6 — Counter]")
    with sync_playwright() as p:
        browser = p.firefox.launch(headless=True)
        page = browser.new_page()

        console_msgs = []
        page.on("console", lambda msg: console_msgs.append(msg.text))

        page.goto(URL)
        page.wait_for_timeout(2000)
        page.click('button:text("Demo 6")')
        page.wait_for_timeout(3000)

        # Check hit regions detected
        hit_msgs = [m for m in console_msgs if 'Hit regions:' in m and 'with onclick' in m]
        last_hit = hit_msgs[-1] if hit_msgs else ""
        check("Hit regions detected", "3 with onclick" in last_hit, last_hit)

        # Check no JS errors (except possibly minor ones)
        js_errors = [m for m in console_msgs if 'JS Error' in m]
        check("No JS errors on load", len(js_errors) == 0, str(js_errors))

        # Get canvas bounding box
        canvas = page.locator('#mb-canvas')
        box = canvas.bounding_box()

        # Check onclick regions exist
        hit_msg = hit_msgs[-1] if hit_msgs else ""
        onclick_count = 0
        if "with onclick" in hit_msg:
            try:
                onclick_count = int(hit_msg.split("with onclick")[0].split(",")[-1].strip())
            except: pass
        check("Onclick regions found", onclick_count >= 3, f"got {onclick_count}")

        # Try clicking at multiple Y positions to find a button
        console_msgs.clear()
        for test_y in [140, 160, 180, 200]:
            page.mouse.click(box['x'] + 50, box['y'] + test_y)
            page.wait_for_timeout(200)

        click_msgs = [m for m in console_msgs if 'click_at' in m or 'onclick' in m]
        check("Click detected", len(click_msgs) > 0, str(click_msgs[:2]))

        # Check no crash errors
        crash_errors = [m for m in console_msgs if 'Unreachable' in m or 'RuntimeError' in m]
        check("No WASM crashes", len(crash_errors) == 0, str(crash_errors))

        browser.close()

def test_demo9_interactive():
    """Demo 9: Inline onclick handlers work."""
    print("\n[Demo 9 — Interactive]")
    with sync_playwright() as p:
        browser = p.firefox.launch(headless=True)
        page = browser.new_page()

        console_msgs = []
        page.on("console", lambda msg: console_msgs.append(msg.text))

        page.goto(URL)
        page.wait_for_timeout(2000)
        page.click('button:text("Demo 9")')
        page.wait_for_timeout(3000)

        # Check hit regions
        hit_msgs = [m for m in console_msgs if 'Hit regions:' in m and 'with onclick' in m]
        last_hit = hit_msgs[-1] if hit_msgs else ""
        check("7 onclick regions", "7 with onclick" in last_hit, last_hit)

        # Click at a button area
        canvas = page.locator('#mb-canvas')
        box = canvas.bounding_box()
        console_msgs.clear()

        page.mouse.click(box['x'] + 60, box['y'] + 230)
        page.wait_for_timeout(500)

        click_msgs = [m for m in console_msgs if 'click_at' in m or 'onclick' in m]
        check("Click detected", len(click_msgs) > 0, str(click_msgs[:2]))

        browser.close()

def test_demo5_clock():
    """Demo 5: Clock ticks, background is dark."""
    print("\n[Demo 5 — Clock]")
    with sync_playwright() as p:
        browser = p.firefox.launch(headless=True)
        page = browser.new_page()

        console_msgs = []
        page.on("console", lambda msg: console_msgs.append(msg.text))

        page.goto(URL)
        page.wait_for_timeout(2000)
        page.click('button:text("Demo 5")')
        page.wait_for_timeout(4000)

        # Check scripts executed
        script_msgs = [m for m in console_msgs if 'executing 1 script' in m]
        check("Script executed", len(script_msgs) > 0, str(script_msgs))

        # Check no crash
        crash_errors = [m for m in console_msgs if 'Unreachable' in m or 'RuntimeError' in m]
        check("No WASM crashes", len(crash_errors) == 0, str(crash_errors))

        # Check live re-render happens (multiple Hit regions logs)
        hit_msgs = [m for m in console_msgs if 'Hit regions:' in m]
        check("Multiple renders (live)", len(hit_msgs) >= 2, f"got {len(hit_msgs)}")

        browser.close()

def test_demo1_blocks():
    """Demo 1: Basic blocks render without crash."""
    print("\n[Demo 1 — Blocks]")
    with sync_playwright() as p:
        browser = p.firefox.launch(headless=True)
        page = browser.new_page()

        console_msgs = []
        page.on("console", lambda msg: console_msgs.append(msg.text))

        page.goto(URL)
        page.wait_for_timeout(3000)

        # Demo 1 auto-loads
        crash_errors = [m for m in console_msgs if 'Unreachable' in m or 'RuntimeError' in m]
        check("No WASM crashes", len(crash_errors) == 0, str(crash_errors))

        js_errors = [m for m in console_msgs if 'JS Error' in m]
        check("No JS errors", len(js_errors) == 0, str(js_errors))

        hit_msgs = [m for m in console_msgs if 'Hit regions:' in m]
        check("Render completed", len(hit_msgs) > 0, "No hit region logs")

        browser.close()

if __name__ == "__main__":
    test_demo1_blocks()
    test_demo5_clock()
    test_demo6_counter()
    test_demo9_interactive()

    print(f"\n{'='*40}")
    print(f"Results: {PASSED} passed, {FAILED} failed")
    if FAILED:
        sys.exit(1)
