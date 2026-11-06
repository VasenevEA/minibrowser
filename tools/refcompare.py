#!/usr/bin/env python3
"""Compares MiniBrowser with Chrome, page by page: the progress meter.

For every page: Chrome (headless) renders a reference at the viewport size with
the same DejaVu Sans faces MiniBrowser embeds, mb-headless renders ours, and the
score is the share of pixels whose colour differs by less than a small
tolerance. Writes the three images per page and report.md to tests/ref/.

  tools/refcompare.py [page.html ...]       default: www/demo/*.html
Env: MB_SIZE=800x480, CHROME=/path/to/chrome
"""
import glob, os, re, subprocess, sys, tempfile
import numpy as np
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "tests", "ref")
CHROME = os.environ.get("CHROME", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
HEADLESS = os.path.join(ROOT, "target", "release", "mb-headless")
FONT = os.path.join(ROOT, "crates", "mb-core", "assets", "DejaVuSans-hmi.ttf")
FONT_BOLD = os.path.join(ROOT, "crates", "mb-core", "assets", "DejaVuSans-Bold-hmi.ttf")
W, H = map(int, os.environ.get("MB_SIZE", "800x480").split("x"))
TOL = 24  # per-channel difference still counted as equal (antialiasing)
SHIFT = 2  # pixels a feature may be off and still count


def window_extra():
    """Rows headless Chrome takes from --window-size for itself (the viewport is
    shorter than the window): measured once with a page of known height."""
    with tempfile.TemporaryDirectory() as d:
        page, shot = os.path.join(d, "vp.html"), os.path.join(d, "vp.png")
        open(page, "w").write('<!doctype html><body style="margin:0;background:#f00">'
                              '<div style="height:2000px;background:#00f"></div>')
        subprocess.run([CHROME, "--headless=new", "--disable-gpu", "--hide-scrollbars",
                        "--force-device-scale-factor=1", f"--window-size={W},{H}",
                        f"--screenshot={shot}", "file://" + page], capture_output=True, timeout=60, check=True)
        im = Image.open(shot).convert("RGB")
        blue = sum(1 for y in range(im.height) if im.getpixel((W // 2, y)) == (0, 0, 255))
        return im.height - blue


EXTRA = None


def chrome(page, out):
    global EXTRA
    if EXTRA is None:
        EXTRA = window_extra()
    html = open(page, encoding="utf-8").read()
    font = ("<style>@font-face{font-family:MBDejaVu;font-weight:400;src:url('file://%s')}"
            "@font-face{font-family:MBDejaVu;font-weight:700;src:url('file://%s')}"
            "html,body,*{font-family:MBDejaVu !important}</style>" % (FONT, FONT_BOLD))
    # After the doctype, if any: a <style> before it would put Chrome in quirks mode.
    m = re.match(r"\s*<!doctype[^>]*>", html, re.I)
    at = m.end() if m else 0
    with tempfile.NamedTemporaryFile("w", suffix=".html", dir=os.path.dirname(page), delete=False, encoding="utf-8") as f:
        f.write(html[:at] + font + html[at:])
        tmp = f.name
    try:
        subprocess.run([CHROME, "--headless=new", "--disable-gpu", "--hide-scrollbars",
                        "--force-device-scale-factor=1", "--allow-file-access-from-files",
                        "--virtual-time-budget=2000", f"--window-size={W},{H + EXTRA}",
                        f"--screenshot={out}", "file://" + tmp],
                       capture_output=True, timeout=60, check=True)
    finally:
        os.unlink(tmp)


def score(a, b):
    """Share of "ink" pixels (not plain page background in either image) that
    match the other image within SHIFT pixels. Counting all pixels would reward
    an empty page on a dark background; demanding exact positions would score a
    page that is one pixel off as wrong everywhere."""
    A = np.asarray(a, dtype=np.int16)
    B = np.asarray(b, dtype=np.int16)
    colors, counts = np.unique(A.reshape(-1, 3), axis=0, return_counts=True)
    bg = colors[counts.argmax()]
    ink = (np.abs(A - bg).max(2) > TOL) | (np.abs(B - bg).max(2) > TOL)

    def best(X, Y):
        """Per pixel of X: smallest difference to Y within the shift window."""
        h, w = X.shape[:2]
        P = np.pad(Y, ((SHIFT, SHIFT), (SHIFT, SHIFT), (0, 0)), mode="edge")
        out = np.full((h, w), 255, dtype=np.int16)
        for dy in range(2 * SHIFT + 1):
            for dx in range(2 * SHIFT + 1):
                out = np.minimum(out, np.abs(X - P[dy:dy + h, dx:dx + w]).max(2))
        return out

    ok = (best(A, B) <= TOL) & (best(B, A) <= TOL)
    diff = Image.fromarray(((~ok & ink) * 255).astype(np.uint8))
    n = int(ink.sum())
    return (float((ok & ink).sum()) / n if n else 1.0), diff


# Explicit scenario metadata: inspect new demos for timers, wall-clock reads,
# randomness and external data before treating them as comparable snapshots.
GROUPS = {
    "static": "Static snapshots",
    "animation": "Animations and timers",
    "clock": "Wall-clock content",
    "random": "Random content",
    "network": "Network-dependent content",
}
SCENARIOS = {
    "demo5-js-clock": ("clock", "current date/time and elapsed time"),
    "demo8-js-hmi": ("clock", "event log includes the current time"),
    "demo21-panel": ("random", "random sensor readings, timers and current time"),
    "demo22-svg-gauges": ("animation", "SVG changes every 500 ms"),
    "demo24-anim": ("animation", "animation frames and current time"),
    "demo25-canvas": ("animation", "animated canvas"),
    "demo26-pixels": ("animation", "time-dependent pixel animation and FPS counter"),
    "demo28-preact": ("animation", "Preact counter changes every second"),
    "demo29-fetch": ("network", "live weather over HTTPS"),
    "demo30-live": ("network", "local ws_feed WebSocket server and live weather"),
}


def report(rows, provenance=None):
    """Build the report from (page name, score) pairs without running renderers."""
    lines = [
        f"# MiniBrowser vs Chrome ({W}x{H}, ink pixels matching within {SHIFT}px, tolerance {TOL})",
        "",
        "[How to read this report](../../docs/testing.md). "
        "The static snapshot average is the rendering indicator. "
        "Dynamic scores are diagnostic only: capture times and external inputs are not synchronized.",
        "",
    ]
    if provenance:
        lines.extend([provenance, ""])
    lines.extend(["| scenario | pages | average match |", "|---|---|---|"])
    grouped = {key: [] for key in GROUPS}
    for name, value in rows:
        group, note = SCENARIOS.get(name, ("static", ""))
        grouped[group].append((name, value, note))
    for group, title in GROUPS.items():
        entries = grouped[group]
        average = f"{sum(v for _, v, _ in entries) / len(entries) * 100:.2f}%" if entries else "n/a"
        lines.append(f"| {title} | {len(entries)} | {average} |")
    for group, title in GROUPS.items():
        entries = grouped[group]
        if not entries:
            continue
        lines.extend(["", f"## {title}", "", "| page | match | note |", "|---|---|---|"])
        for name, value, note in entries:
            lines.append(f"| {name} | {value * 100:.2f}% | {note} |")
    lines.extend(["", "Compare the same static page set, viewport and scoring settings between runs. "
                  "Inspect per-page scores and diff images; an average alone can hide regressions.", ""])
    return "\n".join(lines)


def main():
    pages = sys.argv[1:] or sorted(glob.glob(os.path.join(ROOT, "www", "demo", "*.html")))
    os.makedirs(OUT, exist_ok=True)
    rows = []
    for page in pages:
        name = os.path.splitext(os.path.basename(page))[0]
        ref, ours, diff = (os.path.join(OUT, f"{name}.{k}.png") for k in ("chrome", "mb", "diff"))
        chrome(page, ref)
        subprocess.run([HEADLESS, page, ours, "--size", f"{W}x{H}"], capture_output=True, check=True)
        a = Image.open(ref).convert("RGB").crop((0, 0, W, H))
        b = Image.open(ours).convert("RGB").crop((0, 0, W, H))
        s, d = score(a, b)
        d.save(diff)
        rows.append((name, s))
        print(f"{s * 100:6.2f}%  {name}", flush=True)
    with open(os.path.join(OUT, "report.md"), "w", encoding="utf-8") as f:
        f.write(report(rows))
    static = [s for n, s in rows if n not in SCENARIOS]
    if static:
        print(f"{sum(static) / len(static) * 100:6.2f}%  static snapshot average ({len(static)} pages)")
    else:
        print("n/a  static snapshot average (0 pages)")


if __name__ == "__main__":
    main()
