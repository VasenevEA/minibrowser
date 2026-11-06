# Tests

| what | how to run | needs |
|---|---|---|
| comparison report grouping | `python3 -m unittest discover -s tests -p 'test_refcompare.py'` | numpy, Pillow |
| engine unit tests (`crates/*/src`, ~100) | `cargo test -p mb-core -p mb-qjs` | — |
| WASM integration tests (`wasm_js_engine.rs`, `wasm_new_features.rs`): the browser build's JS bridge in a real browser | `wasm-pack test --headless --firefox` (or `--chrome`) | wasm-pack, a browser |
| end-to-end tests of the WASM demo site (`e2e_test.py`) | `python3 -m http.server 9090 -d www` after a wasm build, then `python3 tests/e2e_test.py` | Playwright (`pip install playwright && playwright install firefox`) |
| rendering vs Chrome (`ref/report.md`) | `python3 tools/refcompare.py [page.html ...]` | Chrome, numpy, Pillow |

`tools/refcompare.py` renders each `www/demo` page in headless Chrome (with the
engine's DejaVu fonts forced, so text metrics match) and in `mb-headless`, and
scores the share of "ink" pixels — not plain background in either image — that
match within 2 px. `ref/report.md` is the last run; the PNGs it writes next to
it (`*.chrome.png`, `*.mb.png`, `*.diff.png`) are not committed. See
[docs/testing.md](../docs/testing.md) for how to read the numbers.
