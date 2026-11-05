# MiniBrowser

**MiniBrowser** — модульный браузерный движок на Rust для HMI-экранов на слабом
embedded-железе. Один и тот же движок работает в обычном браузере (WASM) и
нативно на Linux-панели через фреймбуфер (проверено на Allwinner T113-S3,
2×Cortex-A7, 112 МБ RAM).

## Структура (v0.8)

```
crates/mb-core      движок: html5ever → DOM → CSS → раскладка → пиксели RGBA.
                    Page: состояние страницы, клики, ввод, скролл, таймеры.
                    Без платформенного кода; JS через трейт JsHost.
crates/mb-qjs       JsHost на QuickJS (rquickjs): настоящий JS для нативных сборок,
                    DOM-клей в dom.js, таймеры срабатывают по времени.
crates/mb-headless  страница → PNG (с кликами и ожиданием таймеров).
crates/mb-fb        фреймбуфер + тач (evdev) для Linux-панелей.
src/                wasm-сборка для браузера (API v0.7 для www/), JS хост-браузера.
www/                демо-страницы и веб-обёртка.
tools/              сравнение с Chrome, работа с панелью по USB-консоли.
```

Раскладка: block, flex и grid считает [Taffy](https://github.com/DioxusLabs/taffy)
(с обрушением отступов, auto-margin, minmax/repeat), инлайн-контент — собственный
код (переносы строк, text-align, базовая линия по CSS 2.1). `MB_LAYOUT=builtin`
включает старую самописную раскладку.

## Сборка и запуск

```bash
cargo test -p mb-core -p mb-qjs                       # тесты движка и JS

cargo build --release -p mb-headless                  # PNG на маке/линуксе
./target/release/mb-headless www/demo/demo8-js-hmi.html out.png \
    --size 800x480 --click 50,146 --wait 2000 --dump  # --dump: дерево боксов и hit-зоны

# Панель (armv7 musl, статический бинарник ~3 МБ). Для armv7 у rquickjs нет
# готовых биндингов — генерируем (нужен libclang).
LIBCLANG_PATH=/Library/Developer/CommandLineTools/usr/lib \
cargo zigbuild --release -p mb-fb --features bindgen --target armv7-unknown-linux-musleabihf

# на панели
TZ=MSK-3 /root/mb-fb /root/page.html          # MB_TRACE=1 — время кадра по фазам
                                               # MB_TOUCH=swap,invx,invy — ориентация тача

wasm-pack build --release --target web --out-dir www-pkg   # браузерная сборка
```

`mb-fb` перечитывает файл страницы при изменении: новую версию достаточно
скопировать поверх.

## Измерение прогресса

`tools/refcompare.py` рендерит каждую страницу `www/demo` в headless Chrome
(с теми же шрифтами DejaVu) и в mb-headless и считает долю «чернильных» пикселей
(не фон ни на одной картинке), совпавших с точностью до 2 px. Отчёт —
`tests/ref/report.md`, картинки — `tests/ref/*.png`, `tools/sbs.py` собирает
лист «Chrome | MiniBrowser».

| | совпадение с Chrome |
|---|---|
| v0.7 (старая раскладка) | 27% |
| v0.8 | 92% (21 страница) |

## На T113 (demo21-panel, 800×480)

| | |
|---|---|
| загрузка страницы | ~240 мс (из них ~200 мс — разбор шрифтов) |
| кадр по таймеру | стили 4 + раскладка 8 + отрисовка 4,5 мс (перерисовываются только изменившиеся прямоугольники), вывод в fb ~1 мс (mmap) |
| тап → обработчик → кадр | 33–36 мс |
| CPU при тике раз в секунду | ~3–5% |
| RAM | ~19 МБ |
| бинарник | ~3,5 МБ (движок, QuickJS, resvg, шрифты с кириллицей) |

## Что поддерживается

- HTML5 (html5ever), quirks mode для страниц без doctype (как в браузерах).
- CSS: inline и `<style>`, селекторы тег/класс/id, потомок, дочерний, списки
  через запятую, специфичность; em/%, отступы UA по спецификации HTML.
- block / flex (direction, wrap, justify, align, grow/shrink/basis, gap) /
  grid (repeat, minmax, fr, span), position relative/absolute,
  min/max-width, box-sizing, рамки по сторонам, border-radius.
- Текст: DejaVu Sans обычный и жирный (урезаны до нужных HMI диапазонов:
  латиница, кириллица, ₽, ₂, ●▲, стрелки, ✓), text-align, line-height.
- Встроенный `<svg>` через resvg (пути, дуги, градиенты, transform; без `<text>`),
  JS меняет его через setAttribute — стрелочные приборы, баки, мнемосхемы.
- `overflow: hidden`, перерисовка только изменившихся областей (сравнение
  деревьев боксов).
- JS: QuickJS (натив) или JS хост-браузера (WASM): getElementById (элементы с id
  видят текст и атрибуты страницы), textContent/innerHTML/style/value,
  classList/className, onclick/addEventListener, setTimeout, setInterval,
  requestAnimationFrame, localStorage, canvas 2D (fillRect/fillText),
  `fetch()` локальных файлов (рядом со страницей и в /tmp).

Демо для панели: `www/demo/demo21-panel.html` (котельная, таймеры),
`demo22-svg-gauges.html` (SVG-приборы), `demo23-dash.html` (дашборд из
`/tmp/dash.json`).

## License

MIT
