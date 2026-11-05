# Changelog

## 0.8.0 — 2026-11-06

The engine runs natively on embedded Linux panels.

- **Panels:** `mb-fb` kiosk binary — DRM/KMS output with double buffering and
  vsync, fbdev fallback, evdev touch with tslib calibration, page reload on
  file change; a static armv7 musl build of 3.6 MB.
- **Engine:** Taffy flex/grid layout, inline text with baselines, quirks mode,
  damage-rectangle repaint, fast path for SVG-only and canvas-only frames,
  font families (sans / serif / monospace).
- **JavaScript:** QuickJS host; a live DOM for scripts on both engines (nodes,
  events with capture and bubbling, `querySelector`, `classList`, `dataset`,
  `innerHTML`…); ES modules; `fetch`/XHR over http(s) and WebSocket on threads
  (feature `net`); canvas 2D with `putImageData`; Preact 10 runs unmodified;
  Doom (doomgeneric compiled to JavaScript) runs on a canvas.
- **Blitz engine** (feature `blitz`): Stylo CSS, Taffy, Parley text, painted
  by its own tiny-skia backend with paint-command diffing; pages, images,
  stylesheets and web fonts over http(s).
- **Slint:** `mb-slint` — an `HtmlView` element and a Slint platform for panels
  without a window system.
- **Tools:** `mb-headless` (PNG, scripted taps, frame dumps), a pixel
  comparison against Chrome (`tools/refcompare.py`), CI with prebuilt
  binaries for armv7, aarch64 and x86_64.

## 0.7

The first version: the engine in a web page (WASM), block and flex layout,
inline styles, a mock DOM for `onclick` handlers.
