# Pixel-Perfect Comparison Guide

A methodology for pixel-by-pixel comparison of widget rendering.
Applies to: MiniBrowser vs a browser, ScreenMaster IDE(SX) vs Slint(Rust), or any two renderers.

## Concept

Two renderers draw the same page/widget. The screenshots are compared pixel by pixel.
Metrics:
- **Exact match %** — pixels match exactly (RGB)
- **Close match %** — pixels differ by no more than the threshold (default 30 per channel)
- **Diff image** — visualization: black = match, yellow = close, red = different

## Architecture

```
┌─────────────────┐    ┌─────────────────┐
│  Renderer A      │    │  Renderer B      │
│  (ScreenMaster)  │    │  (Slint/Rust)    │
│                  │    │                  │
│  Widget → PNG    │    │  Widget → PNG    │
└────────┬─────────┘    └────────┬─────────┘
         │                       │
         └───────┬───────────────┘
                 ▼
        ┌────────────────┐
        │  Pixel Compare  │
        │  Script (Python) │
        │                 │
        │  exact_match %  │
        │  close_match %  │
        │  diff_image.png │
        └─────────────────┘
```

## Quick start

### 1. Installation

```bash
pip3 install Pillow playwright
python3 -m playwright install firefox
```

### 2. Script to compare two images

```python
#!/usr/bin/env python3
"""compare.py — pixel-by-pixel comparison of two screenshots."""

from PIL import Image
import sys

def compare(img_a_path, img_b_path, threshold=30, output_diff=None):
    """
    Compare two images.
    
    Args:
        img_a_path: path to renderer A screenshot
        img_b_path: path to renderer B screenshot
        threshold: per-channel tolerance (0-255). 30 = ~12% tolerance
        output_diff: path for the diff image (optional)
    
    Returns:
        dict with exact_pct, close_pct, total_pixels
    """
    a = Image.open(img_a_path).convert('RGB')
    b = Image.open(img_b_path).convert('RGB')
    
    # Bring to the same size
    w = min(a.width, b.width)
    h = min(a.height, b.height)
    a = a.resize((w, h))
    b = b.resize((w, h))
    
    px_a = list(a.getdata())
    px_b = list(b.getdata())
    total = len(px_a)
    
    exact = 0
    close = 0
    diff_pixels = []
    
    for pa, pb in zip(px_a, px_b):
        dr = abs(pa[0] - pb[0])
        dg = abs(pa[1] - pb[1])
        db = abs(pa[2] - pb[2])
        
        if dr == 0 and dg == 0 and db == 0:
            exact += 1
            diff_pixels.append((0, 0, 0))       # black — perfect
        elif dr <= threshold and dg <= threshold and db <= threshold:
            close += 1
            diff_pixels.append((255, 255, 0))    # yellow — close
        else:
            diff_pixels.append((255, 0, 0))      # red — different
    
    result = {
        'exact_pct': exact / total * 100,
        'close_pct': (exact + close) / total * 100,
        'total_pixels': total,
        'exact_pixels': exact,
        'diff_pixels': total - exact - close,
    }
    
    if output_diff:
        diff_img = Image.new('RGB', (w, h))
        diff_img.putdata(diff_pixels)
        diff_img.save(output_diff)
    
    return result


def create_comparison(img_a_path, img_b_path, output_path, labels=('A', 'B')):
    """Create a side-by-side comparison: A | B | Diff"""
    from PIL import ImageDraw
    
    a = Image.open(img_a_path).convert('RGB')
    b = Image.open(img_b_path).convert('RGB')
    
    w = min(a.width, b.width)
    h = min(a.height, b.height)
    a = a.resize((w, h))
    b = b.resize((w, h))
    
    # Diff
    result = compare(img_a_path, img_b_path, output_diff='/tmp/_diff.png')
    diff = Image.open('/tmp/_diff.png').resize((w, h))
    
    # Combine
    combined = Image.new('RGB', (w * 3 + 20, h + 40), (30, 30, 30))
    draw = ImageDraw.Draw(combined)
    
    draw.text((w//2 - 20, 5), labels[0], fill=(79, 195, 247))
    draw.text((w + 10 + w//2 - 20, 5), labels[1], fill=(129, 199, 132))
    pct = f"Diff ({result['exact_pct']:.1f}%)"
    draw.text((w*2 + 20 + w//2 - 40, 5), pct, fill=(255, 82, 82))
    
    combined.paste(a, (0, 30))
    combined.paste(b, (w + 10, 30))
    combined.paste(diff, (w * 2 + 20, 30))
    combined.save(output_path)
    
    return result


if __name__ == '__main__':
    if len(sys.argv) < 3:
        print("Usage: python compare.py <image_a> <image_b> [diff_output]")
        sys.exit(1)
    
    diff_path = sys.argv[3] if len(sys.argv) > 3 else None
    result = compare(sys.argv[1], sys.argv[2], output_diff=diff_path)
    
    print(f"Exact match:  {result['exact_pct']:.1f}%")
    print(f"Close match:  {result['close_pct']:.1f}%")
    print(f"Total pixels: {result['total_pixels']}")
```

