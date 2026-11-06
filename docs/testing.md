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

The report separates five scenario groups and shows a count and average for
each (an empty group is `n/a`):

- **Static snapshots**: the primary rendering indicator. JavaScript may build
  the page, but its captured content does not depend on time or external data.
- **Animations and timers**: SVG, canvas, pixel animations and the Preact timer.
  Chrome captures after a 2000 ms virtual-time budget; `mb-headless` captures
  immediately after loading, without `--wait`. Their frames are not synchronized.
- **Wall-clock content**: dates, clocks and timestamped event logs.
- **Random content**: the panel demo generates random sensor values and also
  displays time-dependent content.
- **Network-dependent content**: live weather and the local WebSocket feed
  (`cargo run -p mb-qjs --features net --example ws_feed`). Results depend on
  connectivity, response timing and server availability.

Dynamic group scores are diagnostic only. A low score there does not establish
a rendering defect; a high one does not establish correct animation or network
behavior. `demo26-pixels` is a time-dependent animation, not random noise, and
no particular score is expected for it.

There is deliberately no combined average across all groups. For regression
checks, compare the **same static page set**, viewport and tolerance settings
between runs, then inspect each page's score and `*.diff.png`: an average can
hide one page getting worse while another improves. Static scores can still
reflect JavaScript compatibility or resource-loading problems, not just painting.

Scenario membership is explicit in `SCENARIOS` in `tools/refcompare.py`; pages
not listed there default to static snapshots. When adding a demo, inspect its
scripts and resources for timers, wall-clock reads, randomness and external data
and add metadata if needed. For mixed scenarios, choose the main source of
variation and describe the others in the note. Low scores alone must never be
used to classify a page as dynamic.

Report-format tests run without Chrome or a Rust build (numpy and Pillow are
still required):

```bash
python3 -m unittest discover -s tests -p 'test_refcompare.py'
```

Headless Chrome puts pages without a doctype into quirks mode, as browsers do;
the core engine imitates it, the Blitz engine does not, so compare Blitz on
pages that have `<!DOCTYPE html>`.
