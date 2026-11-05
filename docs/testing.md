# Measuring rendering against Chrome

The engine is checked against a real browser, page by page, with
`tools/refcompare.py`:

1. Chrome (headless, 800×480, the engine's DejaVu fonts forced on every
   element) takes a screenshot of each page in `www/demo`.
2. `mb-headless` renders the same page at the same size.
3. Pixels that are page background in **both** images are ignored (otherwise
   an empty dark page would score high). For the rest — the "ink" — a pixel
   matches when the other image has a pixel within 24 levels per channel
   somewhere within 2 px of it.
4. The score is the share of ink pixels that match, written to
   `tests/ref/report.md`; `tools/sbs.py out.png name ...` makes a
   Chrome | MiniBrowser sheet of chosen pages.

```bash
python3 tools/refcompare.py                       # all of www/demo
python3 tools/refcompare.py www/demo/demo13-grid.html
python3 tools/sbs.py sheet.png demo13-grid demo21-panel
```

## Reading the numbers

Some pages cannot match by design; the report marks them:

- **animated pages** (`demo24-anim`, `demo5-js-clock`, …): Chrome and the
  engine take their screenshot at different moments of the animation;
- **random content** (`demo26-pixels` draws noise every frame): 0% is expected;
- **network pages** (`demo29-fetch`, `demo30-live`): live data from the web or
  from a local WebSocket server (`cargo run -p mb-qjs --features net
  --example ws_feed`) that is not running during the comparison.

For the other pages a low score is a real rendering difference and worth a
look in the `*.diff.png` the tool writes next to the report.

Headless Chrome puts pages without a doctype into quirks mode, as browsers do;
the core engine imitates it, the Blitz engine does not, so compare Blitz on
pages that have `<!DOCTYPE html>`.
