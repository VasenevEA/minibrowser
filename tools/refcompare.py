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
FONT = os.path.join(ROOT, "crates", "mb-core", "assets", "DejaVuSans.ttf")
FONT_BOLD = os.path.join(ROOT, "crates", "mb-core", "assets", "DejaVuSans-Bold.ttf")
W, H = map(int, os.environ.get("MB_SIZE", "800x480").split("x"))
TOL = 24  # per-channel difference still counted as equal (antialiasing)
SHIFT = 2  # pixels a feature may be off and still count


def chrome(page, out):
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
                        "--virtual-time-budget=2000", f"--window-size={W},{H}",
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
    avg = sum(s for _, s in rows) / len(rows)
    with open(os.path.join(OUT, "report.md"), "w") as f:
        f.write(f"# MiniBrowser vs Chrome ({W}x{H}, ink pixels matching within {SHIFT}px, tolerance {TOL})\n\n| page | match |\n|---|---|\n")
        for n, s in rows:
            f.write(f"| {n} | {s * 100:.2f}% |\n")
        f.write(f"| **average** | **{avg * 100:.2f}%** |\n")
    print(f"{avg * 100:6.2f}%  average")


if __name__ == "__main__":
    main()
