# Pixel-Perfect Comparison Guide

Методология попиксельного сравнения рендеринга виджетов.
Применимо к: MiniBrowser vs Browser, ScreenMaster IDE(SX) vs Slint(Rust), любые два рендерера.

## Концепция

Два рендерера рисуют одну и ту же страницу/виджет. Скриншоты сравниваются попиксельно.
Метрики:
- **Exact match %** — пиксели совпадают полностью (RGB)
- **Close match %** — пиксели отличаются не более чем на threshold (default 30 на канал)
- **Diff image** — визуализация: чёрный=совпадает, жёлтый=близко, красный=разница

## Архитектура

```
┌─────────────────┐    ┌─────────────────┐
│  Renderer A      │    │  Renderer B      │
│  (ScreenMaster)  │    │  (Slint/Rust)    │
│                  │    │                  │
│  Виджет → PNG    │    │  Виджет → PNG    │
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

## Быстрый старт

### 1. Установка

```bash
pip3 install Pillow playwright
python3 -m playwright install firefox
```

### 2. Скрипт сравнения двух изображений

```python
#!/usr/bin/env python3
"""compare.py — сравнение двух скриншотов попиксельно."""

from PIL import Image
import sys

def compare(img_a_path, img_b_path, threshold=30, output_diff=None):
    """
    Сравнить два изображения.
    
    Args:
        img_a_path: путь к скриншоту рендерера A
        img_b_path: путь к скриншоту рендерера B
        threshold: допуск на канал (0-255). 30 = ~12% допуск
        output_diff: путь для diff-изображения (опционально)
    
    Returns:
        dict с exact_pct, close_pct, total_pixels
    """
    a = Image.open(img_a_path).convert('RGB')
    b = Image.open(img_b_path).convert('RGB')
    
    # Привести к одному размеру
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
            diff_pixels.append((0, 0, 0))       # чёрный — идеально
        elif dr <= threshold and dg <= threshold and db <= threshold:
            close += 1
            diff_pixels.append((255, 255, 0))    # жёлтый — близко
        else:
            diff_pixels.append((255, 0, 0))      # красный — разница
    
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
    """Создать side-by-side сравнение: A | B | Diff"""
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

### 3. Использование

```bash
# Простое сравнение
python3 compare.py screenshot_a.png screenshot_b.png diff.png

# Side-by-side
python3 -c "
from compare import create_comparison
result = create_comparison('a.png', 'b.png', 'comparison.png', 
                           labels=('ScreenMaster', 'Slint'))
print(f'Match: {result[\"exact_pct\"]:.1f}%')
"
```

## Применение для ScreenMaster IDE(SX) → Slint(Rust)

### Пайплайн

```
1. В ScreenMaster IDE создать виджет (кнопка, индикатор, график...)
2. Экспортировать скриншот виджета → reference.png
3. Реализовать тот же виджет в Slint
4. Сделать скриншот Slint рендера → slint.png
5. Запустить: python3 compare.py reference.png slint.png diff.png
6. Анализировать diff → исправить → повторить до 90%+
```

### Автоматизация для набора виджетов

```python
#!/usr/bin/env python3
"""batch_compare.py — сравнение набора виджетов."""

import os
from compare import compare, create_comparison

WIDGETS_DIR = "widgets"  # папка с парами скриншотов

# Структура:
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
print("\nНужно улучшить:")
for name, r in sorted(results, key=lambda x: x[1]['exact_pct']):
    if r['exact_pct'] < 80:
        print(f"  {name}: {r['exact_pct']:.1f}%")
```

### Структура директории

```
project/
├── widgets/
│   ├── button_start/
│   │   ├── screenmaster.png    ← скриншот из ScreenMaster
│   │   ├── slint.png           ← скриншот из Slint
│   │   ├── diff.png            ← автогенерация
│   │   └── comparison.png      ← side-by-side
│   ├── gauge_temperature/
│   ├── indicator_alarm/
│   ├── chart_trend/
│   ├── panel_navigation/
│   └── ...
├── compare.py
├── batch_compare.py
└── results.md                  ← отчёт с метриками
```

## Что влияет на pixel match

### Высокий impact (5-20% разницы)
- **Шрифт** — разные font family/metrics дают сдвиг текста
- **Размеры элементов** — padding/margin/border отличия
- **Цвета** — RGB vs HSL, гамма-коррекция
- **Anti-aliasing** — сглаживание шрифтов и линий

### Средний impact (2-5%)
- **Line-height** — межстрочный интервал
- **Вертикальное выравнивание** — baseline vs top vs center
- **Border-radius** — разные алгоритмы скругления
- **Тени** — box-shadow, text-shadow

### Низкий impact (<2%)
- **Subpixel rendering** — позиционирование с дробными пикселями
- **Порядок рисования** — z-index, порядок overlay

## Пороги качества

| Уровень | Exact match | Описание |
|---------|-------------|----------|
| 🔴 Плохо | <60% | Совсем другой вид |
| 🟡 Базовый | 60-75% | Структура похожа, детали отличаются |
| 🟢 Хорошо | 75-85% | Близко, мелкие различия |
| ✅ Отлично | 85-95% | Почти идентично |
| 💎 Идеально | >95% | Неотличимо |

## Советы

1. **Начинай с простых виджетов** — кнопка, метка, индикатор
2. **Фиксируй размер** — одинаковый viewport для обоих рендереров
3. **Используй одинаковый шрифт** — это даёт +10-15% сразу
4. **Итерируй быстро** — diff показывает конкретно где проблема
5. **Close match важнее exact** — если close >90%, визуально уже хорошо
6. **Автоматизируй CI** — запускай сравнение на каждый коммит
