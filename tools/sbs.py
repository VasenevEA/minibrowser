#!/usr/bin/env python3
"""Side-by-side sheet of tests/ref results: Chrome | MiniBrowser, one row per page.
  tools/sbs.py out.png name [name ...]   (names as in tests/ref/<name>.chrome.png)"""
import os, sys
from PIL import Image, ImageDraw
REF = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "tests", "ref")
rows = []
for n in sys.argv[2:]:
    a = Image.open(f"{REF}/{n}.chrome.png").convert("RGB")
    b = Image.open(f"{REF}/{n}.mb.png").convert("RGB")
    r = Image.new("RGB", (a.width + b.width + 6, max(a.height, b.height)), (255, 0, 255))
    r.paste(a, (0, 0)); r.paste(b, (a.width + 6, 0))
    rows.append(r)
sheet = Image.new("RGB", (max(r.width for r in rows), sum(r.height + 6 for r in rows)), (255, 0, 255))
y = 0
for r in rows:
    sheet.paste(r, (0, y)); y += r.height + 6
scale = float(os.environ.get("SCALE", "0.75"))
sheet.resize((int(sheet.width * scale), int(sheet.height * scale))).save(sys.argv[1])