### 3. Usage

```bash
# Simple comparison
python3 compare.py screenshot_a.png screenshot_b.png diff.png

# Side-by-side
python3 -c "
from compare import create_comparison
result = create_comparison('a.png', 'b.png', 'comparison.png', 
                           labels=('ScreenMaster', 'Slint'))
print(f'Match: {result[\"exact_pct\"]:.1f}%')
"
```

## Applying it to ScreenMaster IDE(SX) → Slint(Rust)

### Pipeline

```
1. Create a widget in ScreenMaster IDE (button, indicator, chart...)
2. Export a screenshot of the widget → reference.png
3. Implement the same widget in Slint
4. Take a screenshot of the Slint render → slint.png
5. Run: python3 compare.py reference.png slint.png diff.png
6. Analyze the diff → fix → repeat until 90%+
```

### Automating a set of widgets

```python
#!/usr/bin/env python3
"""batch_compare.py — compare a set of widgets."""

import os
from compare import compare, create_comparison

WIDGETS_DIR = "widgets"  # folder with screenshot pairs

# Layout:
# widgets/
#   button/
#     screenmaster.png
#     slint.png
#   gauge/
#     screenmaster.png
#     slint.png
#   ...

results = []

for widget_name in sorted(os.listdir(WIDGETS_DIR)):
    widget_dir = os.path.join(WIDGETS_DIR, widget_name)
    if not os.path.isdir(widget_dir):
        continue
    
    ref = os.path.join(widget_dir, "screenmaster.png")
    test = os.path.join(widget_dir, "slint.png")
    
    if not os.path.exists(ref) or not os.path.exists(test):
        continue
    
    diff = os.path.join(widget_dir, "diff.png")
    comp = os.path.join(widget_dir, "comparison.png")
    
    result = create_comparison(ref, test, comp,
                               labels=('ScreenMaster', 'Slint'))
    results.append((widget_name, result))
    
    print(f"{widget_name:30s} exact={result['exact_pct']:5.1f}%  "
          f"close={result['close_pct']:5.1f}%")

# Summary
print("\n" + "=" * 60)
avg_exact = sum(r[1]['exact_pct'] for r in results) / len(results)
avg_close = sum(r[1]['close_pct'] for r in results) / len(results)
print(f"AVERAGE: exact={avg_exact:.1f}%  close={avg_close:.1f}%")
print(f"Widgets tested: {len(results)}")

# Worst performers
print("\nNeeds improvement:")
for name, r in sorted(results, key=lambda x: x[1]['exact_pct']):
    if r['exact_pct'] < 80:
        print(f"  {name}: {r['exact_pct']:.1f}%")
```

### Directory layout

```
project/
├── widgets/
│   ├── button_start/
│   │   ├── screenmaster.png    ← screenshot from ScreenMaster
│   │   ├── slint.png           ← screenshot from Slint
│   │   ├── diff.png            ← auto-generated
│   │   └── comparison.png      ← side-by-side
│   ├── gauge_temperature/
│   ├── indicator_alarm/
│   ├── chart_trend/
│   ├── panel_navigation/
│   └── ...
├── compare.py
├── batch_compare.py
└── results.md                  ← report with metrics
```

## What affects pixel match

### High impact (5-20% difference)
- **Font** — different font family/metrics shift the text
- **Element sizes** — padding/margin/border differences
- **Colors** — RGB vs HSL, gamma correction
- **Anti-aliasing** — smoothing of fonts and lines

### Medium impact (2-5%)
- **Line-height** — line spacing
- **Vertical alignment** — baseline vs top vs center
- **Border-radius** — different rounding algorithms
- **Shadows** — box-shadow, text-shadow

### Low impact (<2%)
- **Subpixel rendering** — positioning at fractional pixels
- **Paint order** — z-index, overlay order

## Quality thresholds

| Level | Exact match | Description |
|---------|-------------|----------|
| 🔴 Poor | <60% | Looks completely different |
| 🟡 Basic | 60-75% | Similar structure, details differ |
| 🟢 Good | 75-85% | Close, minor differences |
| ✅ Excellent | 85-95% | Nearly identical |
| 💎 Perfect | >95% | Indistinguishable |

## Tips

1. **Start with simple widgets** — button, label, indicator
2. **Fix the size** — same viewport for both renderers
3. **Use the same font** — this alone gives +10-15%
4. **Iterate fast** — the diff shows exactly where the problem is
5. **Close match matters more than exact** — if close is >90%, it already looks good
6. **Automate in CI** — run the comparison on every commit
