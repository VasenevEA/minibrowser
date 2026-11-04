#!/usr/bin/env python3
"""Pixel-perfect comparison: minibrowser canvas vs browser iframe reference.

For each demo page:
1. Capture the minibrowser canvas rendering
2. Capture the iframe reference rendering
3. Compare pixel-by-pixel
4. Generate diff image highlighting differences
5. Report similarity percentage
"""

from playwright.sync_api import sync_playwright
from PIL import Image, ImageChops, ImageDraw
import os, sys, time

URL = "http://localhost:9090"
OUT_DIR = "/home/mrv/dev/minibrowser/tests/pixel_results"

DEMOS = [
    ("Demo 1", "demo/demo1.html", "Demo 1 — Blocks"),
    ("Demo 2", "demo/demo2.html", "Demo 2 — Typography"),
    ("Demo 3", "demo/demo3.html", "Demo 3 — HMI Dashboard"),
    ("Demo 5", "demo/demo5-js-clock.html", "Demo 5 — Clock"),
    ("Demo 6", "demo/demo6-js-counter.html", "Demo 6 — Counter"),
    ("Demo 7", "demo/demo7-js-todo.html", "Demo 7 — Todo"),
    ("Demo 8", "demo/demo8-js-hmi.html", "Demo 8 — HMI Panel"),
    ("Demo 9", "demo/demo9-interactive.html", "Demo 9 — Interactive"),
    ("Demo 10", "demo/demo10-css.html", "Demo 10 — CSS"),
    ("Demo 13", "demo/demo13-grid.html", "Demo 13 — Grid"),
    ("Demo 14", "demo/demo14-position.html", "Demo 14 — Position"),
]


def capture_panes(page, demo_name, button_text):
    """Click a demo button, wait for render, capture both panes."""
    page.click(f'button:text("{button_text}")')
    time.sleep(3)

    # Get canvas (minibrowser) bounding box
    canvas = page.locator('#mb-canvas')
    cbox = canvas.bounding_box()

    # Get iframe bounding box
    iframe = page.locator('#ref-iframe')
    ibox = iframe.bounding_box()

    # Take full page screenshot
    page.screenshot(path=f"{OUT_DIR}/_full_{demo_name}.png")

    # Crop canvas area
    full = Image.open(f"{OUT_DIR}/_full_{demo_name}.png")
    scale = full.width / page.viewport_size['width']  # handle device pixel ratio

    canvas_img = full.crop((
        int(cbox['x'] * scale),
        int(cbox['y'] * scale),
        int((cbox['x'] + cbox['width']) * scale),
        int((cbox['y'] + cbox['height']) * scale),
    ))

    iframe_img = full.crop((
        int(ibox['x'] * scale),
        int(ibox['y'] * scale),
        int((ibox['x'] + ibox['width']) * scale),
        int((ibox['y'] + ibox['height']) * scale),
    ))

    return canvas_img, iframe_img


def compare_images(canvas_img, iframe_img, name):
    """Compare two images pixel-by-pixel, generate diff, return similarity %."""
    # Resize to same dimensions (use smaller)
    w = min(canvas_img.width, iframe_img.width)
    h = min(canvas_img.height, iframe_img.height)

    c = canvas_img.resize((w, h)).convert('RGB')
    r = iframe_img.resize((w, h)).convert('RGB')

    c.save(f"{OUT_DIR}/{name}_canvas.png")
    r.save(f"{OUT_DIR}/{name}_reference.png")

    # Pixel comparison
    c_pixels = list(c.getdata())
    r_pixels = list(r.getdata())

    total = len(c_pixels)
    exact_match = 0
    close_match = 0  # within threshold
    threshold = 30  # per-channel tolerance

    diff_img = Image.new('RGB', (w, h))
    diff_pixels = []

    for i, (cp, rp) in enumerate(zip(c_pixels, r_pixels)):
        dr = abs(cp[0] - rp[0])
        dg = abs(cp[1] - rp[1])
        db = abs(cp[2] - rp[2])

        if dr == 0 and dg == 0 and db == 0:
            exact_match += 1
            diff_pixels.append((0, 0, 0))  # black = identical
        elif dr <= threshold and dg <= threshold and db <= threshold:
            close_match += 1
            diff_pixels.append((255, 255, 0))  # yellow = close
        else:
            diff_pixels.append((255, 0, 0))  # red = different

    diff_img.putdata(diff_pixels)
    diff_img.save(f"{OUT_DIR}/{name}_diff.png")

    exact_pct = exact_match / total * 100
    close_pct = (exact_match + close_match) / total * 100

    return exact_pct, close_pct, total


def create_side_by_side(canvas_img, iframe_img, diff_img, name):
    """Create a side-by-side comparison image."""
    w = min(canvas_img.width, iframe_img.width)
    h = min(canvas_img.height, iframe_img.height)

    c = canvas_img.resize((w, h)).convert('RGB')
    r = iframe_img.resize((w, h)).convert('RGB')
    d = Image.open(f"{OUT_DIR}/{name}_diff.png")

    combined = Image.new('RGB', (w * 3 + 20, h + 40), (30, 30, 30))
    draw = ImageDraw.Draw(combined)

    # Labels
    draw.text((w // 2 - 40, 5), "MiniBrowser", fill=(79, 195, 247))
    draw.text((w + 10 + w // 2 - 30, 5), "Reference", fill=(129, 199, 132))
    draw.text((w * 2 + 20 + w // 2 - 15, 5), "Diff", fill=(255, 82, 82))

    combined.paste(c, (0, 30))
    combined.paste(r, (w + 10, 30))
    combined.paste(d, (w * 2 + 20, 30))

    combined.save(f"{OUT_DIR}/{name}_comparison.png")


def main():
    os.makedirs(OUT_DIR, exist_ok=True)

    results = []

    with sync_playwright() as p:
        browser = p.firefox.launch(headless=True)
        page = browser.new_page(viewport={'width': 1280, 'height': 800})

        page.goto(URL)
        page.wait_for_timeout(3000)

        for demo_name, demo_url, button_text in DEMOS:
            safe_name = demo_name.lower().replace(' ', '_')
            print(f"\n--- {demo_name} ---")

            try:
                canvas_img, iframe_img = capture_panes(page, safe_name, button_text)
                exact_pct, close_pct, total = compare_images(canvas_img, iframe_img, safe_name)
                create_side_by_side(canvas_img, iframe_img, None, safe_name)

                print(f"  Exact match:  {exact_pct:.1f}%")
                print(f"  Close match:  {close_pct:.1f}% (threshold=30)")
                print(f"  Total pixels: {total}")

                results.append((demo_name, exact_pct, close_pct))
            except Exception as e:
                print(f"  ERROR: {e}")
                results.append((demo_name, 0, 0))

        browser.close()

    # Summary
    print("\n" + "=" * 60)
    print(f"{'Demo':<25} {'Exact %':>10} {'Close %':>10}")
    print("-" * 60)
    for name, exact, close in results:
        print(f"{name:<25} {exact:>9.1f}% {close:>9.1f}%")

    avg_exact = sum(r[1] for r in results) / len(results)
    avg_close = sum(r[2] for r in results) / len(results)
    print("-" * 60)
    print(f"{'AVERAGE':<25} {avg_exact:>9.1f}% {avg_close:>9.1f}%")


if __name__ == "__main__":
    main()
