#!/bin/sh
# Cuts the embedded DejaVu fonts down to the scripts an HMI page uses (Latin,
# Greek, Cyrillic, punctuation, sub/superscripts, currency incl. ₽, arrows, math,
# box drawing, shapes, symbols, dingbats). fontdue keeps every glyph's outline in
# memory, so the full 6k-glyph fonts cost ~30 MB of RAM and ~0.5 s to load on a T113.
# Needs fonttools (pip install fonttools).
set -e
cd "$(dirname "$0")/../crates/mb-core/assets"
RANGES="U+0020-024F,U+0300-036F,U+0370-03FF,U+0400-052F,U+2000-206F,U+2070-209F,U+20A0-20CF,U+2100-214F,U+2190-21FF,U+2200-22FF,U+2300-23FF,U+2460-24FF,U+2500-25FF,U+2600-26FF,U+2700-27BF,U+FFFD"
for f in DejaVuSans DejaVuSans-Bold DejaVuSansMono; do
  python3 -m fontTools.subset "$f.ttf" --unicodes="$RANGES" --layout-features='*' --output-file="$f-hmi.ttf"
done
ls -la *-hmi.ttf
