#!/usr/bin/env python3
"""Compares MiniBrowser with Chrome, page by page: the progress meter.

For every page: Chrome (headless) renders a reference at the viewport size with
the same DejaVu Sans font MiniBrowser embeds, mb-headless renders ours, and the
score is the share of pixels whose colour differs by less than a small
tolerance. Writes the three images per page and report.md to tests/ref/.

  tools/refcompare.py [page.html ...]       default: www/demo/*.html
Env: MB_SIZE=800x480, CHROME=/path/to/chrome
"""
import glob, os, subprocess, sys, tempfile
from PIL import Image, ImageChops

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "tests", "ref")
CHROME = os.environ.get("CHROME", "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
HEADLESS = os.path.join(ROOT, "target", "release", "mb-headless")
FONT = os.path.join(ROOT, "crates", "mb-core", "assets", "DejaVuSans.ttf")
W, H = map(int, os.environ.get("MB_SIZE", "800x480").split("x"))
TOL = 24  # per-channel difference still counted as equal (antialiasing)


def chrome(page, out):
    html = open(page, encoding="utf-8").read()
    font = ("<style>@font-face{font-family:MBDejaVu;src:url('file://%s')}"
            "html,body,*{font-family:MBDejaVu !important}</style>" % FONT)
    with tempfile.NamedTemporaryFile("w", suffix=".html", dir=os.path.dirname(page), delete=False, encoding="utf-8") as f:
        f.write(font + html)
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
    d = ImageChops.difference(a, b).convert("RGB")
    px = d.getdata()
    same = sum(1 for p in px if max(p) <= TOL)
    return same / (a.width * a.height), d


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
        d.point(lambda v: 255 if v > TOL else 0).save(diff)
        rows.append((name, s))
        print(f"{s * 100:6.2f}%  {name}", flush=True)
    avg = sum(s for _, s in rows) / len(rows)
    with open(os.path.join(OUT, "report.md"), "w") as f:
        f.write(f"# MiniBrowser vs Chrome ({W}x{H}, tolerance {TOL})\n\n| page | match |\n|---|---|\n")
        for n, s in rows:
            f.write(f"| {n} | {s * 100:.2f}% |\n")
        f.write(f"| **average** | **{avg * 100:.2f}%** |\n")
    print(f"{avg * 100:6.2f}%  average")


if __name__ == "__main__":
    main()
