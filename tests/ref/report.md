# MiniBrowser vs Chrome (800x480, ink pixels matching within 2px, tolerance 24)

[How to read this report](../../docs/testing.md). The static snapshot average is the rendering indicator. Dynamic scores are diagnostic only: capture times and external inputs are not synchronized.

Historical scores from the previously committed report, regrouped without rerendering.

| scenario | pages | average match |
|---|---|---|
| Static snapshots | 20 | 91.51% |
| Animations and timers | 5 | 68.65% |
| Wall-clock content | 2 | 90.81% |
| Random content | 1 | 93.03% |
| Network-dependent content | 2 | 35.07% |

## Static snapshots

| page | match | note |
|---|---|---|
| demo1 | 98.14% |  |
| demo10-css | 95.94% |  |
| demo11-canvas | 88.01% |  |
| demo12-images | 75.48% |  |
| demo13-grid | 92.89% |  |
| demo14-position | 90.53% |  |
| demo15-canvas2d | 88.40% |  |
| demo16-landing | 97.98% |  |
| demo17-dashboard | 94.26% |  |
| demo18-article | 95.76% |  |
| demo19-form | 97.04% |  |
| demo2 | 97.53% |  |
| demo20-responsive | 99.11% |  |
| demo27-dom | 89.98% |  |
| demo3 | 86.07% |  |
| demo31-modules | 79.17% |  |
| demo4-js | 85.08% |  |
| demo6-js-counter | 93.09% |  |
| demo7-js-todo | 97.31% |  |
| demo9-interactive | 88.39% |  |

## Animations and timers

| page | match | note |
|---|---|---|
| demo22-svg-gauges | 76.99% | SVG changes every 500 ms |
| demo24-anim | 79.33% | animation frames and current time |
| demo25-canvas | 91.85% | animated canvas |
| demo26-pixels | 0.00% | time-dependent pixel animation and FPS counter |
| demo28-preact | 95.09% | Preact counter changes every second |

## Wall-clock content

| page | match | note |
|---|---|---|
| demo5-js-clock | 88.99% | current date/time and elapsed time |
| demo8-js-hmi | 92.63% | event log includes the current time |

## Random content

| page | match | note |
|---|---|---|
| demo21-panel | 93.03% | random sensor readings, timers and current time |

## Network-dependent content

| page | match | note |
|---|---|---|
| demo29-fetch | 32.59% | live weather over HTTPS |
| demo30-live | 37.54% | local ws_feed WebSocket server and live weather |

Compare the same static page set, viewport and scoring settings between runs. Inspect per-page scores and diff images; an average alone can hide regressions.
