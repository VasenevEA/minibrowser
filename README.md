# MiniBrowser

**MiniBrowser** — модульный браузерный движок на Rust.

Компилируется в WASM и запускается в обычном браузере как "браузер в браузере".
Позже — нативный билд для embedded Linux панелей (ARM, MIPS).

## Идея

Полноценные браузерные движки (Blink, Gecko) — это миллионы строк кода и сотни MB RAM.
MiniBrowser — модульный движок с feature flags, который компилируется только с тем, что нужно.

Основной target — **WASM**: MiniBrowser работает внутри обычного браузера,
рендерит HTML/CSS/JS в свой `<canvas>`, как независимый движок.

## Режимы работы

```bash
# 1. URL — открыть сайт
minibrowser https://example.com

# 2. Inline HTML — передать HTML+JS одним блоком
minibrowser --html '<div style="color:red">Hello</div><script>alert(1)</script>'

# 3. Dev-сервер — раздать папку и отрендерить
minibrowser --serve ./my-hmi/
```

## Архитектура

```
┌─────────────────────────────────────────────────────┐
│  Хост-браузер (Chrome/Firefox)                      │
│  ┌───────────────────────────────────────────────┐  │
│  │  minibrowser.wasm                             │  │
│  │                                               │  │
│  │  URL / inline HTML                            │  │
│  │       ↓                                       │  │
│  │  fetch (через JS interop)                     │  │
│  │       ↓                                       │  │
│  │  html5ever → DOM tree                         │  │
│  │       ↓                                       │  │
│  │  cssparser → Style resolution                 │  │
│  │       ↓                                       │  │
│  │  Layout engine (block, flex, inline)           │  │
│  │       ↓                                       │  │
│  │  Paint → tiny-skia → pixel buffer             │  │
│  │       ↓                                       │  │
│  │  wasm-bindgen → host <canvas>                 │  │
│  │                                               │  │
│  │  JS engine (host eval) ↔ DOM bindings         │  │
│  │       ↕                                       │  │
│  │  Canvas 2D API → tiny-skia                    │  │
│  └───────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────┘
```

## Текущие возможности (v0.6)

- HTML5 парсинг (html5ever)
- CSS: inline styles, default styles, shorthand expansion
- Layout: block, flex (gap, wrap, align-items), inline text
- Рендер: tiny-skia pixel painting, fontdue text (Latin + Cyrillic)
- JS: onclick handlers, host eval, mock DOM API
- WASM: canvas rendering, URL fetch, CORS proxy

## Разработка

```bash
# WASM билд
wasm-pack build --target web

# Запустить dev-сервер
cd www && python3 server.py  # port 9090

# Тесты
cargo test
```

## License

MIT
